//! The model's reasoning, while it thinks: a small faint window above the
//! spinner that keeps only its last few rows and redraws them in place. It
//! disappears when the answer or a step begins, so the conversation keeps
//! none of it.

use std::io::Write;
use std::time::{Duration, Instant};

use crate::{layout, screen, style, tty};

/// How many rows of reasoning show at once.
const ROWS: usize = 4;
const REDRAW_EVERY: Duration = Duration::from_millis(100);
/// Only the end of the reasoning is ever drawn; this is plenty for it.
const KEPT: usize = 4096;

#[derive(Default)]
pub struct Window {
    text: String,
    drawn_rows: usize,
    drawn_at: Option<Instant>,
    /// The screen's redraws when it was drawn (`screen::redraws`).
    drawn_in: u64,
}

impl Window {
    pub fn push(&mut self, piece: &str) {
        self.text.push_str(piece);
        if self.text.len() > KEPT * 2 {
            let cut = self.text.floor_char_boundary(self.text.len() - KEPT);
            self.text.drain(..cut);
        }
    }

    /// Redraws the window when it's due. A blank row follows it, and the
    /// cursor ends on the row after that, where the spinner goes.
    pub fn draw(&mut self) {
        if !style::enabled() || self.drawn_at.is_some_and(|at| at.elapsed() < REDRAW_EVERY) {
            return;
        }
        self.forget_if_redrawn();
        // Rows narrower than the terminal, so none of them wraps.
        let width = tty::width().map_or(76, |width| width.saturating_sub(6).max(20));
        let rows = last_rows(&self.text, width, ROWS);
        if rows.is_empty() {
            return;
        }
        let mut out = self.back_to_top();
        for row in &rows {
            out.push_str(&format!("  {}\n", style::faint(row)));
        }
        out.push('\n');
        eprint!("{out}");
        let _ = std::io::stderr().flush();
        self.drawn_rows = rows.len() + 1;
        self.drawn_at = Some(Instant::now());
        self.drawn_in = screen::redraws();
    }

    /// Whether a redraw of the screen (#49) took the window with it: it
    /// draws again now, above the spinner.
    pub fn redrawn(&self) -> bool {
        self.drawn_rows > 0 && self.drawn_in != screen::redraws()
    }

    /// Draws the window at once, its rows gone with a redraw.
    pub fn draw_now(&mut self) {
        self.drawn_at = None;
        self.draw();
    }

    /// Rows a redraw took aren't there to go back over.
    fn forget_if_redrawn(&mut self) {
        if self.drawn_in != screen::redraws() {
            self.drawn_rows = 0;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.drawn_rows == 0 && self.text.is_empty()
    }

    /// Erases the window; what follows starts where it was.
    pub fn clear(&mut self) {
        self.forget_if_redrawn();
        if self.drawn_rows > 0 {
            eprint!("{}", self.back_to_top());
            let _ = std::io::stderr().flush();
            self.drawn_rows = 0;
        }
        self.text.clear();
        self.drawn_at = None;
    }

    /// Up to the window's first row, with it and everything below cleared.
    fn back_to_top(&self) -> String {
        match self.drawn_rows {
            0 => "\r\x1b[J".to_string(),
            rows => format!("\r\x1b[{rows}A\x1b[J"),
        }
    }
}

/// The last `count` rows the text takes when wrapped at `width` characters,
/// blank lines left out.
fn last_rows(text: &str, width: usize, count: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
    {
        // A model's text can carry escapes that would redraw the screen.
        let chars: Vec<char> = layout::printable(line).replace('\t', " ").chars().collect();
        for chunk in chars.chunks(width) {
            rows.push(chunk.iter().collect::<String>());
        }
    }
    let skip = rows.len().saturating_sub(count);
    rows.split_off(skip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_rows_wrapped_to_the_width() {
        let text = "first thought\n\nsecond, which is long enough to wrap\nthird";
        assert_eq!(
            last_rows(text, 12, 3),
            ["h is long en", "ough to wrap", "third"]
        );
        assert_eq!(last_rows("one\ntwo", 12, 4), ["one", "two"]);
        assert!(last_rows("\n \n", 12, 4).is_empty());
        assert_eq!(last_rows("up\x1b[1A", 12, 4), ["up^[[1A"]);
    }
}
