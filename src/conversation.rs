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
use crate::server::Server;
use crate::spinner::Spinner;
use crate::system::Facts;
use crate::tools::{self, Outcome};
use crate::{config, interrupt, keys, prompt, reasoning, style, text};

/// When the spinner starts saying what the model is doing.
const LABEL_AFTER: Duration = Duration::from_secs(2);
/// Stands in for a tool's output once it's dropped to free up context.
const DROPPED_OUTPUT: &str = "[Output dropped to free up context, after you saw it.]";
/// About three characters a token: rough, and on the high side.
const CHARS_PER_TOKEN: usize = 3;
const STOPPED_RESULT: &str = "Stopped by the user.";
const SKIPPED_AFTER_NO: &str = "Not run: the user declined an earlier step.";
const NO_ANSWER_RESULT: &str = "(No answer: the reply stopped before it began.)";
const ASK_AGAIN_NOTE: &str = "(From Lokal, not the user: your last reply ran out of room while \
    still reasoning, before any answer or step. Reason briefly this time, then answer or take the next step.)";

pub struct Conversation {
    pub server: Server,
    facts: Facts,
    system: String,
    /// Everything after the system prompt, in the chat API's format.
    messages: Vec<Value>,
    /// Tokens in use after the last reply.
    tokens: Option<u64>,
    /// How long the model worked on the last turn, not counting commands
    /// running or the person deciding on an approval.
    model_time: Option<Duration>,
    /// `/reasoning`: the model's reasoning on screen, faint, for this session.
    show_reasoning: bool,
}

enum Failure {
    Stopped,
    Error(String),
}

impl Conversation {
    pub fn new(server: Server, facts: Facts) -> Self {
        let system =
            prompt::system_prompt(&facts, server.context, config::read_about_me().as_deref());
        Conversation {
            server,
            facts,
            system,
            messages: Vec::new(),
            tokens: None,
            model_time: None,
            show_reasoning: true,
        }
    }

    pub fn scratch(&self) -> Option<&str> {
        self.facts.scratch.as_deref()
    }

    /// Picks about-me.md up again after it changed.
    pub fn refresh_system_prompt(&mut self) {
        self.system = prompt::system_prompt(
            &self.facts,
            self.server.context,
            config::read_about_me().as_deref(),
        );
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
        self.tokens = None;
        self.model_time = None;
    }

    /// The model's time on the last turn and how full the context is, like
    /// "38s · 12%"; each part once it's known.
    pub fn status(&self) -> Option<String> {
        let time = self.model_time.map(text::duration);
        let context = self
            .tokens
            .zip(self.server.context)
            .map(|(tokens, context)| format!("{}%", (tokens * 100 / context.max(1)).min(100)));
        match (time, context) {
            (Some(time), Some(context)) => Some(format!("{time} · {context}")),
            (time, context) => time.or(context),
        }
    }

