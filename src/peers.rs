//! The person's other Lokal sessions on this machine: finding them and
//! passing mail between them, so work can be split among them. Each session
//! listens on a socket of its own, `<n>.sock`, in the private folder the
//! scratch folders go in (`scratch.rs`): beside its scratch folder, not
//! inside it, since the model may write there. `n` is the smallest number
//! free, and the session's name is its working folder's and that number,
//! like `api-1`.
//!
//! With `mail = auto`, a mail starts a turn on its own once the person has
//! asked the session something, while the `>` waits on an empty line, up to
//! `mail_turns` in a row. With `mail = ask`, it waits above the `>` until
//! Enter on an empty line hands it to the model; Esc drops it.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use rustyline::ExternalPrinter;
use serde_json::{Value, json};

use crate::config::{self, Mail};
use crate::{approvals, recon, style, text};

/// The longest mail, in characters.
pub const MESSAGE_LIMIT: usize = 2_000;
/// The most mail that waits in a session at once.
pub const WAITING_LIMIT: usize = 5;
/// How many turns in a row mail starts on its own, unless the settings say.
pub const TURNS: usize = 64;
/// How long a session has to answer. Each connection is answered on a
/// thread of its own, at once, whatever the session is doing.
const TIMEOUT: Duration = Duration::from_secs(1);
/// The most read of one request: a mail escaped as JSON, and the rest.
const READ_LIMIT: u64 = 64 * 1024;
/// How long mail is on screen before Enter or Esc takes it. One that
/// arrives as Enter is pressed may not be drawn yet, and nobody reads one
/// faster than this.
const SEEN_AFTER: Duration = Duration::from_millis(500);
/// The longest name, branch and folder shown or passed on.
const NAME_LIMIT: usize = 40;
const FOLDER_LIMIT: usize = 300;
/// How much of the person's last request the other sessions see.
const REQUEST_LIMIT: usize = 100;
/// How many of the files a session changed the others see, the latest.
const FILES_SHOWN: usize = 5;
/// Numbers past this aren't tried: something else is wrong by then.
const MOST_SESSIONS: usize = 999;

pub struct Message {
    pub from: String,
    pub folder: String,
    pub text: String,
    /// Typed by the person at the other session's `>`, as `@name …`.
    pub by_user: bool,
    /// To take up right away, rather than with the next turn.
    pub now: bool,
    /// When it showed on screen.
    shown: Option<Instant>,
}

/// Another session, as it says it is.
pub struct Session {
    pub name: String,
    pub folder: String,
    /// The git root around its folder, or the folder.
    pub root: String,
    pub branch: String,
    /// It's working on a turn.
    pub busy: bool,
    /// It's asking its person whether a step may run.
    pub asking: bool,
    /// The person's last request there, cut short.
    pub request: String,
    /// The files it changed, the latest last.
    pub files: Vec<String>,
}

pub enum Undelivered {
    /// No session by that name is open.
    NoSuch,
    /// It has as much mail waiting as it takes.
    Full,
    TooLong,
}

/// What becomes of a mail taken, as its sender hears it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Then {
    /// A turn starts on it now.
    Now,
    /// A turn starts on it when the session is free.
    AfterTurn,
    /// It waits for Enter there: `mail = ask`.
    User,
    /// It waits for the person's next request there: they stopped the last
    /// turn mail started.
    Held,
    /// It waits for the person's next request there: mail started as many
    /// turns in a row as it may.
    Limit,
    /// It goes along with the session's next turn.
    Later,
}

impl Then {
    fn word(self) -> &'static str {
        match self {
            Then::Now => "now",
            Then::AfterTurn => "after_turn",
            Then::User => "user",
            Then::Held => "held",
            Then::Limit => "limit",
            Then::Later => "later",
        }
    }

    fn from_word(word: &str) -> Option<Then> {
        [
            Then::Now,
            Then::AfterTurn,
            Then::User,
            Then::Held,
            Then::Limit,
            Then::Later,
        ]
        .into_iter()
        .find(|then| then.word() == word)
    }
}

struct State {
    mail: Mail,
    turns_limit: usize,
    waiting: Vec<Message>,
    /// A turn is running.
    busy: bool,
    /// An approval question is open.
    asking: bool,
    /// The person is at the `>`: mail prints above it at once.
    at_prompt: bool,
    /// Mail printed at this prompt already, with a blank line after it.
    printed: bool,
    /// The `>` line isn't empty: mail doesn't take it away.
    typing: bool,
    /// Mail waits for the person's next request: they stopped a turn mail
    /// started, or said no in one. Otherwise a mail would start the next
    /// turn at once, and two models could go on with nobody able to stop
    /// them.
    held: bool,
    /// Turns mail started since the person last asked something.
    turns: usize,
    /// Mail ended the `>` to start a turn.
    woken: bool,
    /// That the model takes mail up on its own was said once, so it isn't
    /// again.
    told_taken_up: bool,
    /// What the others see this session doing.
    request: String,
    files: Vec<String>,
}

