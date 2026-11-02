//! One streamed chat request. It runs on its own thread, so the caller can
//! animate the spinner meanwhile and walk away from it on Ctrl+C.

use std::io::BufRead;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::answer::AnswerFilter;
use crate::{http, text};

#[derive(Default)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON text, as the model wrote it.
    pub arguments: String,
}

#[derive(Default)]
pub struct Reply {
    /// The visible answer: no reasoning, no surrounding blank space.
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    /// Tokens the conversation takes, as the server counted them.
    pub tokens: Option<u64>,
    /// Why the model stopped, as the server says it: "stop", "tool_calls",
    /// or "length" when it ran out of room.
    pub finish: Option<String>,
    /// Tokens the reply generated: the server's count when it gives one,
    /// one a piece otherwise.
    pub generated: u64,
    /// From the first token to the last, so the time the server takes to
    /// read the prompt isn't in it.
    pub generating: Duration,
}

pub enum Event {
    /// More of the visible answer.
    Text(String),
    /// More of the model's reasoning, which shows only when the person asked.
    Reasoning(String),
    /// The model is writing a tool call: which tool, how much of its
    /// arguments so far, and the path among them once it has arrived.
    ToolCall {
        name: String,
        path: Option<String>,
        bytes: usize,
    },
    Done(Result<Reply, String>),
}

pub struct Request {
    events: Receiver<Event>,
    abandoned: Arc<AtomicBool>,
}

pub fn start(address: &str, body: Value) -> Request {
    let (sender, events) = mpsc::channel();
    let abandoned = Arc::new(AtomicBool::new(false));
    let address = address.to_string();
    let worker_abandoned = abandoned.clone();
    thread::spawn(move || {
        let result = stream(&address, &body, &sender, &worker_abandoned);
        let _ = sender.send(Event::Done(result));
    });
    Request { events, abandoned }
}

impl Request {
    /// The next event, or None if nothing came within `wait`.
    pub fn next(&self, wait: Duration) -> Option<Event> {
        match self.events.recv_timeout(wait) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                Some(Event::Done(Err(text::CONNECTION_DROPPED.into())))
            }
        }
    }
}

impl Drop for Request {
    /// The worker stops reading at the next piece; closing the connection
    /// tells the server to stop generating.
    fn drop(&mut self) {
        self.abandoned.store(true, Ordering::SeqCst);
    }
}

fn stream(
    address: &str,
    body: &Value,
    sender: &Sender<Event>,
    abandoned: &AtomicBool,
) -> Result<Reply, String> {
    let reader = http::post_json(address, "/v1/chat/completions", body)?;
    let mut reply = Reply::default();
    let mut filter = AnswerFilter::new();
    let mut paths = Vec::new();
    let mut first_token = None;
    // Server-sent events: one `data: {json}` line per piece, then `data: [DONE]`.
    for line in reader.lines() {
        if abandoned.load(Ordering::SeqCst) {
            return Err(String::new());
        }
        let line = line.map_err(|_| text::CONNECTION_DROPPED)?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let piece: Value = serde_json::from_str(data).map_err(|_| text::not_json(address))?;
        if let Some(message) = http::error_message(&piece) {
            return Err(text::server_said(&message));
        }
        if add_piece(&piece, &mut reply, &mut filter, &mut paths, sender) {
            let now = Instant::now();
            reply.generating = now - *first_token.get_or_insert(now);
        }
    }
    let rest = filter.finish();
    if !rest.is_empty() {
        reply.content.push_str(&rest);
        let _ = sender.send(Event::Text(rest));
    }
    Ok(reply)
}

/// Adds one piece of the stream to the reply, with `paths` following each
/// tool call's path as its arguments arrive. True when the model generated
/// something in it, rather than only the usage or a finish.
fn add_piece(
    piece: &Value,
    reply: &mut Reply,
    filter: &mut AnswerFilter,
    paths: &mut Vec<PathSearch>,
    sender: &Sender<Event>,
) -> bool {
    let delta = &piece["choices"][0]["delta"];
    let has_text = |field: &str| delta[field].as_str().is_some_and(|text| !text.is_empty());
    let generated = has_text("reasoning_content")
        || has_text("content")
        || delta["tool_calls"]
            .as_array()
            .is_some_and(|calls| !calls.is_empty());
    if generated {
        reply.generated += 1;
    }
    // A piece isn't always one token: the server's count, at the end, wins.
    if let Some(tokens) = piece["usage"]["completion_tokens"].as_u64() {
        reply.generated = tokens;
    }
    if let Some(tokens) = piece["usage"]["total_tokens"].as_u64() {
        reply.tokens = Some(tokens);
    }
    if let Some(finish) = piece["choices"][0]["finish_reason"].as_str() {
        reply.finish = Some(finish.to_string());
    }
    if let Some(reasoning) = delta["reasoning_content"]
        .as_str()
        .filter(|text| !text.is_empty())
    {
        let _ = sender.send(Event::Reasoning(reasoning.to_string()));
    }
    if let Some(content) = delta["content"].as_str() {
        let shown = filter.push(content);
        if !shown.is_empty() {
            reply.content.push_str(&shown);
            let _ = sender.send(Event::Text(shown));
        }
    }
    // A tool call arrives in pieces: its id and name first, then its arguments bit by bit.
    for call in delta["tool_calls"].as_array().into_iter().flatten() {
        let index = call["index"].as_u64().unwrap_or(0) as usize;
        while reply.tool_calls.len() <= index {
            reply.tool_calls.push(ToolCall {
                id: format!("call_{}", reply.tool_calls.len()),
                ..ToolCall::default()
            });
        }
        paths.resize_with(reply.tool_calls.len(), PathSearch::default);
        let target = &mut reply.tool_calls[index];
        if let Some(id) = call["id"].as_str().filter(|id| !id.is_empty()) {
            target.id = id.to_string();
        }
        if let Some(name) = call["function"]["name"]
            .as_str()
            .filter(|name| !name.is_empty())
        {
            target.name = name.to_string();
        }
        if let Some(arguments) = call["function"]["arguments"].as_str() {
            target.arguments.push_str(arguments);
        }
        let _ = sender.send(Event::ToolCall {
            name: target.name.clone(),
            path: paths[index].update(&target.arguments),
            bytes: target.arguments.len(),
        });
    }
    generated
}