    /// Sends one request and works on it, tools and all, until the model
    /// answers, the person stops it, or something fails.
    pub fn send(&mut self, request: &str, input: &mut Input) {
        interrupt::clear();
        let turn_start = self.messages.len();
        let mut model_time = Duration::ZERO;
        let mut asked_again = false;
        self.messages
            .push(json!({"role": "user", "content": request}));
        loop {
            self.free_context();
            let started = Instant::now();
            let reply = self.ask_model();
            model_time += started.elapsed();
            let reply = match reply {
                Ok(reply) => reply,
                Err(Failure::Stopped) => {
                    eprintln!("{}", style::gray(text::STOPPED));
                    // Otherwise the model takes the stopped request up again next turn.
                    self.messages
                        .push(json!({"role": "assistant", "content": STOPPED_RESULT}));
                    break;
                }
                Err(Failure::Error(message)) => {
                    eprintln!("{}", style::red(&message));
                    // Nothing came of the request, so trying again won't send it twice.
                    if self.messages.len() == turn_start + 1 {
                        self.messages.pop();
                    }
                    break;
                }
            };
            self.tokens = reply.tokens.or(self.tokens);
            if reply.content.is_empty() && reply.tool_calls.is_empty() {
                let ran_out = reply.finish.as_deref() == Some("length");
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
            if reply.tool_calls.is_empty() || !self.run_tools(&reply, input) {
                break;
            }
        }
        self.model_time = Some(model_time);
        self.save();
    }

    /// Runs the tools the model asked for. False if the person stopped them.
    /// Runs the tools the model asked for. False when the turn ends here:
    /// the person stopped it or said no.
    fn run_tools(&mut self, reply: &Reply, input: &mut Input) -> bool {
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
                let outcome = tools::run(call, input, room);
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
        // How much of the context the reply takes as it grows: a piece is
        // about a token. Reasoning in circles shows here before it fills it.
        let prompt_tokens = self.estimated_tokens();
        let mut generated: u64 = 0;
        let used = |generated: u64| {
            self.server
                .context
                .map(|context| text::tokens_of(prompt_tokens + generated, context))
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
            match request.next(Duration::from_millis(40)) {
                None if !mid_line => spinner.tick_with(&working_label(
                    started,
                    writing.as_ref(),
                    used(generated).as_deref(),
                )),
                None => {}
                Some(Event::Reasoning(text)) => {
                    generated += 1;
                    if self.show_reasoning && !mid_line {
                        reasoning.push(&text);
                        reasoning.draw();
                        // The window redrew over the spinner's row: put the spinner back.
                        let label =
                            working_label(started, writing.as_ref(), used(generated).as_deref());
                        spinner.redraw(&label);
                    }
                }
                Some(Event::Text(text)) => {
                    generated += 1;
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
                    generated += 1;
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
        }
    }

    fn request_body(&self) -> Value {
        let mut messages = vec![json!({"role": "system", "content": self.system})];
        messages.extend(self.messages.iter().cloned());
        let mut body = json!({
            "model": self.server.model,
            "messages": messages,
            "tools": tools::definitions(),
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

    /// Near the end of the context, the oldest tool outputs go first: they
    /// take the most room and matter least later on. The outputs the model
    /// hasn't seen yet stay, cut to fit instead (`room_for_output`): dropped
    /// unseen, it would only ask for them again, and again.
    fn free_context(&mut self) {
        let Some(context) = self.server.context else {
            return;
        };
        for index in 0..self.unseen_from() {
            if self.estimated_tokens() <= budget(context) {
                return;
            }
            let message = &mut self.messages[index];
            if message["role"] == "tool" {
                message["content"] = DROPPED_OUTPUT.into();
            }
        }
    }

    /// How many characters the next tool output may take, when the context's
    /// size is known: the room left once every output the model has seen
    /// makes way, since those go first.
    fn room_for_output(&self) -> Option<usize> {
        let context = self.server.context?;
        let mut messages = self.messages.clone();
        for message in &mut messages[..self.unseen_from()] {
            if message["role"] == "tool" {
                message["content"] = DROPPED_OUTPUT.into();
            }
        }
        let room = budget(context).saturating_sub(self.estimate(&messages));
        Some(room as usize * CHARS_PER_TOKEN)
    }

    /// Where the messages the model hasn't replied to yet begin.
    fn unseen_from(&self) -> usize {
        self.messages
            .iter()
            .rposition(|message| message["role"] == "assistant")
            .map_or(0, |index| index + 1)
    }

    fn estimated_tokens(&self) -> u64 {
        self.estimate(&self.messages)
    }

    /// The request's size in tokens with these messages: rough, and on the
    /// high side.
    fn estimate(&self, messages: &[Value]) -> u64 {
        let characters = self.system.len()
            + tools::definitions().to_string().len()
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
        self.tokens = saved["tokens"].as_u64();
        true
    }
}

/// What the conversation may fill: a quarter of the context stays for the
/// model's reasoning and reply.
fn budget(context: u64) -> u64 {
    context * 3 / 4
}

/// A tool call as the model writes it.
struct ToolProgress {
    name: String,
    path: Option<String>,
    bytes: usize,
}

/// What the spinner says while the model works, once it has taken a moment:
/// a long silence looks like a hang otherwise.
fn working_label(started: Instant, writing: Option<&ToolProgress>, used: Option<&str>) -> String {
    let elapsed = started.elapsed();
    if elapsed < LABEL_AFTER {
        return String::new();
    }
    // A server may hold a tool call's arguments back until they're complete:
    // while nothing arrives to count, only the time, rather than a size and a
    // token count that sit still.
    let waiting_on_arguments = writing.is_some_and(|call| call.bytes == 0);
    let time = match used {
        Some(used) if !waiting_on_arguments => format!("{} · {used}", text::duration(elapsed)),
        _ => text::duration(elapsed),
    };
    // On one line, or the spinner's redraw would leave the rest behind.
    let room = style::terminal_width().map_or(80, |width| width.saturating_sub(4));
    let label = match writing {
        None => format!("{} · {time}", text::THINKING),
        Some(call) => {
            let label_with = |path: Option<&str>| {
                let what = text::writing_tool(&call.name, path, call.bytes);
                format!("{what} · {time}")
            };
            // A long path loses its start: the file's name, its size and the time stay.
            let spare = room.saturating_sub(label_with(Some("")).chars().count());
            let path = call
                .path
                .as_deref()
                .map(|path| tail(&config::shown(Path::new(path)), spare));
            label_with(path.as_deref())
        }
    };
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
                    "function": {"name": call.name, "arguments": call.arguments}
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
            system: String::new(),
            messages: vec![json!({"role": "user", "content": "read it"})],
            tokens: None,
            model_time: None,
            show_reasoning: false,
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
    fn room_for_output_is_unknown_without_the_context() {
        let mut conversation = conversation(8192);
        conversation.server.context = None;
        assert_eq!(conversation.room_for_output(), None);
    }
}
