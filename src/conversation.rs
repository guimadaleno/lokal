//! The conversation: the messages so far, one turn at a time, and the copy
//! on disk that `lokal -r` resumes.

use std::fs;
use std::io::Write;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::chat::{self, Event, Reply};
use crate::input::Input;
use crate::server::Server;
use crate::spinner::Spinner;
use crate::system::Facts;
use crate::tools::{self, Outcome};
use crate::{config, interrupt, prompt, style, text};

/// Stands in for a tool's output once it's dropped to free up context.
const DROPPED_OUTPUT: &str = "[Output dropped to free up context.]";
const STOPPED_RESULT: &str = "Stopped by the user.";
const SKIPPED_AFTER_NO: &str = "Not run: the user declined an earlier step.";

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
}

enum Failure {
    Stopped,
    Error(String),
}

impl Conversation {
    pub fn new(server: Server, facts: Facts) -> Self {
        let system = prompt::system_prompt(&facts, config::read_about_me().as_deref());
        Conversation {
            server,
            facts,
            system,
            messages: Vec::new(),
            tokens: None,
            model_time: None,
        }
    }

    /// Picks about-me.md up again after it changed.
    pub fn refresh_system_prompt(&mut self) {
        self.system = prompt::system_prompt(&self.facts, config::read_about_me().as_deref());
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
        for call in &reply.tool_calls {
            // Every call gets a result, even the ones skipped after a stop or a no.
            let result = if stopped || interrupt::happened() {
                stopped = true;
                STOPPED_RESULT.to_string()
            } else if declined {
                SKIPPED_AFTER_NO.to_string()
            } else {
                let outcome = tools::run(call, input);
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
        !stopped && !declined
    }

    /// Streams the model's reply to the screen: the spinner while it thinks
    /// or prepares a tool call, the answer as it's written.
    fn ask_model(&self) -> Result<Reply, Failure> {
        let request = chat::start(&self.server.address, self.request_body());
        let mut spinner = Spinner::new();
        let mut mid_line = false;
        loop {
            if interrupt::happened() {
                spinner.clear();
                if mid_line {
                    println!();
                }
                return Err(Failure::Stopped);
            }
            match request.next(Duration::from_millis(40)) {
                None if !mid_line => spinner.tick(),
                None => {}
                Some(Event::Text(text)) => {
                    spinner.clear();
                    print!("{text}");
                    let _ = std::io::stdout().flush();
                    mid_line = true;
                }
                Some(Event::ToolCall) => {
                    if mid_line {
                        // End the text, then a blank line before the step.
                        println!("\n");
                        mid_line = false;
                    }
                }
                Some(Event::Done(result)) => {
                    spinner.clear();
                    if mid_line {
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
        json!({
            "model": self.server.model,
            "messages": messages,
            "tools": tools::definitions(),
            "stream": true,
            "stream_options": {"include_usage": true}
        })
    }

    /// Near the end of the context, the oldest tool outputs go first: they
    /// take the most room and matter least later on.
    fn free_context(&mut self) {
        let Some(context) = self.server.context else {
            return;
        };
        // Leave a quarter for the model's reasoning and reply.
        let budget = context * 3 / 4;
        for index in 0..self.messages.len() {
            if self.estimated_tokens() <= budget {
                return;
            }
            let message = &mut self.messages[index];
            if message["role"] == "tool" {
                message["content"] = DROPPED_OUTPUT.into();
            }
        }
    }

    /// About three characters a token: rough, and on the high side.
    fn estimated_tokens(&self) -> u64 {
        let characters =
            self.system.len() + serde_json::to_string(&self.messages).map_or(0, |json| json.len());
        characters as u64 / 3
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