impl State {
    /// What becomes of a mail, leaving aside whether the session is free.
    fn fate(&self, message: &Message) -> Then {
        if self.mail == Mail::Ask {
            return Then::User;
        }
        if !message.by_user {
            if !message.now {
                return Then::Later;
            }
            if self.held {
                return Then::Held;
            }
            if self.turns >= self.turns_limit {
                return Then::Limit;
            }
        }
        Then::Now
    }

    /// The same, now: one that would start waits while the session is busy,
    /// or the person is away from the `>` or typing at it.
    fn then(&self, message: &Message) -> Then {
        match self.fate(message) {
            Then::Now if self.busy || !self.at_prompt || self.typing => Then::AfterTurn,
            fate => fate,
        }
    }

    /// What a notice says under a mail: what becomes of it.
    fn footer(&mut self, then: Then) -> Option<String> {
        match then {
            Then::Now if self.told_taken_up => None,
            Then::Now => {
                self.told_taken_up = true;
                Some(text::MAIL_TAKEN_UP.to_string())
            }
            Then::Later => Some(text::MAIL_LATER.to_string()),
            Then::Limit => Some(text::mail_limit(self.turns_limit)),
            Then::Held => Some(text::MAIL_HELD.to_string()),
            // At the `>`, only while the person types: it goes once they're done.
            Then::AfterTurn => Some(text::MAIL_AFTER_YOU.to_string()),
            Then::User => Some(text::MAIL_KEYS.to_string()),
        }
    }
}

/// The mail waiting, shared by the listener and the `>` prompt.
#[derive(Clone)]
pub struct Inbox(Arc<Mutex<State>>);

/// A mail taken: what becomes of it, and what to print above the `>`.
struct Arrival {
    then: Then,
    print: Option<String>,
}

impl Inbox {
    fn new(mail: Mail, turns_limit: usize) -> Inbox {
        Inbox(Arc::new(Mutex::new(State {
            mail,
            turns_limit,
            waiting: Vec::new(),
            busy: false,
            asking: false,
            at_prompt: false,
            printed: false,
            typing: false,
            held: false,
            turns: 0,
            woken: false,
            told_taken_up: false,
            request: String::new(),
            files: Vec::new(),
        })))
    }

    pub fn count(&self) -> usize {
        self.lock().waiting.len()
    }

    /// The person is at the `>` from now on, and what came meanwhile shows.
    /// True when a turn starts on mail at once instead; then the `>` isn't
    /// shown.
    pub fn enter_prompt(&self) -> bool {
        let mut guard = self.lock();
        let state = &mut *guard;
        state.at_prompt = true;
        state.typing = false;
        let start = state
            .waiting
            .iter()
            .any(|message| state.then(message) == Then::Now);
        let mut printed = false;
        for index in 0..state.waiting.len() {
            if state.waiting[index].shown.is_none() {
                let then = state.then(&state.waiting[index]);
                let footer = state.footer(then);
                let notice = notice(&state.waiting[index], footer.as_deref());
                eprint!("{}", spaced(&notice, printed));
                state.waiting[index].shown = Some(Instant::now());
                printed = true;
            }
        }
        state.printed = printed;
        state.at_prompt = !start;
        start
    }

    /// The person left the `>`. True when mail ended it.
    pub fn leave_prompt(&self) -> bool {
        let mut state = self.lock();
        state.at_prompt = false;
        state.typing = false;
        std::mem::take(&mut state.woken)
    }

    pub fn set_typing(&self, typing: bool) {
        self.lock().typing = typing;
    }

    /// An approval question opened or closed, for the others to see.
    pub fn set_asking(&self, asking: bool) {
        self.lock().asking = asking;
    }

    /// Mail that ended the `>`, shown before its turn starts.
    pub fn show_arrived(&self) {
        let mut guard = self.lock();
        let state = &mut *guard;
        let mut printed = false;
        for index in 0..state.waiting.len() {
            if state.waiting[index].shown.is_none() {
                let footer = state.footer(Then::Now);
                let notice = notice(&state.waiting[index], footer.as_deref());
                eprint!("{}", spaced(&notice, printed));
                state.waiting[index].shown = Some(Instant::now());
                printed = true;
            }
        }
    }

    /// Takes the mail that has been on screen a moment, for Enter to pass
    /// on or Esc to drop.
    pub fn take(&self) -> Vec<Message> {
        self.take_where(|_, message| message.shown.is_some_and(|at| at.elapsed() >= SEEN_AFTER))
    }

