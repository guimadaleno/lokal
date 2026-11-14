//! How much Lokal asks before a step: the mode the person picks with
//! Shift+Tab, at the `>` or while Lokal works. Every session starts on
//! manual, and a turn mail started stays manual whatever the mode.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crate::{layout, style, text};

/// Kept in `MODE` as its number, which `current` reads back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Every step that isn't read-only asks.
    Manual = 0,
    /// File changes whose question would open on Yes go through without it.
    AcceptEdits = 1,
    /// Accept edits, and commands whose question would open on Yes run
    /// when the judge says yes (`judge.rs`).
    Auto = 2,
}

static MODE: AtomicU8 = AtomicU8::new(0);
/// Auto's warning has shown this session.
static WARNED: AtomicBool = AtomicBool::new(false);

pub fn current() -> Mode {
    match MODE.load(Ordering::SeqCst) {
        1 => Mode::AcceptEdits,
        2 => Mode::Auto,
        _ => Mode::Manual,
    }
}

/// Shift+Tab: the next mode, after the last one manual again.
pub fn next() {
    let next = match current() {
        Mode::Manual => Mode::AcceptEdits,
        Mode::AcceptEdits => Mode::Auto,
        Mode::Auto => Mode::Manual,
    };
    MODE.store(next as u8, Ordering::SeqCst);
}

/// The mode for a step: manual in a turn mail started, since another model
/// wrote that mail, and it may have been steered.
pub fn for_step(on_mail: bool) -> Mode {
    if on_mail { Mode::Manual } else { current() }
}

/// Auto is on, and its warning hasn't shown this session.
pub fn warning_due() -> bool {
    current() == Mode::Auto && !WARNED.load(Ordering::SeqCst)
}

/// True once a session, the first time it's asked with auto on: auto's
/// warning shows then. Shift+Tab shows it as auto is chosen, at the `>` or
/// as soon as nothing is mid-line while Lokal works, and a step that auto
/// would decide shows it first if it hasn't yet.
pub fn warn_of_auto() -> bool {
    current() == Mode::Auto && !WARNED.swap(true, Ordering::SeqCst)
}

/// Auto's warning, when it's due, with a blank line after it.
pub fn show_warning() {
    if warn_of_auto() {
        crate::screen::fitted(layout::indented_rows(text::AUTO_WARNING, 0, style::gray));
        showln!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_tab_goes_round_and_mail_stays_manual() {
        assert_eq!(current(), Mode::Manual);
        assert!(!warn_of_auto());
        next();
        assert_eq!(current(), Mode::AcceptEdits);
        assert_eq!(for_step(false), Mode::AcceptEdits);
        assert_eq!(for_step(true), Mode::Manual);
        next();
        assert_eq!(current(), Mode::Auto);
        assert_eq!(for_step(true), Mode::Manual);
        // The warning, once.
        assert!(warning_due());
        assert!(warn_of_auto());
        assert!(!warning_due());
        assert!(!warn_of_auto());
        next();
        assert_eq!(current(), Mode::Manual);
    }
}
