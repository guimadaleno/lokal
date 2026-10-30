//! While Lokal works, it reads the keyboard itself, so Ctrl+C or Esc stops
//! the work as a key, the way Ctrl+C does at the prompt. Some terminals never
//! turn Ctrl+C into a signal while a program runs (a Windows console running
//! WSL), and there the key is the only way it arrives. Shift+Tab changes the
//! mode, as at the prompt. Other keys typed meanwhile aren't echoed into the
//! output, where they'd break the spinner's and the reasoning window's
//! redraws; they're dropped.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use crate::{interrupt, mode};

/// Ctrl+C, as a key.
const CTRL_C: u8 = 3;
const ESCAPE: u8 = 0x1b;
/// What terminals send for Shift+Tab.
const SHIFT_TAB: &[u8] = b"\x1b[Z";

/// The keyboard, read for as long as this lives; the terminal is put back as
/// it was however it ends.
pub struct KeyWatch {
    terminal: File,
    saved: libc::termios,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

impl KeyWatch {
    /// None without a terminal: then Ctrl+C can only come as a signal.
    pub fn start() -> Option<KeyWatch> {
        let terminal = OpenOptions::new().read(true).open("/dev/tty").ok()?;
        let fd = terminal.as_raw_fd();
        // SAFETY: termios is plain data, filled in by tcgetattr before use.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
            return None;
        }
        let mut raw = saved;
        // Keys one by one, unechoed, and Ctrl+C a key rather than a signal.
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        // SAFETY: a valid descriptor and a termios read from it.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return None;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let reader_stop = stop.clone();
        let reader = thread::spawn(move || read_keys(fd, &reader_stop));
        Some(KeyWatch {
            terminal,
            saved,
            stop,
            reader: Some(reader),
        })
    }
}

/// Reads until told to stop, looking up every 50 ms; Ctrl+C raises the flag
/// that whatever is running checks.
fn read_keys(fd: i32, stop: &AtomicBool) {
    let mut buffer = [0u8; 64];
    while !stop.load(Ordering::SeqCst) {
        let mut poll = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd, for the length given.
        if unsafe { libc::poll(&mut poll, 1, 50) } <= 0 {
            continue;
        }
        // SAFETY: the buffer is valid for its length, and the descriptor
        // outlives this thread: KeyWatch joins it before closing the file.
        let read = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
        if read <= 0 {
            break;
        }
        let keys = &buffer[..read as usize];
        if keys.contains(&CTRL_C) || (keys == [ESCAPE] && !more_follows(fd)) {
            interrupt::raise();
        }
        // From the next step on; the spinner's label shows it.
        for _ in keys
            .windows(SHIFT_TAB.len())
            .filter(|keys| *keys == SHIFT_TAB)
        {
            mode::next();
        }
    }
}

/// Esc alone stops; an arrow key is Esc followed by the rest of its
/// sequence a moment later, and is only a key someone pressed.
fn more_follows(fd: i32) -> bool {
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd, for the length given.
    unsafe { libc::poll(&mut poll, 1, 30) > 0 }
}

impl Drop for KeyWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        // SAFETY: the descriptor is still open; the termios is the one read from it.
        unsafe {
            libc::tcsetattr(self.terminal.as_raw_fd(), libc::TCSANOW, &self.saved);
        }
    }
}