    /// Takes the mail a turn starts on, with what goes along with the next
    /// turn.
    pub fn take_for_turn(&self) -> Vec<Message> {
        self.take_where(|state, message| {
            message.shown.is_some() && matches!(state.fate(message), Then::Now | Then::Later)
        })
    }

    /// Takes what goes along with the next turn, for one the person asked.
    pub fn take_later(&self) -> Vec<Message> {
        self.take_where(|state, message| {
            message.shown.is_some() && state.fate(message) == Then::Later
        })
    }

    fn take_where(&self, wanted: impl Fn(&State, &Message) -> bool) -> Vec<Message> {
        let mut guard = self.lock();
        let state = &mut *guard;
        let (taken, kept) = std::mem::take(&mut state.waiting)
            .into_iter()
            .partition(|message| wanted(state, message));
        state.waiting = kept;
        taken
    }

    /// The person asked something, or passed mail on: mail may start turns
    /// again, the count starting over. `request` is what the others see.
    pub fn person_acted(&self, request: Option<&str>) {
        let mut state = self.lock();
        state.held = false;
        state.turns = 0;
        if let Some(request) = request {
            state.request = recon::clean(request, REQUEST_LIMIT);
        }
    }

    /// A turn starts on mail: one more in a row, unless the person wrote
    /// some of it at another session's `>`.
    pub fn turn_on_mail(&self, messages: &[Message]) {
        match messages.iter().find(|message| message.by_user) {
            Some(written) => self.person_acted(Some(&written.text)),
            None => self.lock().turns += 1,
        }
    }

    /// The person stopped a turn mail started, or said no in one: mail waits
    /// for their next request.
    pub fn hold(&self) {
        self.lock().held = true;
    }

    /// Mail that ended the `>` makes it return: rustyline ends `readline`
    /// on SIGINT. Sent while the state is held, so the person can't leave
    /// the `>` in between; and handled on this thread before `kill`
    /// returns. Should they have left a moment before, Lokal's own handler
    /// raises its flag instead, and the turn starting clears it first.
    fn wake(&self) {
        let state = self.lock();
        if state.at_prompt && state.woken {
            // SAFETY: a plain call, signalling this process.
            unsafe {
                libc::kill(libc::getpid(), libc::SIGINT);
            }
        }
    }

    /// A mail checked and taken, or why not.
    fn add(&self, mut message: Message) -> Result<Arrival, &'static str> {
        let mut state = self.lock();
        if state.waiting.len() >= WAITING_LIMIT {
            return Err("full");
        }
        let then = state.then(&message);
        let mut print = None;
        if then == Then::Now {
            // Shown by the `>` as it ends, before the turn.
            state.woken = true;
        } else if state.at_prompt {
            message.shown = Some(Instant::now());
            let footer = state.footer(then);
            print = Some(spaced(&notice(&message, footer.as_deref()), state.printed));
            state.printed = true;
        }
        state.waiting.push(message);
        Ok(Arrival { then, print })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Where the sessions' sockets are, to ask each who it is.
#[derive(Clone)]
pub struct Directory {
    base: PathBuf,
    own: PathBuf,
}

impl Directory {
    /// The other sessions open, in the order of their numbers. A socket
    /// nobody answers on is skipped.
    pub fn sessions(&self) -> Vec<Session> {
        let Ok(entries) = fs::read_dir(&self.base) else {
            return Vec::new();
        };
        let mut sessions: Vec<Session> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| *path != self.own && path.extension().is_some_and(|e| e == "sock"))
            .filter_map(|path| ask(&path, &json!({"ask": "who"})))
            .filter_map(|(reply, _)| session_from(&reply))
            .collect();
        sessions.sort_by_key(|session| number_in(&session.name));
        sessions
    }

    pub fn names(&self) -> Vec<String> {
        self.sessions().into_iter().map(|s| s.name).collect()
    }
}

/// What the listener and its connections share.
struct Shared {
    name: String,
    folder: String,
    root: PathBuf,
    base: PathBuf,
    inbox: Inbox,
    printer: Mutex<Option<Box<dyn ExternalPrinter + Send>>>,
}

impl Shared {
    fn who(&self) -> Value {
        let branch = branch_of(&self.root);
        let state = self.inbox.lock();
        json!({
            "name": self.name,
            "folder": self.folder,
            "root": self.root.display().to_string(),
            "branch": branch,
            "busy": state.busy,
            "asking": state.asking,
            "request": state.request,
            "files": state.files,
        })
    }
}

/// This session among the others: its name, its socket, and its mail.
pub struct Peers {
    shared: Arc<Shared>,
    socket: PathBuf,
    /// The sessions in this repository at the last turn, with their branches.
    known: Mutex<Vec<(String, String)>>,
}

