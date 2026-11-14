//! What a conversation printed, kept so the screen can be printed again at
//! another width when the terminal's changes (#49). Text the terminal wraps
//! by itself is kept as the bytes that went out, which print the same at
//! any width; what Lokal fitted to the width (a rule, a time at the right
//! edge) is kept as how to lay it out, and laid out again.
//!
//! Only lasting lines are kept. What's redrawn or erased (the spinner, the
//! draft row, the reasoning window, an open question, the `>` while it's
//! typed) is drawn again by what owns it.

use std::io::{IsTerminal, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::{style, text, tty};

/// Text on stderr, as `eprint!` prints it, kept for a redraw.
macro_rules! show {
    ($($arg:tt)*) => {
        $crate::screen::print(&format!($($arg)*))
    };
}

/// A line on stderr, as `eprintln!` prints it, kept for a redraw.
macro_rules! showln {
    () => {
        $crate::screen::print("\n")
    };
    ($($arg:tt)*) => {
        $crate::screen::print(&format!("{}\n", format_args!($($arg)*)))
    };
}

/// How a fitted piece prints at a width, line breaks included.
pub type Layout = Box<dyn Fn(usize) -> String + Send>;

/// How often the watcher asks the terminal its width: a change shows within
/// this, and a drag's changes are joined until two asks agree.
const POLL: Duration = Duration::from_millis(100);

/// How long a redraw asked of the `>` waits before it's asked again: the
/// signal may have come before rustyline was listening.
const ASK_AGAIN: Duration = Duration::from_millis(400);

/// The most a redraw prints again, about a terminal's scrollback of text;
/// the oldest pieces go past it, so a long session redraws as fast as a
/// short one and keeps little memory.
const MOST_KEPT: usize = 512 * 1024;

/// What a fitted piece is counted as toward `MOST_KEPT`.
const FITTED_SIZE: usize = 256;

enum Piece {
    Text(String),
    Fitted(Layout),
}

struct Screen {
    keeping: bool,
    pieces: Vec<Piece>,
    /// What the pieces count toward `MOST_KEPT`.
    size: usize,
    /// Pieces went past `MOST_KEPT`: a redraw says the earlier lines aren't there.
    dropped: bool,
    /// The width the screen was last laid out at.
    width: Option<usize>,
}

impl Screen {
    const fn new() -> Screen {
        Screen {
            keeping: false,
            pieces: Vec::new(),
            size: 0,
            dropped: false,
            width: None,
        }
    }

    fn keep(&mut self, piece: Piece) {
        if !self.keeping {
            return;
        }
        self.size += size_of(&piece);
        // Text that follows text joins it: an answer comes a word at a time.
        match (self.pieces.last_mut(), piece) {
            (Some(Piece::Text(last)), Piece::Text(text)) => last.push_str(&text),
            (_, piece) => self.pieces.push(piece),
        }
        while self.size > MOST_KEPT && self.pieces.len() > 1 {
            let oldest = self.pieces.remove(0);
            self.size -= size_of(&oldest);
            self.dropped = true;
        }
    }
}

fn size_of(piece: &Piece) -> usize {
    match piece {
        Piece::Text(text) => text.len(),
        Piece::Fitted(_) => FITTED_SIZE,
    }
}

static SCREEN: Mutex<Screen> = Mutex::new(Screen::new());

/// The `>` is being read, so a redraw ends the reading, as mail does.
static AT_PROMPT: Mutex<bool> = Mutex::new(false);

/// A signal went to end the `>` for a redraw, and hasn't been seen yet.
static SIGNALLED: AtomicBool = AtomicBool::new(false);

fn screen() -> MutexGuard<'static, Screen> {
    SCREEN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// From now on, what's printed is kept and the width watched: a
/// conversation whose answers and messages both reach the terminal.
pub fn keep() {
    if !(std::io::stdout().is_terminal() && std::io::stderr().is_terminal()) {
        return;
    }
    {
        let mut screen = screen();
        screen.keeping = true;
        screen.width = tty::width();
    }
    watch();
}

/// Text on stderr, kept. What `show!` and `showln!` print with.
pub fn print(text: &str) {
    let mut screen = screen();
    let _ = std::io::stderr().write_all(text.as_bytes());
    screen.keep(Piece::Text(text.to_string()));
}

/// The answer's text on stdout, at once, kept.
pub fn answer(text: &str) {
    let mut screen = screen();
    let mut out = std::io::stdout();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
    screen.keep(Piece::Text(text.to_string()));
}

/// A piece laid out at the terminal's width now, printed on stderr and kept.
pub fn fitted(layout: Layout) {
    let mut screen = screen();
    let text = layout(tty::width().unwrap_or(80));
    let _ = std::io::stderr().write_all(text.as_bytes());
    screen.keep(Piece::Fitted(layout));
}

/// A piece already on screen another way, kept as it would print: the `>`
/// line rustyline drew, a mail rustyline prints.
pub fn kept(layout: Layout) {
    screen().keep(Piece::Fitted(layout));
}

/// Text already on screen another way, kept: a question folded into its
/// answer, a reply typed at a question.
pub fn kept_text(text: &str) {
    screen().keep(Piece::Text(text.to_string()));
}

/// The screen was cleared (`/new`, Ctrl+L): a redraw clears it there too,
/// pushing what came before into the scrollback and going on from the top,
/// as the clear left it.
pub fn cleared() {
    kept(Box::new(|_| {
        format!("{}\x1b[H", "\n".repeat(tty::height().unwrap_or(24)))
    }));
}

/// Whether the terminal's width changed since the screen was laid out.
pub fn resized() -> bool {
    let screen = screen();
    screen.keeping && tty::width().is_some_and(|width| screen.width != Some(width))
}

/// The screen again at the terminal's width now: cleared with its
/// scrollback, every kept piece laid out anew, written at once, in one
/// synchronized frame where the terminal has them. What's live is drawn by
/// its owner after.
pub fn redraw() {
    let mut screen = screen();
    if !screen.keeping {
        return;
    }
    let width = tty::width().unwrap_or(80);
    let earlier = screen
        .dropped
        .then(|| format!("{}\n", style::faint(text::EARLIER_NOT_KEPT)));
    let out = laid_out(earlier.as_deref(), &screen.pieces, width);
    let _ = std::io::stderr().write_all(out.as_bytes());
    screen.width = Some(width);
}

/// Every piece at `width`, after what clears the screen and its scrollback
/// and the line saying `earlier` lines aren't there, when they aren't.
fn laid_out(earlier: Option<&str>, pieces: &[Piece], width: usize) -> String {
    let mut out = String::from("\x1b[?2026h\x1b[H\x1b[2J\x1b[3J");
    out.push_str(earlier.unwrap_or_default());
    for piece in pieces {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Fitted(layout) => out.push_str(&layout(width)),
        }
    }
    out.push_str("\x1b[?2026l");
    out
}

