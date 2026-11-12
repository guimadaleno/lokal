//! The one-character pulse shown while Lokal waits on the model or a command:
//! a dot that breathes. All its frames share one width class in Unicode, so
//! it never jitters, whatever the terminal's setting for ambiguous widths.
//! What the person types meanwhile shows on the row under it (`queue`).

use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

use crate::{queue, style, tty};

const FRAMES: [&str; 5] = ["·", "•", "●", "•", "·"];
const FRAME_TIME: Duration = Duration::from_millis(160);

/// Runs `work` on a thread of its own, with the spinner meanwhile, and
/// returns what it gave, or Err if it panicked.
pub fn while_running<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> thread::Result<T> {
    let running = thread::spawn(work);
    let mut spinner = Spinner::new();
    while !running.is_finished() {
        spinner.tick();
        thread::sleep(Duration::from_millis(40));
    }
    spinner.clear();
    running.join()
}

/// Draws at the start of the current line, which must be empty, and the
/// draft on the row under it; the cursor stays on the spinner's row.
pub struct Spinner {
    frame: usize,
    drawn_at: Option<Instant>,
    /// The draft as last drawn (`queue::version`).
    draft: u64,
}

impl Spinner {
    pub fn new() -> Self {
        Spinner {
            frame: 0,
            drawn_at: None,
            draft: 0,
        }
    }

    /// Draws the next frame when it's due; call it often.
    pub fn tick(&mut self) {
        self.tick_with("");
    }

    /// Whether the next frame is due, or the draft changed, so a label
    /// needn't be worked out before.
    pub fn due(&self) -> bool {
        self.frame_due() || queue::version() != self.draft
    }

    fn frame_due(&self) -> bool {
        self.drawn_at.is_none_or(|at| at.elapsed() >= FRAME_TIME)
    }

    /// The same, with a few words after the pulse.
    pub fn tick_with(&mut self, label: &str) {
        if !style::enabled() || !self.due() {
            return;
        }
        let hide_cursor = if self.drawn_at.is_none() {
            "\x1b[?25l"
        } else {
            ""
        };
        // A key redraws at once, without moving the pulse on.
        let next_frame = self.frame_due();
        self.draft = queue::version();
        // The draft's row, then back up; with none, a row left from one goes.
        let below = match queue::row(tty::width().unwrap_or(80)) {
            Some(row) => format!("\x1b[K\n\r{row}\x1b[K\x1b[1A\r"),
            None => "\x1b[J".to_string(),
        };
        eprint!(
            "{hide_cursor}\r{}{label}{below}",
            style::accent(FRAMES[self.frame])
        );
        let _ = std::io::stderr().flush();
        if next_frame {
            self.frame = (self.frame + 1) % FRAMES.len();
            self.drawn_at = Some(Instant::now());
        }
    }

    /// Draws the current frame again at once, after something wrote over it.
    pub fn redraw(&mut self, label: &str) {
        if self.drawn_at.is_some() {
            self.drawn_at = None;
            self.frame = (self.frame + FRAMES.len() - 1) % FRAMES.len();
        }
        self.tick_with(label);
    }

    /// Erases the spinner, and the draft's row under it, so other output
    /// can take its place.
    pub fn clear(&mut self) {
        if self.drawn_at.take().is_some() {
            eprint!("\r\x1b[J\x1b[?25h");
            let _ = std::io::stderr().flush();
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.clear();
    }
}
