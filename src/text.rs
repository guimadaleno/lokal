//! Everything Lokal says to the person, in one place.

use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
pub const AUTHOR: &str = env!("CARGO_PKG_AUTHORS");
pub const WEBSITE: &str = env!("CARGO_PKG_HOMEPAGE");
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub const MADE_BY: &str = "Made by";

pub const USAGE: &str = "\
Usage:
  lokal              start a conversation
  lokal <request>    ask one thing, get the answer, done
  lokal -r           pick up the last conversation
  lokal -r <request> add to the last conversation
  lokal --install    put this copy in ~/.local/bin, where it keeps itself up to date

Settings live in ~/.config/lokal/config (server = 192.0.2.10:1234, model = name, updates = off).";

/// The slash commands, with what each does.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/new", "start fresh"),
    ("/prefs", "edit what Lokal knows about you"),
    ("/help", "show commands and keys"),
    ("/about", "who made Lokal"),
    ("/exit", "quit"),
];

pub const HELP_KEYS: &str = "\
Tab completes a command. Ctrl+C stops whatever's running, or clears what you typed;
twice on an empty line, it quits. lokal -r picks up the last conversation.";

pub const FIRST_QUESTION: &str = "Before we start: how do you like your answers? (Enter to skip)";
pub const PREFS_HEADING: &str = "How I like answers:";
pub const NEW_CONVERSATION: &str = "Fresh start.";
pub const RESUMED: &str = "Picking up where we left off.";
pub const NOTHING_TO_RESUME: &str = "Nothing to pick up yet.";
pub const STOPPED: &str = "Stopped.";
pub const QUIT_AGAIN: &str = "Press Ctrl+C again to quit";

pub const RUN_QUESTION: &str = "Run it?";
pub const WRITE_QUESTION: &str = "Write it?";
pub const CHOICE_YES: &str = "Yes";
pub const CHOICE_YES_SESSION: &str = "Yes, and don't ask again for this command";
pub const CHOICE_NO: &str = "No";
pub const APPROVED_FOR_SESSION: &str = "approved earlier in this session";
pub const NO_TERMINAL: &str = "Skipped: there's no terminal to ask you on.";

pub const STEP_READ: &str = "read";
pub const STEP_CREATE: &str = "create";
pub const STEP_EDIT: &str = "edit";

pub const NO_HTTPS: &str =
    "Lokal only speaks plain HTTP to the server, so `server` can't be an https:// address.";
pub const CONNECTION_DROPPED: &str = "The server hung up before finishing its answer.";
pub const NO_MODEL_LOADED: &str = "No model is loaded in LM Studio. Load one there, or pin one with `model` in ~/.config/lokal/config.";
pub const NO_MODELS: &str = "The server doesn't list any models.";

/// 7s, 1m 05s, 2h 01m 05s
pub fn duration(time: Duration) -> String {
    let total = time.as_secs_f64().round().max(1.0) as u64;
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// The goodbye line: how long the session lasted, and an update it brought.
pub fn lasted(time: Duration, updated_to: Option<&str>) -> String {
    match updated_to {
        Some(version) => format!(
            "Lasted {} · {version} is ready for next time",
            duration(time)
        ),
        None => format!("Lasted {}", duration(time)),
    }
}

pub const NOT_ON_PATH: &str = "~/.local/bin isn't on your PATH yet. Add this line to ~/.bashrc, then open a new terminal:\n  export PATH=\"$HOME/.local/bin:$PATH\"";

pub fn installed(path: &str) -> String {
    format!("Installed at {path}. Type lokal to start.")
}

pub fn already_installed(path: &str) -> String {
    format!("This is already the installed copy, at {path}.")
}

pub fn cant_install(error: &str) -> String {
    format!("Couldn't install: {error}")
}

pub fn more_lines(count: usize) -> String {
    format!("… +{count} lines")
}

pub fn unknown_option(option: &str) -> String {
    format!("There's no {option} option. Try lokal --help.")
}

pub fn unknown_command(command: &str) -> String {
    format!("There's no {command}. Try /help.")
}

pub fn saved_prefs(path: &str) -> String {
    format!("Got it, saved to {path}. /prefs changes it anytime.")
}

pub fn long_prefs(lines: usize) -> String {
    format!(
        "about-me.md is getting long ({lines} lines). It goes along with every conversation, so trimming it saves room. /prefs opens it."
    )
}

pub fn no_editor(editor: &str) -> String {
    format!("Couldn't open `{editor}`. Set $EDITOR to the editor you use.")
}

pub fn cant_save(path: &str, error: &str) -> String {
    format!("Couldn't save {path}: {error}")
}

pub fn bad_config_line(path: &str, line: usize) -> String {
    format!("{path}, line {line}: expected `name = value`.")
}

pub fn bad_updates_setting(path: &str, line: usize) -> String {
    format!("{path}, line {line}: `updates` is either on or off.")
}

pub fn unknown_setting(path: &str, line: usize, name: &str) -> String {
    format!(
        "{path}, line {line}: there's no setting called `{name}`. The ones that exist are `server`, `model` and `updates`."
    )
}

pub fn no_server(tried: &[String]) -> String {
    format!(
        "Couldn't find a model server (tried {}). Is LM Studio's server running? You can also set `server` in ~/.config/lokal/config.",
        tried.join(" and ")
    )
}

pub fn cant_reach(address: &str) -> String {
    format!("Can't reach the server at {address}. Is it running?")
}

pub fn server_said(message: &str) -> String {
    format!("The server said: {message}")
}

pub fn server_status(status: u16) -> String {
    format!("The server answered with HTTP {status}.")
}

pub fn not_json(address: &str) -> String {
    format!("The server at {address} sent something that isn't the JSON Lokal expects.")
}

pub fn no_such_model(model: &str) -> String {
    format!("The server has no model called `{model}`.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_drop_empty_leading_units() {
        assert_eq!(duration(Duration::from_millis(300)), "1s");
        assert_eq!(duration(Duration::from_secs(42)), "42s");
        assert_eq!(duration(Duration::from_secs(65)), "1m 05s");
        assert_eq!(duration(Duration::from_secs(2 * 3600 + 65)), "2h 01m 05s");
    }
}
