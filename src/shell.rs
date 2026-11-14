//! Runs one command for the model. Its output shows as it comes (the first
//! lines of it), and its start and end are kept for the model; Ctrl+C
//! stops it.

use std::collections::VecDeque;
use std::env;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crate::spinner::Spinner;
use crate::{gitconfig, interrupt, keys, layout, mode, style, system, text, tools};

/// How many lines of output show on screen; the model gets more.
const SHOWN_LINES: usize = 12;
/// Longer lines are cut on screen.
const SHOWN_WIDTH: usize = 200;
/// When the time a command has been running starts to show beside the spinner.
const SHOW_TIME_AFTER: Duration = Duration::from_secs(3);
/// How many bytes of an output's start, and of its end, are kept.
const KEPT: usize = tools::OUTPUT_LIMIT;

pub enum Run {
    /// `output` is whole, or its start and end with `hidden_lines` line
    /// breaks left out between them.
    Finished {
        output: String,
        hidden_lines: usize,
        code: Option<i32>,
    },
    Stopped,
}

/// Runs `command` in bash. One that runs `unasked` gets
/// `gitconfig::UNASKED` over what git would read, and a PATH without
/// folders relative to where it runs.
pub fn run(command: &str, unasked: bool) -> io::Result<Run> {
    let (reader, writer) = io::pipe()?;
    let mut shell = Command::new("bash");
    shell
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer)
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat");
    if unasked {
        // What `git -c` passes on, which every git reads and hands to the
        // submodules it goes into; after any the person set, so these win.
        let mut parameters = env::var("GIT_CONFIG_PARAMETERS").unwrap_or_default();
        for (name, value) in gitconfig::UNASKED {
            parameters.push_str(&format!(" '{name}={value}'"));
        }
        shell.env("GIT_CONFIG_PARAMETERS", parameters.trim_start());
        // With `.` or an empty entry in it, `ls` could be a program the
        // project planted, run with no one's yes.
        if let Some(path) = env::var_os("PATH") {
            shell.env("PATH", system::absolute_only(&path.to_string_lossy()));
        }
    }
    // SAFETY: setsid is safe to call between fork and exec.
    // A session of its own: no terminal to wait on for input (sudo fails at
    // once instead of hanging), and Ctrl+C reaches it only through Lokal.
    unsafe {
        shell.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = shell.spawn()?;
    // The Command holds the pipe's writing ends: only once they're closed
    // does the reader see the output end.
    drop(shell);
    let chunks = read_chunks(reader);

    // Ctrl+C as a key too, and nothing typed echoed over the output.
    let _keys = keys::KeyWatch::start();
    let mut screen = Screen::new();
    let mut stopping_since: Option<Instant> = None;
    let status = loop {
        if interrupt::happened() && stopping_since.is_none() {
            signal_group(child.id(), libc::SIGINT);
            stopping_since = Some(Instant::now());
        }
        if stopping_since.is_some_and(|since| since.elapsed() > Duration::from_secs(1)) {
            signal_group(child.id(), libc::SIGKILL);
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        match chunks.recv_timeout(Duration::from_millis(40)) {
            Ok(chunk) => screen.add(&chunk),
            Err(_) => screen.tick(),
        }
    };
    // Collect what's left in the pipe, without waiting on a background process
    // that keeps it open.
    let deadline = Instant::now() + Duration::from_millis(200);
    while let Some(wait) = deadline.checked_duration_since(Instant::now()) {
        match chunks.recv_timeout(wait) {
            Ok(chunk) => screen.add(&chunk),
            Err(_) => break,
        }
    }
    screen.finish();
    if stopping_since.is_some() {
        return Ok(Run::Stopped);
    }
    Ok(Run::Finished {
        hidden_lines: screen.kept.hidden_lines,
        output: screen.kept.text(),
        code: status.code(),
    })
}

/// Reads output on a thread, in pieces of any length, so a line that never
/// ends can't fill the memory. It keeps reading after Lokal stops listening,
/// so a process left in the background never blocks on a full pipe.
fn read_chunks(mut reader: io::PipeReader) -> Receiver<Vec<u8>> {
    // Bounded, so output faster than the screen waits in the pipe.
    let (sender, chunks) = mpsc::sync_channel(16);
    thread::spawn(move || {
        let mut buffer = [0; 8192];
        while let Ok(read @ 1..) = reader.read(&mut buffer) {
            let _ = sender.send(buffer[..read].to_vec());
        }
    });
    chunks
}

/// What the model gets of an output: its start and its end, `KEPT` bytes
/// each, with the line breaks between counted, not kept. That's more than
/// `tools::clip` ever gives it, so only what it would leave out goes.
#[derive(Default)]
struct Kept {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    hidden_lines: usize,
}

impl Kept {
    fn add(&mut self, chunk: &[u8]) {
        let room = KEPT.saturating_sub(self.head.len()).min(chunk.len());
        self.head.extend_from_slice(&chunk[..room]);
        self.tail.extend(&chunk[room..]);
        let over = self.tail.len().saturating_sub(KEPT);
        self.hidden_lines += self
            .tail
            .drain(..over)
            .filter(|&byte| byte == b'\n')
            .count();
    }

    fn text(&self) -> String {
        let mut bytes = self.head.clone();
        bytes.extend(&self.tail);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// A signal to a process and everything in its group.
pub fn signal_group(pid: u32, signal: libc::c_int) {
    // SAFETY: plain syscall; the command leads its own process group.
    unsafe {
        libc::kill(-(pid as libc::pid_t), signal);
    }
}

/// The output so far, and what of it is on screen.
struct Screen {
    kept: Kept,
    /// Lines the output has had, each ended by a line break.
    lines: usize,
    /// The line being shown, as far as it has come.
    line: Vec<u8>,
    /// The last byte so far isn't a line break: a last line without one.
    mid_line: bool,
    spinner: Spinner,
    started: Instant,
    /// The blank line before the output, owed until there is some: a silent
    /// command leaves no gap.
    gap: bool,
}

impl Screen {
    fn new() -> Self {
        Screen {
            kept: Kept::default(),
            lines: 0,
            line: Vec::new(),
            mid_line: false,
            spinner: Spinner::new(),
            started: Instant::now(),
            gap: true,
        }
    }

    fn add(&mut self, chunk: &[u8]) {
        self.kept.add(chunk);
        self.mid_line = chunk.last().is_some_and(|&byte| byte != b'\n');
        let mut rest = chunk;
        while self.lines < SHOWN_LINES && !rest.is_empty() {
            let end = rest.iter().position(|&byte| byte == b'\n');
            let part = &rest[..end.unwrap_or(rest.len())];
            // Only its start shows, so only that much is held.
            let room = (SHOWN_WIDTH * 4).saturating_sub(self.line.len());
            self.line.extend_from_slice(&part[..room.min(part.len())]);
            match end {
                Some(end) => {
                    self.show_line();
                    rest = &rest[end + 1..];
                }
                None => rest = &[],
            }
        }
        if rest.is_empty() && self.lines < SHOWN_LINES {
            return;
        }
        self.lines += rest.iter().filter(|&&byte| byte == b'\n').count();
        self.tick();
    }

    fn show_line(&mut self) {
        self.lines += 1;
        self.spinner.clear();
        if std::mem::take(&mut self.gap) {
            showln!();
        }
        let line = String::from_utf8_lossy(&self.line);
        let line: String = layout::printable(line.trim_end())
            .chars()
            .take(SHOWN_WIDTH)
            .collect();
        showln!("  {}", style::gray(&line));
        self.line.clear();
    }

    /// The spinner, with the mode when it isn't manual, so Shift+Tab is seen
    /// to take; and once the command has taken a while, for how long: a
    /// silent command looks like the model thinking otherwise.
    fn tick(&mut self) {
        let running = self.started.elapsed();
        let mut parts: Vec<String> = text::mode_name(mode::current())
            .into_iter()
            .map(String::from)
            .collect();
        if running >= SHOW_TIME_AFTER {
            parts.push(text::duration(running));
        }
        if parts.is_empty() {
            self.spinner.tick();
        } else {
            let label = format!(" {}", style::faint(&parts.join(" · ")));
            self.spinner.tick_with(&label);
        }
    }

    fn finish(&mut self) {
        if self.mid_line {
            if self.lines < SHOWN_LINES {
                self.show_line();
            } else {
                self.lines += 1;
            }
        }
        self.spinner.clear();
        if self.lines > SHOWN_LINES {
            showln!(
                "  {}",
                style::gray(&text::more_lines(self.lines - SHOWN_LINES))
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endless_output_keeps_its_start_and_end_and_counts_the_rest() {
        let text: String = (1..=10_000).map(|n| format!("line {n}\n")).collect();
        let mut kept = Kept::default();
        for chunk in text.as_bytes().chunks(8192) {
            kept.add(chunk);
        }
        let shown = kept.text();
        assert_eq!(shown.len(), 2 * KEPT);
        assert!(shown.starts_with("line 1\n") && shown.ends_with("line 10000\n"));
        let middle = &text[KEPT..text.len() - KEPT];
        assert_eq!(kept.hidden_lines, middle.matches('\n').count());
        // Short, it's kept whole.
        let mut kept = Kept::default();
        kept.add(b"a\nb");
        assert_eq!((kept.text().as_str(), kept.hidden_lines), ("a\nb", 0));
    }
}
