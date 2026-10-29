//! A choice among a few options on the terminal: the arrow keys, or a key
//! per option, move to one, and Enter picks it. Approvals use it, so Esc
//! always picks the safe option, wherever the choice started.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::time::Instant;

use crate::style;

/// How often the selected option's marker is repainted while it pulses, in ms.
const PULSE_FRAME: i32 = 60;

pub struct Item<'a> {
    pub label: &'a str,
    /// Moves to the option, as does its number; Enter then picks it.
    pub key: char,
}

pub enum Choice {
    Picked(usize),
    /// Ctrl+C.
    Interrupted,
}

enum Key {
    Up,
    Down,
    Enter,
    Escape,
    Interrupt,
    Char(char),
    Other,
}

/// Shows the question and its options, starting on `start`; Esc picks
/// `safe`. When it's done, the options fold into one line with the answer.
/// None without a terminal.
pub fn choose(question: &str, items: &[Item], start: usize, safe: usize) -> Option<Choice> {
    let mut terminal = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let raw = RawMode::enter(&terminal)?;
    let mut selected = start;
    let shown = Instant::now();
    let _ = write!(terminal, "\x1b[?25l  {question}");
    draw(&mut terminal, items, selected, false, shown);
    let choice = loop {
        if !key_within(&terminal, PULSE_FRAME) {
            pulse_marker(&mut terminal, items.len(), selected, shown);
            continue;
        }
        let picked = match read_key(&mut terminal) {
            Key::Up => {
                selected = selected.saturating_sub(1);
                None
            }
            Key::Down => {
                selected = (selected + 1).min(items.len() - 1);
                None
            }
            Key::Enter => Some(selected),
            Key::Escape => Some(safe),
            Key::Interrupt => break Choice::Interrupted,
            // Moves there; only Enter picks, so a stray key never answers.
            Key::Char(key) => {
                selected = option_by_key(items, key).unwrap_or(selected);
                None
            }
            Key::Other => None,
        };
        match picked {
            Some(index) => break Choice::Picked(index),
            None => draw(&mut terminal, items, selected, true, shown),
        }
    };
    // Back up to the question and replace the options with the answer.
    let answer = match choice {
        Choice::Picked(index) => style::gray(items[index].label),
        Choice::Interrupted => String::new(),
    };
    let _ = write!(
        terminal,
        "\r\x1b[{}A\x1b[J  {question} {answer}\n",
        items.len()
    );
    drop(raw);
    Some(choice)
}

fn option_by_key(items: &[Item], key: char) -> Option<usize> {
    let key = key.to_ascii_lowercase();
    if let Some(digit) = key.to_digit(10) {
        let index = (digit as usize).checked_sub(1)?;
        return (index < items.len()).then_some(index);
    }
    items.iter().position(|item| item.key == key)
}

/// One line per option, the selected one marked. `again` redraws in place.
fn draw(terminal: &mut File, items: &[Item], selected: usize, again: bool, shown: Instant) {
    let mut text = String::new();
    if again {
        text.push_str(&format!("\r\x1b[{}A", items.len() - 1));
        text.push_str("\x1b[J");
    } else {
        text.push('\n');
    }
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        if index == selected {
            text.push_str(&format!(
                "  {} {}",
                style::pulse_accent("›", shown),
                item.label
            ));
        } else {
            text.push_str(&format!("    {}", style::gray(item.label)));
        }
    }
    let _ = terminal.write_all(text.as_bytes());
    let _ = terminal.flush();
}

/// Paints the selected option's marker again, in the pulse's color of now.
/// The cursor goes back to the last option's line, where `draw` left it.
fn pulse_marker(terminal: &mut File, count: usize, selected: usize, shown: Instant) {
    let up = count - 1 - selected;
    let marker = style::pulse_accent("›", shown);
    let text = if up == 0 {
        format!("\r  {marker}")
    } else {
        format!("\x1b[{up}A\r  {marker}\x1b[{up}B")
    };
    let _ = terminal.write_all(text.as_bytes());
    let _ = terminal.flush();
}

fn read_key(terminal: &mut File) -> Key {
    let Some(byte) = read_byte(terminal) else {
        return Key::Escape;
    };
    match byte {
        b'\r' | b'\n' => Key::Enter,
        3 => Key::Interrupt,
        // Ctrl+D
        4 => Key::Escape,
        0x1b => read_escape(terminal),
        byte if byte.is_ascii() => Key::Char(byte as char),
        _ => Key::Other,
    }
}

/// An arrow key is Esc followed by [A or [B (or OA, OB); Esc alone is Esc.
fn read_escape(terminal: &mut File) -> Key {
    if !key_within(terminal, 30) {
        return Key::Escape;
    }
    let mut last = 0;
    while key_within(terminal, 30) {
        match read_byte(terminal) {
            Some(byte) => last = byte,
            None => break,
        }
        if last.is_ascii_alphabetic() || last == b'~' {
            break;
        }
    }
    match last {
        b'A' => Key::Up,
        b'B' => Key::Down,
        _ => Key::Other,
    }
}

fn read_byte(terminal: &mut File) -> Option<u8> {
    let mut byte = [0u8; 1];
    match terminal.read(&mut byte) {
        Ok(1) => Some(byte[0]),
        _ => None,
    }
}

/// Whether a byte arrives within `milliseconds`: a key, or the rest of one's
/// sequence.
fn key_within(terminal: &File, milliseconds: i32) -> bool {
    let mut poll = libc::pollfd {
        fd: terminal.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd, for the length given.
    unsafe { libc::poll(&mut poll, 1, milliseconds) > 0 }
}

/// Keys arrive one by one, unechoed, and Ctrl+C arrives as a key. The
/// terminal is put back as it was however the choice ends.
struct RawMode {
    fd: i32,
    saved: libc::termios,
}

impl RawMode {
    fn enter(terminal: &File) -> Option<RawMode> {
        let fd = terminal.as_raw_fd();
        // SAFETY: termios is plain data, filled in by tcgetattr before use.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
            return None;
        }
        let mut raw = saved;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        // SAFETY: a valid descriptor and a termios read from it.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return None;
        }
        Some(RawMode { fd, saved })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let show_cursor = b"\x1b[?25h";
        // SAFETY: the descriptor outlives this guard; the buffer is valid.
        unsafe {
            libc::write(self.fd, show_cursor.as_ptr().cast(), show_cursor.len());
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.saved);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_numbers_find_options() {
        let items = [
            Item {
                label: "Yes",
                key: 'y',
            },
            Item {
                label: "Yes, and don't ask again",
                key: 'a',
            },
            Item {
                label: "No",
                key: 'n',
            },
        ];
        assert_eq!(option_by_key(&items, 'y'), Some(0));
        assert_eq!(option_by_key(&items, 'N'), Some(2));
        assert_eq!(option_by_key(&items, '2'), Some(1));
        assert_eq!(option_by_key(&items, '4'), None);
        assert_eq!(option_by_key(&items, '0'), None);
        assert_eq!(option_by_key(&items, 'x'), None);
    }
}
