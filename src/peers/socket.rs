//! The wire between sessions, and its trust boundary: who is on the other
//! end, as the kernel says, whether a mail comes from the session it names,
//! and what of it is safe to show and to pass on.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::PoisonError;
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use super::inbox::Arrival;
use super::{
    FILES_SHOWN, FOLDER_LIMIT, MESSAGE_LIMIT, Message, NAME_LIMIT, READ_LIMIT, REQUEST_LIMIT,
    Session, Shared, TIMEOUT, Then, number_in,
};
use crate::layout::{clean, clean_message};
use crate::{screen, tty};

/// Answers one connection, then prints a mail that came while the person
/// is at the `>`, or ends the `>` for a turn to start on it.
pub(super) fn serve(stream: UnixStream, shared: &Shared) {
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
    if let Some(layout) = arrival.print {
        let shown = layout(tty::width().unwrap_or(80));
        // rustyline prints it above the `>`; kept as it would print, for a
        // redraw (#49).
        screen::kept(layout);
        let _ = printer.print(shown);
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
    let from = clean(field("from"), NAME_LIMIT);
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
        folder: clean(field("folder"), FOLDER_LIMIT),
        text,
        by_user: request["by_user"] == true,
        now: request["now"] != false,
        shown: None,
        arrived: SystemTime::now(),
    })
}

/// Whether the session called `from` is the process `pid`: its socket is
/// asked who it is, and the kernel says whose socket it is. Only a session
/// can send, so the model can't pass a command's words off as the person's.
fn sender_is(base: &Path, from: &str, pid: i32) -> bool {
    let Some(number) = number_in(from) else {
        return false;
    };
    ask(
        &base.join(format!("{number}.sock")),
        &json!({"ask": "who"}),
        TIMEOUT,
    )
    .is_some_and(|(reply, peer)| peer.pid == pid && reply["name"] == from)
}

pub(super) fn session_from(reply: &Value) -> Option<Session> {
    let field = |key: &str, limit: usize| clean(reply[key].as_str().unwrap_or_default(), limit);
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
            .map(|file| clean(file, FOLDER_LIMIT))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        name,
    })
}

/// One request and its answer, with a session that is the person's own,
/// and who the kernel says is listening there.
pub(super) fn ask(
    socket: &Path,
    request: &Value,
    timeout: Duration,
) -> Option<(Value, libc::ucred)> {
    let stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
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
pub(super) fn lock_folder(base: &Path) -> Option<File> {
    let folder = File::open(base).ok()?;
    // SAFETY: a plain call on a descriptor that stays open for it.
    (unsafe { libc::flock(folder.as_raw_fd(), libc::LOCK_EX) } == 0).then_some(folder)
}

/// Removes the sockets nobody listens on: a session that was killed
/// couldn't remove its own.
pub(super) fn clear_left_behind(base: &Path) {
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