impl Peers {
    /// Listens in `base`, the private folder `scratch.rs` checked, on the
    /// smallest number free. None when no socket can be made; then the
    /// model isn't told of the others.
    pub fn start(
        base: &Path,
        folder: &Path,
        mail: Mail,
        turns_limit: usize,
        printer: Option<Box<dyn ExternalPrinter + Send>>,
    ) -> Option<Peers> {
        // Held while a number is picked: two sessions starting at once
        // can't then both clear and take the same one.
        let _lock = lock_folder(base)?;
        clear_left_behind(base);
        for number in 1..=MOST_SESSIONS {
            let socket = base.join(format!("{number}.sock"));
            match UnixListener::bind(&socket) {
                Ok(listener) => {
                    let shared = Shared {
                        name: name_for(folder, number),
                        folder: folder.display().to_string(),
                        root: approvals::project_root(folder),
                        base: base.to_path_buf(),
                        inbox: Inbox::new(mail, turns_limit),
                        printer: Mutex::new(printer),
                    };
                    return Some(Peers::listen(listener, socket, shared));
                }
                Err(error) if error.kind() == ErrorKind::AddrInUse => continue,
                Err(_) => return None,
            }
        }
        None
    }

    fn listen(listener: UnixListener, socket: PathBuf, shared: Shared) -> Peers {
        let shared = Arc::new(shared);
        let serving = shared.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let shared = serving.clone();
                thread::spawn(move || serve(stream, &shared));
            }
        });
        Peers {
            shared,
            socket,
            known: Mutex::new(Vec::new()),
        }
    }

    pub fn name(&self) -> &str {
        &self.shared.name
    }

    pub fn inbox(&self) -> Inbox {
        self.shared.inbox.clone()
    }

    pub fn directory(&self) -> Directory {
        Directory {
            base: self.shared.base.clone(),
            own: self.socket.clone(),
        }
    }

    pub fn set_busy(&self, busy: bool) {
        self.shared.inbox.lock().busy = busy;
    }

    /// A file this session changed, for the others to see.
    pub fn wrote(&self, path: &Path) {
        let shown = match path.strip_prefix(&self.shared.root) {
            Ok(inside) => inside.display().to_string(),
            Err(_) => config::shown(path),
        };
        let mut state = self.shared.inbox.lock();
        state.files.retain(|file| *file != shown);
        state.files.push(shown);
        let extra = state.files.len().saturating_sub(FILES_SHOWN);
        state.files.drain(..extra);
    }

    pub fn others(&self) -> Vec<Session> {
        self.directory().sessions()
    }

    /// Sends `text` to the session called `to`, with or without its `@`.
    pub fn send(
        &self,
        to: &str,
        text: &str,
        now: bool,
        by_user: bool,
    ) -> Result<Then, Undelivered> {
        let to = to.trim_start_matches('@');
        let number = number_in(to).ok_or(Undelivered::NoSuch)?;
        let request = json!({
            "ask": "message",
            "from": self.shared.name,
            "to": to,
            "folder": self.shared.folder,
            "text": text,
            "now": now,
            "by_user": by_user,
        });
        let socket = self.shared.base.join(format!("{number}.sock"));
        let (reply, _) = ask(&socket, &request).ok_or(Undelivered::NoSuch)?;
        if reply["taken"] == true {
            return Ok(reply["then"]
                .as_str()
                .and_then(Then::from_word)
                .unwrap_or(Then::User));
        }
        Err(match reply["why"].as_str() {
            Some("full") => Undelivered::Full,
            Some("long") => Undelivered::TooLong,
            _ => Undelivered::NoSuch,
        })
    }

    /// What changed among the sessions working where this one does since
    /// the last turn, as a note for the model; None when nothing did. It
    /// doesn't show: these are Lokal's own facts, not another model's
    /// words, and /sessions shows the same.
    pub fn news(&self) -> Option<String> {
        let root = self.shared.root.display().to_string();
        let here: Vec<Session> = self
            .others()
            .into_iter()
            .filter(|session| session.root == root)
            .collect();
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        let mut lines = Vec::new();
        for session in &here {
            match known.iter().find(|(name, _)| *name == session.name) {
                None => lines.push(here_too(session)),
                Some((_, branch)) if *branch != session.branch => {
                    lines.push(format!(
                        "{} is on branch {} now.",
                        session.name, session.branch
                    ));
                }
                Some(_) => {}
            }
        }
        for (name, _) in known.iter() {
            if !here.iter().any(|session| session.name == *name) {
                lines.push(format!("{name} closed."));
            }
        }
        *known = here
            .iter()
            .map(|session| (session.name.clone(), session.branch.clone()))
            .collect();
        if lines.is_empty() {
            return None;
        }
        Some(format!(
            "(From Lokal, not the user: news of the user's other Lokal sessions working where you do. {} \
             Agree with them by mail on who works where before you both change the same files.)",
            lines.join(" ")
        ))
    }
}

impl Drop for Peers {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
    }
}

