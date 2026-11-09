//! One MCP server: its process, and the JSON-RPC lines between it and
//! Lokal on its stdin and stdout, in either era of the protocol.

use std::collections::VecDeque;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::spinner::Spinner;
use crate::{interrupt, keys, shell, system, text};

/// Sent with `initialize`: the newest revision that starts with it.
const LEGACY_VERSION: &str = "2025-11-25";
/// The revisions that start with `initialize`, all of which Lokal speaks:
/// the tools it uses are the same in each.
const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// The stateless revision: no `initialize`, and `_meta` on every request.
const MODERN_VERSION: &str = "2026-07-28";
/// The longest line a server may send; a result with an image fits.
const LINE_MOST: u64 = 32 * 1024 * 1024;
/// How long a server may take to answer the first request: `npx` may
/// fetch its package first.
const START_TIME: Duration = Duration::from_secs(60);
/// How long any other request but a call may take.
const ANSWER_TIME: Duration = Duration::from_secs(30);
/// The most pages of tools read: a server can't list forever.
const PAGES_MOST: usize = 20;
/// The lines of its stderr kept, to say why a server failed, and the most
/// of a line read at once: a longer one counts as several.
const STDERR_LINES: usize = 8;
const STDERR_WIDTH: usize = 300;
const STDERR_READ: u64 = 64 * 1024;
/// The most servers whose processes a signal handler can end.
const GROUPS_MOST: usize = 64;
/// JSON-RPC's code for a method the other end doesn't have.
const NO_SUCH_METHOD: i64 = -32601;

/// Which revision of the protocol a server speaks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Era {
    /// It starts with `initialize`.
    Legacy,
    /// Stateless: `_meta` on every request.
    Modern,
}

/// The process groups of the servers that may still run, 0 where none:
/// what a handler for SIGHUP or SIGTERM ends, since no destructor runs
/// then. A group leaves once its leader is reaped, so a number reused by
/// another process is never signalled.
static GROUPS: [AtomicI32; GROUPS_MOST] = [const { AtomicI32::new(0) }; GROUPS_MOST];

pub struct Client {
    child: Child,
    /// Lines for the server's stdin, which a thread of their own writes:
    /// a server that stops reading can't hang a turn past Esc. Shared with
    /// the reader, which answers pings; None once closed, which ends the
    /// writer and so closes the stdin.
    lines: Arc<Mutex<Option<Sender<String>>>>,
    replies: Receiver<Value>,
    /// The id of the request waiting for its answer, 0 while none does:
    /// the reader passes on only that, so a server can't fill Lokal's
    /// memory with answers no one asked for.
    awaited: Arc<AtomicU64>,
    era: Era,
    next_id: u64,
    stderr: Arc<Mutex<VecDeque<String>>>,
    ended: bool,
}

#[derive(Debug)]
pub enum Failure {
    /// The person stopped it.
    Stopped,
    /// The server answered with an error.
    Refused(String),
    /// No answer: the server ended, took too long, or sent too much.
    Lost(String),
}

impl Failure {
    /// What went wrong, in words.
    pub fn message(&self) -> String {
        match self {
            Failure::Stopped => text::STOPPED.to_string(),
            Failure::Refused(message) | Failure::Lost(message) => message.clone(),
        }
    }
}

