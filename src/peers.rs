//! The person's other Lokal sessions on this machine: finding them and
//! passing messages, so work can be split among them. Each session listens
//! on a socket of its own, `<pid>.sock`, in the private folder the scratch
//! folders go in (`scratch.rs`): beside its scratch folder, not inside it,
//! since the model may write there.
//!
//! A message never starts a turn on its own. Another model wrote it, and a
//! file it read may have steered it. It shows whole above the `>`, and
//! Enter on an empty line hands it to the model; Esc drops it.

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use rustyline::ExternalPrinter;
use serde_json::{Value, json};

use crate::{config, recon, style, text};

/// The longest message, in characters.
pub const MESSAGE_LIMIT: usize = 2_000;
/// The most messages that wait in a session at once.
pub const WAITING_LIMIT: usize = 5;
/// How long a session has to answer. Its listener answers at once, from a
/// thread of its own, whatever the session is doing.
const TIMEOUT: Duration = Duration::from_secs(1);
/// The most read of one request: a message escaped as JSON, and the rest.
const READ_LIMIT: u64 = 64 * 1024;
/// How long a message is on screen before Enter or Esc takes it. One that
/// arrives as Enter is pressed may not be drawn yet, and nobody reads one
/// faster than this.
const SEEN_AFTER: Duration = Duration::from_millis(500);
/// The longest name and folder shown or passed on.
const NAME_LIMIT: usize = 40;
const FOLDER_LIMIT: usize = 300;

pub struct Message {
    pub from: String,
    pub folder: String,
    pub text: String,
    /// When it showed on screen.
    shown: Option<Instant>,
}

/// Another session, as it says it is.
pub struct Session {
    pub name: String,
    pub folder: String,
    /// It's working on a turn, so a message waits for that to end.
    pub busy: bool,
}

pub enum Undelivered {
    /// No session by that name is open.
    NoSuch,
    /// It has as many messages waiting as it takes.
    Full,
    TooLong,
}

#[derive(Default)]
struct State {
    waiting: Vec<Message>,
    busy: bool,
    /// The person is at the `>`: a message prints above it at once.
    at_prompt: bool,
    /// A message printed at this prompt already, with a blank line after it.
    printed: bool,
}

/// The messages waiting, shared by the listener and the `>` prompt.
#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<State>>);

impl Inbox {
    pub fn count(&self) -> usize {
        self.lock().waiting.len()
    }

    /// The person is at the `>` from now on. What came meanwhile shows now,
    /// and what comes later shows as it arrives.
    pub fn enter_prompt(&self) {
        let mut state = self.lock();
        let mut printed = false;
        for message in state.waiting.iter_mut().filter(|m| m.shown.is_none()) {
            eprint!("{}", spaced(&notice(message), printed));
            message.shown = Some(Instant::now());
            printed = true;
        }
        state.at_prompt = true;
        state.printed = printed;
    }

    pub fn leave_prompt(&self) {
        self.lock().at_prompt = false;
    }

    /// Takes the messages that have been on screen a moment, for Enter to
    /// pass on or Esc to drop.
    pub fn take(&self) -> Vec<Message> {
        let mut state = self.lock();
        let (seen, rest) = std::mem::take(&mut state.waiting)
            .into_iter()
            .partition(|m| m.shown.is_some_and(|at| at.elapsed() >= SEEN_AFTER));
        state.waiting = rest;
        seen
    }

