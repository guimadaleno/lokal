//! The conversation: the messages so far, one turn at a time, and the copy
//! on disk that `lokal -r` resumes.

use std::fs;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::chat::{self, Event, Reply};
use crate::context::{self, CHARS_PER_TOKEN, is_object, kept_arguments};
use crate::input::Input;
use crate::markdown::Markdown;
use crate::memory::{self, Note};
use crate::peers::Peers;
use crate::progress::{ToolProgress, speed, working_label};
use crate::server::Server;
use crate::spinner::Spinner;
use crate::system::Facts;
use crate::tools::{self, Outcome};
use crate::{config, interrupt, judge, keys, layout, mode, prompt, reasoning, style, text, tty};

/// The project's notes go along with every request, so they get an eighth
/// of the context; this much when its size isn't known, an eighth of 32K.
const NOTES_SHARE: u64 = 8;
const NOTES_ROOM_UNKNOWN: usize = 4096 * CHARS_PER_TOKEN;
const STOPPED_RESULT: &str = "Stopped by the user.";
const SKIPPED_AFTER_NO: &str = "Not run: the user declined an earlier step.";
const NO_ANSWER_RESULT: &str = "(No answer: the reply stopped before it began.)";
const ASK_AGAIN_NOTE: &str = "(From Lokal, not the user: your last reply ran out of room while \
    still reasoning, before any answer or step. Reason briefly this time, then answer or take the next step.)";
const CUT_SHORT_RESULT: &str = "Not run: your reply ran out of room before this step was complete. \
    Reason briefly this time, then take it again. A file too long for one reply goes in parts: \
    write_file with its start, then edit_file to add the rest.";

pub struct Conversation {
    pub server: Server,
    facts: Facts,
    /// What the last /recon found, for this session.
    recon: Vec<(&'static str, String)>,
    /// LOKAL.md and AGENTS.md as last read: at the start of each turn, and
    /// after /memory.
    notes: Vec<Note>,
    system: String,
    /// Everything after the system prompt, in the chat API's format.
    messages: Vec<Value>,
    /// Tokens in use after the last reply.
    tokens: Option<u64>,
    /// How long the model worked on the last turn, not counting commands
    /// running or the person deciding on an approval.
    model_time: Option<Duration>,
    /// Tokens a second the model generated on the last turn.
    speed: Option<u64>,
    /// `/reasoning`: the model's reasoning on screen, faint, for this
    /// session. Off until asked for.
    show_reasoning: bool,
    /// The person's other sessions, in a conversation at a terminal.
    peers: Option<Peers>,
    /// The commands the person ran at the `>` since their last request,
    /// framed for the model: they go along with the next one.
    own: Vec<String>,
    /// The person's last requests, as they typed them, for auto's judge.
    asked: Vec<String>,
    /// The files written this turn, for auto's judge.
    written: Vec<String>,
    /// What the tools' definitions take in a request, in characters: the
    /// same all session.
    tools_size: usize,
}

enum Failure {
    Stopped,
    Error(String),
}

/// How a turn ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Turn {
    /// The model answered.
    Answered,
    /// The person stopped the turn, or said no to a step.
    SteppedIn,
    /// No answer came: the server failed, or the reply ran out of room.
    Failed,
}

impl Conversation {
    pub fn new(server: Server, facts: Facts, peers: Option<Peers>) -> Self {
        let tools_size = tools::definitions(peers.is_some()).to_string().len();
        let mut conversation = Conversation {
            server,
            facts,
            recon: Vec::new(),
            notes: Vec::new(),
            system: String::new(),
            messages: Vec::new(),
            tokens: None,
            model_time: None,
            speed: None,
            show_reasoning: false,
            peers,
            own: Vec::new(),
            asked: Vec::new(),
            written: Vec::new(),
            tools_size,
        };
        conversation.notes = conversation.read_notes();
        conversation.refresh_system_prompt();
        conversation
    }

    pub fn scratch(&self) -> Option<&str> {
        self.facts.scratch.as_deref()
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    pub fn peers(&self) -> Option<&Peers> {
        self.peers.as_ref()
    }

    /// The system prompt as the next request sends it, the names of the
    /// tools that go with it, and what the two cost: what the model is told
    /// before the conversation.
    pub fn system_prompt(&self) -> (&str, Vec<String>, u64) {
        let names = tools::definitions(self.peers.is_some())
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|tool| tool["function"]["name"].as_str().map(String::from))
            .collect();
        (&self.system, names, context::estimate(self.fixed(), &[]))
    }

