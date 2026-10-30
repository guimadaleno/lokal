//! How much Lokal asks before a step: the mode the person picks with
//! Shift+Tab, at the `>` or while Lokal works. Every session starts on
//! manual, and a turn mail started stays manual whatever the mode.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Every step that isn't read-only asks.
    Manual,
    /// File changes whose question would open on Yes go through without it.
    AcceptEdits,
}

static MODE: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Mode {
    match MODE.load(Ordering::SeqCst) {
        1 => Mode::AcceptEdits,
        _ => Mode::Manual,
    }
}

/// Shift+Tab: the next mode, after the last one manual again.
pub fn next() {
    let next = match current() {
        Mode::Manual => Mode::AcceptEdits,
        Mode::AcceptEdits => Mode::Manual,
    };
    MODE.store(next as u8, Ordering::SeqCst);
}

/// The mode for a step: manual in a turn mail started, since another model
/// wrote that mail, and it may have been steered.
pub fn for_step(on_mail: bool) -> Mode {
    if on_mail { Mode::Manual } else { current() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_tab_goes_round_and_mail_stays_manual() {
        assert_eq!(current(), Mode::Manual);
        next();
        assert_eq!(current(), Mode::AcceptEdits);
        assert_eq!(for_step(false), Mode::AcceptEdits);
        assert_eq!(for_step(true), Mode::Manual);
        next();
        assert_eq!(current(), Mode::Manual);
    }
}