    /// A message checked and taken; what to print, when the person is at the `>`.
    fn add(&self, mut message: Message) -> Option<String> {
        let mut state = self.lock();
        let shown = if state.at_prompt {
            message.shown = Some(Instant::now());
            let printed = state.printed;
            state.printed = true;
            Some(spaced(&notice(&message), printed))
        } else {
            None
        };
        state.waiting.push(message);
        shown
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// This session among the others: its name, its socket, and what waits.
pub struct Peers {
    name: String,
    folder: String,
    base: PathBuf,
    socket: PathBuf,
    inbox: Inbox,
}

impl Peers {
    /// Listens in `base`, the private folder `scratch.rs` checked. None when
    /// the socket can't be made; then the model isn't told of the others.
    pub fn start(
        base: &Path,
        folder: &Path,
        printer: Box<dyn ExternalPrinter + Send>,
    ) -> Option<Peers> {
        clear_left_behind(base);
        let pid = std::process::id();
        let socket = base.join(format!("{pid}.sock"));
        let folder = folder.display().to_string();
        Peers::listen(base, socket, name_for(&folder, pid), folder, Some(printer)).ok()
    }

    fn listen(
        base: &Path,
        socket: PathBuf,
        name: String,
        folder: String,
        mut printer: Option<Box<dyn ExternalPrinter + Send>>,
    ) -> io::Result<Peers> {
        // One left with this pid is from a session long gone.
        let _ = fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket)?;
        let inbox = Inbox::default();
        let (own_name, own_folder, own_inbox) = (name.clone(), folder.clone(), inbox.clone());
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let notice = answer(&stream, &own_inbox, &own_name, &own_folder);
                drop(stream);
                // Only at the `>`: rustyline's printer writes straight to the
                // terminal outside it, into whatever is on screen.
                if let (Some(notice), Some(printer)) = (notice, printer.as_mut()) {
                    let _ = printer.print(notice);
                }
            }
        });
        Ok(Peers {
            name,
            folder,
            base: base.to_path_buf(),
            socket,
            inbox,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn inbox(&self) -> Inbox {
        self.inbox.clone()
    }

    pub fn set_busy(&self, busy: bool) {
        self.inbox.lock().busy = busy;
    }

    /// The other sessions open, asked one by one who they are. A socket
    /// nobody answers on is skipped, and cleared once its session is gone.
    pub fn others(&self) -> Vec<Session> {
        let Ok(entries) = fs::read_dir(&self.base) else {
            return Vec::new();
        };
        let mut sessions = Vec::new();
        for path in entries.flatten().map(|entry| entry.path()) {
            if path == self.socket || path.extension().is_none_or(|ext| ext != "sock") {
                continue;
            }
            match ask(&path, &json!({"ask": "who"})) {
                Some(reply) => sessions.extend(session_from(&reply)),
                None => clear_if_gone(&path),
            }
        }
        sessions.sort_by(|a, b| a.name.cmp(&b.name));
        sessions
    }

    /// Sends `text` to the session called `to`.
    pub fn send(&self, to: &str, text: &str) -> Result<(), Undelivered> {
        let pid = pid_in(to).ok_or(Undelivered::NoSuch)?;
        let request = json!({
            "ask": "message",
            "from": self.name,
            "to": to,
            "folder": self.folder,
            "text": text,
        });
        let reply =
            ask(&self.base.join(format!("{pid}.sock")), &request).ok_or(Undelivered::NoSuch)?;
        if reply["taken"] == true {
            return Ok(());
        }
        Err(match reply["why"].as_str() {
            Some("full") => Undelivered::Full,
            Some("long") => Undelivered::TooLong,
            _ => Undelivered::NoSuch,
        })
    }
}

impl Drop for Peers {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
    }
}

