//! `lokal mcp add <name> <command or address>`: puts a server's line in the
//! person's settings once they've seen it whole and said yes at a terminal,
//! the question opening on No. Without a terminal nothing is written. A
//! model's command that runs Lokal opens on No too (`allowlist/hard.rs`),
//! since a scripted terminal could answer the question here.

use std::fs;
use std::io::{self, ErrorKind, IsTerminal};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use crate::{config, layout, menu, style, text, tty};

pub fn run(name: &str, command: &[String]) -> ExitCode {
    if name.is_empty() || command.is_empty() {
        eprintln!("{}", text::MCP_ADD_USAGE);
        return ExitCode::from(2);
    }
    let path = config::settings_path();
    let shown = config::shown(&path);
    let line = match config::mcp_line(name, command) {
        Ok(line) => line,
        Err(message) => return failed(&message),
    };
    let existing = match read(&path) {
        Ok(existing) => existing,
        Err(error) => return failed(&text::cant_read_settings(&shown, &error.to_string())),
    };
    // A first file is written out as /config writes it, every setting commented.
    let contents = existing.as_deref().unwrap_or(text::SETTINGS_TEMPLATE);
    let new = match config::with_mcp_line(contents, name, &line, &shown) {
        Ok(new) => new,
        Err(message) => return failed(&message),
    };
    // The line and the question both reach the person, or nothing shows: a
    // `!` line at Lokal's `>` has no terminal, and stderr may go elsewhere.
    if !io::stderr().is_terminal() || tty::open().is_none() {
        return failed(text::MCP_ADD_NO_TERMINAL);
    }
    let address = matches!(command, [word] if config::is_address(word));
    eprintln!("{}", text::mcp_add_goes_to(&shown));
    eprintln!();
    eprintln!("  {}", layout::printable(&line));
    eprintln!();
    if address {
        layout::print_fitted(text::MCP_ADD_ADDRESS_NOTE, style::gray);
        layout::print_fitted(&text::mcp_add_header_hint(name), style::gray);
    } else {
        layout::print_fitted(text::MCP_ADD_NOTE, style::gray);
    }
    eprintln!();
    let items = [
        menu::Item {
            label: text::CHOICE_YES,
            key: 'y',
        },
        menu::Item {
            label: text::CHOICE_NO,
            key: 'n',
        },
    ];
    // Keys pressed before the question showed aren't its answer.
    tty::discard_typed_ahead();
    match menu::choose(text::MCP_ADD_QUESTION, &items, 1, 1, Duration::ZERO) {
        Some(menu::Choice::Picked(0)) => write(&path, &shown, existing.as_deref(), &new),
        Some(_) => {
            layout::print_fitted(text::MCP_NOT_ADDED, style::gray);
            ExitCode::FAILURE
        }
        None => failed(text::MCP_ADD_NO_TERMINAL),
    }
}

/// The settings as they are, None when there's no file yet.
fn read(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes `new` unless the settings changed while the question was open:
/// what was added in /config meanwhile would be lost.
fn write(path: &Path, shown: &str, existing: Option<&str>, new: &str) -> ExitCode {
    match read(path) {
        Ok(now) if now.as_deref() == existing => {}
        Ok(_) => return failed(text::MCP_ADD_SETTINGS_CHANGED),
        Err(error) => return failed(&text::cant_read_settings(shown, &error.to_string())),
    }
    match config::write(path, new) {
        Ok(()) => {
            layout::print_fitted(&text::mcp_added(shown), |line| line.to_string());
            ExitCode::SUCCESS
        }
        Err(error) => failed(&text::cant_save(shown, &error.to_string())),
    }
}

fn failed(message: &str) -> ExitCode {
    layout::print_fitted(message, style::red);
    ExitCode::FAILURE
}