impl Client {
    /// Starts the server `command` names and agrees on the protocol with it.
    /// It starts in the home folder, never the project's, whose own
    /// settings (an `.npmrc`) could change what `npx` fetches and runs; its
    /// program comes from the PATH's absolute folders, as for what runs
    /// unasked; and it runs in a session of its own.
    pub fn start(command: &[String]) -> Result<Client, String> {
        let home = env::var_os("HOME").unwrap_or_default();
        let words: Vec<String> = command
            .iter()
            .map(|word| match word.strip_prefix("~/") {
                Some(rest) => Path::new(&home).join(rest).display().to_string(),
                None => word.clone(),
            })
            .collect();
        let Some((program, arguments)) = words.split_first() else {
            return Err(text::MCP_NO_PROGRAM.to_string());
        };
        // A relative path would be relative to a folder no one chose.
        if program.contains('/') && !program.starts_with('/') {
            return Err(text::mcp_relative_program(program));
        }
        let mut process = Command::new(program);
        process
            .args(arguments)
            .current_dir(&home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path) = env::var_os("PATH") {
            process.env("PATH", system::absolute_only(&path.to_string_lossy()));
        }
        // SAFETY: setsid is safe to call between fork and exec.
        unsafe {
            process.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let mut child = process
            .spawn()
            .map_err(|error| text::mcp_cant_start(program, &error.to_string()))?;
        watch_group(child.id() as i32);
        let (writes, written) = mpsc::channel::<String>();
        if let Some(stdin) = child.stdin.take() {
            thread::spawn(move || write_lines(stdin, &written));
        }
        let lines = Arc::new(Mutex::new(Some(writes)));
        let awaited = Arc::new(AtomicU64::new(0));
        // One answer at a time; a server that sends more waits for room.
        let (sender, replies) = mpsc::sync_channel(1);
        if let Some(stdout) = child.stdout.take() {
            let answers = Arc::clone(&lines);
            let awaited = Arc::clone(&awaited);
            thread::spawn(move || read_messages(stdout, &sender, &answers, &awaited));
        }
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(output) = child.stderr.take() {
            let kept = Arc::clone(&stderr);
            thread::spawn(move || keep_stderr(output, &kept));
        }
        let mut client = Client {
            child,
            lines,
            replies,
            awaited,
            era: Era::Legacy,
            next_id: 1,
            stderr,
            ended: false,
        };
        match client.agree() {
            Ok(()) => Ok(client),
            Err(failure) => Err(client.with_stderr(&failure.message())),
        }
    }

    #[cfg(test)]
    pub fn era(&self) -> Era {
        self.era
    }

    /// `initialize` first, which servers of both eras but the newest answer,
    /// and `server/discover` when it fails. Probing with `server/discover`
    /// first, as the 2026-07-28 revision suggests, would cost a timeout at
    /// every start on the many servers that answer nothing before
    /// `initialize`.
    fn agree(&mut self) -> Result<(), Failure> {
        let params = json!({
            "protocolVersion": LEGACY_VERSION,
            "capabilities": {},
            "clientInfo": client_info(),
        });
        match self.request("initialize", params, Some(START_TIME)) {
            Ok(result) => {
                let version = result["protocolVersion"].as_str().unwrap_or_default();
                if !LEGACY_VERSIONS.contains(&version) {
                    return Err(Failure::Refused(text::mcp_unknown_version(version)));
                }
                self.notify("notifications/initialized");
                Ok(())
            }
            Err(Failure::Refused(_)) => {
                self.era = Era::Modern;
                let result = self.request("server/discover", json!({}), Some(ANSWER_TIME))?;
                let supported = result["supportedVersions"].as_array();
                if supported.is_some_and(|versions| versions.iter().any(|v| v == MODERN_VERSION)) {
                    Ok(())
                } else {
                    let versions = supported.map(|v| Value::from(v.clone()).to_string());
                    Err(Failure::Refused(text::mcp_unknown_version(
                        versions.as_deref().unwrap_or("?"),
                    )))
                }
            }
            Err(failure) => Err(failure),
        }
    }

    /// The tools the server lists, page after page.
    pub fn list_tools(&mut self) -> Result<Vec<Value>, Failure> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..PAGES_MOST {
            let params = match &cursor {
                Some(cursor) => json!({"cursor": cursor}),
                None => json!({}),
            };
            let result = self.request("tools/list", params, Some(ANSWER_TIME))?;
            tools.extend(result["tools"].as_array().into_iter().flatten().cloned());
            match result["nextCursor"].as_str() {
                Some(next) if !next.is_empty() => cursor = Some(next.to_string()),
                _ => break,
            }
        }
        Ok(tools)
    }

