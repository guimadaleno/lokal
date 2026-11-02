//! While Lokal works, it reads the keyboard itself, so Ctrl+C or Esc stops
//! the work as a key, the way Ctrl+C does at the prompt. Some terminals never
//! turn Ctrl+C into a signal while a program runs (a Windows console running
//! WSL), and there the key is the only way it arrives. Shift+Tab changes the
//! mode, as at the prompt. Other keys typed meanwhile aren't echoed into the
//! output, where they'd break the spinner's and the reasoning window's
//! redraws; they're dropped.

use std::fs::File;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::thread::{self, JoinHandle};

use crate::tty::{self, RawMode};
use crate::{interrupt, mode};

/// Ctrl+C, as a key.
const CTRL_C: u8 = 3;
const ESCAPE: u8 = 0x1b;
/// What terminals send for Shift+Tab.
const SHIFT_TAB: &[u8] = b"\x1b[Z";

/// The keyboard, read for as long as this lives; the terminal is put back as
/// it was however it ends.
pub struct KeyWatch {
    /// Written to when the watch ends, which wakes the reader at once.
    stop: File,
    reader: Option<JoinHandle<()>>,
    // Dropped in this order, after `drop` has joined the reader: the
    // terminal put back, then its file closed.
    _raw: RawMode,
    _terminal: File,
}

impl KeyWatch {
    /// None without a terminal: then Ctrl+C can only come as a signal.
    pub fn start() -> Option<KeyWatch> {
        let terminal = tty::open()?;
        let (stopped, stop) = pipe()?;
        let raw = RawMode::enter(&terminal)?;
        let fd = terminal.as_raw_fd();
        let reader = thread::spawn(move || read_keys(fd, &stopped));
        Some(KeyWatch {
            stop,
            reader: Some(reader),
            _raw: raw,
            _terminal: terminal,
        })
    }
}

/// A pipe's two ends, read and write, closed on exec so no command gets them.
fn pipe() -> Option<(OwnedFd, File)> {
    let mut ends = [0; 2];
    // SAFETY: an array of two descriptors for the kernel to fill.
    if unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return None;
    }
    // SAFETY: both were just opened, and nothing else owns them.
    unsafe { Some((OwnedFd::from_raw_fd(ends[0]), File::from_raw_fd(ends[1]))) }
}

/// Reads until `stopped` is written to; Ctrl+C raises the flag that
/// whatever is running checks.
fn read_keys(fd: i32, stopped: &OwnedFd) {
    let mut buffer = [0u8; 64];
    let mut tail = Vec::new();
    loop {
        let mut polls = [
            libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stopped.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: two valid pollfds, for the length given. A signal
        // interrupting it only goes round again.
        if unsafe { libc::poll(polls.as_mut_ptr(), 2, -1) } <= 0 {
            continue;
        }
        if polls[1].revents != 0 {
            break;
        }
        // SAFETY: the buffer is valid for its length, and the descriptor
        // outlives this thread: KeyWatch joins it before closing the file.
        let read = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
        if read <= 0 {
            break;
        }
        let keys = &buffer[..read as usize];
        if keys.contains(&CTRL_C) || (keys == [ESCAPE] && !more_follows(fd)) {
            interrupt::raise();
        }
        // From the next step on; the spinner's label shows it.
        for _ in 0..shift_tabs(&mut tail, keys) {
            mode::next();
        }
    }
}

/// How many Shift+Tabs `keys` holds, with the start of one the last read
/// ended on (`tail`): a sequence can arrive in two reads. `tail` keeps the
/// end of these keys for the next.
fn shift_tabs(tail: &mut Vec<u8>, keys: &[u8]) -> usize {
    let mut seen = std::mem::take(tail);
    seen.extend_from_slice(keys);
    let count = seen
        .windows(SHIFT_TAB.len())
        .filter(|keys| *keys == SHIFT_TAB)
        .count();
    // The longest end that could still become one, never a whole one.
    if let Some(length) = (1..SHIFT_TAB.len())
        .rev()
        .find(|length| seen.ends_with(&SHIFT_TAB[..*length]))
    {
        tail.extend_from_slice(&seen[seen.len() - length..]);
    }
    count
}

/// Esc alone stops; an arrow key is Esc followed by the rest of its
/// sequence a moment later, and is only a key someone pressed.
fn more_follows(fd: i32) -> bool {
    tty::readable(fd, 30)
}

impl Drop for KeyWatch {
    fn drop(&mut self) {
        let _ = self.stop.write_all(b"x");
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shift_tab_split_across_reads_still_counts() {
        let mut tail = Vec::new();
        assert_eq!(shift_tabs(&mut tail, b"\x1b[Z\x1b[Z"), 2);
        assert!(tail.is_empty());
        // Esc in one read, the rest in the next.
        assert_eq!(shift_tabs(&mut tail, b"\x1b"), 0);
        assert_eq!(shift_tabs(&mut tail, b"[Z"), 1);
        assert_eq!(shift_tabs(&mut tail, b"x\x1b["), 0);
        assert_eq!(shift_tabs(&mut tail, b"Z"), 1);
        // An arrow key isn't one, and leaves nothing behind.
        assert_eq!(shift_tabs(&mut tail, b"\x1b[A"), 0);
        assert!(tail.is_empty());
    }
}
