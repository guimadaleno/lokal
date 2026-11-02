//! Ctrl+C while the model or a command is running doesn't quit Lokal: it sets
//! a flag, and whatever is running checks it and stops.

use std::sync::atomic::{AtomicBool, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigint(_signal: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

pub fn install() {
    let handler: extern "C" fn(libc::c_int) = on_sigint;
    // sigaction rather than signal(), whose meaning differs between C
    // libraries. SA_RESTART: a read the signal interrupts goes on.
    // SAFETY: sigaction is plain data, zeroed then filled in; the handler
    // only stores to an atomic, which is safe in a signal handler.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
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
