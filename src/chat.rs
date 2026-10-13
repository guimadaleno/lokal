//! One streamed chat request. It runs on its own thread, so the caller can
//! animate the spinner meanwhile and walk away from it on Ctrl+C.

use std::io::BufRead;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::answer::AnswerFilter;
use crate::{http, text};

#[derive(Default, Clone)]
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
}

pub enum Event {
    /// More of the visible answer.
    Text(String),
    /// The model began calling a tool.
    ToolCall,
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
        add_piece(&piece, &mut reply, &mut filter, sender);
    }
    let rest = filter.finish();
    if !rest.is_empty() {
        reply.content.push_str(&rest);
        let _ = sender.send(Event::Text(rest));
    }
    Ok(reply)
}

fn add_piece(piece: &Value, reply: &mut Reply, filter: &mut AnswerFilter, sender: &Sender<Event>) {
    if let Some(tokens) = piece["usage"]["total_tokens"].as_u64() {
        reply.tokens = Some(tokens);
    }
    let delta = &piece["choices"][0]["delta"];
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
            let _ = sender.send(Event::ToolCall);
        }
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
    }
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
        let pieces = [
            json!({"choices": [{"delta": {"reasoning_content": "hmm"}}]}),
            json!({"choices": [{"delta": {"content": "\n\n"}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "41", "function": {"name": "run_command", "arguments": ""}}]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\"command\":"}}]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "\"ls\"}"}}]}}]}),
            json!({"choices": [], "usage": {"total_tokens": 386}}),
        ];
        for piece in &pieces {
            add_piece(piece, &mut reply, &mut filter, &sender);
        }
        assert_eq!(reply.content, "");
        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].id, "41");
        assert_eq!(reply.tool_calls[0].name, "run_command");
        assert_eq!(reply.tool_calls[0].arguments, "{\"command\":\"ls\"}");
        assert_eq!(reply.tokens, Some(386));
        assert!(matches!(events.try_recv(), Ok(Event::ToolCall)));
        assert!(events.try_recv().is_err());
    }
}
