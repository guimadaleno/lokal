//! What the person types while Lokal works (#3): a draft, shown under the
//! spinner, and the lines Enter queued. When the turn ends, the queued
//! lines go as the next request, by the same path as a line typed at the
//! `>`. After a stop, a No or a failure, they wait at the `>` with the
//! draft instead, to edit or send: the person stopped to steer. Kept in a
//! conversation at a terminal only; elsewhere keys are dropped, as before.
//!
//! The draft is Lokal's memory, never the terminal's buffer, so nothing
//! typed here can answer an approval (`tty::discard_typed_ahead`).

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::{layout, style, text};

/// A paste of a whole file stops here: the draft is a message.
const MOST: usize = 64 * 1024;

struct Queue {
    open: bool,
    draft: String,
    queued: Vec<String>,
    /// The last turn was stopped, said no to or failed: what's queued
    /// waits at the `>`.
    held: bool,
    /// Bumped on every change, so the spinner redraws the row at once.
    version: u64,
    typed_at: Option<Instant>,
}

impl Queue {
    const fn new() -> Self {
        Queue {
            open: false,
            draft: String::new(),
            queued: Vec::new(),
            held: false,
            version: 0,
            typed_at: None,
        }
    }

    fn changed(&mut self) {
        self.version += 1;
        self.typed_at = Some(Instant::now());
    }

    fn type_text(&mut self, text: &str) {
        let room = MOST.saturating_sub(self.draft.len());
        let text = &text[..text.floor_char_boundary(room.min(text.len()))];
        if self.open && !text.is_empty() {
            self.draft.push_str(text);
            self.changed();
        }
    }

    fn backspace(&mut self) {
        if self.draft.pop().is_some() {
            self.changed();
        }
    }

    fn clear_draft(&mut self) {
        if !self.draft.is_empty() {
            self.draft.clear();
            self.changed();
        }
    }

    /// Ctrl+W: the word before the cursor, and the spaces after it.
    fn delete_word(&mut self) {
        let kept = self.draft.trim_end().len();
        let start = self.draft[..kept]
            .rfind(char::is_whitespace)
            .map_or(0, |at| at + 1);
        if start < self.draft.len() {
            self.draft.truncate(start);
            self.changed();
        }
    }

    fn enter(&mut self) {
        if !self.draft.trim().is_empty() {
            let line = std::mem::take(&mut self.draft);
            self.queued.push(line.trim().to_string());
            self.changed();
        }
    }

    fn take_ready(&mut self) -> Option<String> {
        if self.held || self.queued.is_empty() {
            return None;
        }
        self.version += 1;
        Some(std::mem::take(&mut self.queued).join("\n"))
    }

    fn take_typed(&mut self) -> String {
        let mut lines = if std::mem::take(&mut self.held) {
            std::mem::take(&mut self.queued)
        } else {
            Vec::new()
        };
        if !self.draft.trim().is_empty() {
            lines.push(std::mem::take(&mut self.draft));
        }
        self.draft.clear();
        self.version += 1;
        lines.join("\n")
    }

    /// One row of `width` columns, or None when nothing was typed: the
    /// screen stays as it was for whoever doesn't type.
    fn row(&self, width: usize) -> Option<String> {
        // Short of the edge, so the terminal never wraps it.
        let room = width.saturating_sub(4).max(10);
        let queued = (!self.queued.is_empty()).then(|| text::queued(self.queued.len()));
        if !self.draft.is_empty() {
            let after = queued
                .map(|queued| format!(" · {queued}"))
                .unwrap_or_default();
            let room = room.saturating_sub(after.chars().count() + 3);
            return Some(format!(
                "{}{}{}{}",
                style::accent("> "),
                style::gray(&tail(&shown(&self.draft), room)),
                cursor(),
                style::faint(&after)
            ));
        }
        let last = self.queued.last()?;
        let label = format!("{} · ", queued.unwrap_or_default());
        let room = room.saturating_sub(label.chars().count());
        Some(style::faint(&format!(
            "{label}{}",
            tail(&shown(last), room)
        )))
    }
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue::new());

fn queue() -> MutexGuard<'static, Queue> {
    QUEUE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// From now on, what's typed while Lokal works is kept.
pub fn open() {
    queue().open = true;
}

pub fn type_text(text: &str) {
    queue().type_text(text);
}

pub fn backspace() {
    queue().backspace();
}

pub fn clear_draft() {
    queue().clear_draft();
}