    /// Calls a tool, for as long as it takes; Esc or Ctrl+C cancels it.
    pub fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<Value, Failure> {
        let _keys = keys::KeyWatch::start();
        self.request(
            "tools/call",
            json!({"name": name, "arguments": arguments}),
            None,
        )
    }

    /// Sends a request and waits for its answer, until `time` passes when
    /// there's a limit, or the person stops it when there isn't.
    fn request(
        &mut self,
        method: &str,
        mut params: Value,
        time: Option<Duration>,
    ) -> Result<Value, Failure> {
        let id = self.next_id;
        self.next_id += 1;
        if self.era == Era::Modern {
            params["_meta"] = json!({
                "io.modelcontextprotocol/protocolVersion": MODERN_VERSION,
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": client_info(),
            });
        }
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.awaited.store(id, Ordering::SeqCst);
        if !self.send(&message) {
            return Err(Failure::Lost(text::MCP_ENDED.to_string()));
        }
        let started = Instant::now();
        // A call can take long: the spinner shows it's still going.
        let mut spinner = time.is_none().then(Spinner::new);
        let reply = self.wait_for(id, method, time, started, spinner.as_mut());
        self.awaited.store(0, Ordering::SeqCst);
        if let Some(spinner) = &mut spinner {
            spinner.clear();
        }
        reply
    }

    fn wait_for(
        &mut self,
        id: u64,
        method: &str,
        time: Option<Duration>,
        started: Instant,
        mut spinner: Option<&mut Spinner>,
    ) -> Result<Value, Failure> {
        loop {
            if let Some(spinner) = spinner.as_deref_mut() {
                spinner.tick();
            }
            if time.is_none() && interrupt::happened() {
                self.send(&json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/cancelled",
                    "params": {"requestId": id, "reason": text::STOPPED},
                }));
                return Err(Failure::Stopped);
            }
            if time.is_some_and(|time| started.elapsed() > time) {
                return Err(Failure::Lost(text::mcp_no_answer(method)));
            }
            match self.replies.recv_timeout(Duration::from_millis(50)) {
                Ok(reply) if reply["id"] == id => return answer(reply),
                // An answer to a request given up on.
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Failure::Lost(text::MCP_ENDED.to_string()));
                }
            }
        }
    }

    fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc": "2.0", "method": method}));
    }

    /// One line for the server's stdin, never waiting on it. False once
    /// it's closed or gone.
    fn send(&self, message: &Value) -> bool {
        queue_line(&self.lines, message)
    }

    /// `message`, with the last lines the server wrote to stderr, which
    /// usually say why it failed.
    pub fn with_stderr(&self, message: &str) -> String {
        let tail = self
            .stderr
            .lock()
            .map(|tail| tail.clone())
            .unwrap_or_default();
        let lines: Vec<String> = tail.into_iter().collect();
        text::mcp_with_stderr(message, &lines)
    }

    /// Closes its stdin, once what's queued is written, which asks the
    /// server to end.
    pub fn close(&mut self) {
        if let Ok(mut lines) = self.lines.lock() {
            lines.take();
        }
    }

    /// Whether the server has ended; once it has, it's reaped and its group
    /// no longer signalled.
    pub fn has_ended(&mut self) -> bool {
        if !self.ended && !matches!(self.child.try_wait(), Ok(None)) {
            self.ended = true;
            unwatch_group(self.child.id() as i32);
        }
        self.ended
    }

    /// A signal to the server and everything it started.
    pub fn signal(&self, signal: libc::c_int) {
        shell::signal_group(self.child.id(), signal);
    }
}

/// A server dropped while it runs, one the person said no to or that
/// started too late, is asked to end, and told to.
impl Drop for Client {
    fn drop(&mut self) {
        self.close();
        if !self.has_ended() {
            self.signal(libc::SIGTERM);
        }
    }
}

