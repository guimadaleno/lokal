//! One MCP server and the JSON-RPC between it and Lokal, in either era of
//! the protocol: a process Lokal started, a message a line on its stdin
//! and stdout, or an address, a message a POST (`http.rs`).

use std::collections::{HashMap, VecDeque};
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

use super::http::{self, Http, Param};
use crate::spinner::Spinner;
use crate::{interrupt, keys, shell, system, text};

/// A message from the server, or why the answer to a request (its id)
/// won't come.
pub(super) type Reply = Result<Value, (u64, String)>;

/// Sent with `initialize`: the newest revision that starts with it.
const LEGACY_VERSION: &str = "2025-11-25";
/// The revisions that start with `initialize`, all of which Lokal speaks:
/// the tools it uses are the same in each.
const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// The stateless revision: no `initialize`, and `_meta` on every request.
const MODERN_VERSION: &str = "2026-07-28";
/// The longest line a server may send; a result with an image fits.
pub(super) const LINE_MOST: u64 = 32 * 1024 * 1024;
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
/// How long a server's failure waits for the reader to take in its last
/// stderr lines: a server that dies writes why just before, and the reader
/// may not have it yet.
const STDERR_SETTLE: Duration = Duration::from_millis(300);
/// The most servers whose processes a signal handler can end.
const GROUPS_MOST: usize = 64;
/// JSON-RPC's code for a method the other end doesn't have.
pub(super) const NO_SUCH_METHOD: i64 = -32601;

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
    wire: Wire,
    replies: Receiver<Reply>,
    /// The id of the request waiting for its answer, 0 while none does:
    /// only that is passed on, so a server can't fill Lokal's memory with
    /// answers no one asked for.
    awaited: Arc<AtomicU64>,
    era: Era,
    /// The revision agreed on, which goes with every request over HTTP.
    version: &'static str,
    next_id: u64,
    /// Over HTTP in the 2026-07-28 era: the arguments each tool wants as
    /// headers too, by the tool's name.
    params: HashMap<String, Vec<Param>>,
    /// Tools left out for a header mark that revision doesn't allow.
    invalid: Vec<String>,
    /// A call is under way, so Esc stops what it waits for: a new
    /// session's start too, which has a time limit of its own.
    in_call: bool,
}

/// How Lokal reaches a server.
enum Wire {
    /// A process it started: a JSON message a line on its stdin and stdout.
    Process(Process),
    /// Its address: a POST a message.
    Http(Http),
}