/// What the model gets when a turn starts on mail, or when Enter passes it
/// on: each framed with who sent it, from where, and as another model's
/// words unless the person wrote it.
pub fn framed(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|message| {
            let frame = if message.by_user {
                format!(
                    "Mail from the user, written in {}, another of their Lokal sessions, working in {}. \
                     To answer where they are, mail {}.",
                    message.from, message.folder, message.from
                )
            } else {
                let reply = if message.now {
                    "Answer by mail only if it asks or needs something from you, a plain yes or no included."
                } else {
                    "It needs no answer."
                };
                format!(
                    "Mail from {}, another of the user's Lokal sessions, working in {}. Its model wrote \
                     this, not the user. {reply}",
                    message.from, message.folder
                )
            };
            format!("[{frame}]\n{}", message.text)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Another session working where this one does, as the model first hears
/// of it: "web-2 works in this repository too: branch main, working, last
/// asked "fold the menu", changed src/menu.rs."
fn here_too(session: &Session) -> String {
    let mut parts = Vec::new();
    if !session.branch.is_empty() {
        parts.push(format!("branch {}", session.branch));
    }
    parts.push(session_state(session).to_string());
    if !session.request.is_empty() {
        parts.push(format!("last asked \"{}\"", session.request));
    }
    if !session.files.is_empty() {
        parts.push(format!("changed {}", session.files.join(", ")));
    }
    // Without a branch, it isn't a repository.
    let place = if session.branch.is_empty() {
        "folder"
    } else {
        "repository"
    };
    format!(
        "{} works in this {place} too: {}.",
        session.name,
        parts.join(", ")
    )
}

/// What a session is doing, in a word or three: the same words /sessions
/// shows the person.
pub fn session_state(session: &Session) -> &'static str {
    match (session.busy, session.asking) {
        (true, true) => text::SESSION_ASKING,
        (true, false) => text::WORKING,
        (false, _) => text::SESSION_IDLE,
    }
}

/// The senders' names, each once, in the order their mail came.
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

/// A line about mail and the other sessions: a faint `·`, then `text` in
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

/// A mail as it shows: who sent it and from where, the mail as it came,
/// and `footer`, what becomes of it.
fn notice(message: &Message, footer: Option<&str>) -> String {
    let width = style::terminal_width().unwrap_or(80);
    let folder = config::shown(Path::new(&message.folder));
    let from = if message.by_user {
        text::mail_from_user(&message.from, &folder)
    } else {
        text::mail_from(&message.from, &folder)
    };
    let mut notice = notice_line(&from, width);
    notice.push_str(&indented(&message.text, 4, width));
    for line in indented(footer.unwrap_or_default(), 4, width).lines() {
        notice.push_str(&format!("{}\n", style::faint(line)));
    }
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

/// Answers one connection, then prints a mail that came while the person
/// is at the `>`, or ends the `>` for a turn to start on it.
fn serve(stream: UnixStream, shared: &Shared) {
    let Some(arrival) = answer(&stream, shared) else {
        return;
    };
    drop(stream);
    // Without a terminal, as in a test, there's no `>` to print above or end.
    let mut printer = shared
        .printer
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let Some(printer) = printer.as_mut() else {
        return;
    };
    // Only at the `>`: rustyline's printer writes straight to the terminal
    // outside it, into whatever is on screen.
    if let Some(print) = arrival.print {
        let _ = printer.print(print);
    }
    if arrival.then == Then::Now {
        shared.inbox.wake();
    }
}

/// Who this session is, or whether it takes a mail.
fn answer(stream: &UnixStream, shared: &Shared) -> Option<Arrival> {
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    let peer = peer_of(stream)?;
    if peer.uid != own_uid() {
        return None;
    }
    let request = read_json(stream)?;
    let taken = match request["ask"].as_str()? {
        "who" => {
            let _ = write_json(stream, &shared.who());
            return None;
        }
        "message" => check(&request, &shared.name)
            .and_then(|message| {
                if sender_is(&shared.base, &message.from, peer.pid) {
                    Ok(message)
                } else {
                    Err("sender")
                }
            })
            .and_then(|message| shared.inbox.add(message)),
        _ => return None,
    };
    match taken {
        Ok(arrival) => {
            let _ = write_json(stream, &json!({"taken": true, "then": arrival.then.word()}));
            Some(arrival)
        }
        Err(why) => {
            let _ = write_json(stream, &json!({"taken": false, "why": why}));
            None
        }
    }
}

/// A mail, made safe to show and to pass on, or why it's refused.
fn check(request: &Value, name: &str) -> Result<Message, &'static str> {
    let field = |key: &str| request[key].as_str().unwrap_or_default();
    let from = recon::clean(field("from"), NAME_LIMIT);
    if field("to").trim_start_matches('@') != name {
        return Err("gone");
    }
    let text = clean_message(field("text"));
    if text.is_empty() {
        return Err("empty");
    }
    if text.chars().count() > MESSAGE_LIMIT {
        return Err("long");
    }
    Ok(Message {
        from,
        folder: recon::clean(field("folder"), FOLDER_LIMIT),
        text,
        by_user: request["by_user"] == true,
        now: request["now"] != false,
        shown: None,
    })
}

/// Whether the session called `from` is the process `pid`: its socket is
/// asked who it is, and the kernel says whose socket it is. Only a session
/// can send, so the model can't pass a command's words off as the person's.
fn sender_is(base: &Path, from: &str, pid: i32) -> bool {
    let Some(number) = number_in(from) else {
        return false;
    };
    ask(&base.join(format!("{number}.sock")), &json!({"ask": "who"}))
        .is_some_and(|(reply, peer)| peer.pid == pid && reply["name"] == from)
}

/// A mail's text as it shows: no control characters, which could redraw
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
    let field =
        |key: &str, limit: usize| recon::clean(reply[key].as_str().unwrap_or_default(), limit);
    let name = field("name", NAME_LIMIT);
    number_in(&name)?;
    Some(Session {
        folder: field("folder", FOLDER_LIMIT),
        root: field("root", FOLDER_LIMIT),
        branch: field("branch", NAME_LIMIT),
        busy: reply["busy"] == true,
        asking: reply["asking"] == true,
        request: field("request", REQUEST_LIMIT),
        files: reply["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .rev()
            .take(FILES_SHOWN)
            .map(|file| recon::clean(file, FOLDER_LIMIT))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        name,
    })
}

/// The working folder's name and the session's number, like `api-1`: plain
/// enough for a model, or the person after an `@`, to type back.
fn name_for(folder: &Path, number: usize) -> String {
    let base = folder
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
    format!("{base}-{number}")
}

/// The number a session's name ends with.
fn number_in(name: &str) -> Option<usize> {
    name.rsplit_once('-')?.1.parse().ok()
}

/// The branch checked out in the repository at `root`, read from its files
/// rather than by running git, which runs what a repository's settings name.
fn branch_of(root: &Path) -> String {
    let git = root.join(".git");
    let folder = if git.is_dir() {
        git
    } else {
        // A worktree's .git is a file naming its folder.
        let pointer = fs::read_to_string(&git).unwrap_or_default();
        match pointer.trim().strip_prefix("gitdir:") {
            Some(path) => root.join(path.trim()),
            None => return String::new(),
        }
    };
    let head = fs::read_to_string(folder.join("HEAD")).unwrap_or_default();
    let head = head.trim();
    let branch = match head.strip_prefix("ref: refs/heads/") {
        Some(branch) => branch,
        None => head.get(..7).unwrap_or(head),
    };
    recon::clean(branch, NAME_LIMIT)
}

/// One request and its answer, with a session that is the person's own,
/// and who the kernel says is listening there.
fn ask(socket: &Path, request: &Value) -> Option<(Value, libc::ucred)> {
    let stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    let peer = peer_of(&stream)?;
    if peer.uid != own_uid() {
        return None;
    }
    write_json(&stream, request).ok()?;
    Some((read_json(&stream)?, peer))
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

/// The folder, locked for as long as the file lives.
fn lock_folder(base: &Path) -> Option<File> {
    let folder = File::open(base).ok()?;
    // SAFETY: a plain call on a descriptor that stays open for it.
    (unsafe { libc::flock(folder.as_raw_fd(), libc::LOCK_EX) } == 0).then_some(folder)
}

/// Removes the sockets nobody listens on: a session that was killed
/// couldn't remove its own.
fn clear_left_behind(base: &Path) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.extension().is_some_and(|e| e == "sock")
            && UnixStream::connect(&path)
                .is_err_and(|error| error.kind() == ErrorKind::ConnectionRefused)
        {
            let _ = fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of the test's own, with a git repository in it.
    fn base(test: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("lokal-peers-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("repo/.git")).unwrap();
        fs::write(base.join("repo/.git/HEAD"), "ref: refs/heads/main\n").unwrap();
        base
    }

    fn session(base: &Path, folder: &str, mail: Mail) -> Peers {
        Peers::start(base, &base.join(folder), mail, 3, None).unwrap()
    }

    /// `from` mails `to` as a model would, now.
    fn mail(from: &Peers, to: &Peers, text: &str) -> Result<Then, Undelivered> {
        from.send(to.name(), text, true, false)
    }

    fn set(peers: &Peers, change: impl FnOnce(&mut State)) {
        change(&mut peers.shared.inbox.lock());
    }

    #[test]
    fn sessions_take_the_smallest_number_free() {
        let base = base("numbers");
        let a = session(&base, "repo", Mail::Auto);
        let b = session(&base, "web", Mail::Auto);
        assert_eq!((a.name(), b.name()), ("repo-1", "web-2"));
        drop(a);
        // Bound and closed, so nobody answers on it: left by a killed session.
        drop(UnixListener::bind(base.join("3.sock")).unwrap());
        let c = session(&base, "api", Mail::Auto);
        let d = session(&base, "api", Mail::Auto);
        assert_eq!((c.name(), d.name()), ("api-1", "api-3"));
        drop((b, c, d));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_session_says_who_it_is_and_what_it_does() {
        let base = base("who");
        let (a, b) = (
            session(&base, "repo", Mail::Auto),
            session(&base, "repo/src", Mail::Auto),
        );
        b.set_busy(true);
        b.inbox()
            .person_acted(Some("add a menu that folds\nand more"));
        b.wrote(&base.join("repo/src/menu.rs"));
        b.wrote(&base.join("elsewhere.txt"));
        let others = a.others();
        assert_eq!(others.len(), 1);
        let other = &others[0];
        assert_eq!(other.name, "src-2");
        assert_eq!(other.root, base.join("repo").display().to_string());
        assert_eq!((other.branch.as_str(), other.busy), ("main", true));
        assert_eq!(other.request, "add a menu that folds");
        assert_eq!(other.files[0], "src/menu.rs");
        drop((a, b));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn mail_starts_a_turn_on_its_own_until_stopped_or_at_the_limit() {
        let base = base("fate");
        let (a, b) = (
            session(&base, "repo", Mail::Auto),
            session(&base, "web", Mail::Auto),
        );
        // Away from the `>`, it waits for it; at it, it starts at once.
        assert_eq!(mail(&a, &b, "hi").ok(), Some(Then::AfterTurn));
        set(&b, |state| state.at_prompt = true);
        assert_eq!(mail(&a, &b, "hi").ok(), Some(Then::Now));
        assert_eq!(
            a.send(b.name(), "news", false, false).ok(),
            Some(Then::Later)
        );
        // Stopped by the person, until they ask something.
        b.inbox().hold();
        assert_eq!(mail(&a, &b, "hi").ok(), Some(Then::Held));
        b.inbox().person_acted(Some("work"));
        set(&b, |state| {
            state.turns = 3;
            state.waiting.clear();
        });
        assert_eq!(mail(&a, &b, "hi").ok(), Some(Then::Limit));
        // The person's own mail starts one whatever the count.
        assert_eq!(
            a.send(&format!("@{}", b.name()), "go", true, true).ok(),
            Some(Then::Now)
        );
        drop((a, b));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn with_mail_ask_everything_waits_for_the_person() {
        let base = base("ask");
        let (a, b) = (
            session(&base, "repo", Mail::Auto),
            session(&base, "web", Mail::Ask),
        );
        set(&b, |state| state.at_prompt = true);
        b.inbox().person_acted(None);
        assert_eq!(mail(&a, &b, "hi").ok(), Some(Then::User));
        assert_eq!(a.send(b.name(), "go", true, true).ok(), Some(Then::User));
        drop((a, b));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn mail_too_long_full_or_not_from_its_sender_is_refused() {
        let base = base("refused");
        let (a, b) = (
            session(&base, "repo", Mail::Auto),
            session(&base, "web", Mail::Auto),
        );
        assert!(matches!(
            mail(&a, &b, &"x".repeat(MESSAGE_LIMIT + 1)),
            Err(Undelivered::TooLong)
        ));
        for _ in 0..WAITING_LIMIT {
            assert!(mail(&a, &b, "hi").is_ok());
        }
        assert!(matches!(mail(&a, &b, "one more"), Err(Undelivered::Full)));
        // A name that isn't the session's at that number, and a number nobody has.
        for from in ["other-1", "repo-9"] {
            let request = json!({"ask": "message", "from": from, "to": b.name(), "text": "hi"});
            let (reply, _) = ask(&b.socket, &request).unwrap();
            assert_eq!(reply["why"], "sender");
        }
        // A name that isn't the session's: its number went to another one.
        let request = json!({"ask": "message", "from": a.name(), "to": "old-2", "text": "hi"});
        assert_eq!(ask(&b.socket, &request).unwrap().0["why"], "gone");
        drop((a, b));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn mail_is_taken_once_it_has_been_on_screen_a_moment() {
        let inbox = Inbox::new(Mail::Auto, 3);
        let message = |shown: Option<Duration>, now: bool| Message {
            from: "api-1".into(),
            folder: "/".into(),
            text: "hi".into(),
            by_user: false,
            now,
            shown: shown.map(|ago| Instant::now() - ago),
        };
        let second = Some(Duration::from_secs(1));
        inbox.lock().waiting.extend([
            message(second, true),
            message(Some(Duration::ZERO), true),
            message(None, true),
            message(second, false),
        ]);
        assert_eq!(inbox.take_later().len(), 1);
        assert_eq!(inbox.take().len(), 1);
        inbox.person_acted(None);
        // A turn on mail takes what's on screen, however briefly.
        assert_eq!(inbox.take_for_turn().len(), 1);
        assert_eq!(inbox.count(), 1);
    }

    #[test]
    fn news_says_who_came_moved_and_went_in_this_repository() {
        let base = base("news");
        let a = session(&base, "repo", Mail::Auto);
        let elsewhere = session(&base, "web", Mail::Auto);
        let b = session(&base, "repo", Mail::Auto);
        b.inbox().person_acted(Some("fold the menu"));
        b.set_busy(true);
        b.wrote(&base.join("repo/src/menu.rs"));
        let note = a.news().unwrap();
        assert!(
            note.contains(
                "repo-3 works in this repository too: branch main, working, last asked \
                 \"fold the menu\", changed src/menu.rs."
            ),
            "{note}"
        );
        assert!(!note.contains("web-2"), "{note}");
        assert!(a.news().is_none());
        fs::write(base.join("repo/.git/HEAD"), "ref: refs/heads/menu\n").unwrap();
        assert!(a.news().unwrap().contains("repo-3 is on branch menu now"));
        drop(b);
        assert!(a.news().unwrap().contains("repo-3 closed"));
        drop((a, elsewhere));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn outside_a_repository_another_session_is_in_this_folder() {
        let base = base("folder");
        let (a, b) = (
            session(&base, "loose", Mail::Auto),
            session(&base, "loose", Mail::Auto),
        );
        b.inbox().set_asking(true);
        set(&b, |state| state.busy = true);
        let note = a.news().unwrap();
        assert!(
            note.contains("loose-2 works in this folder too: waiting for a yes."),
            "{note}"
        );
        drop((a, b));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn that_the_model_takes_mail_up_on_its_own_is_said_once() {
        let inbox = Inbox::new(Mail::Auto, 3);
        let mut state = inbox.lock();
        assert_eq!(
            state.footer(Then::Now).as_deref(),
            Some(text::MAIL_TAKEN_UP)
        );
        assert_eq!(state.footer(Then::Now), None);
        assert_eq!(state.footer(Then::User).as_deref(), Some(text::MAIL_KEYS));
    }

    #[test]
    fn names_carry_the_folder_and_the_number() {
        assert_eq!(name_for(Path::new("/home/ana/sites/api"), 1), "api-1");
        assert_eq!(
            name_for(Path::new("/home/ana/My Project!"), 12),
            "My-Project-12"
        );
        assert_eq!(name_for(Path::new("/"), 3), "lokal-3");
        assert_eq!(number_in("My-Project-12"), Some(12));
        assert_eq!(number_in("api"), None);
    }

    #[test]
    fn a_branch_is_read_from_the_repositorys_files() {
        let base = base("branch");
        assert_eq!(branch_of(&base.join("repo")), "main");
        fs::write(base.join("repo/.git/HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(branch_of(&base.join("repo")), "0123456");
        fs::create_dir_all(base.join("tree")).unwrap();
        fs::write(base.join("tree/.git"), "gitdir: ../repo/.git\n").unwrap();
        assert_eq!(branch_of(&base.join("tree")), "0123456");
        assert_eq!(branch_of(&base), "");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn mail_shows_without_control_characters() {
        assert_eq!(
            clean_message("\n\nhi\x1b[2J there\r\n\n\n\n\tnext  \n\n"),
            "hi[2J there\n\n    next"
        );
    }

    #[test]
    fn mail_goes_to_the_model_framed_by_who_wrote_it() {
        let message = |from: &str, by_user: bool, now: bool| Message {
            from: from.into(),
            folder: format!("/home/ana/{from}"),
            text: "Tests pass.".into(),
            by_user,
            now,
            shown: None,
        };
        let messages = [
            message("api-1", false, true),
            message("web-2", true, true),
            message("api-1", false, false),
        ];
        let framed = framed(&messages);
        assert!(framed.starts_with(
            "[Mail from api-1, another of the user's Lokal sessions, working in /home/ana/api-1. \
             Its model wrote this, not the user. Answer by mail only if it asks or needs something \
             from you, a plain yes or no included.]\nTests pass.\n\n[Mail from the user, written in web-2,"
        ));
        assert!(framed.contains("To answer where they are, mail web-2.]\nTests pass."));
        assert!(framed.ends_with("not the user. It needs no answer.]\nTests pass."));
        assert_eq!(senders(&messages), ["api-1", "web-2"]);
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
            notice_line(&format!("Mail from api-1, in /{}:", "x".repeat(20)), 25),
            format!("  · Mail from api-1, in\n    /{}\n    x:\n", "x".repeat(19))
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
