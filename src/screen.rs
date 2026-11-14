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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::tty::{self, RawMode};
use crate::{style, text};

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
    /// Where a step began whose question may be open: what follows is
    /// never let go for `MOST_KEPT`, so a redraw shows what's asked about.
    Pin,
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
        self.trim();
    }

    /// Past `MOST_KEPT`, the oldest go: whole pieces, or the start of a
    /// text up to a line's end, so a long answer keeps its latest lines (a
    /// line longer than all that's kept keeps its end). Never past a pin: a
    /// step's own output stays whole until its question is answered.
    fn trim(&mut self) {
        while self.size > MOST_KEPT {
            let excess = self.size - MOST_KEPT;
            let cut = match self.pieces.first() {
                None | Some(Piece::Pin) => return,
                Some(Piece::Text(text)) => Some(text_cut(text, excess)),
                Some(Piece::Fitted(_)) => None,
            };
            match (cut, self.pieces.first_mut()) {
                (Some(cut), Some(Piece::Text(text))) if cut < text.len() => {
                    text.drain(..cut);
                    // What a huge output held goes back, not only its count.
                    if text.capacity() > 2 * text.len() + 64 * 1024 {
                        text.shrink_to_fit();
                    }
                    self.size -= cut;
                }
                _ => {
                    let oldest = self.pieces.remove(0);
                    self.size -= size_of(&oldest);
                }
            }
            self.dropped = true;
        }
    }
}

/// Where `excess` bytes or a little more go from the start of `text`: at
/// the end of the line `excess` falls in, or right there when a line ends
/// before it or none ends after it.
fn text_cut(text: &str, excess: usize) -> usize {
    let from = text.ceil_char_boundary(excess);
    if from == 0 || text.as_bytes()[from - 1] == b'\n' {
        return from;
    }
    text[from..].find('\n').map_or(from, |at| from + at + 1)
}

fn size_of(piece: &Piece) -> usize {
    match piece {
        Piece::Text(text) => text.len(),
        Piece::Fitted(_) => FITTED_SIZE,
        Piece::Pin => 0,
    }
}

static SCREEN: Mutex<Screen> = Mutex::new(Screen::new());

/// The `>` is being read, so a redraw ends the reading, as mail does.
static AT_PROMPT: Mutex<bool> = Mutex::new(false);

/// A signal went to end the `>` for a redraw, and hasn't been seen yet.
static SIGNALLED: AtomicBool = AtomicBool::new(false);

/// The width settled on one the screen wasn't laid out at: a redraw is due
/// at the next place that can draw (`due`).
static DUE: AtomicBool = AtomicBool::new(false);

/// How many redraws there were, so what drew live parts knows they're gone.
static REDRAWS: AtomicU64 = AtomicU64::new(0);

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

/// A step begins whose question may open: until `unpin`, nothing from here
/// on goes for `MOST_KEPT`, so a redraw while the question is open still
/// shows what it asks about.
pub fn pin() {
    let mut screen = screen();
    if screen.keeping {
        screen.pieces.retain(|piece| !matches!(piece, Piece::Pin));
        screen.pieces.push(Piece::Pin);
    }
}

/// The step is done: what it showed may go like the rest.
pub fn unpin() {
    let mut screen = screen();
    screen.pieces.retain(|piece| !matches!(piece, Piece::Pin));
    screen.trim();
}

/// The screen was cleared (`/new`, Ctrl+L): a redraw clears it there too,
/// pushing what came before into the scrollback and going on from the top,
/// as the clear left it.
pub fn cleared() {
    kept(Box::new(|_| {
        format!("{}\x1b[H", "\n".repeat(tty::height().unwrap_or(24)))
    }));
}

/// Whether a redraw is due: during a turn, the spinner's tick and the
/// answer as it streams ask, and redraw then.
pub fn due() -> bool {
    DUE.load(Ordering::SeqCst)
}

/// How many redraws there were: a live part drawn before the last one is
/// gone from the screen, and draws anew rather than over what was there.
pub fn redraws() -> u64 {
    REDRAWS.load(Ordering::SeqCst)
}

/// Whether the terminal's width changed since the screen was laid out.
pub fn resized() -> bool {
    let screen = screen();
    screen.keeping && tty::width().is_some_and(|width| screen.width != Some(width))
}

