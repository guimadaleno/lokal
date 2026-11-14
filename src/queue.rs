//! What the person types while Lokal works (#3): a draft, shown under the
//! spinner, and the lines Enter queued. When the turn ends, the queued
//! lines go as the next request, by the same path as a line typed at the
//! `>`. After a stop, a No or a failure, they wait at the `>` with the
//! draft instead, to edit or send: the person stopped to steer. Kept in a
//! conversation at a terminal only; elsewhere keys are dropped, as before.
//!
//! The draft is Lokal's memory, never the terminal's buffer, so nothing
//! typed here can answer an approval (`tty::discard_typed_ahead`).

use std::io::Write;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use crate::{layout, screen, style, text, tty};

/// A paste of a whole file stops here: the draft is a message.
const MOST: usize = 64 * 1024;

struct Queue {
    open: bool,
    draft: String,
    /// Each line Enter queued, and when: its time on screen is then (#46).
    queued: Vec<(String, SystemTime)>,
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

    /// Whether the row has anything to show.
    fn shows(&self) -> bool {
        !self.draft.is_empty() || !self.queued.is_empty()
    }

    fn enter(&mut self) {
        if !self.draft.trim().is_empty() {
            let line = std::mem::take(&mut self.draft);
            self.queued
                .push((line.trim().to_string(), SystemTime::now()));
            self.changed();
        }
    }

    /// The queued lines the model can take mid-turn: those before the first
    /// slash, `!` or `@` line, which waits to work as typed at the `>`.
    fn take_for_model(&mut self) -> Option<(String, SystemTime)> {
        if self.held {
            return None;
        }
        let count = self
            .queued
            .iter()
            .take_while(|(line, _)| !line.starts_with(['/', '!', '@']))
            .count();
        if count == 0 {
            return None;
        }
        self.version += 1;
        joined(self.queued.drain(..count).collect())
    }

    /// ↑ on an empty draft: what's queued comes back to it, to edit.
    fn edit_queued(&mut self) {
        if self.draft.is_empty() && !self.queued.is_empty() {
            self.draft = lines(std::mem::take(&mut self.queued)).join("\n");
            self.changed();
        }
    }

    fn take_ready(&mut self) -> Option<(String, SystemTime)> {
        if self.held || self.queued.is_empty() {
            return None;
        }
        self.version += 1;
        joined(std::mem::take(&mut self.queued))
    }

