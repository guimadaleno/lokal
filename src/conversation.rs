//! The conversation: the messages so far, one turn at a time, and the copy
//! on disk that `lokal -r` resumes.

use std::fs;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::chat::{self, Event, Reply};
use crate::input::Input;
use crate::markdown::Markdown;
use crate::memory::{self, Note};
use crate::peers::Peers;
use crate::server::Server;
use crate::spinner::Spinner;
use crate::system::Facts;
use crate::tools::{self, Outcome};
use crate::{config, interrupt, judge, keys, mode, prompt, reasoning, style, text};

/// When the spinner starts saying what the model is doing.
const LABEL_AFTER: Duration = Duration::from_secs(2);
/// How long the model has to generate before its speed shows: a few
/// tokens' worth says little about it.
const SPEED_AFTER: Duration = Duration::from_secs(1);
/// Stands in for a tool's output once it's dropped to free up context.
const DROPPED_OUTPUT: &str = "[Output dropped to free up context, after you saw it.]";
/// Stands in for the text a file change carried once it's dropped.
const DROPPED_WRITE: &str = "[Dropped to free up context; read the file for what it holds now.]";
/// About three characters a token: rough, and on the high side.
const CHARS_PER_TOKEN: usize = 3;
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
/// Frame a command the person ran at the `>`, ahead of their next request:
/// the command between the first two, its result before the last, which
/// `make_way` looks for to drop it.
const OWN_START: &str = "[The user ran this command themselves, at their prompt: `";
const OWN_OUTPUT: &str = "`. What it gave:]\n";
const OWN_END: &str = "\n[End of what the user's command gave.]";

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
}

enum Failure {
    Stopped,
    Error(String),
}