const PATH_KEY: &str = "\"path\"";

/// A tool call's path, looked for in each new piece of its arguments
/// rather than in all of them every time: a long file can come in
/// thousands of pieces, and its path after its content.
#[derive(Default)]
struct PathSearch {
    /// How far the arguments have been looked through for the key.
    searched: usize,
    /// Where the key is, once found.
    key: Option<usize>,
    path: Option<String>,
}

impl PathSearch {
    fn update(&mut self, arguments: &str) -> Option<String> {
        if self.path.is_none() {
            if self.key.is_none() {
                // Back a little, for a key split between two pieces.
                let from =
                    arguments.floor_char_boundary(self.searched.saturating_sub(PATH_KEY.len()));
                self.key = arguments[from..].find(PATH_KEY).map(|at| from + at);
                self.searched = arguments.len();
            }
            self.path = self.key.and_then(|key| path_in(&arguments[key..]));
        }
        self.path.clone()
    }
}

/// The `"path"` in a tool call's arguments, read from the JSON while it is
/// still arriving: None until its closing quote is there.
fn path_in(arguments: &str) -> Option<String> {
    let after_key = &arguments[arguments.find(PATH_KEY)? + PATH_KEY.len()..];
    let value = after_key
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    let mut path = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(path),
            '\\' => path.push(chars.next()?),
            c => path.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn assembles_tool_calls_and_usage_from_pieces() {
        let (sender, events) = mpsc::channel();
        let mut reply = Reply::default();
        let mut filter = AnswerFilter::new();
        let mut paths = Vec::new();
        let pieces = [
            json!({"choices": [{"delta": {"reasoning_content": "hmm"}}]}),
            json!({"choices": [{"delta": {"content": "\n\n"}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "41", "function": {"name": "run_command", "arguments": ""}}]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\"command\":"}}]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "\"ls\"}"}}]}}]}),
            json!({"choices": [], "usage": {"total_tokens": 386}}),
        ];
        for piece in &pieces {
            add_piece(piece, &mut reply, &mut filter, &mut paths, &sender);
        }
        assert_eq!(reply.content, "");
        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].id, "41");
        assert_eq!(reply.tool_calls[0].name, "run_command");
        assert_eq!(reply.tool_calls[0].arguments, "{\"command\":\"ls\"}");
        assert_eq!(reply.tokens, Some(386));
        let mut last = None;
        while let Ok(event) = events.try_recv() {
            last = Some(event);
        }
        assert!(
            matches!(last, Some(Event::ToolCall { ref name, bytes: 16, .. }) if name == "run_command")
        );
    }

    #[test]
    fn counts_a_token_a_piece_until_the_server_counts() {
        let (sender, _events) = mpsc::channel();
        let mut reply = Reply::default();
        let mut filter = AnswerFilter::new();
        let mut paths = Vec::new();
        let generated = [
            json!({"choices": [{"delta": {"reasoning_content": "hmm"}}]}),
            json!({"choices": [{"delta": {"content": "Hi"}}]}),
        ];
        let not_generated = [
            json!({"choices": [{"delta": {"role": "assistant", "content": ""}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}),
        ];
        for piece in &generated {
            assert!(add_piece(
                piece,
                &mut reply,
                &mut filter,
                &mut paths,
                &sender
            ));
        }
        for piece in &not_generated {
            assert!(!add_piece(
                piece,
                &mut reply,
                &mut filter,
                &mut paths,
                &sender
            ));
        }
        assert_eq!(reply.generated, 2);
        let usage = json!({"choices": [], "usage": {"completion_tokens": 5, "total_tokens": 90}});
        assert!(!add_piece(
            &usage,
            &mut reply,
            &mut filter,
            &mut paths,
            &sender
        ));
        assert_eq!(reply.generated, 5);
    }

    #[test]
    fn finds_the_path_while_the_arguments_arrive() {
        assert_eq!(
            path_in(r#"{"path": "/tmp/a.html", "content": "<h"#),
            Some("/tmp/a.html".into())
        );
        assert_eq!(
            path_in(r#"{"content":"x","path":"~/n\"q.md"}"#),
            Some("~/n\"q.md".into())
        );
        assert_eq!(path_in(r#"{"path": "/tmp/unfini"#), None);
        assert_eq!(path_in(r#"{"command": "ls"}"#), None);
    }

    #[test]
    fn finds_the_path_piece_by_piece_as_a_whole_read_would() {
        let arguments =
            r#"{"content":"ação \"path\" x","pa"#.to_string() + r#"th": "~/n\"q.md", "x": 1}"#;
        let mut search = PathSearch::default();
        let mut so_far = String::new();
        for c in arguments.chars() {
            so_far.push(c);
            assert_eq!(search.update(&so_far), path_in(&so_far), "{so_far}");
        }
        assert_eq!(search.update(&so_far).as_deref(), Some("~/n\"q.md"));
    }
}
