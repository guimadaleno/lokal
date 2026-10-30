//! Runs one command for the model. Its output shows as it comes (the first
//! lines of it) and is kept whole for the model; Ctrl+C stops it.

use std::env;
use std::io::{self, BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crate::spinner::Spinner;
use crate::{interrupt, keys, mode, style, text};

/// How many lines of output show on screen; the model gets more.
const SHOWN_LINES: usize = 12;
/// Longer lines are cut on screen.
const SHOWN_WIDTH: usize = 200;
/// When the time a command has been running starts to show beside the spinner.
const SHOW_TIME_AFTER: Duration = Duration::from_secs(3);

pub enum Run {
    Finished { output: String, code: Option<i32> },
    Stopped,
}

/// Runs `command` in bash, with `git_settings` over what git would read.
pub fn run(command: &str, git_settings: &[(&str, &str)]) -> io::Result<Run> {
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
    // What `git -c` passes on, which every git reads and hands to the
    // submodules it goes into; after any the person set, so these win.
    if !git_settings.is_empty() {
        let mut parameters = env::var("GIT_CONFIG_PARAMETERS").unwrap_or_default();
        for (name, value) in git_settings {
            parameters.push_str(&format!(" '{name}={value}'"));
        }
        shell.env("GIT_CONFIG_PARAMETERS", parameters.trim_start());
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
    drop(shell);
    let lines = read_lines(reader);

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
        match lines.recv_timeout(Duration::from_millis(40)) {
            Ok(line) => screen.add(&line),
            Err(_) => screen.tick(),
        }
    };
    // Collect what's left in the pipe, without waiting on a background process
    // that keeps it open.
    let deadline = Instant::now() + Duration::from_millis(200);
    while let Some(wait) = deadline.checked_duration_since(Instant::now()) {
        match lines.recv_timeout(wait) {
            Ok(line) => screen.add(&line),
            Err(_) => break,
        }
    }
    screen.finish();
    if stopping_since.is_some() {
        return Ok(Run::Stopped);
    }
    Ok(Run::Finished {
        output: screen.output,
        code: status.code(),
    })
}

/// Reads output on a thread, line by line. It keeps reading after Lokal stops
/// listening, so a process left in the background never blocks on a full pipe.
fn read_lines(reader: io::PipeReader) -> Receiver<String> {
    let (sender, lines) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();
        while reader
            .read_until(b'\n', &mut line)
            .is_ok_and(|read| read > 0)
        {
            let _ = sender.send(String::from_utf8_lossy(&line).into_owned());
            line.clear();
        }
    });
    lines
}

fn signal_group(pid: u32, signal: libc::c_int) {
    // SAFETY: plain syscall; the command leads its own process group.
    unsafe {
        libc::kill(-(pid as libc::pid_t), signal);
    }
}

/// The output so far, and what of it is on screen.
struct Screen {
    output: String,
    lines: usize,
    spinner: Spinner,
    started: Instant,
    /// The blank line before the output, owed until there is some: a silent
    /// command leaves no gap.
    gap: bool,
}

impl Screen {
    fn new() -> Self {
        Screen {
            output: String::new(),
            lines: 0,
            spinner: Spinner::new(),
            started: Instant::now(),
            gap: true,
        }
    }

    fn add(&mut self, line: &str) {
        self.output.push_str(line);
        self.lines += 1;
        if self.lines <= SHOWN_LINES {
            self.spinner.clear();
            if std::mem::take(&mut self.gap) {
                eprintln!();
            }
            let line: String = line.trim_end().chars().take(SHOWN_WIDTH).collect();
            eprintln!("  {}", style::gray(&line));
        } else {
            self.tick();
        }
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
        self.spinner.clear();
        if self.lines > SHOWN_LINES {
            eprintln!(
                "  {}",
                style::gray(&text::more_lines(self.lines - SHOWN_LINES))
            );
        }
    }
}
