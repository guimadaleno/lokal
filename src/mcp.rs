//! The MCP servers the person set up: started with a conversation, their
//! tools offered to the model once the person has seen what they say, and
//! ended with Lokal.

mod client;
mod seen;

use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::config::McpServer;
use crate::input::{Approval, Input};
use crate::spinner::Spinner;
use crate::{config, context, interrupt, keys, layout, style, text, tty};
use client::Client;
pub use client::Failure;

/// The lines of a new tool list shown before the question; "Show the rest"
/// prints the others.
const REVIEW_SHOWN: usize = 24;
/// The chat API's longest tool name.
const NAME_MOST: usize = 64;
/// How long servers may take to end once asked, before they're made to.
const END_TIME: Duration = Duration::from_millis(500);

/// A server that started, and the tools it listed; or why it didn't.
type Start = Result<(Client, Vec<Value>), String>;

pub struct Mcp {
    /// Each server as set up, and its start on a thread of its own, until
    /// `ready` takes them.
    starting: Vec<(McpServer, JoinHandle<Start>)>,
    servers: Vec<Server>,
    tools: Vec<Tool>,
    ready: bool,
}

struct Server {
    name: String,
    /// None once it's left out.
    client: Option<Client>,
    state: State,
}

enum State {
    /// Its tools go to the model: how many, and about their tokens.
    Offered { tools: usize, tokens: u64 },
    /// Why they don't.
    LeftOut(&'static str),
}

pub struct Tool {
    /// As the model calls it: `<server>__<tool>`, in what the chat API takes.
    pub exposed: String,
    /// As the server knows it.
    pub name: String,
    server: usize,
    definition: Value,
}

impl Mcp {
    /// Starts each server on a thread of its own; None when there's none.
    pub fn start(servers: &[McpServer]) -> Option<Mcp> {
        if servers.is_empty() {
            return None;
        }
        let starting = servers
            .iter()
            .map(|server| {
                let command = server.command.clone();
                let start = thread::spawn(move || {
                    let mut client = Client::start(&command)?;
                    match client.list_tools() {
                        Ok(tools) => Ok((client, tools)),
                        Err(failure) => Err(client.with_stderr(&failure.message())),
                    }
                });
                (server.clone(), start)
            })
            .collect();
        Some(Mcp {
            starting,
            servers: Vec::new(),
            tools: Vec::new(),
            ready: false,
        })
    }

    /// The first time a request needs the tools: waits for the servers to
    /// start, and shows the person each tool list that's new or changed
    /// before the model gets it. True when that happened now.
    pub fn ready(&mut self, input: &mut Input) -> bool {
        if self.ready {
            return false;
        }
        self.ready = true;
        for (setup, start) in wait_for(std::mem::take(&mut self.starting)) {
            let server = match start {
                Ok((client, listed)) => self.offer(&setup, client, &listed, input),
                Err(reason) => {
                    // Lokal's words, then the server's last ones, which are its own.
                    let mut lines = reason.lines();
                    let why = lines.next().unwrap_or_default();
                    layout::print_fitted(&text::mcp_couldnt_start(&setup.name, why), style::red);
                    for line in lines {
                        eprintln!("  {}", style::gray(&layout::printable(line)));
                    }
                    eprintln!();
                    left_out(&setup.name, text::MCP_COULDNT_START)
                }
            };
            self.servers.push(server);
        }
        true
    }

    /// The tools for the chat API, the same all session.
    pub fn definitions(&self) -> impl Iterator<Item = &Value> {
        self.tools.iter().map(|tool| &tool.definition)
    }

    /// The tool the model calls `exposed`, if one is offered.
    pub fn find(&self, exposed: &str) -> Option<usize> {
        self.tools.iter().position(|tool| tool.exposed == exposed)
    }

    pub fn tool(&self, index: usize) -> &Tool {
        &self.tools[index]
    }

    pub fn server_name(&self, tool: &Tool) -> &str {
        &self.servers[tool.server].name
    }