    fn take_typed(&mut self) -> String {
        let mut lines = if std::mem::take(&mut self.held) {
            lines(std::mem::take(&mut self.queued))
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
        let (last, _) = self.queued.last()?;
        let label = format!("{} · ", queued.unwrap_or_default());
        let hint = format!(" · {}", text::EDIT_QUEUED);
        let room = room.saturating_sub(label.chars().count() + hint.chars().count());
        Some(style::faint(&format!(
            "{label}{}{hint}",
            tail(&shown(last), room)
        )))
    }
}

/// Queued lines as one message, sent once its last line was queued: its
/// time shows beside that line.
fn joined(queued: Vec<(String, SystemTime)>) -> Option<(String, SystemTime)> {
    let at = queued.last()?.1;
    Some((lines(queued).join("\n"), at))
}

fn lines(queued: Vec<(String, SystemTime)>) -> Vec<String> {
    queued.into_iter().map(|(line, _)| line).collect()
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

/// ↑: what's queued comes back to the draft, when it's empty.
pub fn edit_queued() {
    queue().edit_queued();
}

/// What the model takes after its step, mid-turn: the queued lines before
/// any slash, `!` or `@` one, and when the first was queued.
pub fn take_for_model() -> Option<(String, SystemTime)> {
    queue().take_for_model()
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

/// The queued lines, as one request, when the last turn ended on its own,
/// and when the first was queued. The draft stays for the next turn.
pub fn take_ready() -> Option<(String, SystemTime)> {
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

/// The answer's text while the row shows: printed a whole line at a time,
/// with the row drawn on the row after, where the next line goes. The
/// cursor is then always at the start of a line, so the row is redrawn
/// without knowing where in a line the answer stands. Without a draft,
/// the answer streams word by word as before.
#[derive(Default)]
pub struct UnderAnswer {
    /// The end of a line, printed once the line ends.
    held: String,
    /// The row is drawn where the cursor is, as of this `version`.
    drawn: Option<u64>,
}

impl UnderAnswer {
    /// Whether text goes through here now: the row shows something, or a
    /// line is held.
    pub fn active(&self) -> bool {
        !self.held.is_empty() || self.drawn.is_some() || queue().shows()
    }

    /// More of the answer, styled: its whole lines printed, the rest held.
    pub fn push(&mut self, text: &str) {
        self.held.push_str(text);
        if let Some(lines) = whole_lines(&mut self.held) {
            self.erase();
            screen::answer(&lines);
            self.draw();
        }
    }

    /// After a redraw of the screen (#49), which took the row with it.
    pub fn after_redraw(&mut self) {
        if self.drawn.take().is_some() {
            self.draw();
        }
    }

    /// The row again, when the draft changed since it was drawn.
    pub fn redraw(&mut self) {
        if self.drawn.is_some_and(|drawn| drawn != version()) {
            self.draw();
        }
    }

    /// Before anything else is printed: the row goes, and what's held is
    /// printed, the cursor after it.
    pub fn flush(&mut self) {
        self.erase();
        screen::answer(&std::mem::take(&mut self.held));
    }

    fn draw(&mut self) {
        let row = row(tty::width().unwrap_or(80)).unwrap_or_default();
        eprint!("\r{row}\x1b[K\r");
        let _ = std::io::stderr().flush();
        self.drawn = Some(version());
    }

    fn erase(&mut self) {
        if self.drawn.take().is_some() {
            eprint!("\r\x1b[K");
            let _ = std::io::stderr().flush();
        }
    }
}

/// What of `held` ends with a line break, taken out of it; the rest stays.
fn whole_lines(held: &mut String) -> Option<String> {
    let end = held.rfind('\n')?;
    let rest = held.split_off(end + 1);
    Some(std::mem::replace(held, rest))
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

    /// What was taken, without when.
    fn text(taken: Option<(String, SystemTime)>) -> Option<String> {
        taken.map(|(line, _)| line)
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
        assert_eq!(lines(queue.queued.clone()), ["fix the docs"]);
        assert_eq!(text(queue.take_ready()).as_deref(), Some("fix the docs"));
        // The draft waits for the next turn.
        assert_eq!(queue.take_ready(), None);
        assert_eq!(queue.take_typed(), "and the ");
        assert_eq!(queue.take_typed(), "");
    }

    #[test]
    fn the_model_takes_plain_lines_mid_turn_and_commands_wait() {
        let mut queue = open();
        for line in ["and the docs", "also the readme", "/compact", "then this"] {
            queue.type_text(line);
            queue.enter();
        }
        assert_eq!(
            text(queue.take_for_model()).as_deref(),
            Some("and the docs\nalso the readme")
        );
        // A slash command first: nothing for the model until it runs.
        assert_eq!(queue.take_for_model(), None);
        assert_eq!(
            text(queue.take_ready()).as_deref(),
            Some("/compact\nthen this")
        );
        queue.type_text("held");
        queue.enter();
        queue.held = true;
        assert_eq!(queue.take_for_model(), None);
    }

    #[test]
    fn what_is_taken_was_sent_when_its_last_line_was_queued() {
        let mut queue = open();
        queue.type_text("first");
        queue.enter();
        let first = queue.queued[0].1;
        queue.type_text("second");
        queue.enter();
        let second = queue.queued[1].1;
        assert!(second >= first);
        assert_eq!(
            queue.take_ready(),
            Some(("first\nsecond".to_string(), second))
        );
    }

    #[test]
    fn up_brings_what_is_queued_back_to_an_empty_draft() {
        let mut queue = open();
        queue.type_text("one");
        queue.enter();
        queue.type_text("two");
        queue.enter();
        queue.type_text("x");
        queue.edit_queued();
        assert_eq!(queue.draft, "x");
        queue.clear_draft();
        queue.edit_queued();
        assert_eq!((queue.draft.as_str(), queue.queued.len()), ("one\ntwo", 0));
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
    fn under_an_answer_only_whole_lines_go_out() {
        let mut held = "a partial".to_string();
        assert_eq!(whole_lines(&mut held), None);
        held.push_str(" line\nnext\nand the start");
        assert_eq!(
            whole_lines(&mut held).as_deref(),
            Some("a partial line\nnext\n")
        );
        assert_eq!(held, "and the start");
    }

    #[test]
    fn the_row_shows_the_end_of_what_is_typed() {
        let mut queue = open();
        assert_eq!(queue.row(40), None);
        queue.type_text("first line\nsecond");
        let row = queue.row(40).unwrap_or_default();
        assert!(row.contains("first line ⏎ second"), "{row}");
        queue.enter();
        let row = queue.row(60).unwrap_or_default();
        assert!(
            row.contains("queued · first line ⏎ second · ↑ to edit"),
            "{row}"
        );
        queue.type_text(&"x".repeat(100));
        let row = queue.row(40).unwrap_or_default();
        assert!(row.contains("…xxx") && row.contains("· queued"), "{row}");
        assert!(!row.contains("first"));
    }
}
