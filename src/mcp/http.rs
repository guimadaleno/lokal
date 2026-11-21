//! The wire to an MCP server reached by its address (Streamable HTTP): each
//! message a POST through the system's curl, its answer one JSON object or
//! a stream of events (SSE). The address, the headers and the body reach
//! curl on its stdin, never its command line, which anyone on the machine
//! can read; `-q` keeps the person's `.curlrc` out, and a redirect is never
//! followed, since the token would go with it.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::client::{self, LINE_MOST, NO_SUCH_METHOD, Reply};
use crate::{config, system, text};

/// The longest status or header line read, and how many header lines.
const HEAD_LINE_MOST: u64 = 16 * 1024;
const HEAD_LINES_MOST: usize = 200;
/// The most of an error's body read, to say what it was.
const ERROR_BODY_MOST: u64 = 64 * 1024;
/// How long a connection may take to open.
const CONNECT_TIME: &str = "10";
/// How long a message that isn't a request may take: a notification, an
/// answer to a ping, the DELETE that ends a session. Shorter in tests,
/// which wait it out.
pub(super) const NOTICE_TIME: Duration = Duration::from_secs(if cfg!(test) { 2 } else { 10 });

pub struct Http {
    endpoint: Arc<Endpoint>,
    /// Where the readers of answers pass them on.
    replies: SyncSender<Reply>,
    /// The curl of the last request, ended once it's given up on.
    current: Option<Child>,
    /// The DELETE that ends the session, once closed.
    closing: Option<Child>,
}

/// What every message needs, shared with the threads that read answers.
struct Endpoint {
    /// The server's name in the settings, for what Lokal says about it.
    name: String,
    address: String,
    header: Option<String>,
    /// The session the server gave at `initialize`, in the era that has
    /// them.
    session: Mutex<Option<String>>,
    /// The server dropped the session: a 404 inside one.
    dropped: AtomicBool,
}

/// An argument a server wants as a header too (2026-07-28's
/// `x-mcp-header`): where it is in the arguments, and the header's name
/// after `Mcp-Param-`.
#[derive(Debug, PartialEq)]
pub struct Param {
    path: Vec<String>,
    header: String,
}

impl Http {
    pub fn new(
        name: &str,
        address: &str,
        header: Option<String>,
        replies: SyncSender<Reply>,
    ) -> Result<Http, String> {
        system::program("curl").ok_or_else(|| text::MCP_NO_CURL.to_string())?;
        Ok(Http {
            endpoint: Arc::new(Endpoint {
                name: name.to_string(),
                address: address.to_string(),
                header,
                session: Mutex::new(None),
                dropped: AtomicBool::new(false),
            }),
            replies,
            current: None,
            closing: None,
        })
    }

    /// Posts request `id`, whose answer a thread passes on while it's the
    /// one `awaited`. `pings` are the headers to answer a ping with, in the
    /// era where a server may send one on the stream.
    pub fn post(
        &mut self,
        message: &Value,
        headers: &[(String, String)],
        id: u64,
        awaited: &Arc<AtomicU64>,
        pings: Option<Vec<(String, String)>>,
    ) -> Result<(), String> {
        self.give_up();
        let mut child = spawn(&self.endpoint, "POST", headers, Some(message), None)?;
        client::watch_group(child.id() as i32);
        let (Some(output), Some(errors)) = (child.stdout.take(), child.stderr.take()) else {
            return Err(text::MCP_NO_CURL.to_string());
        };
        let endpoint = Arc::clone(&self.endpoint);
        let replies = self.replies.clone();
        let awaited = Arc::clone(awaited);
        thread::spawn(move || {
            let reply = answer_of(output, errors, id, &endpoint, &awaited, pings.as_deref());
            if awaited.load(Ordering::SeqCst) == id {
                let _ = replies.send(reply);
            }
        });
        self.current = Some(child);
        Ok(())
    }

    /// Posts a notification. With `wait`, until the server took it, or
    /// `stop` says to give up: `notifications/initialized` must come before
    /// the requests after it.
    pub fn notify(
        &self,
        message: &Value,
        headers: &[(String, String)],
        wait: bool,
        stop: impl Fn() -> bool,
    ) {
        if wait {
            send_notice(&self.endpoint, message, headers, stop);
        } else {
            let endpoint = Arc::clone(&self.endpoint);
            let (message, headers) = (message.clone(), headers.to_vec());
            thread::spawn(move || send_notice(&endpoint, &message, &headers, || false));
        }
    }