    /// Calls a tool, for as long as it takes; Esc or Ctrl+C cancels it.
    pub fn call(&mut self, index: usize, arguments: &Value) -> Result<Value, Failure> {
        let tool = &self.tools[index];
        let name = tool.name.clone();
        match self.servers[tool.server].client.as_mut() {
            Some(client) => client.call_tool(&name, arguments),
            None => Err(Failure::Lost(text::MCP_ENDED.to_string())),
        }
    }

    /// Each server and how it's doing, for /info.
    pub fn summary(&self) -> Vec<String> {
        if !self.ready {
            return self
                .starting
                .iter()
                .map(|(setup, _)| text::mcp_starts_later(&setup.name))
                .collect();
        }
        self.servers
            .iter()
            .map(|server| match server.state {
                State::Offered { tools, tokens } => text::mcp_offered(&server.name, tools, tokens),
                State::LeftOut(why) => format!("{} · {why}", server.name),
            })
            .collect()
    }

    /// A server that started: its tools as the model would get them, shown
    /// to the person first when they've changed since the last time.
    fn offer(
        &mut self,
        setup: &McpServer,
        mut client: Client,
        listed: &[Value],
        input: &mut Input,
    ) -> Server {
        let index = self.servers.len();
        let (tools, notes) = offered(setup, listed, index, &self.tools);
        if tools.is_empty() {
            layout::print_fitted(&text::mcp_no_tools(&setup.name), style::gray);
            eprintln!();
            client.close();
            return left_out(&setup.name, text::MCP_NO_TOOLS);
        }
        let definitions: Vec<&Value> = tools.iter().map(|tool| &tool.definition).collect();
        let offered = serde_json::to_string(&definitions).unwrap_or_default();
        let tokens = context::tokens_in(&offered);
        let hash = seen::hash(&setup.command, &offered);
        if !seen::is_seen(&setup.name, &hash) {
            let mut lines = review(&setup.name, &tools, tokens, &notes);
            let rest = lines.split_off(lines.len().min(REVIEW_SHOWN));
            print_lines(&lines);
            eprintln!();
            let approval =
                input.approve_shown(text::OFFER_QUESTION, rest, print_lines, false, false);
            eprintln!();
            if let Approval::Stop = approval {
                interrupt::raise();
            }
            if !matches!(approval, Approval::Yes) {
                client.close();
                return left_out(&setup.name, text::MCP_DECLINED);
            }
            if let Err(error) = seen::keep(&setup.name, &hash) {
                let path = config::shown(&config::mcp_seen_path());
                layout::print_fitted(&text::cant_save(&path, &error.to_string()), style::red);
            }
        }
        let count = tools.len();
        self.tools.extend(tools);
        Server {
            name: setup.name.clone(),
            client: Some(client),
            state: State::Offered {
                tools: count,
                tokens,
            },
        }
    }
}

/// Ends every server: its stdin closed, which asks it to end, then a
/// SIGTERM, then a SIGKILL, each after `END_TIME`.
impl Drop for Mcp {
    fn drop(&mut self) {
        let mut clients: Vec<&mut Client> = self
            .servers
            .iter_mut()
            .filter_map(|server| server.client.as_mut())
            .collect();
        for client in &mut clients {
            client.close();
        }
        for signal in [libc::SIGTERM, libc::SIGKILL] {
            let asked = Instant::now();
            while asked.elapsed() < END_TIME && clients.iter_mut().any(|c| !c.has_ended()) {
                thread::sleep(Duration::from_millis(20));
            }
            for client in &mut clients {
                if !client.has_ended() {
                    client.signal(signal);
                }
            }
        }
    }
}

fn left_out(name: &str, why: &'static str) -> Server {
    Server {
        name: name.to_string(),
        client: None,
        state: State::LeftOut(why),
    }
}

/// Waits for the servers to start, with the spinner meanwhile. Esc or
/// Ctrl+C stops waiting: those still starting are left out.
fn wait_for(starting: Vec<(McpServer, JoinHandle<Start>)>) -> Vec<(McpServer, Start)> {
    if starting.iter().any(|(_, start)| !start.is_finished()) {
        let _keys = keys::KeyWatch::start();
        let mut spinner = Spinner::new();
        let label = format!(" {}", style::faint(text::MCP_STARTING));
        while !interrupt::happened() && starting.iter().any(|(_, start)| !start.is_finished()) {
            spinner.tick_with(&label);
            thread::sleep(Duration::from_millis(40));
        }
        spinner.clear();
    }
    starting
        .into_iter()
        .map(|(setup, start)| {
            let start = if start.is_finished() {
                start
                    .join()
                    .unwrap_or_else(|_| Err(text::MCP_FAILED.to_string()))
            } else {
                // Its thread ends on its own, and its server with it.
                Err(text::STOPPED.to_string())
            };
            (setup, start)
        })
        .collect()
}

/// The tools a server listed, as the model gets them: only those
/// `mcp.<name>.tools` names, when it's set, each with a name the chat API
/// takes, unique among all the servers'. And notes on the names that went
/// wrong.
fn offered(
    setup: &McpServer,
    listed: &[Value],
    server: usize,
    taken: &[Tool],
) -> (Vec<Tool>, Vec<String>) {
    let mut tools: Vec<Tool> = Vec::new();
    let mut notes = Vec::new();
    for tool in listed {
        let Some(name) = tool["name"].as_str() else {
            continue;
        };
        if let Some(wanted) = &setup.tools
            && !wanted.iter().any(|wanted| wanted == name)
        {
            continue;
        }
        let exposed = exposed_name(&setup.name, name);
        if taken
            .iter()
            .chain(&tools)
            .any(|tool| tool.exposed == exposed)
        {
            notes.push(text::mcp_same_name(name));
            continue;
        }
        let mut parameters = tool
            .get("inputSchema")
            .filter(|schema| schema.is_object())
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
        if let Some(schema) = parameters.as_object_mut() {
            schema.remove("$schema");
        }
        let mut function = json!({"name": exposed, "parameters": parameters});
        if let Some(description) = tool["description"].as_str() {
            function["description"] = description.into();
        }
        tools.push(Tool {
            exposed,
            name: name.to_string(),
            server,
            definition: json!({"type": "function", "function": function}),
        });
    }
    for wanted in setup.tools.iter().flatten() {
        if !listed.iter().any(|tool| tool["name"] == wanted.as_str()) {
            notes.push(text::mcp_no_such_tool(wanted));
        }
    }
    (tools, notes)
}

/// `<server>__<tool>`, with what the chat API doesn't take in a name made
/// `_`, and no longer than it takes.
fn exposed_name(server: &str, tool: &str) -> String {
    format!("{server}__{tool}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(NAME_MOST)
        .collect()
}

/// What a server tells the model about its tools, as the person sees it
/// before the model does: every word in each tool's description and
/// parameters, with control characters as marks.
fn review(name: &str, tools: &[Tool], tokens: u64, notes: &[String]) -> Vec<String> {
    let width = tty::width().unwrap_or(80);
    // The heading after a faint `·`, its later rows under its start.
    let heading = text::mcp_offers(name, tools.len(), tokens);
    let mut lines: Vec<String> = layout::broken(&heading, width.saturating_sub(3))
        .iter()
        .enumerate()
        .map(|(row, part)| {
            let mark = if row == 0 {
                style::faint("·")
            } else {
                " ".into()
            };
            format!("{mark} {}", style::gray(part))
        })
        .collect();
    for note in notes {
        lines.extend(wrapped(note, 2, width, style::gray));
    }
    for tool in tools {
        lines.push(format!("  {}", layout::printable(&tool.name)));
        let function = &tool.definition["function"];
        if let Some(description) = function["description"].as_str() {
            lines.extend(wrapped(description, 4, width, style::gray));
        }
        let mut words = Vec::new();
        words_in(&function["parameters"], "", &mut words);
        for (path, text) in words {
            let line = if path.is_empty() {
                text
            } else {
                format!("{path}: {text}")
            };
            lines.extend(wrapped(&line, 4, width, style::gray));
        }
    }
    lines
}

/// `text` with control characters as marks, broken to fit under `indent`,
/// each row painted on its own.
fn wrapped(text: &str, indent: usize, width: usize, paint: fn(&str) -> String) -> Vec<String> {
    let pad = " ".repeat(indent);
    layout::indented(&layout::printable(text), indent, width)
        .lines()
        .map(|line| format!("{pad}{}", paint(line.trim_start())))
        .collect()
}

/// Every string in a parameters schema, which all reach the model, with
/// where it is: a parameter's name, or the key it's under. Only a `type`
/// and the `required` names, words of JSON Schema's own, are left out.
fn words_in(value: &Value, path: &str, words: &mut Vec<(String, String)>) {
    match value {
        Value::String(text) => words.push((path.to_string(), text.clone())),
        Value::Array(items) => {
            for item in items {
                words_in(item, path, words);
            }
        }
        Value::Object(fields) => {
            for (key, inner) in fields {
                match key.as_str() {
                    "type" | "required" => {}
                    // Each named schema in them on its own path, a parameter
                    // called `type` included.
                    "properties" | "patternProperties" | "$defs" | "definitions" => {
                        for (name, schema) in inner.as_object().into_iter().flatten() {
                            words_in(schema, &joined(path, name), words);
                        }
                    }
                    "items" | "description" | "anyOf" | "oneOf" | "allOf" => {
                        words_in(inner, path, words);
                    }
                    _ => words_in(inner, &joined(path, key), words),
                }
            }
        }
        _ => {}
    }
}

fn joined(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn print_lines(lines: &[String]) {
    for line in lines {
        eprintln!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(tools: Option<&[&str]>) -> McpServer {
        McpServer {
            name: "files".into(),
            command: vec!["x".into()],
            tools: tools.map(|tools| tools.iter().map(|t| t.to_string()).collect()),
        }
    }

    fn listed() -> Vec<Value> {
        vec![
            json!({"name": "read.text", "description": "Reads a file.",
                   "inputSchema": {"$schema": "x", "type": "object",
                                   "properties": {"path": {"type": "string", "description": "Where."}}}}),
            json!({"name": "write", "inputSchema": {"type": "object"}}),
        ]
    }

    #[test]
    fn tools_get_names_the_chat_api_takes() {
        let (tools, notes) = offered(&setup(None), &listed(), 0, &[]);
        let names: Vec<&str> = tools.iter().map(|t| t.exposed.as_str()).collect();
        assert_eq!(names, ["files__read_text", "files__write"]);
        assert_eq!(tools[0].name, "read.text");
        let function = &tools[0].definition["function"];
        assert_eq!(function["description"], "Reads a file.");
        assert!(function["parameters"].get("$schema").is_none());
        assert!(notes.is_empty());
        assert_eq!(exposed_name("s", &"x".repeat(100)).len(), NAME_MOST);
    }

    #[test]
    fn only_the_tools_named_are_offered() {
        let (tools, notes) = offered(&setup(Some(&["write", "delete"])), &listed(), 0, &[]);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "write");
        assert_eq!(notes, [text::mcp_no_such_tool("delete")]);
    }

    #[test]
    fn a_name_taken_already_is_left_out_with_a_note() {
        let (first, _) = offered(&setup(None), &listed(), 0, &[]);
        let twin = vec![json!({"name": "read_text", "inputSchema": {}})];
        let (tools, notes) = offered(&setup(None), &twin, 1, &first);
        assert!(tools.is_empty());
        assert_eq!(notes, [text::mcp_same_name("read_text")]);
    }

    #[test]
    fn every_word_of_the_schema_shows() {
        let schema = json!({"type": "object", "description": "Top.", "required": ["path"],
        "properties": {
            "path": {"type": "string", "description": "Where."},
            "type": {"type": "string", "description": "Named like a keyword."},
            "mode": {"type": "string", "enum": ["fast", "safe"], "default": "safe"},
            "deep": {"type": "object", "properties": {"x": {"type": "string",
                     "description": "Hidden in a nest."}}}
        }});
        let mut words = Vec::new();
        words_in(&schema, "", &mut words);
        let words: Vec<String> = words.iter().map(|(p, t)| format!("{p}: {t}")).collect();
        assert_eq!(
            words,
            [
                ": Top.",
                "deep.x: Hidden in a nest.",
                "mode.default: safe",
                "mode.enum: fast",
                "mode.enum: safe",
                "path: Where.",
                "type: Named like a keyword.",
            ]
        );
    }
}