/// The same name and version in both eras.
fn client_info() -> Value {
    json!({"name": text::NAME, "version": text::VERSION})
}

/// A request's result, or the server's error as words.
fn answer(reply: Value) -> Result<Value, Failure> {
    match reply.get("error") {
        Some(error) => {
            let message = error["message"].as_str().unwrap_or_default();
            Err(Failure::Refused(text::mcp_refused(message)))
        }
        None => Ok(reply.get("result").cloned().unwrap_or(Value::Null)),
    }
}

/// Queues a line for the writer. False once the stdin is closed or the
/// writer gave up.
fn queue_line(lines: &Mutex<Option<Sender<String>>>, message: &Value) -> bool {
    let Ok(lines) = lines.lock() else {
        return false;
    };
    // serde_json writes no line break inside a message.
    lines
        .as_ref()
        .is_some_and(|lines| lines.send(format!("{message}\n")).is_ok())
}

/// Writes queued lines to the server's stdin, for as long as it reads them;
/// when the queue closes, the stdin does, which asks the server to end.
fn write_lines(mut stdin: ChildStdin, lines: &Receiver<String>) {
    for line in lines {
        if stdin.write_all(line.as_bytes()).is_err() {
            return;
        }
    }
}

/// Ends every server still running with `signal`: from a signal handler,
/// where only atomics and `kill` may be used, or as Lokal quits.
pub fn signal_all(signal: libc::c_int) {
    for group in &GROUPS {
        let pid = group.load(Ordering::SeqCst);
        if pid > 0 {
            // SAFETY: kill is async-signal-safe; the group is a server's.
            unsafe {
                libc::kill(-pid, signal);
            }
        }
    }
}

/// A closed terminal (SIGHUP) or a SIGTERM ends Lokal without its
/// destructors: first, the servers get a SIGTERM, then Lokal ends as the
/// signal would have ended it.
pub fn end_with_lokal() {
    let handler: extern "C" fn(libc::c_int) = on_ending;
    // SAFETY: sigaction is plain data, zeroed then filled in; the handler
    // only reads atomics and calls kill, signal and raise, all safe there.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGHUP, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
    }
}

