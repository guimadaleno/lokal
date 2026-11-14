//! A choice among a few options on the terminal: the arrow keys, or a key
//! per option, move to one, and Enter picks it. Approvals use it, so Esc
//! always picks the safe option, wherever the choice started.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use crate::tty::{self, RawMode};
use crate::{layout, screen, style};

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
/// `safe`. For `deaf` after it shows, only Esc and Ctrl+C count: a key
/// meant for a message being typed isn't its answer. When it's done, the
/// options fold into one line with the answer. None without a terminal.
pub fn choose(
    question: &str,
    items: &[Item],
    start: usize,
    safe: usize,
    deaf: Duration,
) -> Option<Choice> {
    let mut terminal = tty::open()?;
    let raw = RawMode::enter(&terminal)?;
    let full_question = question;
    let (mut width, mut question, mut labels) = fitted(full_question, items);
    let mut selected = start;
    let shown = Instant::now();
    let _ = write!(terminal, "\x1b[?25l  {question}");
    draw(&mut terminal, &labels, selected, false, shown);
    let choice = loop {
        // The width changed while it's open (#49): the screen comes back
        // laid out at it, and the question under it, cut to the new width,
        // on the option it was on. A resize is no key.
        if screen::due() && screen::redraw() {
            (width, question, labels) = fitted(full_question, items);
            let _ = write!(terminal, "\x1b[?25l  {question}");
            draw(&mut terminal, &labels, selected, false, shown);
            continue;
        }
        if !key_within(&terminal, PULSE_FRAME) {
            pulse_marker(&mut terminal, items.len(), selected, shown);
            continue;
        }
        let key = read_key(&mut terminal);
        if shown.elapsed() < deaf && !matches!(key, Key::Escape | Key::Interrupt) {
            continue;
        }
        let picked = match key {
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
            None => draw(&mut terminal, &labels, selected, true, shown),
        }
    };
    // Back up to the question, replace the options with the answer, and
    // show the cursor again.
    let answer = match choice {
        Choice::Picked(index) => {
            let room = width.saturating_sub(question.chars().count() + 4);
            style::gray(&layout::cut(items[index].label, room))
        }
        Choice::Interrupted => String::new(),
    };
    let folded = format!("  {question} {answer}\n");
    let _ = write!(terminal, "\r\x1b[{}A\x1b[J{folded}\x1b[?25h", items.len());
    // What stays of the question, for a redraw (#49).
    screen::kept_text(&folded);
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

/// The question and the options' labels at the terminal's width now, each
/// on one line, cut with … in a narrow terminal: one that wrapped would
/// throw off the redraws, which go up a line per option.
fn fitted(question: &str, items: &[Item]) -> (usize, String, Vec<String>) {
    let width = tty::width().unwrap_or(80);
    let question = layout::cut(question, width.saturating_sub(3));
    let labels = items
        .iter()
        .map(|item| layout::cut(item.label, width.saturating_sub(5)))
        .collect();
    (width, question, labels)
}

/// One line per option, the selected one marked. `again` redraws in place.
fn draw(terminal: &mut File, labels: &[String], selected: usize, again: bool, shown: Instant) {
    let mut text = String::new();
    if again {
        text.push_str(&format!("\r\x1b[{}A", labels.len() - 1));
        text.push_str("\x1b[J");
    } else {
        text.push('\n');
    }
    for (index, label) in labels.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        if index == selected {
            text.push_str(&format!("  {} {label}", style::pulse_accent("›", shown)));
        } else {
            text.push_str(&format!("    {}", style::gray(label)));
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

fn key_within(terminal: &File, milliseconds: i32) -> bool {
    tty::readable(terminal.as_raw_fd(), milliseconds)
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