impl Conversation {
    pub fn new(server: Server, facts: Facts, peers: Option<Peers>) -> Self {
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
        (&self.system, names, self.estimate(&[]))
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
            self.own
                .push(format!("{OWN_START}{command}{OWN_OUTPUT}{result}{OWN_END}"));
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
    /// answers, the person stops it, or something fails. True when the
    /// person stepped in: stopped the turn or said no to a step. `asked` is
    /// what the person typed, for auto's judge; None when the request is
    /// another session's mail, and then what the person's own commands gave
    /// waits for one of theirs.
    pub fn send(&mut self, request: &str, input: &mut Input, asked: Option<&str>) -> bool {
        interrupt::clear();
        let on_mail = asked.is_none();
        if let Some(asked) = asked {
            self.asked.push(asked.to_string());
            let extra = self.asked.len().saturating_sub(judge::REQUESTS_KEPT);
            self.asked.drain(..extra);
        }
        self.written.clear();
        let mut stepped_in = false;
        // Other sessions hear it's working: their mail waits for the turn to end.
        if let Some(peers) = &self.peers {
            peers.set_busy(true);
        }
        // Notes edited since the last turn count from this one on.
        if self.refresh_notes() {
            eprintln!("{}\n", style::gray(text::NOTES_CHANGED));
        }
        // Chosen at the `>`, auto's warning shows here, before it acts.
        if !on_mail && mode::warn_of_auto() {
            let width = style::terminal_width().unwrap_or(80);
            for line in crate::peers::indented(text::AUTO_WARNING, 0, width).lines() {
                eprintln!("{}", style::gray(line));
            }
            eprintln!();
        }
        let turn_start = self.messages.len();
        let mut model_time = Duration::ZERO;
        let (mut generated, mut generating) = (0, Duration::ZERO);
        let mut asked_again = false;
        // In the request's own message: some chat templates refuse two from
        // the user in a row.
        let own = if on_mail {
            Vec::new()
        } else {
            std::mem::take(&mut self.own)
        };
        let content = own
            .iter()
            .map(String::as_str)
            .chain([request])
            .collect::<Vec<_>>()
            .join("\n\n");
        self.messages
            .push(json!({"role": "user", "content": content}));
        loop {
            self.free_context();
            let started = Instant::now();
            let reply = self.ask_model();
            model_time += started.elapsed();
            let reply = match reply {
                Ok(reply) => reply,
                Err(Failure::Stopped) => {
                    stepped_in = true;
                    eprintln!("{}", style::gray(text::STOPPED));
                    // Otherwise the model takes the stopped request up again next turn.
                    self.messages
                        .push(json!({"role": "assistant", "content": STOPPED_RESULT}));
                    break;
                }
                Err(Failure::Error(message)) => {
                    eprintln!("{}", style::red(&message));
                    // Nothing came of the request, so trying again won't send
                    // it twice; what the person's commands gave goes with it.
                    // In a turn mail started, that's still waiting untouched.
                    if self.messages.len() == turn_start + 1 {
                        self.messages.pop();
                        self.own.splice(0..0, own);
                    }
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
                stepped_in = true;
                break;
            }
        }
        self.model_time = Some(model_time);
        self.speed = speed(generated, generating);
        self.save();
        if let Some(peers) = &self.peers {
            peers.set_busy(false);
        }
        stepped_in
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
                if let Some(path) = tools::written_path(call, &outcome) {
                    self.written.push(path);
                }
                // A blank line after each step, so its output doesn't run into what follows.
                eprintln!();
                match outcome {
                    Outcome::Done(result) => result,
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
        let progress = |generated: u64, first_piece: Option<Instant>| -> Vec<String> {
            let used = self
                .server
                .context
                .map(|context| text::tokens_of(prompt_tokens + generated, context));
            let rate = first_piece
                .and_then(|first| speed(generated.saturating_sub(1), first.elapsed()))
                .map(text::speed);
            [used, rate].into_iter().flatten().collect()
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
                        let label = working_label(
                            started,
                            writing.as_ref(),
                            &progress(generated, first_piece),
                            text::mode_name(mode::current()),
                        );
                        spinner.redraw(&label);
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
                let label = working_label(
                    started,
                    writing.as_ref(),
                    &progress(generated, first_piece),
                    text::mode_name(mode::current()),
                );
                spinner.tick_with(&label);
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

    /// Near the end of the context, the oldest of what the model has seen
    /// and replied after goes first: tool outputs, and the text of the files
    /// it wrote, which the files hold now. They take the most room and matter
    /// least later on. The outputs the model hasn't seen yet stay, cut to fit
    /// instead (`room_for_output`): dropped unseen, it would only ask for them
    /// again, and again.
    fn free_context(&mut self) {
        let Some(context) = self.server.context else {
            return;
        };
        for index in 0..self.last_reply() {
            if self.estimated_tokens() <= budget(context) {
                return;
            }
            make_way(&mut self.messages[index]);
        }
    }

    /// How many characters the next tool output may take, when the context's
    /// size is known: the room left once everything that can make way has,
    /// since that goes first.
    fn room_for_output(&self) -> Option<usize> {
        let context = self.server.context?;
        let mut messages = self.messages.clone();
        for message in &mut messages[..self.last_reply()] {
            make_way(message);
        }
        let room = budget(context).saturating_sub(self.estimate(&messages));
        Some(room as usize * CHARS_PER_TOKEN)
    }

    /// Where the model's last reply is: everything before it, the model has
    /// seen and replied after.
    fn last_reply(&self) -> usize {
        self.messages
            .iter()
            .rposition(|message| message["role"] == "assistant")
            .unwrap_or(0)
    }

    fn estimated_tokens(&self) -> u64 {
        self.estimate(&self.messages)
    }

    /// The request's size in tokens with these messages: rough, and on the
    /// high side.
    fn estimate(&self, messages: &[Value]) -> u64 {
        let characters = self.system.len()
            + tools::definitions(self.peers.is_some()).to_string().len()
            + serde_json::to_string(messages).map_or(0, |json| json.len());
        (characters / CHARS_PER_TOKEN) as u64
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
        mend_arguments(&mut self.messages);
        self.tokens = saved["tokens"].as_u64();
        true
    }
}

/// A text's size in tokens: rough, and on the high side.
pub fn tokens_in(text: &str) -> u64 {
    (text.len() / CHARS_PER_TOKEN) as u64
}

/// What the conversation may fill: a quarter of the context stays for the
/// model's reasoning and reply.
fn budget(context: u64) -> u64 {
    context * 3 / 4
}

/// Drops what a message the model has replied after holds: a tool's output,
/// what the person's own commands gave, or the text of the file changes it
/// asked for.
fn make_way(message: &mut Value) {
    if message["role"] == "tool" {
        message["content"] = DROPPED_OUTPUT.into();
    }
    if message["role"] == "user"
        && let Some(content) = message["content"].as_str().and_then(drop_own)
    {
        message["content"] = content.into();
    }
    // get_mut, since indexing to change would add the keys a message lacks.
    let calls = message.get_mut("tool_calls").and_then(Value::as_array_mut);
    for call in calls.into_iter().flatten() {
        if let Some(function) = call.get_mut("function") {
            drop_written(function);
        }
    }
}

/// A request with what each of the person's own commands gave dropped, the
/// commands kept. None when there's nothing to drop.
fn drop_own(content: &str) -> Option<String> {
    let mut kept = String::new();
    let mut rest = content;
    let mut dropped = false;
    while let Some(start) = rest.find(OWN_START)
        && let Some(output) = rest[start..]
            .find(OWN_OUTPUT)
            .map(|at| start + at + OWN_OUTPUT.len())
        && let Some(end) = rest[output..].find(OWN_END).map(|at| output + at)
    {
        kept.push_str(&rest[..output]);
        if end - output > DROPPED_OUTPUT.len() {
            kept.push_str(DROPPED_OUTPUT);
            dropped = true;
        } else {
            kept.push_str(&rest[output..end]);
        }
        rest = &rest[end..];
    }
    kept.push_str(rest);
    dropped.then_some(kept)
}

/// A write_file or edit_file call keeps its path and loses its text. The
/// arguments stay valid JSON: the server reads them back into the prompt.
fn drop_written(function: &mut Value) {
    let fields: &[&str] = match function["name"].as_str() {
        Some("write_file") => &["content"],
        Some("edit_file") => &["old_text", "new_text"],
        _ => return,
    };
    let Some(Ok(mut arguments)) = function["arguments"]
        .as_str()
        .map(serde_json::from_str::<Value>)
    else {
        return;
    };
    let mut dropped = false;
    for &field in fields {
        if arguments[field]
            .as_str()
            .is_some_and(|text| text.len() > DROPPED_WRITE.len())
        {
            arguments[field] = DROPPED_WRITE.into();
            dropped = true;
        }
    }
    if dropped {
        function["arguments"] = arguments.to_string().into();
    }
}

/// Tokens a second, once the model has generated for long enough.
fn speed(tokens: u64, time: Duration) -> Option<u64> {
    (time >= SPEED_AFTER).then(|| (tokens as f64 / time.as_secs_f64()).round() as u64)
}

/// A tool call as the model writes it.
struct ToolProgress {
    name: String,
    path: Option<String>,
    bytes: usize,
}

/// What the spinner says while the model works, once it has taken a moment:
/// a long silence looks like a hang otherwise. The mode, when it isn't
/// manual, shows at once and first, so Shift+Tab is seen to take; then the
/// time, and `progress`, most important first.
fn working_label(
    started: Instant,
    writing: Option<&ToolProgress>,
    progress: &[String],
    mode: Option<&str>,
) -> String {
    let elapsed = started.elapsed();
    if elapsed < LABEL_AFTER && mode.is_none() {
        return String::new();
    }
    let mut parts: Vec<String> = mode.into_iter().map(String::from).collect();
    if elapsed >= LABEL_AFTER {
        parts.push(text::duration(elapsed));
        // A server may hold a tool call's arguments back until they're
        // complete: while nothing arrives to count, only the time, rather
        // than a size, a token count and a speed that sit still.
        if writing.is_none_or(|call| call.bytes > 0) {
            parts.extend_from_slice(progress);
        }
    }
    // On one line, or the spinner's redraw would leave the rest behind.
    let room = style::terminal_width().map_or(80, |width| width.saturating_sub(4));
    let what = |path: Option<&str>| match writing {
        None => text::THINKING.to_string(),
        Some(call) => text::writing_tool(&call.name, path, call.bytes),
    };
    let path = writing
        .and_then(|call| call.path.as_deref())
        .map(|path| config::shown(Path::new(path)));
    // The progress gives way from its end, before the file's name would.
    let name = path.as_deref().map(|path| {
        let name = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy());
        format!("…{}", name.unwrap_or(path.into()))
    });
    while parts.len() > 1
        && format!("{} · {}", what(name.as_deref()), parts.join(" · "))
            .chars()
            .count()
            > room
    {
        parts.pop();
    }
    let time = parts.join(" · ");
    // A long path loses its start: the file's name, its size and the time stay.
    let spare = room.saturating_sub(format!("{} · {time}", what(Some(""))).chars().count());
    let path = path.as_deref().map(|path| tail(path, spare));
    let label = format!("{} · {time}", what(path.as_deref()));
    format!(" {}", style::pulse_faint(&label, started))
}

/// The end of a text that fits in `room` characters, marked with … when cut.
fn tail(text: &str, room: usize) -> String {
    let length = text.chars().count();
    if length <= room {
        return text.to_string();
    }
    let kept: String = text.chars().skip(length + 1 - room.max(1)).collect();
    format!("…{kept}")
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

/// A call's arguments as the conversation keeps them. The server reads them
/// back into the prompt, and arguments that aren't JSON, such as ones cut
/// short, make it fail every request after. Those are kept as `{}`, and so
/// is anything but an object, which no tool takes.
fn kept_arguments(arguments: &str) -> &str {
    if is_object(arguments) {
        arguments
    } else {
        "{}"
    }
}

fn is_object(arguments: &str) -> bool {
    serde_json::from_str::<Value>(arguments).is_ok_and(|value| value.is_object())
}

/// A saved conversation may hold arguments the server can't read back:
/// kept as they are, it couldn't go on.
fn mend_arguments(messages: &mut [Value]) {
    let calls = messages
        .iter_mut()
        .filter_map(|message| message.get_mut("tool_calls"))
        .filter_map(Value::as_array_mut)
        .flatten();
    for arguments in calls.filter_map(|call| call.pointer_mut("/function/arguments")) {
        let kept = kept_arguments(arguments.as_str().unwrap_or_default()).to_string();
        *arguments = kept.into();
    }
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
        }
    }

    /// The model replies with a step.
    fn reply(conversation: &mut Conversation) {
        let call = json!({"id": "1", "type": "function",
            "function": {"name": "read_file", "arguments": "{}"}});
        conversation
            .messages
            .push(json!({"role": "assistant", "content": "", "tool_calls": [call]}));
    }

    /// The step's output arrives.
    fn output(conversation: &mut Conversation, characters: usize) {
        conversation
            .messages
            .push(json!({"role": "tool", "tool_call_id": "1", "content": "x".repeat(characters)}));
    }

    /// The model writes a file with this much text, and it's written.
    fn write(conversation: &mut Conversation, characters: usize) {
        let arguments = json!({"path": "a.txt", "content": "x".repeat(characters)}).to_string();
        let call = json!({"id": "2", "type": "function",
            "function": {"name": "write_file", "arguments": arguments}});
        conversation
            .messages
            .push(json!({"role": "assistant", "content": "", "tool_calls": [call]}));
        conversation
            .messages
            .push(json!({"role": "tool", "tool_call_id": "2", "content": "Wrote a.txt."}));
    }

    fn arguments(conversation: &Conversation, index: usize) -> Value {
        let arguments = &conversation.messages[index]["tool_calls"][0]["function"]["arguments"];
        serde_json::from_str(arguments.as_str().unwrap()).unwrap()
    }

    fn content_length(conversation: &Conversation, index: usize) -> usize {
        conversation.messages[index]["content"]
            .as_str()
            .unwrap()
            .len()
    }

    #[test]
    fn older_outputs_make_way_for_the_newest() {
        let mut conversation = conversation(8192);
        reply(&mut conversation);
        output(&mut conversation, 9000);
        reply(&mut conversation);
        output(&mut conversation, 12_000);
        conversation.free_context();
        assert_eq!(conversation.messages[2]["content"], DROPPED_OUTPUT);
        assert_eq!(content_length(&conversation, 4), 12_000);
    }

    #[test]
    fn an_output_the_model_hasnt_seen_is_never_dropped() {
        let mut conversation = conversation(8192);
        reply(&mut conversation);
        output(&mut conversation, 30_000);
        conversation.free_context();
        assert_eq!(content_length(&conversation, 2), 30_000);
    }

    #[test]
    fn room_for_output_counts_seen_outputs_as_gone() {
        let mut conversation = conversation(8192);
        reply(&mut conversation);
        output(&mut conversation, 9000);
        reply(&mut conversation);
        // About 18,000 characters under the budget; the 9,000 seen don't count.
        let room = conversation.room_for_output().unwrap();
        assert!(room > 12_000, "room {room}");
        // A first output of this step, not seen yet, does count.
        output(&mut conversation, 3000);
        let after = conversation.room_for_output().unwrap();
        assert!(room - after >= 3000, "room {room}, after {after}");
    }

    #[test]
    fn a_written_file_makes_way_once_the_model_replied_after_it() {
        let mut conversation = conversation(8192);
        write(&mut conversation, 20_000);
        reply(&mut conversation);
        conversation.free_context();
        let arguments = arguments(&conversation, 1);
        assert_eq!(arguments["path"], "a.txt");
        assert_eq!(arguments["content"], DROPPED_WRITE);
    }

    #[test]
    fn the_last_reply_keeps_what_it_wrote() {
        let mut conversation = conversation(8192);
        write(&mut conversation, 20_000);
        conversation.free_context();
        let arguments = arguments(&conversation, 1);
        assert_eq!(arguments["content"].as_str().unwrap().len(), 20_000);
    }

    #[test]
    fn room_for_output_counts_written_files_as_gone() {
        let mut conversation = conversation(8192);
        write(&mut conversation, 15_000);
        reply(&mut conversation);
        // About 18,000 characters under the budget; kept, the file would leave under 3,500.
        let room = conversation.room_for_output().unwrap();
        assert!(room > 10_000, "room {room}");
    }

    #[test]
    fn an_edit_loses_its_long_text_and_keeps_the_rest() {
        let arguments = json!({"path": "a.txt", "old_text": "x".repeat(500), "new_text": "y"});
        let mut function = json!({"name": "edit_file", "arguments": arguments.to_string()});
        drop_written(&mut function);
        let arguments: Value =
            serde_json::from_str(function["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(arguments["path"], "a.txt");
        assert_eq!(arguments["old_text"], DROPPED_WRITE);
        assert_eq!(arguments["new_text"], "y");
    }

    #[test]
    fn what_the_persons_commands_gave_makes_way_and_the_rest_stays() {
        let own = |command: &str, result: &str| {
            format!("{OWN_START}{command}{OWN_OUTPUT}{result}{OWN_END}")
        };
        let request = format!(
            "{}\n\n{}\n\nwhy does it fail?",
            own("cargo test", &"x".repeat(30_000)),
            own("git status", "ok")
        );
        let mut conversation = conversation(8192);
        conversation.messages = vec![json!({"role": "user", "content": request})];
        reply(&mut conversation);
        conversation.free_context();
        let kept = conversation.messages[0]["content"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            kept,
            format!(
                "{}\n\n{}\n\nwhy does it fail?",
                own("cargo test", DROPPED_OUTPUT),
                own("git status", "ok")
            )
        );
        // Already dropped, or as short as the note: nothing more to drop.
        assert_eq!(drop_own(&kept), None);
        assert_eq!(drop_own("[The user ran this command themselves"), None);
    }

    #[test]
    fn making_way_adds_nothing_to_a_message_without_calls() {
        let mut message = json!({"role": "user", "content": "hi"});
        make_way(&mut message);
        assert_eq!(message, json!({"role": "user", "content": "hi"}));
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
    fn a_saved_conversation_with_arguments_the_server_cant_read_is_mended() {
        let call = |arguments: &str| {
            json!({"id": "1", "type": "function",
                "function": {"name": "write_file", "arguments": arguments}})
        };
        let calls = [
            call(r#"{"path": "a"#),
            call("[1]"),
            call(r#"{"path": "b"}"#),
        ];
        let mut messages = vec![
            json!({"role": "user", "content": "hi"}),
            json!({"role": "assistant", "content": "", "tool_calls": calls}),
        ];
        mend_arguments(&mut messages);
        let kept: Vec<&str> = messages[1]["tool_calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|call| call["function"]["arguments"].as_str().unwrap())
            .collect();
        assert_eq!(kept, ["{}", "{}", r#"{"path": "b"}"#]);
        assert_eq!(messages[0], json!({"role": "user", "content": "hi"}));
    }

    #[test]
    fn room_for_output_is_unknown_without_the_context() {
        let mut conversation = conversation(8192);
        conversation.server.context = None;
        assert_eq!(conversation.room_for_output(), None);
    }

    #[test]
    fn speed_shows_after_a_second_of_generating() {
        assert_eq!(speed(9, Duration::from_millis(900)), None);
        assert_eq!(speed(0, Duration::ZERO), None);
        assert_eq!(speed(50, Duration::from_millis(2100)), Some(24));
    }

    #[test]
    fn the_mode_shows_at_once_and_first() {
        let progress = ["~2k of 33k tokens".to_string()];
        assert_eq!(working_label(Instant::now(), None, &progress, None), "");
        let at_once = working_label(Instant::now(), None, &progress, Some("accept edits"));
        assert!(at_once.contains("thinking · accept edits") && !at_once.contains("tokens"));
        let later = Instant::now() - Duration::from_secs(3);
        let label = working_label(later, None, &progress, Some("accept edits"));
        assert!(label.contains("thinking · accept edits · 3s · ~2k of 33k tokens"));
    }

    #[test]
    fn the_label_leaves_progress_out_while_arguments_are_held_back() {
        let started = Instant::now() - Duration::from_secs(3);
        let progress = ["~2k of 33k tokens".to_string(), "52 tok/s".to_string()];
        assert!(
            working_label(started, None, &progress, None)
                .contains("3s · ~2k of 33k tokens · 52 tok/s")
        );
        let held = ToolProgress {
            name: "run_command".into(),
            path: None,
            bytes: 0,
        };
        assert!(!working_label(started, Some(&held), &progress, None).contains("tokens"));
        // Where it doesn't fit on the line, the progress goes from its end.
        let long = ["~2k of 33k tokens".to_string(), "x".repeat(80)];
        let label = working_label(started, None, &long, None);
        assert!(label.contains("3s · ~2k of 33k tokens") && !label.contains('x'));
    }

    #[test]
    fn the_speed_gives_way_before_the_file_name() {
        let started = Instant::now() - Duration::from_secs(3);
        let progress = ["~2k of 33k tokens".to_string(), "52 tok/s".to_string()];
        let writing = ToolProgress {
            name: "write_file".into(),
            path: Some("/srv/some/deep/folder/a_rather_long_file_name_123.rs".into()),
            bytes: 2000,
        };
        let label = working_label(started, Some(&writing), &progress, None);
        assert!(label.contains("a_rather_long_file_name_123.rs"));
        assert!(label.contains("tokens") && !label.contains("tok/s"));
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
