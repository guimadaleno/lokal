//! The terminal itself, `/dev/tty`, for reading keys one by one: while
//! Lokal works and in an approval's menu. Read there rather than on stdin,
//! so piped input never answers a question.

use std::fs::{File, OpenOptions};
use std::os::fd::{AsRawFd, RawFd};

/// The terminal, or None without one.
pub fn open() -> Option<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()
}

/// Keys arrive one by one, unechoed, and Ctrl+C arrives as a key rather
/// than a signal, for as long as this lives; then the terminal is put back
/// as it was, however that comes about. The terminal's file must outlive it.
pub struct RawMode {
    fd: RawFd,
    saved: libc::termios,
}

impl RawMode {
    pub fn enter(terminal: &File) -> Option<RawMode> {
        let fd = terminal.as_raw_fd();
        // SAFETY: termios is plain data, filled in by tcgetattr before use.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
            return None;
        }
        let mut raw = saved;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        // SAFETY: a valid descriptor and a termios read from it.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return None;
        }
        Some(RawMode { fd, saved })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        // SAFETY: the descriptor outlives this guard; the termios is the one read from it.
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.saved);
        }
    }
}

/// Whether a byte arrives on `fd` within `milliseconds`: a key, or the rest
/// of one's sequence.
pub fn readable(fd: RawFd, milliseconds: i32) -> bool {
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd, for the length given.
    unsafe { libc::poll(&mut poll, 1, milliseconds) > 0 }
}

/// Keys pressed while Lokal was working were pressed before the question was
/// on screen, so they can't be its answer: a "y" typed early would approve a
/// command nobody has seen.
pub fn discard_typed_ahead() {
    if let Some(terminal) = open() {
        // SAFETY: a plain call on a descriptor that stays open for it.
        unsafe {
            libc::tcflush(terminal.as_raw_fd(), libc::TCIFLUSH);
        }
    }
}