extern "C" fn on_ending(signal: libc::c_int) {
    signal_all(libc::SIGTERM);
    // SAFETY: both are async-signal-safe.
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

fn watch_group(pid: i32) {
    for group in &GROUPS {
        if group
            .compare_exchange(0, pid, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return;
        }
    }
}

fn unwatch_group(pid: i32) {
    for group in &GROUPS {
        let _ = group.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// Reads the server's stdout, a JSON message a line: answers go to the
/// client, a request of the server's is answered here, and the rest is
/// left. It ends with the server, or at a line over `LINE_MOST`.
fn read_messages(
    stdout: ChildStdout,
    replies: &SyncSender<Value>,
    lines: &Mutex<Option<Sender<String>>>,
    awaited: &AtomicU64,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = Vec::new();
        match (&mut reader)
            .take(LINE_MOST + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) | Err(_) => return,
            Ok(read) if read as u64 > LINE_MOST => return,
            Ok(_) => {}
        }
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        match (message.get("method"), message.get("id")) {
            // Lokal offers nothing a server may ask for but a ping.
            (Some(method), Some(id)) => {
                let reply = if method == "ping" {
                    json!({"jsonrpc": "2.0", "id": id, "result": {}})
                } else {
                    json!({"jsonrpc": "2.0", "id": id,
                           "error": {"code": NO_SUCH_METHOD, "message": "Method not found"}})
                };
                queue_line(lines, &reply);
            }
            // A notification: a log line, a list that changed.
            (Some(_), None) => {}
            // Only the answer awaited; any other is dropped.
            (None, Some(id))
                if id
                    .as_u64()
                    .is_some_and(|id| id != 0 && id == awaited.load(Ordering::SeqCst)) =>
            {
                if replies.send(message).is_err() {
                    return;
                }
            }
            (None, Some(_)) => {}
            (None, None) => {}
        }
    }
}

/// Keeps the last lines of the server's stderr, each cut short, reading
/// no more than `STDERR_READ` of one at once.
fn keep_stderr(stderr: ChildStderr, kept: &Mutex<VecDeque<String>>) {
    let mut reader = BufReader::new(stderr);
    loop {
        let mut line = Vec::new();
        match (&mut reader).take(STDERR_READ).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line: String = String::from_utf8_lossy(&line)
            .trim_end()
            .chars()
            .take(STDERR_WIDTH)
            .collect();
        if line.is_empty() {
            continue;
        }
        if let Ok(mut kept) = kept.lock() {
            if kept.len() == STDERR_LINES {
                kept.pop_front();
            }
            kept.push_back(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(era: &str) -> Vec<String> {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/mcp/fake_server.py");
        vec!["python3".into(), script.into(), era.into()]
    }

    #[test]
    fn speaks_to_a_server_that_starts_with_initialize() {
        let mut client = Client::start(&fake("legacy")).unwrap();
        assert_eq!(client.era(), Era::Legacy);
        // One tool a page, so the cursor is followed.
        let tools = client.list_tools().unwrap();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names, ["echo", "picture", "slow"]);
        let result = client.call_tool("echo", &json!({"text": "hi"})).unwrap();
        assert_eq!(result["content"][0]["text"], "hi");
    }

    #[test]
    fn speaks_to_a_stateless_server_with_meta_on_every_request() {
        let mut client = Client::start(&fake("modern")).unwrap();
        assert_eq!(client.era(), Era::Modern);
        // The fake refuses any request without the version in `_meta`.
        assert_eq!(client.list_tools().unwrap().len(), 3);
        let result = client.call_tool("echo", &json!({"text": "hi"})).unwrap();
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["content"][0]["text"], "hi");
    }

    #[test]
    fn a_server_that_fails_says_why_with_its_stderr() {
        let error = Client::start(&fake("crash")).err().unwrap();
        assert!(error.contains("ended"), "{error}");
        assert!(error.contains("missing token"), "{error}");
        let error = Client::start(&["no-such-program-of-lokal".into()])
            .err()
            .unwrap();
        assert!(error.contains("no-such-program-of-lokal"), "{error}");
        let error = Client::start(&["bin/server".into()]).err().unwrap();
        assert!(error.contains("absolute"), "{error}");
    }

    #[test]
    fn a_tool_that_fails_comes_back_as_the_servers_words() {
        let mut client = Client::start(&fake("legacy")).unwrap();
        let failure = client.call_tool("missing", &json!({})).unwrap_err();
        assert!(failure.message().contains("Unknown tool: missing"));
    }

    #[test]
    fn closing_its_input_ends_the_server() {
        let mut client = Client::start(&fake("legacy")).unwrap();
        client.close();
        let started = Instant::now();
        while !client.has_ended() && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(client.has_ended());
    }

    #[test]
    fn a_server_ended_with_lokal_is_no_longer_signalled() {
        let mut client = Client::start(&fake("legacy")).unwrap();
        let pid = client.child.id() as i32;
        let watched = || GROUPS.iter().any(|g| g.load(Ordering::SeqCst) == pid);
        assert!(watched());
        client.close();
        let started = Instant::now();
        while !client.has_ended() && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!watched());
    }

    #[test]
    fn a_server_that_stops_reading_can_still_be_stopped() {
        let mut client = Client::start(&fake("stuck")).unwrap();
        // It reads nothing more: a call larger than the pipe fills it, and
        // Lokal waits on the answer, never on the pipe.
        let started = Instant::now();
        let failure = client.request(
            "tools/call",
            json!({"name": "slow", "arguments": {"text": "x".repeat(1_000_000)}}),
            Some(Duration::from_millis(300)),
        );
        assert!(matches!(failure, Err(Failure::Lost(_))));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