struct Process {
    child: Child,
    /// Lines for the server's stdin, which a thread of their own writes:
    /// a server that stops reading can't hang a turn past Esc. Shared with
    /// the reader, which answers pings; None once closed, which ends the
    /// writer and so closes the stdin.
    lines: Arc<Mutex<Option<Sender<String>>>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    /// The thread keeping `stderr`, done once the server closed it.
    stderr_reader: Option<thread::JoinHandle<()>>,
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
        let stderr_reader = child.stderr.take().map(|output| {
            let kept = Arc::clone(&stderr);
            thread::spawn(move || keep_stderr(output, &kept))
        });
        let process = Process {
            child,
            lines,
            stderr,
            stderr_reader,
            ended: false,
        };
        Client::agreed(Wire::Process(process), replies, awaited)
    }

    /// Reaches the server at `address`, with `header` on every request, and
    /// agrees on the protocol with it.
    pub fn reach(name: &str, address: &str, header: Option<String>) -> Result<Client, String> {
        let (sender, replies) = mpsc::sync_channel(1);
        let http = Http::new(name, address, header, sender)?;
        Client::agreed(Wire::Http(http), replies, Arc::new(AtomicU64::new(0)))
    }

    fn agreed(
        wire: Wire,
        replies: Receiver<Reply>,
        awaited: Arc<AtomicU64>,
    ) -> Result<Client, String> {
        let mut client = Client {
            wire,
            replies,
            awaited,
            era: Era::Legacy,
            version: LEGACY_VERSION,
            next_id: 1,
            params: HashMap::new(),
            invalid: Vec::new(),
            in_call: false,
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
        self.era = Era::Legacy;
        self.version = LEGACY_VERSION;
        let params = json!({
            "protocolVersion": LEGACY_VERSION,
            "capabilities": {},
            "clientInfo": client_info(),
        });
        match self.request("initialize", params, Some(START_TIME)) {
            Ok(result) => {
                let version = result["protocolVersion"].as_str().unwrap_or_default();
                let Some(agreed) = LEGACY_VERSIONS.iter().find(|known| **known == version) else {
                    return Err(Failure::Refused(text::mcp_unknown_version(version)));
                };
                self.version = agreed;
                self.notify("notifications/initialized");
                Ok(())
            }
            Err(Failure::Refused(_)) => {
                self.era = Era::Modern;
                self.version = MODERN_VERSION;
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

    /// The tools the server lists, page after page. Over HTTP in the
    /// 2026-07-28 era, a tool whose header marks that revision doesn't allow
    /// is left out, as it asks (`invalid` says which).
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
        if matches!(self.wire, Wire::Http(_)) && self.era == Era::Modern {
            tools.retain(|tool| {
                let name = tool["name"].as_str().unwrap_or_default().to_string();
                match http::params_of(&tool["inputSchema"]) {
                    Some(params) => {
                        self.params.insert(name, params);
                        true
                    }
                    None => {
                        self.invalid.push(name);
                        false
                    }
                }
            });
        }
        Ok(tools)
    }

    /// The tools left out for a header mark the protocol doesn't allow.
    pub fn invalid(&self) -> &[String] {
        &self.invalid
    }

    /// Calls a tool, for as long as it takes; Esc or Ctrl+C cancels it.
    pub fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<Value, Failure> {
        let _keys = keys::KeyWatch::start();
        self.in_call = true;
        let reply = self.request(
            "tools/call",
            json!({"name": name, "arguments": arguments}),
            None,
        );
        self.in_call = false;
        reply
    }

    /// Sends a request and waits for its answer, until `time` passes when
    /// there's a limit, or the person stops it when there isn't. A server
    /// reached by its address that dropped the session gets a new one, and
    /// the request again, once.
    fn request(
        &mut self,
        method: &str,
        params: Value,
        time: Option<Duration>,
    ) -> Result<Value, Failure> {
        let reply = self.request_once(method, params.clone(), time);
        let dropped = match &self.wire {
            Wire::Http(http) => http.take_dropped(),
            Wire::Process(_) => false,
        };
        if dropped && method != "initialize" && matches!(reply, Err(Failure::Lost(_))) {
            self.agree()?;
            return self.request_once(method, params, time);
        }
        reply
    }

    fn request_once(
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
        let headers = self.headers(method, &params);
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.awaited.store(id, Ordering::SeqCst);
        let pings = (self.era == Era::Legacy).then(|| self.headers("", &Value::Null));
        let sent = match &mut self.wire {
            Wire::Process(process) => queue_line(&process.lines, &message)
                .then_some(())
                .ok_or_else(|| text::MCP_ENDED.to_string()),
            Wire::Http(http) => http.post(&message, &headers, id, &self.awaited, pings),
        };
        if let Err(why) = sent {
            self.awaited.store(0, Ordering::SeqCst);
            return Err(Failure::Lost(why));
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

    /// What goes with a message over HTTP: the revision, and in the
    /// 2026-07-28 one the method, the tool's name and the arguments it
    /// wants as headers. Nothing goes over stdio.
    fn headers(&self, method: &str, params: &Value) -> Vec<(String, String)> {
        if matches!(self.wire, Wire::Process(_)) {
            return Vec::new();
        }
        let mut headers = vec![("MCP-Protocol-Version".to_string(), self.version.to_string())];
        if self.era == Era::Modern {
            headers.push(("Mcp-Method".to_string(), method.to_string()));
            if method == "tools/call" {
                let name = params["name"].as_str().unwrap_or_default();
                headers.push(("Mcp-Name".to_string(), http::encoded(name)));
                if let Some(marked) = self.params.get(name) {
                    headers.extend(http::param_headers(marked, &params["arguments"]));
                }
            }
        }
        headers
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
            if (time.is_none() || self.in_call) && interrupt::happened() {
                self.cancel(id);
                return Err(Failure::Stopped);
            }
            if time.is_some_and(|time| started.elapsed() > time) {
                if let Wire::Http(http) = &mut self.wire {
                    http.give_up();
                }
                return Err(Failure::Lost(text::mcp_no_answer(method)));
            }
            match self.replies.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(reply)) if reply["id"] == id => return answer(reply),
                Ok(Err((from, why))) if from == id => return Err(Failure::Lost(why)),
                // An answer to a request given up on.
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Failure::Lost(text::MCP_ENDED.to_string()));
                }
            }
        }
    }

    /// Tells the server the person stopped request `id`. Over HTTP the
    /// stream is closed too, which is all the 2026-07-28 revision takes.
    fn cancel(&mut self, id: u64) {
        let cancelled = json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": {"requestId": id, "reason": text::STOPPED},
        });
        let headers = self.headers("notifications/cancelled", &Value::Null);
        match &mut self.wire {
            Wire::Process(process) => {
                queue_line(&process.lines, &cancelled);
            }
            Wire::Http(http) => {
                http.give_up();
                if self.era == Era::Legacy {
                    http.notify(&cancelled, &headers, false, || false);
                }
            }
        }
    }

    fn notify(&mut self, method: &str) {
        let message = json!({"jsonrpc": "2.0", "method": method});
        let headers = self.headers(method, &Value::Null);
        match &self.wire {
            Wire::Process(process) => {
                queue_line(&process.lines, &message);
            }
            Wire::Http(http) => {
                let stoppable = self.in_call;
                http.notify(&message, &headers, true, || {
                    stoppable && interrupt::happened()
                });
            }
        }
    }

    /// `message`, with the last lines a server Lokal started wrote to
    /// stderr, which usually say why it failed: read once the server closed
    /// its stderr, or after `STDERR_SETTLE` if it's still open.
    pub fn with_stderr(&self, message: &str) -> String {
        let Wire::Process(process) = &self.wire else {
            return message.to_string();
        };
        let started = Instant::now();
        while process
            .stderr_reader
            .as_ref()
            .is_some_and(|reader| !reader.is_finished())
            && started.elapsed() < STDERR_SETTLE
        {
            thread::sleep(Duration::from_millis(5));
        }
        let tail = process
            .stderr
            .lock()
            .map(|tail| tail.clone())
            .unwrap_or_default();
        let lines: Vec<String> = tail.into_iter().collect();
        text::mcp_with_stderr(message, &lines)
    }

    /// Asks the server to end: a process by closing its stdin, once what's
    /// queued is written; a session over HTTP with a DELETE.
    pub fn close(&mut self) {
        let headers = self.headers("", &Value::Null);
        match &mut self.wire {
            Wire::Process(process) => {
                if let Ok(mut lines) = process.lines.lock() {
                    lines.take();
                }
            }
            Wire::Http(http) => http.close(&headers),
        }
    }

    /// Whether the server, or the curl reaching it, has ended; once it has,
    /// it's reaped and no longer signalled.
    pub fn has_ended(&mut self) -> bool {
        match &mut self.wire {
            Wire::Process(process) => {
                if !process.ended && !matches!(process.child.try_wait(), Ok(None)) {
                    process.ended = true;
                    unwatch_group(process.child.id() as i32);
                }
                process.ended
            }
            Wire::Http(http) => http.has_ended(),
        }
    }

    /// A signal to the server and everything it started, or to the curl
    /// reaching it.
    pub fn signal(&self, signal: libc::c_int) {
        match &self.wire {
            Wire::Process(process) => shell::signal_group(process.child.id(), signal),
            Wire::Http(http) => http.signal(signal),
        }
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

pub(super) fn watch_group(pid: i32) {
    for group in &GROUPS {
        if group
            .compare_exchange(0, pid, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return;
        }
    }
}

pub(super) fn unwatch_group(pid: i32) {
    for group in &GROUPS {
        let _ = group.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// Reads the server's stdout, a JSON message a line: answers go to the
/// client, a request of the server's is answered here, and the rest is
/// left. It ends with the server, or at a line over `LINE_MOST`.
fn read_messages(
    stdout: ChildStdout,
    replies: &SyncSender<Reply>,
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
                if replies.send(Ok(message)).is_err() {
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
        let Wire::Process(process) = &client.wire else {
            unreachable!()
        };
        let pid = process.child.id() as i32;
        let watched = || GROUPS.iter().any(|g| g.load(Ordering::SeqCst) == pid);
        assert!(watched());
        client.close();
        let started = Instant::now();
        while !client.has_ended() && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!watched());
    }

    /// The fake server over HTTP, its address, and the file it logs what
    /// it gets to; ended when dropped.
    struct HttpFake {
        child: Child,
        address: String,
        log: std::path::PathBuf,
    }

    impl Drop for HttpFake {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_file(&self.log);
        }
    }

    fn http_fake(mode: &str) -> HttpFake {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/mcp/fake_http_server.py");
        let log = env::temp_dir().join(format!(
            "lokal-mcp-http-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::SeqCst)
        ));
        let mut child = Command::new("python3")
            .arg(script)
            .arg(mode)
            .arg(&log)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let port = line.trim().strip_prefix("PORT ").unwrap().to_string();
        HttpFake {
            child,
            address: format!("http://127.0.0.1:{port}/mcp"),
            log,
        }
    }

    /// What the fake got, a request a line.
    fn logged(fake: &HttpFake) -> Vec<Value> {
        std::fs::read_to_string(&fake.log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn echo(client: &mut Client, words: &str) -> String {
        let result = client.call_tool("echo", &json!({"text": words})).unwrap();
        result["content"][0]["text"].as_str().unwrap().to_string()
    }

    #[test]
    fn speaks_to_a_server_by_its_address_in_a_session() {
        let fake = http_fake("legacy");
        let mut client = Client::reach("test", &fake.address, None).unwrap();
        assert_eq!(client.era(), Era::Legacy);
        assert_eq!(client.list_tools().unwrap().len(), 3);
        assert_eq!(echo(&mut client, "hi"), "hi");
        client.close();
        let started = Instant::now();
        while !client.has_ended() && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(20));
        }
        let log = logged(&fake);
        let session = log[0]["body"]["method"] == "initialize";
        assert!(session);
        // Every message after initialize carries the session and the version.
        for entry in &log[1..] {
            assert!(entry["headers"]["mcp-session-id"].is_string(), "{entry}");
            assert_eq!(entry["headers"]["mcp-protocol-version"], "2025-11-25");
            assert_eq!(
                entry["headers"]["accept"],
                "application/json, text/event-stream"
            );
        }
        assert_eq!(log.last().unwrap()["method"], "DELETE");
    }

    #[test]
    fn an_answer_streamed_as_events_comes_after_a_ping_is_answered() {
        let fake = http_fake("sse");
        let mut client = Client::reach("test", &fake.address, None).unwrap();
        assert_eq!(client.list_tools().unwrap().len(), 3);
        assert_eq!(echo(&mut client, "hi"), "hi");
        let pongs = logged(&fake)
            .iter()
            .filter(|entry| entry["body"]["id"] == "ping-1" && entry["body"]["result"] == json!({}))
            .count();
        // One for each request: initialize, tools/list, the call.
        assert_eq!(pongs, 3);
    }

    #[test]
    fn a_token_goes_in_a_header_and_never_on_a_command_line() {
        let fake = http_fake("token");
        let error = Client::reach("test", &fake.address, None).err().unwrap();
        assert!(error.contains("401"), "{error}");
        assert!(
            error.contains("`mcp.test.header = Authorization: Bearer <token>`"),
            "{error}"
        );
        let header = Some("Authorization: Bearer secret-token".to_string());
        let mut client = Client::reach("test", &fake.address, header).unwrap();
        let result = client.call_tool("argv", &json!({})).unwrap();
        // The call's own curl runs as the server looks.
        assert_eq!(
            result["content"][0]["text"],
            "curls: True, token in argv: False"
        );
        let refused = Client::reach(
            "test",
            &fake.address,
            Some("Authorization: Bearer x".into()),
        );
        assert!(refused.err().unwrap().contains("refused the token"));
    }

    #[test]
    fn a_session_the_server_dropped_starts_again() {
        let fake = http_fake("expire");
        let mut client = Client::reach("test", &fake.address, None).unwrap();
        assert_eq!(echo(&mut client, "one"), "one");
        assert_eq!(echo(&mut client, "two"), "two");
        let initializes = logged(&fake)
            .iter()
            .filter(|entry| entry["body"]["method"] == "initialize")
            .count();
        assert_eq!(initializes, 2);
    }

    #[test]
    fn the_stateless_era_sends_its_headers_and_leaves_out_bad_marks() {
        let fake = http_fake("modern");
        let mut client = Client::reach("test", &fake.address, None).unwrap();
        assert_eq!(client.era(), Era::Modern);
        let tools = client.list_tools().unwrap();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names, ["echo", "argv", "slow", "where"]);
        assert_eq!(client.invalid(), ["broken"]);
        for region in ["us-west1", "Hello, 世界", " padded "] {
            let result = client
                .call_tool("where", &json!({"region": region}))
                .unwrap();
            assert_eq!(result["content"][0]["text"], format!("region {region}"));
        }
        assert_eq!(echo(&mut client, "hi"), "hi");
    }

    #[test]
    fn a_redirect_is_never_followed() {
        let fake = http_fake("redirect");
        let error = Client::reach("test", &fake.address, None).err().unwrap();
        assert!(
            error.contains("redirect to https://elsewhere.example/mcp"),
            "{error}"
        );
    }

    #[test]
    fn a_request_given_up_on_closes_its_stream() {
        let fake = http_fake("sse");
        let mut client = Client::reach("test", &fake.address, None).unwrap();
        let failure = client.request(
            "tools/call",
            json!({"name": "slow", "arguments": {}}),
            Some(Duration::from_millis(1500)),
        );
        assert!(matches!(failure, Err(Failure::Lost(_))));
        let started = Instant::now();
        while !logged(&fake).iter().any(|entry| entry["method"] == "GONE")
            && started.elapsed() < Duration::from_secs(5)
        {
            thread::sleep(Duration::from_millis(50));
        }
        assert!(logged(&fake).iter().any(|entry| entry["method"] == "GONE"));
        assert_eq!(echo(&mut client, "after"), "after");
    }

    #[test]
    fn a_flood_of_pings_starts_one_curl_at_a_time() {
        let fake = http_fake("flood");
        let mut client = Client::reach("test", &fake.address, None).unwrap();
        let started = Instant::now();
        assert_eq!(echo(&mut client, "hi"), "hi");
        assert!(started.elapsed() < Duration::from_secs(2));
        // 30 pings came at once, with the answer right after them: one is
        // answered at most, and that one is given up on with the request.
        thread::sleep(Duration::from_millis(500));
        let answered = logged(&fake)
            .iter()
            .filter(|entry| entry["body"]["result"] == json!({}))
            .count();
        assert!(answered <= 1, "{answered} pings answered");
    }

    #[test]
    fn a_server_that_holds_a_notification_holds_the_start_only_so_long() {
        let fake = http_fake("hangnote");
        let started = Instant::now();
        let mut client = Client::reach("test", &fake.address, None).unwrap();
        assert!(started.elapsed() < http::NOTICE_TIME + Duration::from_secs(2));
        assert_eq!(echo(&mut client, "hi"), "hi");
    }

    #[test]
    fn an_address_no_one_answers_says_why() {
        let error = Client::reach("test", "http://127.0.0.1:9/mcp", None)
            .err()
            .unwrap();
        assert!(error.contains("couldn't reach it: curl:"), "{error}");
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
