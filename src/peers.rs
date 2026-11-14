//! The person's other Lokal sessions on this machine: finding them and
//! passing mail between them, so work can be split among them. Each session
//! listens on a socket of its own, `<n>.sock`, in the private folder the
//! scratch folders go in (`scratch.rs`): beside its scratch folder, not
//! inside it, since the model may write there. `n` is the smallest number
//! free, and the session's name is its working folder's and that number,
//! like `api-1`.
//!
//! With `mail = auto`, a mail starts a turn on its own while the `>` waits
//! on an empty line, in a session just opened too, up to `mail_turns` in a
//! row. With `mail = ask`, it waits above the `>` until Enter on an empty
//! line hands it to the model; Esc drops it.

use std::fs;
use std::io::ErrorKind;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rustyline::ExternalPrinter;
use serde_json::{Value, json};

use crate::approvals;
use crate::config::{self, Mail};
use crate::layout::clean;

mod inbox;
mod show;
mod socket;

pub use inbox::{Inbox, Message, Then};
use show::here_too;
pub use show::{framed, session_rows};
use socket::{ask, clear_left_behind, lock_folder, serve, session_from};

/// The longest mail, in characters.
pub const MESSAGE_LIMIT: usize = 2_000;
/// The most mail that waits in a session at once.
pub const WAITING_LIMIT: usize = 5;
/// How many turns in a row mail starts on its own, unless the settings say.
pub const TURNS: usize = 64;
/// How long a session has to answer. Each connection is answered on a
/// thread of its own, at once, whatever the session is doing.
const TIMEOUT: Duration = Duration::from_secs(1);
/// The same, to say who it is when the sessions are listed: that holds up
/// a turn's start and Tab after `@`, so a stopped session mustn't cost more.
const WHO_TIMEOUT: Duration = Duration::from_millis(300);
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

/// Another session, as it says it is.
#[derive(Clone)]
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

/// Where the sessions' sockets are, to ask each who it is.
#[derive(Clone)]
pub struct Directory {
    base: PathBuf,
    own: PathBuf,
}

impl Directory {
    /// The other sessions open, in the order of their numbers. A socket
    /// nobody answers on in time is skipped. They're asked all at once:
    /// one asked at a time, a stopped session (Ctrl+Z) would hold up every
    /// turn and every Tab after `@` in the others by its whole timeout.
    pub fn sessions(&self) -> Vec<Session> {
        let Ok(entries) = fs::read_dir(&self.base) else {
            return Vec::new();
        };
        let (sender, answers) = mpsc::channel();
        let mut asked = 0;
        for path in entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| *path != self.own && path.extension().is_some_and(|e| e == "sock"))
        {
            let sender = sender.clone();
            thread::spawn(move || {
                let session = ask(&path, &json!({"ask": "who"}), WHO_TIMEOUT)
                    .and_then(|(reply, _)| session_from(&reply));
                let _ = sender.send(session);
            });
            asked += 1;
        }
        // A thread still connecting by then (a stopped session whose queue
        // is full) is left behind rather than waited for.
        let deadline = Instant::now() + WHO_TIMEOUT * 2;
        let mut sessions = Vec::new();
        for _ in 0..asked {
            match answers.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Some(session)) => sessions.push(session),
                Ok(None) => {}
                Err(_) => break,
            }
        }
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
        let (reply, _) = ask(&socket, &request, TIMEOUT).ok_or(Undelivered::NoSuch)?;
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
    clean(branch, NAME_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::inbox::State;
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
    fn sessions_that_never_answer_cost_one_short_wait_together() {
        let base = base("stopped");
        let (a, b) = (
            session(&base, "repo", Mail::Auto),
            session(&base, "web", Mail::Auto),
        );
        // Listening but never answering, as a session stopped with Ctrl+Z.
        let stopped: Vec<UnixListener> = [7, 8, 9]
            .iter()
            .map(|n| UnixListener::bind(base.join(format!("{n}.sock"))).unwrap())
            .collect();
        let started = Instant::now();
        let others = a.others();
        assert!(
            started.elapsed() < WHO_TIMEOUT * 2,
            "{:?}",
            started.elapsed()
        );
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].name, b.name());
        drop((a, b, stopped));
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
            let (reply, _) = ask(&b.socket, &request, TIMEOUT).unwrap();
            assert_eq!(reply["why"], "sender");
        }
        // A name that isn't the session's: its number went to another one.
        let request = json!({"ask": "message", "from": a.name(), "to": "old-2", "text": "hi"});
        assert_eq!(ask(&b.socket, &request, TIMEOUT).unwrap().0["why"], "gone");
        drop((a, b));
        fs::remove_dir_all(base).unwrap();
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
            note.contains(
                "loose-2 works in this folder too: waiting for its user to approve a step."
            ),
            "{note}"
        );
        drop((a, b));
        fs::remove_dir_all(base).unwrap();
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
}