    /// Ends the curl of the request in flight: closing its stream is how
    /// the 2026-07-28 revision cancels a request.
    pub fn give_up(&mut self) {
        if let Some(mut child) = self.current.take() {
            let _ = child.kill();
            let _ = child.wait();
            client::unwatch_group(child.id() as i32);
        }
    }

    /// Whether the server dropped the session since this was last asked:
    /// it's then forgotten, for `initialize` to start a new one.
    pub fn take_dropped(&self) -> bool {
        let dropped = self.endpoint.dropped.swap(false, Ordering::SeqCst);
        if dropped && let Ok(mut session) = self.endpoint.session.lock() {
            session.take();
        }
        dropped
    }

    /// Gives up on what's in flight, and ends the session, if there's one,
    /// with a DELETE, as the 2025 revisions ask.
    pub fn close(&mut self, headers: &[(String, String)]) {
        self.give_up();
        let has_session = self
            .endpoint
            .session
            .lock()
            .is_ok_and(|session| session.is_some());
        if has_session && self.closing.is_none() {
            self.closing = spawn(&self.endpoint, "DELETE", headers, None, Some(NOTICE_TIME)).ok();
            if let Some(child) = &self.closing {
                client::watch_group(child.id() as i32);
            }
        }
    }

    /// Whether nothing of it runs any more; a curl that ended is no longer
    /// signalled.
    pub fn has_ended(&mut self) -> bool {
        [&mut self.current, &mut self.closing]
            .into_iter()
            .all(|child| {
                child.as_mut().is_none_or(|child| {
                    let ended = !matches!(child.try_wait(), Ok(None));
                    if ended {
                        client::unwatch_group(child.id() as i32);
                    }
                    ended
                })
            })
    }