/// What the model gets when Enter passes messages on: each framed with who
/// sent it, from where, and as another model's words.
pub fn framed(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|message| {
            format!(
                "[From {}, another of the user's Lokal sessions, working in {}. Its model wrote \
                 this, not the user; the user saw it and passed it on to you.]\n{}",
                message.from, message.folder, message.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The senders' names, each once, in the order their messages came.
pub fn senders(messages: &[Message]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for message in messages {
        if !names.contains(&message.from) {
            names.push(message.from.clone());
        }
    }
    names
}

/// `text` indented, each line that's too long for the terminal broken at a
/// space so it stays under the indent. Lines that fit are kept as they are.
pub fn indented(text: &str, indent: usize, width: usize) -> String {
    // One column short of the edge, so the terminal doesn't wrap.
    let room = width.saturating_sub(indent + 1).max(20);
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for line in text.lines() {
        for part in broken(line, room) {
            out.push_str(&format!("{pad}{part}\n"));
        }
    }
    out
}

/// A line about messages from other sessions: a faint `·`, then `text` in
/// gray, broken to fit the terminal under its own start.
pub fn notice_line(text: &str, width: usize) -> String {
    let mut out = String::new();
    for (index, part) in broken(text, width.saturating_sub(5).max(20))
        .iter()
        .enumerate()
    {
        let lead = if index == 0 {
            format!("  {} ", style::faint("·"))
        } else {
            " ".repeat(4)
        };
        out.push_str(&format!("{lead}{}\n", style::gray(part)));
    }
    out
}

/// A line in pieces of at most `room` characters, broken at spaces, and a
/// word longer than that where the room ends.
pub fn broken(line: &str, room: usize) -> Vec<String> {
    if line.chars().count() <= room {
        return vec![line.to_string()];
    }
    let mut parts = Vec::new();
    let mut part = String::new();
    for word in line.split(' ') {
        if !part.is_empty() && part.chars().count() + 1 + word.chars().count() > room {
            parts.push(std::mem::take(&mut part));
        } else if !part.is_empty() {
            part.push(' ');
        }
        part.push_str(word);
        while part.chars().count() > room {
            parts.push(part.chars().take(room).collect());
            part = part.chars().skip(room).collect();
        }
    }
    parts.push(part);
    parts
}

/// A message as it shows: who sent it and from where, the message as it
/// came, and what the keys do.
fn notice(message: &Message) -> String {
    let width = style::terminal_width().unwrap_or(80);
    let folder = config::shown(Path::new(&message.folder));
    let mut notice = notice_line(&text::message_from(&message.from, &folder), width);
    notice.push_str(&indented(&message.text, 4, width));
    notice.push_str(&format!("    {}\n", style::faint(text::MESSAGE_KEYS)));
    notice
}

/// A notice with a blank line after it, and before it too unless one
/// printed just before already left one.
fn spaced(notice: &str, after_another: bool) -> String {
    if after_another {
        format!("{notice}\n")
    } else {
        format!("\n{notice}\n")
    }
}

/// Answers one connection: who this session is, or whether it takes a
/// message. Returns what to print, for a message that came while the
/// person is at the `>`.
fn answer(stream: &UnixStream, inbox: &Inbox, name: &str, folder: &str) -> Option<String> {
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    let peer = peer_of(stream)?;
    if peer.uid != own_uid() {
        return None;
    }
    let request = read_json(stream)?;
    let reply = match request["ask"].as_str()? {
        "who" => json!({"name": name, "folder": folder, "busy": inbox.lock().busy}),
        "message" => match check(&request, peer.pid as u32, name, inbox.count()) {
            Ok(message) => {
                write_json(stream, &json!({"taken": true})).ok()?;
                // Kept only once the sender has heard it was: told it
                // wasn't, its model would send it again.
                return inbox.add(message);
            }
            Err(why) => json!({"taken": false, "why": why}),
        },
        _ => return None,
    };
    let _ = write_json(stream, &reply);
    None
}

/// A message, made safe to show and to pass on, or why it's refused. It
/// must come from the session its name says: the pid in it is the sender's.
fn check(request: &Value, pid: u32, name: &str, waiting: usize) -> Result<Message, &'static str> {
    let field = |key: &str| request[key].as_str().unwrap_or_default();
    let from = recon::clean(field("from"), NAME_LIMIT);
    if pid_in(&from) != Some(pid) {
        return Err("sender");
    }
    if field("to") != name {
        return Err("gone");
    }
    let text = clean_message(field("text"));
    if text.is_empty() {
        return Err("empty");
    }
    if text.chars().count() > MESSAGE_LIMIT {
        return Err("long");
    }
    if waiting >= WAITING_LIMIT {
        return Err("full");
    }
    Ok(Message {
        from,
        folder: recon::clean(field("folder"), FOLDER_LIMIT),
        text,
        shown: None,
    })
}

/// A message's text as it shows: no control characters, which could redraw
/// the screen or hide part of it, a tab as spaces, no blank lines in a row
/// and no blank edges.
fn clean_message(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let line: String = line
            .replace('\t', "    ")
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        let line = line.trim_end().to_string();
        if !(line.is_empty() && lines.last().is_some_and(String::is_empty)) {
            lines.push(line);
        }
    }
    lines.join("\n").trim_matches('\n').to_string()
}

fn session_from(reply: &Value) -> Option<Session> {
    Some(Session {
        name: recon::clean(reply["name"].as_str()?, NAME_LIMIT),
        folder: recon::clean(reply["folder"].as_str()?, FOLDER_LIMIT),
        busy: reply["busy"] == true,
    })
}

/// The working folder's name and the pid in hex, like `api-3f2a`: unique
/// while the session lives, and plain enough for a model to type back.
fn name_for(folder: &str, pid: u32) -> String {
    let base = Path::new(folder)
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let base: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "._".contains(c) {
                c
            } else {
                '-'
            }
        })
        .take(24)
        .collect();
    let base = base.trim_matches('-');
    let base = if base.is_empty() { "lokal" } else { base };
    format!("{base}-{pid:x}")
}