/// The `>` is about to be read: a resize may end the reading from now on.
pub fn enter_prompt() {
    *AT_PROMPT.lock().unwrap_or_else(PoisonError::into_inner) = true;
}

/// The `>` was left. True when a signal for a redraw went meanwhile: if the
/// reading ended for it, the redraw is due, and if it didn't, the signal
/// only raised Lokal's own flag, which is no Ctrl+C of the person's.
pub fn leave_prompt() -> bool {
    *AT_PROMPT.lock().unwrap_or_else(PoisonError::into_inner) = false;
    SIGNALLED.swap(false, Ordering::SeqCst)
}

/// Asks the terminal its width every `POLL`, on a thread of its own. Once
/// it settles on one the screen wasn't laid out at, a `>` being read is
/// ended with a SIGINT, as mail ends it, for the redraw: rustyline has the
/// keyboard there. Elsewhere the redraw waits for the next `>`.
fn watch() {
    thread::spawn(|| {
        let Some(terminal) = tty::open() else {
            return;
        };
        let mut seen = width_of(&terminal);
        let mut asked: Option<Instant> = None;
        loop {
            thread::sleep(POLL);
            let now = width_of(&terminal);
            if now != seen {
                seen = now;
                continue;
            }
            if now.is_none() || now == screen().width {
                asked = None;
                continue;
            }
            // Held while the signal goes, so the `>` can't be marked left
            // meanwhile: a signal that comes as the line ends finds
            // `SIGNALLED` set when it's left, and only cleared Lokal's flag.
            let at_prompt = AT_PROMPT.lock().unwrap_or_else(PoisonError::into_inner);
            if *at_prompt && asked.is_none_or(|at| at.elapsed() >= ASK_AGAIN) {
                SIGNALLED.store(true, Ordering::SeqCst);
                // SAFETY: a plain call, signalling this process.
                unsafe {
                    libc::kill(libc::getpid(), libc::SIGINT);
                }
                asked = Some(Instant::now());
            }
        }
    });
}

fn width_of(terminal: &std::fs::File) -> Option<usize> {
    // SAFETY: winsize is plain data, filled in by the ioctl.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let found = unsafe { libc::ioctl(terminal.as_raw_fd(), libc::TIOCGWINSZ, &mut size) } == 0;
    (found && size.ws_col > 0).then_some(size.ws_col as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_prints_as_it_went_and_fitted_pieces_at_the_width() {
        let mut screen = Screen::new();
        screen.keeping = true;
        screen.keep(Piece::Text("> hi\n".into()));
        screen.keep(Piece::Text("Hel".into()));
        screen.keep(Piece::Text("lo.\n".into()));
        screen.keep(Piece::Fitted(Box::new(|width| {
            format!("{}\n", "─".repeat(width - 1))
        })));
        // Text that follows text is one piece.
        assert_eq!(screen.pieces.len(), 2);
        let at = |width| laid_out(None, &screen.pieces, width);
        assert_eq!(
            at(6),
            "\x1b[?2026h\x1b[H\x1b[2J\x1b[3J> hi\nHello.\n─────\n\x1b[?2026l"
        );
        assert!(at(4).ends_with("Hello.\n───\n\x1b[?2026l"));
    }

    #[test]
    fn nothing_is_kept_outside_a_conversation() {
        let mut screen = Screen::new();
        screen.keep(Piece::Text("lokal --help".into()));
        assert!(screen.pieces.is_empty());
    }

    #[test]
    fn the_oldest_pieces_go_past_the_most_kept() {
        let mut screen = Screen::new();
        screen.keeping = true;
        let line = format!("{}\n", "x".repeat(1023));
        for _ in 0..(MOST_KEPT / 1024) {
            screen.keep(Piece::Text(line.clone()));
            screen.keep(Piece::Fitted(Box::new(|_| "─\n".into())));
        }
        assert!(screen.size <= MOST_KEPT && screen.dropped);
        assert!(matches!(
            screen.pieces[0],
            Piece::Text(_) | Piece::Fitted(_)
        ));
        let out = laid_out(Some("Earlier.\n"), &screen.pieces, 10);
        assert!(out.starts_with("\x1b[?2026h\x1b[H\x1b[2J\x1b[3JEarlier.\n"));
        assert!(out.len() < MOST_KEPT + 4096);
    }
}