    pub fn signal(&self, signal: libc::c_int) {
        for child in [&self.current, &self.closing].into_iter().flatten() {
            crate::shell::signal_group(child.id(), signal);
        }
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        self.give_up();
        if let Some(mut child) = self.closing.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A curl that sends `message` to the endpoint with `method`, started in a
/// session of its own and told all of it on its stdin.
fn spawn(
    endpoint: &Endpoint,
    method: &str,
    headers: &[(String, String)],
    message: Option<&Value>,
    time: Option<Duration>,
) -> Result<Child, String> {
    let curl = system::program("curl").ok_or_else(|| text::MCP_NO_CURL.to_string())?;
    let config = config_for(endpoint, method, headers, message, time)?;
    let mut process = Command::new(curl);
    process
        .args(["-q", "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: setsid is safe to call between fork and exec.
    unsafe {
        process.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = process
        .spawn()
        .map_err(|error| text::mcp_cant_start("curl", &error.to_string()))?;
    // curl reads all of its options before it connects.
    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(config.as_bytes()).is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(text::MCP_NO_CURL.to_string());
    }
    Ok(child)
}

/// All curl is told, one option a line.
fn config_for(
    endpoint: &Endpoint,
    method: &str,
    headers: &[(String, String)],
    message: Option<&Value>,
    time: Option<Duration>,
) -> Result<String, String> {
    let session = endpoint.session.lock().ok().and_then(|s| s.clone());
    let mut options: Vec<(&str, String)> = vec![
        ("url", endpoint.address.clone()),
        ("request", method.to_string()),
        ("header", "Content-Type: application/json".into()),
        (
            "header",
            "Accept: application/json, text/event-stream".into(),
        ),
        // curl would wait for a 100 Continue before a long body.
        ("header", "Expect:".into()),
        ("proto", "=http,https".into()),
        ("connect-timeout", CONNECT_TIME.into()),
        ("dump-header", "-".into()),
    ];
    // Plain http is local by Lokal's rule, and a proxy would carry the
    // token on in the clear.
    if endpoint.address.starts_with("http://") {
        options.push(("noproxy", "*".into()));
    }
    for (name, value) in headers {
        // `Name:` with nothing after it would drop the header.
        let header = if value.is_empty() {
            format!("{name};")
        } else {
            format!("{name}: {value}")
        };
        options.push(("header", header));
    }
    if let Some(session) = session {
        options.push(("header", format!("MCP-Session-Id: {session}")));
    }
    if let Some(header) = &endpoint.header {
        options.push(("header", header.clone()));
    }
    if let Some(message) = message {
        options.push(("data-binary", message.to_string()));
    }
    if let Some(time) = time {
        options.push(("max-time", time.as_secs().to_string()));
    }
    // A line break would end the option it's in and start another.
    if options
        .iter()
        .any(|(_, value)| value.contains(['\n', '\r']))
    {
        return Err(text::MCP_LINE_BREAK.to_string());
    }
    let mut config: String = options
        .iter()
        .map(|(name, value)| option(name, value))
        .collect();
    // globoff: `{a,b}` or `[1-3]` in an address would make it several.
    config.push_str("silent\nshow-error\nno-buffer\nsuppress-connect-headers\ngloboff\n");
    Ok(config)
}

/// One line of curl's options: the value quoted, its `\` and `"` escaped.
fn option(name: &str, value: &str) -> String {
    format!(
        "{name} = \"{}\"\n",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

/// Sends a message whose answer no one reads, a notification or the
/// answer to a ping, and waits until the server took it: at most
/// `NOTICE_TIME`, or until `stop` says to give up. Its curl is signalled
/// with the servers' if Lokal ends meanwhile.
fn send_notice(
    endpoint: &Endpoint,
    message: &Value,
    headers: &[(String, String)],
    stop: impl Fn() -> bool,
) {
    let Ok(mut child) = spawn(endpoint, "POST", headers, Some(message), Some(NOTICE_TIME)) else {
        return;
    };
    // Unread: once the answer comes, curl can't write it, and ends.
    drop(child.stdout.take());
    drop(child.stderr.take());
    let pid = child.id() as i32;
    client::watch_group(pid);
    let started = Instant::now();
    while matches!(child.try_wait(), Ok(None)) {
        if stop() || started.elapsed() > NOTICE_TIME {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    client::unwatch_group(pid);
}

/// The status and the headers Lokal reads.
struct Head {
    status: u16,
    event_stream: bool,
    session: Option<String>,
    location: Option<String>,
}

/// What a request's curl gave: the answer to request `id`, or why there's
/// none.
fn answer_of(
    output: ChildStdout,
    errors: ChildStderr,
    id: u64,
    endpoint: &Arc<Endpoint>,
    awaited: &Arc<AtomicU64>,
    pings: Option<&[(String, String)]>,
) -> Reply {
    let mut reader = BufReader::new(output);
    let failed = |why: String| Err((id, why));
    let Some(head) = read_head(&mut reader) else {
        return failed(curl_failure(errors));
    };
    if let Some(session) = head.session.filter(|session| is_session_id(session))
        && let Ok(mut kept) = endpoint.session.lock()
    {
        kept.get_or_insert(session);
    }
    match head.status {
        200..=299 if head.event_stream => read_events(reader, id, endpoint, awaited, pings),
        200..=299 => match read_body(&mut reader, LINE_MOST) {
            Some(body) => match serde_json::from_slice::<Value>(&body) {
                Ok(message) if message["id"] == id => Ok(message),
                Ok(_) => failed(text::MCP_OTHER_ANSWER.to_string()),
                Err(_) => failed(text::MCP_NOT_JSON.to_string()),
            },
            None => failed(text::MCP_TOO_LONG.to_string()),
        },
        300..=399 => failed(text::mcp_redirect(head.location.as_deref().unwrap_or("?"))),
        401 => failed(text::mcp_wants_token(
            &endpoint.name,
            endpoint.header.is_some(),
        )),
        403 => failed(text::mcp_forbidden(
            &endpoint.name,
            endpoint.header.is_some(),
        )),
        404 if endpoint.session.lock().is_ok_and(|s| s.is_some()) => {
            endpoint.dropped.store(true, Ordering::SeqCst);
            failed(text::MCP_SESSION_DROPPED.to_string())
        }
        status => {
            let body = read_body(&mut reader, ERROR_BODY_MOST).unwrap_or_default();
            match serde_json::from_slice::<Value>(&body) {
                // A JSON-RPC error, as a server of the other era answers.
                Ok(mut message) if message.get("error").is_some() => {
                    message["id"] = id.into();
                    Ok(message)
                }
                _ => {
                    let words = String::from_utf8_lossy(&body);
                    let first = words.lines().find(|line| !line.trim().is_empty());
                    failed(text::mcp_http_status(status, first.unwrap_or_default()))
                }
            }
        }
    }
}

/// The status and headers, after any `1xx` ones; None when curl wrote none,
/// since it couldn't connect.
fn read_head(reader: &mut impl BufRead) -> Option<Head> {
    loop {
        let status_line = read_line(reader, HEAD_LINE_MOST)?;
        let status: u16 = status_line.split_whitespace().nth(1)?.parse().ok()?;
        let mut head = Head {
            status,
            event_stream: false,
            session: None,
            location: None,
        };
        for _ in 0..HEAD_LINES_MOST {
            let line = read_line(reader, HEAD_LINE_MOST)?;
            if line.trim().is_empty() {
                break;
            }
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim().to_string();
            match name.trim().to_ascii_lowercase().as_str() {
                "content-type" => {
                    head.event_stream = value.to_ascii_lowercase().starts_with("text/event-stream");
                }
                "mcp-session-id" => head.session = Some(value),
                "location" => head.location = Some(value),
                _ => {}
            }
        }
        if !(100..200).contains(&status) {
            return Some(head);
        }
    }
}

/// A line without its line break; None at the end.
fn read_line(reader: &mut impl BufRead, most: u64) -> Option<String> {
    let mut line = Vec::new();
    match reader.take(most).read_until(b'\n', &mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(
            String::from_utf8_lossy(&line)
                .trim_end_matches(['\r', '\n'])
                .to_string(),
        ),
    }
}

/// The body, or None when it's over `most`.
fn read_body(reader: &mut impl Read, most: u64) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    reader.take(most + 1).read_to_end(&mut body).ok()?;
    (body.len() as u64 <= most).then_some(body)
}

/// A stream of events, each a JSON-RPC message in its `data` lines, until
/// the answer to request `id`. A ping on it is answered in the era that
/// has them, any other request of the server's with "method not found",
/// one at a time: while an answer is on its way, the next is left, so a
/// server can't have Lokal start curl after curl. Notifications are left.
fn read_events(
    mut reader: BufReader<ChildStdout>,
    id: u64,
    endpoint: &Arc<Endpoint>,
    awaited: &Arc<AtomicU64>,
    pings: Option<&[(String, String)]>,
) -> Reply {
    let answering = Arc::new(AtomicBool::new(false));
    let mut data = String::new();
    loop {
        let Some(line) = read_line(&mut reader, LINE_MOST) else {
            return Err((id, text::MCP_STREAM_ENDED.to_string()));
        };
        if let Some(part) = line.strip_prefix("data:") {
            if data.len() as u64 + part.len() as u64 > LINE_MOST {
                return Err((id, text::MCP_TOO_LONG.to_string()));
            }
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(part.strip_prefix(' ').unwrap_or(part));
            continue;
        }
        // An event ends at a blank line; `event:`, `id:`, `retry:` and
        // comments are left.
        if !line.is_empty() || data.is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&std::mem::take(&mut data)) else {
            continue;
        };
        match (message.get("method"), message.get("id")) {
            (Some(method), Some(from)) => {
                if let Some(headers) = pings
                    && !answering.swap(true, Ordering::SeqCst)
                {
                    let reply = if method == "ping" {
                        json!({"jsonrpc": "2.0", "id": from, "result": {}})
                    } else {
                        json!({"jsonrpc": "2.0", "id": from,
                               "error": {"code": NO_SUCH_METHOD, "message": "Method not found"}})
                    };
                    let (endpoint, awaited) = (Arc::clone(endpoint), Arc::clone(awaited));
                    let (headers, answering) = (headers.to_vec(), Arc::clone(&answering));
                    thread::spawn(move || {
                        let given_up = || awaited.load(Ordering::SeqCst) != id;
                        send_notice(&endpoint, &reply, &headers, given_up);
                        answering.store(false, Ordering::SeqCst);
                    });
                }
            }
            (None, Some(from)) if *from == id => return Ok(message),
            _ => {}
        }
    }
}

/// Why curl wrote nothing: its own last line, such as "curl: (7) Failed
/// to connect".
fn curl_failure(errors: ChildStderr) -> String {
    let mut words = String::new();
    let _ = errors.take(ERROR_BODY_MOST).read_to_string(&mut words);
    match words.lines().rev().find(|line| !line.trim().is_empty()) {
        Some(line) => text::mcp_unreachable(line.trim()),
        None => text::mcp_unreachable("curl ended without a word"),
    }
}

/// A session id as the spec allows it: visible ASCII only. Anything else
/// could break the line it goes into.
fn is_session_id(session: &str) -> bool {
    !session.is_empty() && session.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// The arguments of a tool its server wants as headers too, or None when
/// a mark breaks the 2026-07-28 rules, which then leave the tool out: on a
/// property reached from the root through `properties` alone, of a type
/// string, integer or boolean, named as an HTTP token, and unique in any
/// case.
pub fn params_of(schema: &Value) -> Option<Vec<Param>> {
    let mut params = Vec::new();
    reachable(schema, &mut Vec::new(), &mut params)?;
    if marks_in(schema) != params.len() {
        return None;
    }
    let mut seen = HashSet::new();
    params
        .iter()
        .all(|param| seen.insert(param.header.to_ascii_lowercase()))
        .then_some(params)
}

fn reachable(schema: &Value, path: &mut Vec<String>, params: &mut Vec<Param>) -> Option<()> {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Some(());
    };
    for (name, property) in properties {
        path.push(name.clone());
        if let Some(mark) = property.get("x-mcp-header") {
            let header = mark
                .as_str()
                .filter(|header| !header.is_empty() && header.chars().all(config::is_token_char))?;
            if !is_header_type(&property["type"]) {
                return None;
            }
            params.push(Param {
                path: path.clone(),
                header: header.to_string(),
            });
        }
        reachable(property, path, params)?;
        path.pop();
    }
    Some(())
}

/// A type an argument sent as a header may have: string, integer or
/// boolean, or one of them or null.
fn is_header_type(kind: &Value) -> bool {
    let primitive = |kind: &Value| matches!(kind.as_str(), Some("string" | "integer" | "boolean"));
    match kind {
        Value::Array(kinds) => {
            let others: Vec<&Value> = kinds.iter().filter(|kind| *kind != "null").collect();
            matches!(others.as_slice(), [one] if primitive(one))
        }
        kind => primitive(kind),
    }
}

/// Every `x-mcp-header` anywhere in a schema.
fn marks_in(value: &Value) -> usize {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, inner)| usize::from(key == "x-mcp-header") + marks_in(inner))
            .sum(),
        Value::Array(items) => items.iter().map(marks_in).sum(),
        _ => 0,
    }
}

/// The `Mcp-Param-` headers for a call's arguments: one for each marked
/// argument that's there and not null.
pub fn param_headers(params: &[Param], arguments: &Value) -> Vec<(String, String)> {
    params
        .iter()
        .filter_map(|param| {
            let value = param
                .path
                .iter()
                .fold(arguments, |value, key| &value[key.as_str()]);
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Number(number) if !number.is_f64() => number.to_string(),
                Value::Bool(yes) => yes.to_string(),
                _ => return None,
            };
            Some((format!("Mcp-Param-{}", param.header), encoded(&text)))
        })
        .collect()
}

/// A header's value as the 2026-07-28 revision sends it: as it is when
/// it's plain printable ASCII with no space at either end, in Base64
/// between `=?base64?` and `?=` otherwise, or when it looks like that
/// already.
pub fn encoded(value: &str) -> String {
    let plain = value.chars().all(|c| (' '..='~').contains(&c))
        && value.trim() == value
        && !(value.starts_with("=?base64?") && value.ends_with("?="));
    if plain {
        value.to_string()
    } else {
        format!("=?base64?{}?=", base64(value.as_bytes()))
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, byte)| n | u32::from(*byte) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_encoded_as_the_spec_shows() {
        // The spec's own examples.
        assert_eq!(encoded("Hello, 世界"), "=?base64?SGVsbG8sIOS4lueVjA==?=");
        assert_eq!(encoded(" padded "), "=?base64?IHBhZGRlZCA=?=");
        assert_eq!(encoded("line1\nline2"), "=?base64?bGluZTEKbGluZTI=?=");
        assert_eq!(encoded("us-west1"), "us-west1");
        assert_eq!(encoded("a b"), "a b");
        assert_eq!(encoded("=?base64?x?="), "=?base64?PT9iYXNlNjQ/eD89?=");
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
    }

    #[test]
    fn marked_arguments_become_headers() {
        let schema = json!({"type": "object", "properties": {
            "region": {"type": "string", "x-mcp-header": "Region"},
            "options": {"type": "object", "properties": {
                "dry": {"type": "boolean", "x-mcp-header": "Dry-Run"},
                "count": {"type": "integer", "x-mcp-header": "Count"}}},
            "note": {"type": "string"}}});
        let params = params_of(&schema).unwrap();
        let headers = param_headers(
            &params,
            &json!({"region": "us-west1", "options": {"dry": true, "count": null}}),
        );
        assert_eq!(
            headers,
            [
                ("Mcp-Param-Dry-Run".to_string(), "true".to_string()),
                ("Mcp-Param-Region".to_string(), "us-west1".to_string()),
            ]
        );
        assert_eq!(params_of(&json!({"type": "object"})), Some(vec![]));
    }

    #[test]
    fn a_mark_the_rules_dont_allow_leaves_the_tool_out() {
        let property = |extra: Value| json!({"type": "object", "properties": {"a": extra}});
        for wrong in [
            property(json!({"type": "number", "x-mcp-header": "A"})),
            property(json!({"type": "string", "x-mcp-header": ""})),
            property(json!({"type": "string", "x-mcp-header": "Two Words"})),
            property(json!({"type": "string", "x-mcp-header": "A\r\nX-Evil: 1"})),
            property(json!({"type": "array", "items": {"type": "string", "x-mcp-header": "A"}})),
            json!({"type": "object", "properties": {
                "a": {"type": "string", "x-mcp-header": "Same"},
                "b": {"type": "string", "x-mcp-header": "SAME"}}}),
            json!({"type": "object", "oneOf": [{"properties": {
                "a": {"type": "string", "x-mcp-header": "A"}}}]}),
        ] {
            assert_eq!(params_of(&wrong), None, "{wrong}");
        }
    }

    #[test]
    fn curl_never_globs_and_a_local_address_never_goes_through_a_proxy() {
        let endpoint = |address: &str| Endpoint {
            name: "docs".into(),
            address: address.into(),
            header: Some("Authorization: Bearer t".into()),
            session: Mutex::new(None),
            dropped: AtomicBool::new(false),
        };
        let headers = [("Mcp-Param-Note".to_string(), String::new())];
        let local = config_for(
            &endpoint("http://127.0.0.1/{a,b}"),
            "POST",
            &headers,
            None,
            None,
        )
        .unwrap();
        assert!(local.contains("\ngloboff\n"), "{local}");
        assert!(local.contains("noproxy = \"*\"\n"), "{local}");
        // An empty value would drop the header: `Name;` sends it empty.
        assert!(local.contains("header = \"Mcp-Param-Note;\"\n"), "{local}");
        let remote =
            config_for(&endpoint("https://a.example/mcp"), "POST", &[], None, None).unwrap();
        assert!(!remote.contains("noproxy"), "{remote}");
    }

    #[test]
    fn a_nullable_marked_argument_is_allowed() {
        let schema = |kind: Value| json!({"type": "object", "properties": {"a": {"type": kind, "x-mcp-header": "A"}}});
        assert!(params_of(&schema(json!(["string", "null"]))).is_some());
        assert!(params_of(&schema(json!(["null", "boolean"]))).is_some());
        assert!(params_of(&schema(json!(["string", "integer"]))).is_none());
        assert!(params_of(&schema(json!(["number", "null"]))).is_none());
    }

    #[test]
    fn curl_gets_each_option_on_a_line_of_its_own() {
        assert_eq!(
            option("header", r#"Authorization: Bearer a"b\c"#),
            "header = \"Authorization: Bearer a\\\"b\\\\c\"\n"
        );
        assert!(is_session_id("1868a90c-4e4a"));
        assert!(!is_session_id("a\nurl = \"http://x\""));
        assert!(!is_session_id("a b"));
        assert!(!is_session_id(""));
    }
}
