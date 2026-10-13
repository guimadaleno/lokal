//! Ctrl+C while the model or a command is running doesn't quit Lokal: it sets
//! a flag, and whatever is running checks it and stops.

use std::sync::atomic::{AtomicBool, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigint(_signal: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

pub fn install() {
    let handler: extern "C" fn(libc::c_int) = on_sigint;
    // SAFETY: the handler only stores to an atomic, which is safe in a signal handler.
    unsafe {
        libc::signal(libc::SIGINT, handler as libc::sighandler_t);
    }
}

/// The same as Ctrl+C arriving as a signal, for when it arrives as a key.
pub fn raise() {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

pub fn happened() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

pub fn clear() {
    INTERRUPTED.store(false, Ordering::SeqCst);
}