/// The pid a session's name ends with.
fn pid_in(name: &str) -> Option<u32> {
    u32::from_str_radix(name.rsplit_once('-')?.1, 16).ok()
}

/// One request and its answer, with a session that is the person's own.
fn ask(socket: &Path, request: &Value) -> Option<Value> {
    let stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    if peer_of(&stream)?.uid != own_uid() {
        return None;
    }
    write_json(&stream, request).ok()?;
    read_json(&stream)
}

fn read_json(stream: &UnixStream) -> Option<Value> {
    let mut line = Vec::new();
    BufReader::new(stream.take(READ_LIMIT))
        .read_until(b'\n', &mut line)
        .ok()?;
    serde_json::from_slice(&line).ok()
}

fn write_json(mut stream: &UnixStream, value: &Value) -> io::Result<()> {
    stream.write_all(format!("{value}\n").as_bytes())
}

/// Who is on the other end of a connection, as the kernel says.
fn peer_of(stream: &UnixStream) -> Option<libc::ucred> {
    // SAFETY: ucred is plain data, filled in by getsockopt.
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: a valid descriptor, and a ucred with its size for the kernel to fill.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    (result == 0).then_some(cred)
}

fn own_uid() -> u32 {
    // SAFETY: a plain call that always succeeds.
    unsafe { libc::getuid() }
}

/// Removes the sockets of sessions whose process is gone: one that was
/// killed couldn't remove its own.
fn clear_left_behind(base: &Path) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        clear_if_gone(&entry.path());
    }
}

