//! Terminal colors. Lokal's own output goes to stderr, so colors are on when
//! stderr is a terminal.

use std::fs::OpenOptions;
use std::io::IsTerminal;
use std::os::fd::AsRawFd;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::io::stderr().is_terminal())
}

fn paint(code: &str, text: &str) -> String {
    if enabled() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn bold(text: &str) -> String {
    paint("1", text)
}

pub fn gray(text: &str) -> String {
    paint("90", text)
}

/// Dimmer than gray, for what should barely be noticed.
pub fn faint(text: &str) -> String {
    paint("38;5;239", text)
}

/// A soft orange: the spinner and the person's own messages.
pub fn accent(text: &str) -> String {
    paint("38;5;173", text)
}

/// Mail sent to another session reads like a mention: `@name` in the
/// accent and in italics.
pub fn mention(text: &str) -> String {
    paint("3;38;5;173", text)
}

/// The mail itself, under its mention: faint, in italics.
pub fn faint_italic(text: &str) -> String {
    paint("3;38;5;239", text)
}

/// The faint label while the model works, breathing slowly a little brighter.
pub fn pulse_faint(text: &str, since: Instant) -> String {
    pulse(
        text,
        [78, 78, 78],
        [118, 118, 118],
        since,
        Duration::from_millis(2400),
    )
}

/// The accent, breathing quicker toward a lighter orange: the marker on the
/// option an approval is on.
pub fn pulse_accent(text: &str, since: Instant) -> String {
    pulse(
        text,
        [215, 135, 95],
        [250, 178, 138],
        since,
        Duration::from_millis(1000),
    )
}

/// Text whose color goes from `from` to `to` and back once every `period`,
/// easing at both ends. It starts on `from`: faint's and accent's own color.
fn pulse(text: &str, from: [u8; 3], to: [u8; 3], since: Instant, period: Duration) -> String {
    let phase = since.elapsed().as_secs_f32() / period.as_secs_f32();
    let mix = (1.0 - (phase * std::f32::consts::TAU).cos()) / 2.0;
    let [red, green, blue] =
        [0, 1, 2].map(|i| (from[i] as f32 + (to[i] as f32 - from[i] as f32) * mix).round() as u8);
    paint(&format!("38;2;{red};{green};{blue}"), text)
}

pub fn red(text: &str) -> String {
    paint("31", text)
}

pub fn green(text: &str) -> String {
    paint("32", text)
}

/// Columns in the terminal, asked of it each time: the window may have changed.
pub fn terminal_width() -> Option<usize> {
    let terminal = OpenOptions::new().read(true).open("/dev/tty").ok()?;
    // SAFETY: winsize is plain data, filled in by the ioctl.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let found = unsafe { libc::ioctl(terminal.as_raw_fd(), libc::TIOCGWINSZ, &mut size) } == 0;
    (found && size.ws_col > 0).then_some(size.ws_col as usize)
}
