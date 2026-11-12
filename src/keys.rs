//! While Lokal works, it reads the keyboard itself, so Ctrl+C or Esc stops
//! the work as a key, the way Ctrl+C does at the prompt. Some terminals never
//! turn Ctrl+C into a signal while a program runs (a Windows console running
//! WSL), and there the key is the only way it arrives. Shift+Tab changes the
//! mode, as at the prompt. What else is typed goes to the next message's
//! draft (`queue`), shown under the spinner rather than echoed into the
//! output, where it would break the spinner's and the reasoning window's
//! redraws; Enter queues it.

use std::fs::File;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::thread::{self, JoinHandle};

use crate::tty::{self, RawMode};
use crate::{interrupt, mode, queue};

/// Ctrl+C, as a key.
const CTRL_C: u8 = 3;
const CTRL_U: u8 = 0x15;
const CTRL_W: u8 = 0x17;
const ESCAPE: u8 = 0x1b;
/// What terminals send for Shift+Tab.
const SHIFT_TAB: &[u8] = b"\x1b[Z";
/// The up arrow, in its usual form (`ESC O A` is the other).
const UP: &[u8] = b"\x1b[A";
/// Around a paste, with bracketed paste on: its line breaks are text.
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
const BRACKETED_PASTE_ON: &[u8] = b"\x1b[?2004h";
const BRACKETED_PASTE_OFF: &[u8] = b"\x1b[?2004l";
/// A sequence longer than this without its end is noise, not a key.
const LONGEST_SEQUENCE: usize = 32;

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
        let mut terminal = tty::open()?;
        let (stopped, stop) = pipe()?;
        let raw = RawMode::enter(&terminal)?;
        let _ = terminal.write_all(BRACKETED_PASTE_ON);
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

/// Reads until `stopped` is written to, and acts on each key.
fn read_keys(fd: i32, stopped: &OwnedFd) {
    let mut buffer = [0u8; 256];
    let mut keys = Keys::default();
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
        for key in keys.take(&buffer[..read as usize], || more_follows(fd)) {
            act(key);
        }
    }
}

/// What a key does.
#[derive(Debug, PartialEq)]
enum Key {
    /// Ctrl+C, or Esc alone: the turn stops, and what's queued waits at
    /// the `>`.
    Stop,
    /// Shift+Tab; from the next step on, and the spinner's label shows it.
    NextMode,
    Text(String),
    Enter,
    /// ↑: what's queued comes back to the draft, to edit.
    EditQueued,
    Backspace,
    ClearDraft,
    DeleteWord,
}

fn act(key: Key) {
    match key {
        Key::Stop => {
            interrupt::raise();
            queue::hold();
        }
        Key::NextMode => mode::next(),
        Key::Text(text) => queue::type_text(&text),
        Key::Enter => queue::enter(),
        Key::EditQueued => queue::edit_queued(),
        Key::Backspace => queue::backspace(),
        Key::ClearDraft => queue::clear_draft(),
        Key::DeleteWord => queue::delete_word(),
    }
}

/// The keys as they arrive, a read at a time: a sequence or a character
/// can come in two reads, so the end of one waits for the next.
#[derive(Default)]
struct Keys {
    tail: Vec<u8>,
    /// Inside a bracketed paste, where a line break is text.
    pasting: bool,
    /// The last byte was a paste's `\r`, so a `\n` after it is the same break.
    after_return: bool,
}

enum Step {
    Took(usize, Option<Key>),
    /// The rest of a sequence or a character is still to come.
    Unfinished,
}

impl Keys {
    /// The keys in `bytes`, with what the last read left unfinished.
    /// `more_follows` says whether an Esc at the end is the start of a
    /// sequence rather than Esc alone.
    fn take(&mut self, bytes: &[u8], more_follows: impl Fn() -> bool) -> Vec<Key> {
        let mut seen = std::mem::take(&mut self.tail);
        seen.extend_from_slice(bytes);
        let mut keys: Vec<Key> = Vec::new();
        let mut at = 0;
        while at < seen.len() {
            match self.step(&seen[at..], &more_follows) {
                Step::Took(length, key) => {
                    at += length;
                    // Text typed in a row goes to the draft as one piece.
                    match (keys.last_mut(), key) {
                        (Some(Key::Text(text)), Some(Key::Text(more))) => text.push_str(&more),
                        (_, Some(key)) => keys.push(key),
                        (_, None) => {}
                    }
                }
                Step::Unfinished => {
                    self.tail = seen[at..].to_vec();
                    break;
                }
            }
        }
        keys
    }

