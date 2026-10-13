//! Terminal colors. Lokal's own output goes to stderr, so colors are on when
//! stderr is a terminal.

use std::io::IsTerminal;
use std::sync::OnceLock;

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

pub fn red(text: &str) -> String {
    paint("31", text)
}

pub fn green(text: &str) -> String {
    paint("32", text)
}