    /// Picks user.md up again after it changed.
    pub fn refresh_system_prompt(&mut self) {
        self.system = prompt::system_prompt(
            &self.facts,
            self.peers.as_ref().map(Peers::name),
            &self.recon,
            self.server.context,
            &self.notes,
            config::read_user_md().as_deref(),
        );
    }

    /// What /recon found goes along from the next request, in place of
    /// what it found before.
    pub fn set_recon(&mut self, rows: Vec<(&'static str, String)>) {
        self.recon = rows;
        self.refresh_system_prompt();
    }

    /// Reads the project's notes again; true when they changed, and the
    /// model gets the new ones from the next request.
    pub fn refresh_notes(&mut self) -> bool {
        let notes = self.read_notes();
        if notes == self.notes {
            return false;
        }
        self.notes = notes;
        self.refresh_system_prompt();
        true
    }

    fn read_notes(&self) -> Vec<Note> {
        let room = self.server.context.map_or(NOTES_ROOM_UNKNOWN, |context| {
            (context / NOTES_SHARE) as usize * CHARS_PER_TOKEN
        });
        memory::load(Path::new(&self.facts.cwd), room)
    }

    /// Turns the reasoning on screen on or off; true when it's on now.
    pub fn toggle_reasoning(&mut self) -> bool {
        self.show_reasoning = !self.show_reasoning;
        self.show_reasoning
    }

    /// Tokens in use after the last reply, once the server has said.
    pub fn tokens(&self) -> Option<u64> {
        self.tokens
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.own.clear();
        self.asked.clear();
        self.tokens = None;
        self.model_time = None;
        self.speed = None;
    }

    /// `!command` at the `>`: the person's own command runs, and what it
    /// gave goes to the model with their next request, cut to the room left
    /// with what's already waiting to go.
    pub fn run_own(&mut self, command: &str) {
        let waiting: usize = self.own.iter().map(String::len).sum();
        let room = self
            .room_for_output()
            .map(|room| room.saturating_sub(waiting));
        if let Some(peers) = &self.peers {
            peers.set_busy(true);
        }
        if let Some(result) = tools::run_own(command, room) {
            let result = result.trim_end();
            self.own.push(context::own_command(command, result));
        }
        if let Some(peers) = &self.peers {
            peers.set_busy(false);
        }
    }

    /// The model's time on the last turn, how full the context is and how
    /// fast the model generated, like "38s · 12% · 24 tok/s"; each part once
    /// it's known.
    pub fn status(&self) -> Option<String> {
        let time = self.model_time.map(text::duration);
        let context = self
            .tokens
            .zip(self.server.context)
            .map(|(tokens, context)| format!("{}%", (tokens * 100 / context.max(1)).min(100)));
        let speed = self.speed.map(text::speed);
        let parts: Vec<String> = [time, context, speed].into_iter().flatten().collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// Sends one request and works on it, tools and all, until the model
    /// answers, the person stops it, or something fails. `asked` is what the
    /// person typed, for auto's judge; None when the request is another
    /// session's mail, and then what the person's own commands gave waits
    /// for one of theirs.
    pub fn send(&mut self, request: &str, input: &mut Input, asked: Option<&str>) -> Turn {
        let on_mail = asked.is_none();
        self.begin_turn(asked);
        let mut end = Turn::Answered;
        let turn_start = self.messages.len();
        let mut model_time = Duration::ZERO;
        let (mut generated, mut generating) = (0, Duration::ZERO);
        let mut asked_again = false;
        let own = self.open_request(request, on_mail);
        loop {
            self.free_context();
            let started = Instant::now();
            let reply = self.ask_model();
            model_time += started.elapsed();
            let reply = match reply {
                Ok(reply) => reply,
                Err(Failure::Stopped) => {
                    end = Turn::SteppedIn;
                    eprintln!("{}", style::gray(text::STOPPED));
                    // Otherwise the model takes the stopped request up again next turn.
                    self.messages
                        .push(json!({"role": "assistant", "content": STOPPED_RESULT}));
                    break;
                }
                Err(Failure::Error(message)) => {
                    end = Turn::Failed;
                    eprintln!("{}", style::red(&message));
                    self.take_back_request(turn_start, own);
                    break;
                }
            };
            self.tokens = reply.tokens.or(self.tokens);
            // The first token starts the clock, so it isn't among those it times.
            generated += reply.generated.saturating_sub(1);
            generating += reply.generating;
            let ran_out = reply.finish.as_deref() == Some("length");
            if reply.content.is_empty() && reply.tool_calls.is_empty() {
                // Out of room before a word: once, ask again for less reasoning.
                // The reasoning isn't kept, so the second try starts with room.
                if ran_out && !asked_again {
                    asked_again = true;
                    eprintln!("{}", style::gray(text::ASKING_AGAIN));
                    self.messages
                        .push(json!({"role": "user", "content": ASK_AGAIN_NOTE}));
                    continue;
                }
                // Nothing came back: say why, rather than handing back the prompt in silence.
                let message = match (ran_out, self.server.context) {
                    (true, Some(context)) => text::ran_out_of_room(context),
                    (true, None) => text::RAN_OUT.to_string(),
                    _ => text::NO_ANSWER.to_string(),
                };
                end = Turn::Failed;
                eprintln!("{}", style::red(&message));
                // So the model doesn't take the unanswered request up again unasked.
                self.messages
                    .push(json!({"role": "assistant", "content": NO_ANSWER_RESULT}));
                break;
            }
            self.messages.push(assistant_message(&reply));
            // Out of room in the middle of a step: it isn't whole, so nothing
            // the reply asked for runs. Once, the model hears why and tries again.
            if ran_out
                && reply
                    .tool_calls
                    .iter()
                    .any(|call| !is_object(&call.arguments))
            {
                for call in &reply.tool_calls {
                    self.messages.push(
                        json!({"role": "tool", "tool_call_id": call.id, "content": CUT_SHORT_RESULT}),
                    );
                }
                if asked_again {
                    end = Turn::Failed;
                    eprintln!("{}", style::red(text::CUT_SHORT_AGAIN));
                    break;
                }
                asked_again = true;
                eprintln!("{}", style::gray(text::CUT_SHORT));
                continue;
            }
            if reply.tool_calls.is_empty() {
                break;
            }
            if !self.run_tools(&reply, input, on_mail) {
                end = Turn::SteppedIn;
                break;
            }
        }
        self.model_time = Some(model_time);
        self.speed = speed(generated, generating);
        self.save();
        if let Some(peers) = &self.peers {
            peers.set_busy(false);
        }
        end
    }

    /// What a turn starts with: the person's request kept for auto's judge,
    /// the other sessions told it's working, and anything the person should
    /// know before it acts.
    fn begin_turn(&mut self, asked: Option<&str>) {
        interrupt::clear();
        if let Some(asked) = asked {
            self.asked.push(asked.to_string());
            let extra = self.asked.len().saturating_sub(judge::REQUESTS_KEPT);
            self.asked.drain(..extra);
        }
        self.written.clear();
        // Other sessions hear it's working: their mail waits for the turn to end.
        if let Some(peers) = &self.peers {
            peers.set_busy(true);
        }
        // Notes edited since the last turn count from this one on.
        if self.refresh_notes() {
            eprintln!("{}\n", style::gray(text::NOTES_CHANGED));
        }
        // Chosen at the `>`, auto's warning shows here, before it acts.
        if asked.is_some() && mode::warn_of_auto() {
            let width = tty::width().unwrap_or(80);
            for line in layout::indented(text::AUTO_WARNING, 0, width).lines() {
                eprintln!("{}", style::gray(line));
            }
            eprintln!();
        }
    }

    /// Adds the request to the conversation, with what the person's own
    /// commands gave ahead of it, and returns those, to put back if the
    /// request is taken back. A turn mail started leaves them waiting.
    fn open_request(&mut self, request: &str, on_mail: bool) -> Vec<String> {
        let own = if on_mail {
            Vec::new()
        } else {
            std::mem::take(&mut self.own)
        };
        // In the request's own message: some chat templates refuse two from
        // the user in a row.
        let content = own
            .iter()
            .map(String::as_str)
            .chain([request])
            .collect::<Vec<_>>()
            .join("\n\n");
        self.messages
            .push(json!({"role": "user", "content": content}));
        own
    }

    /// When nothing came of the request, it's taken back, so trying again
    /// won't send it twice; what the person's commands gave waits again,
    /// ahead of any that came since.
    fn take_back_request(&mut self, turn_start: usize, own: Vec<String>) {
        if self.messages.len() == turn_start + 1 {
            self.messages.pop();
            self.own.splice(0..0, own);
        }
    }

    /// Runs the tools the model asked for, each in the mode chosen when it
    /// starts. False when the turn ends here: the person stopped it or said
    /// no.
    fn run_tools(&mut self, reply: &Reply, input: &mut Input, on_mail: bool) -> bool {
        let mut stopped = false;
        let mut declined = false;
        for (index, call) in reply.tool_calls.iter().enumerate() {
            // Every call gets a result, even the ones skipped after a stop or a no.
            let result = if stopped || interrupt::happened() {
                stopped = true;
                STOPPED_RESULT.to_string()
            } else if declined {
                SKIPPED_AFTER_NO.to_string()
            } else {
                // An even share of the room with the calls still to come, so
                // the first doesn't take it all; what a short output leaves
                // goes to the next.
                let calls_left = reply.tool_calls.len() - index;
                let room = self.room_for_output().map(|room| room / calls_left);
                let scratch = self.facts.scratch.as_deref().map(Path::new);
                let mode = mode::for_step(on_mail);
                let case = judge::Case {
                    server: &self.server,
                    asked: &self.asked,
                    folder: &self.facts.cwd,
                    written: &self.written,
                };
                let peers = self.peers.as_ref();
                let outcome = tools::run(call, input, mode, &case, room, scratch, peers);
                // A blank line after each step, so its output doesn't run into what follows.
                eprintln!();
                match outcome {
                    Outcome::Done(result) => result,
                    Outcome::Wrote { result, path } => {
                        self.written.push(path.display().to_string());
                        result
                    }
                    Outcome::Declined(result) => {
                        declined = true;
                        result
                    }
                    Outcome::Stopped => {
                        stopped = true;
                        STOPPED_RESULT.to_string()
                    }
                }
            };
            self.messages
                .push(json!({"role": "tool", "tool_call_id": call.id, "content": result}));
        }
        if stopped {
            eprintln!("{}", style::gray(text::STOPPED));
        }
        // A note `remember` added counts from the next request on.
        if reply.tool_calls.iter().any(|call| call.name == "remember") {
            self.refresh_system_prompt();
        }
        !stopped && !declined
    }

    /// Streams the model's reply to the screen: the spinner while it thinks
    /// or prepares a tool call, the answer as it's written.
    fn ask_model(&self) -> Result<Reply, Failure> {
        // Ctrl+C as a key too, and nothing typed echoed over the output.
        let _keys = keys::KeyWatch::start();
        let request = chat::start(&self.server.address, self.request_body());
        let mut spinner = Spinner::new();
        let mut mid_line = false;
        let started = Instant::now();
        // How much of the context the reply takes as it grows, and how fast
        // it's generated from its first piece on: a piece is about a token.
        // Reasoning in circles shows here before it fills the context.
        let prompt_tokens = self.estimated_tokens();
        let mut generated: u64 = 0;
        let mut first_piece: Option<Instant> = None;
        let label = |generated: u64,
                     first_piece: Option<Instant>,
                     writing: Option<&ToolProgress>|
         -> String {
            let used = self
                .server
                .context
                .map(|context| text::tokens_of(prompt_tokens + generated, context));
            let rate = first_piece
                .and_then(|first| speed(generated.saturating_sub(1), first.elapsed()))
                .map(text::speed);
            let progress: Vec<String> = [used, rate].into_iter().flatten().collect();
            working_label(
                started,
                writing,
                &progress,
                text::mode_name(mode::current()),
            )
        };
        let mut writing: Option<ToolProgress> = None;
        // The reasoning, when shown, sits in a small window that goes away.
        let mut reasoning = reasoning::Window::default();
        // Styled on a terminal only: piped, the answer stays as the model wrote it.
        let mut markdown = std::io::stdout().is_terminal().then(Markdown::new);
        loop {
            if interrupt::happened() {
                spinner.clear();
                reasoning.clear();
                if mid_line {
                    print_rest(&mut markdown);
                    println!();
                }
                return Err(Failure::Stopped);
            }
            let event = request.next(Duration::from_millis(40));
            if matches!(
                event,
                Some(Event::Reasoning(_) | Event::Text(_) | Event::ToolCall { .. })
            ) {
                generated += 1;
                first_piece.get_or_insert_with(Instant::now);
            }
            match event {
                None => {}
                Some(Event::Reasoning(text)) => {
                    if self.show_reasoning && !mid_line {
                        reasoning.push(&text);
                        reasoning.draw();
                        // The window redrew over the spinner's row: put the spinner back.
                        spinner.redraw(&label(generated, first_piece, writing.as_ref()));
                    }
                }
                Some(Event::Text(text)) => {
                    spinner.clear();
                    reasoning.clear();
                    let shown = match &mut markdown {
                        Some(markdown) => markdown.push(&text),
                        None => text,
                    };
                    print!("{shown}");
                    let _ = std::io::stdout().flush();
                    mid_line = true;
                }
                Some(Event::ToolCall { name, path, bytes }) => {
                    if !reasoning.is_empty() {
                        spinner.clear();
                        reasoning.clear();
                    }
                    if mid_line {
                        // End the text, then a blank line before the step.
                        print_rest(&mut markdown);
                        println!("\n");
                        mid_line = false;
                    }
                    writing = Some(ToolProgress { name, path, bytes });
                }
                Some(Event::Done(result)) => {
                    spinner.clear();
                    reasoning.clear();
                    if mid_line {
                        print_rest(&mut markdown);
                        println!();
                    }
                    return result.map_err(Failure::Error);
                }
            }
            // After every piece too, not only in a pause between them: hidden
            // reasoning or a file being written can stream with no pause for
            // seconds, and the spinner would freeze.
            if !mid_line && spinner.due() {
                spinner.tick_with(&label(generated, first_piece, writing.as_ref()));
            }
        }
    }

    fn request_body(&self) -> Value {
        let mut messages = vec![json!({"role": "system", "content": self.system})];
        messages.extend(self.messages.iter().cloned());
        let mut body = json!({
            "model": self.server.model,
            "messages": messages,
            "tools": tools::definitions(self.peers.is_some()),
            "stream": true,
            "stream_options": {"include_usage": true}
        });
        // At most half the context for one reply, so a model reasoning in
        // circles stops there rather than filling all of it. Half is still
        // room for a large file.
        if let Some(context) = self.server.context {
            body["max_tokens"] = json!(context / 2);
        }
        body
    }

    /// Near the end of the context, what the model has seen makes way
    /// (`context::free`).
    fn free_context(&mut self) {
        if let Some(context) = self.server.context {
            let fixed = self.fixed();
            context::free(&mut self.messages, fixed, context);
        }
    }

    /// How many characters the next tool output may take, when the context's
    /// size is known (`context::room_for_output`).
    fn room_for_output(&self) -> Option<usize> {
        let context = self.server.context?;
        Some(context::room_for_output(
            &self.messages,
            self.fixed(),
            context,
        ))
    }

    fn estimated_tokens(&self) -> u64 {
        context::estimate(self.fixed(), &self.messages)
    }

    /// What every request carries besides the messages, in characters: the
    /// system prompt and the tools.
    fn fixed(&self) -> usize {
        self.system.len() + self.tools_size
    }

    fn save(&self) {
        if self.messages.is_empty() {
            return;
        }
        let path = config::last_conversation_path();
        let saved = json!({"messages": self.messages, "tokens": self.tokens});
        if let Err(error) = config::write(&path, &saved.to_string()) {
            eprintln!(
                "{}",
                style::red(&text::cant_save(&config::shown(&path), &error.to_string()))
            );
        }
    }

    /// Loads the last conversation. False if there's none.
    pub fn resume(&mut self) -> bool {
        let saved = fs::read_to_string(config::last_conversation_path()).unwrap_or_default();
        let Ok(saved) = serde_json::from_str::<Value>(&saved) else {
            return false;
        };
        let Some(messages) = saved["messages"]
            .as_array()
            .filter(|messages| !messages.is_empty())
        else {
            return false;
        };
        self.messages = messages.clone();
        context::mend_arguments(&mut self.messages);
        self.tokens = saved["tokens"].as_u64();
        true
    }
}

/// What the Markdown styling still holds, with every style closed.
fn print_rest(markdown: &mut Option<Markdown>) {
    if let Some(markdown) = markdown {
        print!("{}", markdown.finish());
    }
}

fn assistant_message(reply: &Reply) -> Value {
    let mut message = json!({"role": "assistant", "content": reply.content});
    if !reply.tool_calls.is_empty() {
        let calls: Vec<Value> = reply
            .tool_calls
            .iter()
            .map(|call| {
                json!({
                    "id": call.id,
                    "type": "function",
                    "function": {"name": call.name, "arguments": kept_arguments(&call.arguments)}
                })
            })
            .collect();
        message["tool_calls"] = Value::Array(calls);
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(context: u64) -> Conversation {
        Conversation {
            server: Server {
                address: String::new(),
                model: String::new(),
                context: Some(context),
                max_context: None,
                pinned: false,
            },
            facts: Facts {
                os: String::new(),
                wsl: false,
                user: String::new(),
                home: String::new(),
                cwd: String::new(),
                windows_folders: Vec::new(),
                scratch: None,
            },
            recon: Vec::new(),
            notes: Vec::new(),
            system: String::new(),
            messages: vec![json!({"role": "user", "content": "read it"})],
            tokens: None,
            model_time: None,
            speed: None,
            show_reasoning: false,
            peers: None,
            own: Vec::new(),
            asked: Vec::new(),
            written: Vec::new(),
            tools_size: tools::definitions(false).to_string().len(),
        }
    }

    #[test]
    fn arguments_cut_short_are_kept_as_an_empty_object() {
        let call = |id: &str, arguments: &str| chat::ToolCall {
            id: id.into(),
            name: "write_file".into(),
            arguments: arguments.into(),
        };
        let reply = Reply {
            tool_calls: vec![
                call("1", r#"{"path": "a.txt", "content": "unfini"#),
                call("2", r#"{"path": "b.txt", "content": "whole"}"#),
            ],
            ..Reply::default()
        };
        let message = assistant_message(&reply);
        assert_eq!(message["tool_calls"][0]["function"]["arguments"], "{}");
        assert_eq!(
            message["tool_calls"][1]["function"]["arguments"],
            r#"{"path": "b.txt", "content": "whole"}"#
        );
    }

    #[test]
    fn room_for_output_is_unknown_without_the_context() {
        let mut conversation = conversation(8192);
        conversation.server.context = None;
        assert_eq!(conversation.room_for_output(), None);
    }

    #[test]
    fn a_request_taken_back_leaves_the_persons_commands_waiting() {
        let mut conversation = conversation(8192);
        conversation.own = vec!["ran ls".to_string()];
        let before = conversation.messages.clone();
        // The person's request carries them, and puts them back.
        let start = conversation.messages.len();
        let own = conversation.open_request("why?", false);
        assert!(conversation.own.is_empty());
        assert_eq!(conversation.messages[start]["content"], "ran ls\n\nwhy?");
        conversation.take_back_request(start, own);
        assert_eq!(conversation.own, ["ran ls"]);
        assert_eq!(conversation.messages, before);
        // Mail leaves them waiting, and taken back, still there.
        let own = conversation.open_request("from a session", true);
        assert_eq!(conversation.messages[start]["content"], "from a session");
        conversation.take_back_request(start, own);
        assert_eq!(conversation.own, ["ran ls"]);
        assert_eq!(conversation.messages, before);
    }

    #[test]
    fn status_has_each_part_once_known() {
        let mut conversation = conversation(8192);
        assert_eq!(conversation.status(), None);
        conversation.speed = Some(24);
        assert_eq!(conversation.status().as_deref(), Some("24 tok/s"));
        conversation.model_time = Some(Duration::from_secs(38));
        conversation.tokens = Some(1000);
        assert_eq!(
            conversation.status().as_deref(),
            Some("38s · 12% · 24 tok/s")
        );
    }
}