pub fn delete_word() {
    queue().delete_word();
}

/// Enter: the draft joins the queue.
pub fn enter() {
    queue().enter();
}

/// The turn was stopped, said no to or failed: what's queued goes back to
/// the `>` rather than out on its own.
pub fn hold() {
    queue().held = true;
}

/// What changed last, for the spinner to redraw the row.
pub fn version() -> u64 {
    queue().version
}

/// Whether the person is typing a message: a draft, or a key within
/// `time`. An approval that appears meanwhile takes a moment before it
/// listens, since an Enter meant to queue the draft would answer it.
pub fn typing(time: Duration) -> bool {
    let queue = queue();
    !queue.draft.is_empty() || queue.typed_at.is_some_and(|at| at.elapsed() < time)
}

/// The queued lines, as one request, when the last turn ended on its own.
/// The draft stays for the next turn.
pub fn take_ready() -> Option<String> {
    queue().take_ready()
}

/// What goes to the `>` as typed text: the draft, after what was queued
/// when the last turn was stopped.
pub fn take_typed() -> String {
    queue().take_typed()
}

/// The row under the spinner, or None.
pub fn row(width: usize) -> Option<String> {
    queue().row(width)
}

/// The draft on one row: a line break as ⏎, a tab as a space, control
/// characters as marks.
fn shown(text: &str) -> String {
    let text = text.replace('\n', " ⏎ ").replace('\t', " ");
    layout::printable(&text).into_owned()
}

/// The end of `text` in `room` characters, marked with … when cut: the
/// person is typing at its end.
fn tail(text: &str, room: usize) -> String {
    let length = text.chars().count();
    if length <= room {
        return text.to_string();
    }
    let kept: String = text.chars().skip(length + 1 - room.max(1)).collect();
    format!("…{kept}")
}

/// Where the next key goes, as a faint block.
fn cursor() -> String {
    if style::enabled() {
        "\x1b[48;5;242m \x1b[0m".to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> Queue {
        let mut queue = Queue::new();
        queue.open = true;
        queue
    }

    #[test]
    fn keys_make_a_draft_and_enter_queues_it() {
        let mut queue = open();
        queue.type_text("fix the tests");
        queue.backspace();
        queue.type_text("s too ");
        queue.delete_word();
        assert_eq!(queue.draft, "fix the tests ");
        queue.delete_word();
        assert_eq!(queue.draft, "fix the ");
        queue.type_text("docs");
        queue.enter();
        // An empty line queues nothing.
        queue.enter();
        queue.type_text("and the ");
        assert_eq!(queue.queued, ["fix the docs"]);
        assert_eq!(queue.take_ready().as_deref(), Some("fix the docs"));
        // The draft waits for the next turn.
        assert_eq!(queue.take_ready(), None);
        assert_eq!(queue.take_typed(), "and the ");
        assert_eq!(queue.take_typed(), "");
    }

    #[test]
    fn after_a_stop_everything_goes_back_to_the_prompt() {
        let mut queue = open();
        queue.type_text("one");
        queue.enter();
        queue.type_text("two");
        queue.enter();
        queue.type_text("three");
        queue.held = true;
        assert_eq!(queue.take_ready(), None);
        assert_eq!(queue.take_typed(), "one\ntwo\nthree");
        assert!(!queue.held && queue.queued.is_empty());
    }

    #[test]
    fn nothing_is_kept_until_opened_nor_past_the_limit() {
        let mut queue = Queue::new();
        queue.type_text("lost");
        assert_eq!(queue.draft, "");
        queue.open = true;
        queue.type_text(&"é".repeat(MOST));
        assert_eq!(queue.draft.len(), MOST);
        queue.type_text("x");
        assert_eq!(queue.draft.len(), MOST);
    }

    #[test]
    fn the_row_shows_the_end_of_what_is_typed() {
        let mut queue = open();
        assert_eq!(queue.row(40), None);
        queue.type_text("first line\nsecond");
        let row = queue.row(40).unwrap_or_default();
        assert!(row.contains("first line ⏎ second"), "{row}");
        queue.enter();
        let row = queue.row(40).unwrap_or_default();
        assert!(row.contains("queued · first line ⏎ second"), "{row}");
        queue.type_text(&"x".repeat(100));
        let row = queue.row(40).unwrap_or_default();
        assert!(row.contains("…xxx") && row.contains("· queued"), "{row}");
        assert!(!row.contains("first"));
    }
}