    fn step(&mut self, keys: &[u8], more_follows: &impl Fn() -> bool) -> Step {
        let after_return = std::mem::take(&mut self.after_return);
        let key = match keys[0] {
            CTRL_C => Key::Stop,
            ESCAPE => return self.escape(keys, more_follows),
            b'\n' if self.pasting && after_return => return Step::Took(1, None),
            b'\r' if self.pasting => {
                self.after_return = true;
                Key::Text("\n".into())
            }
            b'\n' if self.pasting => Key::Text("\n".into()),
            b'\r' | b'\n' => Key::Enter,
            b'\t' => Key::Text(" ".into()),
            0x7f | 0x08 => Key::Backspace,
            CTRL_U => Key::ClearDraft,
            CTRL_W => Key::DeleteWord,
            byte if byte < 0x20 => return Step::Took(1, None),
            byte => {
                let length = match byte {
                    0x00..=0x7f => 1,
                    0xc0..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf7 => 4,
                    // A stray continuation byte.
                    _ => return Step::Took(1, None),
                };
                if keys.len() < length {
                    return Step::Unfinished;
                }
                let key = std::str::from_utf8(&keys[..length])
                    .ok()
                    .map(|text| Key::Text(text.into()));
                return Step::Took(length, key);
            }
        };
        Step::Took(1, Some(key))
    }

    /// Esc alone stops; Esc and more is a sequence: Shift+Tab, a paste's
    /// start or end, or a key that does nothing here (arrows, Alt+a key).
    fn escape(&mut self, keys: &[u8], more_follows: &impl Fn() -> bool) -> Step {
        let Some(&next) = keys.get(1) else {
            if more_follows() {
                return Step::Unfinished;
            }
            return Step::Took(1, (!self.pasting).then_some(Key::Stop));
        };
        match next {
            b'[' => {
                let Some(end) = keys[2..]
                    .iter()
                    .position(|byte| (0x40..=0x7e).contains(byte))
                else {
                    if keys.len() < LONGEST_SEQUENCE {
                        return Step::Unfinished;
                    }
                    return Step::Took(keys.len(), None);
                };
                let sequence = &keys[..end + 3];
                let key = match sequence {
                    PASTE_START => {
                        self.pasting = true;
                        None
                    }
                    PASTE_END => {
                        self.pasting = false;
                        None
                    }
                    SHIFT_TAB if !self.pasting => Some(Key::NextMode),
                    UP if !self.pasting => Some(Key::EditQueued),
                    _ => None,
                };
                Step::Took(sequence.len(), key)
            }
            b'O' if keys.len() < 3 => Step::Unfinished,
            b'O' if keys[2] == b'A' && !self.pasting => Step::Took(3, Some(Key::EditQueued)),
            b'O' => Step::Took(3, None),
            _ => Step::Took(2, None),
        }
    }
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
        let _ = self._terminal.write_all(BRACKETED_PASTE_OFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(text: &str) -> Key {
        Key::Text(text.into())
    }

    #[test]
    fn typing_goes_to_the_draft_and_enter_queues_it() {
        let mut keys = Keys::default();
        let taken = keys.take("olá\x7f\x17\x15mundo\r".as_bytes(), || false);
        assert_eq!(
            taken,
            [
                text("olá"),
                Key::Backspace,
                Key::DeleteWord,
                Key::ClearDraft,
                text("mundo"),
                Key::Enter
            ]
        );
    }

    #[test]
    fn a_shift_tab_or_a_character_split_across_reads_still_counts() {
        let mut keys = Keys::default();
        assert_eq!(
            keys.take(b"\x1b[Z\x1b[Z", || false),
            [Key::NextMode, Key::NextMode]
        );
        // Esc in one read, the rest in the next.
        assert_eq!(keys.take(b"\x1b", || true), []);
        assert_eq!(keys.take(b"[Z", || false), [Key::NextMode]);
        assert_eq!(keys.take(b"x\x1b[", || false), [text("x")]);
        assert_eq!(keys.take(b"Z", || false), [Key::NextMode]);
        // Up brings the queue back, in either form; another arrow does
        // nothing, and leaves nothing behind.
        assert_eq!(
            keys.take(b"\x1b[A\x1bOB\x1bOA", || false),
            [Key::EditQueued, Key::EditQueued]
        );
        assert!(keys.tail.is_empty());
        // "é" in two reads.
        assert_eq!(keys.take(&[0xc3], || false), []);
        assert_eq!(keys.take(&[0xa9], || false), [text("é")]);
    }

    #[test]
    fn esc_alone_and_ctrl_c_stop() {
        let mut keys = Keys::default();
        assert_eq!(keys.take(b"\x1b", || false), [Key::Stop]);
        assert_eq!(keys.take(b"ab\x03", || false), [text("ab"), Key::Stop]);
        // Alt+x is Esc and a key together: no stop.
        assert_eq!(keys.take(b"\x1bx", || false), []);
    }

    #[test]
    fn a_paste_keeps_its_line_breaks_as_text() {
        let mut keys = Keys::default();
        let taken = keys.take(
            b"\x1b[200~one\r\ntwo\rthree\n\nfour\x1b[Z\x1b[201~\r",
            || false,
        );
        assert_eq!(taken, [text("one\ntwo\nthree\n\nfour"), Key::Enter]);
        // A paste's end in the next read.
        assert_eq!(keys.take(b"\x1b[200~a\nb\x1b[20", || false), [text("a\nb")]);
        assert_eq!(keys.take(b"1~\n", || false), [Key::Enter]);
    }
}