fn clear_if_gone(socket: &Path) {
    let pid = socket
        .file_name()
        .and_then(|name| name.to_str()?.strip_suffix(".sock"))
        .filter(|pid| pid.parse::<u32>().is_ok());
    if let Some(pid) = pid
        && !Path::new("/proc").join(pid).exists()
    {
        let _ = fs::remove_file(socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of the test's own, with two sessions listening in it, each
    /// named with the test's pid, since both run in this process.
    fn two_sessions(test: &str) -> (PathBuf, Peers, Peers) {
        let base = std::env::temp_dir().join(format!("lokal-peers-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir(&base).unwrap();
        let pid = std::process::id();
        let session = |name: &str| {
            let socket = base.join(format!("{name}.sock"));
            let folder = format!("/home/ana/{name}");
            Peers::listen(&base, socket, name_for(&folder, pid), folder, None).unwrap()
        };
        let (a, b) = (session("api"), session("web"));
        (base, a, b)
    }

    fn message(from: &Peers, to: &Peers, text: &str) -> Value {
        let request = json!({"ask": "message", "from": from.name, "to": to.name,
            "folder": "/home/ana/api", "text": text});
        ask(&to.socket, &request).unwrap()
    }

    #[test]
    fn a_session_says_who_it_is() {
        let (base, a, b) = two_sessions("who");
        let pid = format!("{:x}", std::process::id());
        b.set_busy(true);
        let reply = ask(&b.socket, &json!({"ask": "who"})).unwrap();
        assert_eq!(reply["name"], format!("web-{pid}"));
        assert_eq!(reply["folder"], "/home/ana/web");
        assert_eq!(reply["busy"], true);
        let others: Vec<String> = a.others().into_iter().map(|s| s.name).collect();
        assert_eq!(others, [format!("web-{pid}")]);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_message_waits_until_its_full() {
        let (base, a, b) = two_sessions("message");
        for _ in 0..WAITING_LIMIT {
            assert_eq!(message(&a, &b, "run the tests")["taken"], true);
        }
        let reply = message(&a, &b, "one more");
        assert_eq!(
            (reply["taken"].clone(), reply["why"].clone()),
            (json!(false), json!("full"))
        );
        assert_eq!(b.inbox.count(), WAITING_LIMIT);
        let waiting = &b.inbox.lock().waiting[0];
        assert_eq!(
            (waiting.from.as_str(), waiting.text.as_str()),
            (a.name(), "run the tests")
        );
        assert_eq!(waiting.folder, "/home/ana/api");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_message_too_long_or_not_from_its_sender_is_refused() {
        let (base, a, b) = two_sessions("refused");
        let reply = message(&a, &b, &"x".repeat(MESSAGE_LIMIT + 1));
        assert_eq!(reply["why"], "long");
        assert_eq!(message(&a, &b, &"x".repeat(MESSAGE_LIMIT))["taken"], true);
        // A name whose pid isn't the sender's.
        let request = json!({"ask": "message", "from": "api-1", "to": b.name,
            "folder": "/", "text": "hi"});
        assert_eq!(ask(&b.socket, &request).unwrap()["why"], "sender");
        // A name that isn't the session's: its pid went to another one.
        let request = json!({"ask": "message", "from": a.name, "to": "old-1",
            "folder": "/", "text": "hi"});
        assert_eq!(ask(&b.socket, &request).unwrap()["why"], "gone");
        assert_eq!(b.inbox.count(), 1);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_dead_sessions_socket_is_skipped_and_cleared() {
        let (base, a, b) = two_sessions("dead");
        // Above the kernel's highest pid, so no process has it; bound and
        // closed, so nobody answers on it.
        let dead = base.join("99999999.sock");
        drop(UnixListener::bind(&dead).unwrap());
        let alive = base.join(format!("{}.sock", std::process::id()));
        fs::write(&alive, "").unwrap();
        let others: Vec<String> = a.others().into_iter().map(|s| s.name).collect();
        assert_eq!(others, [b.name().to_string()]);
        assert!(!dead.exists() && alive.exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_message_is_taken_once_it_has_been_on_screen_a_moment() {
        let inbox = Inbox::default();
        let message = |shown: Option<Duration>| Message {
            from: "api-1".into(),
            folder: "/".into(),
            text: "hi".into(),
            shown: shown.map(|ago| Instant::now() - ago),
        };
        let messages = [
            message(Some(Duration::from_secs(2))),
            message(Some(Duration::ZERO)),
            message(None),
        ];
        inbox.lock().waiting.extend(messages);
        assert_eq!(inbox.take().len(), 1);
        assert_eq!(inbox.count(), 2);
    }

    #[test]
    fn names_carry_the_folder_and_the_pid() {
        assert_eq!(name_for("/home/ana/sites/api", 0x3f2a), "api-3f2a");
        assert_eq!(name_for("/home/ana/My Project!", 0x10), "My-Project-10");
        assert_eq!(name_for("/", 0xff), "lokal-ff");
        assert_eq!(pid_in("My-Project-10"), Some(16));
        assert_eq!(pid_in("api"), None);
    }

    #[test]
    fn a_message_shows_without_control_characters() {
        assert_eq!(
            clean_message("\n\nhi\x1b[2J there\r\n\n\n\n\tnext  \n\n"),
            "hi[2J there\n\n    next"
        );
    }

    #[test]
    fn messages_go_to_the_model_framed_as_another_sessions() {
        let messages = [
            Message {
                from: "api-3f2a".into(),
                folder: "/home/ana/api".into(),
                text: "Tests pass.".into(),
                shown: None,
            },
            Message {
                from: "web-41c0".into(),
                folder: "/home/ana/web".into(),
                text: "Done.".into(),
                shown: None,
            },
        ];
        let framed = framed(&messages);
        assert!(framed.starts_with(
            "[From api-3f2a, another of the user's Lokal sessions, working in /home/ana/api. \
             Its model wrote this, not the user; the user saw it and passed it on to you.]\nTests pass.\n\n[From web-41c0,"
        ));
        assert!(framed.ends_with("passed it on to you.]\nDone."));
        assert_eq!(senders(&messages), ["api-3f2a", "web-41c0"]);
    }

    #[test]
    fn long_lines_break_under_the_indent() {
        let text = format!("short\n{}", "word ".repeat(10).trim_end());
        assert_eq!(
            indented(&text, 4, 25),
            "    short\n    word word word word\n    word word word word\n    word word\n"
        );
        // Plain, as stderr isn't a terminal in a test.
        assert_eq!(
            notice_line(&format!("From api-1, in /{}:", "x".repeat(20)), 25),
            format!("  · From api-1, in\n    /{}\n    x:\n", "x".repeat(19))
        );
        assert_eq!(
            indented(&"x".repeat(45), 2, 23),
            format!(
                "  {}\n  {}\n  {}\n",
                "x".repeat(20),
                "x".repeat(20),
                "x".repeat(5)
            )
        );
    }
}