/// The screen again at the terminal's width now, when it isn't the one the
/// screen was laid out at: cleared with its scrollback, every kept piece
/// laid out anew, written at once, in one synchronized frame where the
/// terminal has them, with keys typed meanwhile left unechoed for whatever
/// reads next. True when it redrew: what's live is drawn by its owner then.
pub fn redraw() -> bool {
    DUE.store(false, Ordering::SeqCst);
    let mut screen = screen();
    let Some(width) = tty::width() else {
        return false;
    };
    if !screen.keeping || screen.width == Some(width) {
        return false;
    }
    let terminal = tty::open();
    let _unechoed = terminal.as_ref().and_then(RawMode::enter);
    let earlier = screen
        .dropped
        .then(|| format!("{}\n", style::faint(text::EARLIER_NOT_KEPT)));
    let out = laid_out(earlier.as_deref(), &screen.pieces, width);
    let _ = std::io::stderr().write_all(out.as_bytes());
    screen.width = Some(width);
    REDRAWS.fetch_add(1, Ordering::SeqCst);
    true
}

/// Every piece at `width`, after what clears the screen and its scrollback
/// and the line saying `earlier` lines aren't there, when they aren't.
fn laid_out(earlier: Option<&str>, pieces: &[Piece], width: usize) -> String {
    // From no style: a redraw mid-answer may come with one open.
    let mut out = String::from("\x1b[?2026h\x1b[0m\x1b[H\x1b[2J\x1b[3J");
    out.push_str(earlier.unwrap_or_default());
    for piece in pieces {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Fitted(layout) => out.push_str(&layout(width)),
            Piece::Pin => {}
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
/// it settles on one the screen wasn't laid out at, a redraw is due: a `>`
/// being read is ended with a SIGINT for it, as mail ends it, since
/// rustyline has the keyboard there; during a turn, the spinner and the
/// answer's loop redraw at their next step (`due`).
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
                // Back where it was laid out: nothing is due any more.
                DUE.store(false, Ordering::SeqCst);
                asked = None;
                continue;
            }
            DUE.store(true, Ordering::SeqCst);
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
            "\x1b[?2026h\x1b[0m\x1b[H\x1b[2J\x1b[3J> hi\nHello.\n─────\n\x1b[?2026l"
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
        assert!(out.starts_with("\x1b[?2026h\x1b[0m\x1b[H\x1b[2J\x1b[3JEarlier.\n"));
        assert!(out.len() < MOST_KEPT + 4096);
    }

    #[test]
    fn a_text_is_cut_at_a_line_end_or_where_none_comes() {
        let text = "aaaa\nbbbb\ncccc";
        // A line ends right before: nothing more goes.
        assert_eq!(text_cut(text, 5), 5);
        // Inside a line: to its end.
        assert_eq!(text_cut(text, 6), 10);
        // In the last line, which has no end: right there.
        assert_eq!(text_cut(text, 12), 12);
        // Never inside a character.
        assert_eq!(text_cut("ééé", 1), 2);
    }

    #[test]
    fn a_long_text_keeps_its_latest_lines_and_a_pin_holds() {
        let mut screen = Screen::new();
        screen.keeping = true;
        // One answer bigger than all that's kept, in pieces as it streams.
        for n in 0..20_000 {
            screen.keep(Piece::Text(format!("line {n:05} of a long answer\n")));
        }
        screen.keep(Piece::Fitted(Box::new(|_| "─\n".into())));
        let Piece::Text(text) = &screen.pieces[0] else {
            panic!("the answer went whole");
        };
        assert!(text.starts_with("line ") && text.ends_with("line 19999 of a long answer\n"));
        assert!(screen.size <= MOST_KEPT && screen.dropped);
        // A pin: what follows stays, however much comes after it.
        screen.keep(Piece::Pin);
        screen.keep(Piece::Text("$ rm -rf build\n".into()));
        for _ in 0..1000 {
            screen.keep(Piece::Text(format!("{}\n", "x".repeat(1000))));
        }
        assert!(matches!(screen.pieces[0], Piece::Pin));
        assert!(
            matches!(&screen.pieces[1], Piece::Text(text) if text.starts_with("$ rm -rf build"))
        );
    }
}
