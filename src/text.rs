//! Everything Lokal says to the person, in one place.

use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
pub const AUTHOR: &str = env!("CARGO_PKG_AUTHORS");
pub const WEBSITE: &str = env!("CARGO_PKG_HOMEPAGE");
pub const MADE_BY: &str = "Made by";

pub const USAGE: &str = "\
Usage:
  lokal              start a conversation
  lokal <request>    ask one thing, get the answer, done
  lokal -r           pick up the last conversation
  lokal -r <request> add to the last conversation
  lokal --install    put this copy in ~/.local/bin, where it keeps itself up to date
  lokal --uninstall  remove it from ~/.local/bin, and your files if you say so

Settings live in ~/.config/lokal/config (server = 192.0.2.10:1234, model = name, updates = off).";

/// The slash commands, with what each does.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/new", "start fresh"),
    ("/prefs", "edit what Lokal knows about you"),
    ("/memory", "edit the project's notes for the model"),
    ("/config", "edit Lokal's settings"),
    ("/info", "show the model, the server and what's in use"),
    (
        "/recon",
        "tell the model about this machine and the project",
    ),
    ("/update", "check for a newer Lokal now"),
    ("/reasoning", "show or hide the model's reasoning"),
    ("/feedback", "report a problem or an idea"),
    ("/help", "show commands and keys"),
    ("/about", "who made Lokal"),
    ("/exit", "quit"),
];

pub const HELP_KEYS: &str = "\
Tab completes a command. Ctrl+C or Esc stops whatever's running; at the >,
Ctrl+C clears what you typed, and twice on an empty line it quits.
A message from another Lokal session shows above the >: Enter on an empty
line passes it to the model, and Esc drops it.
lokal -r picks up the last conversation.";

pub const SETTINGS_TEMPLATE: &str = "\
# Lokal's settings. Each one is optional: remove the # to use it.

# The model server, when Lokal shouldn't look for it on its own:
# server = 192.0.2.10:1234

# The model, when it shouldn't take the one LM Studio has loaded:
# model = name-of-a-model

# The daily update check:
# updates = off
";
pub const SETTINGS_NEXT_START: &str = "Saved. Settings apply the next time Lokal starts.";

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
pub const CHOICE_YES_SESSION: &str = "Yes, and don't ask again this session";
pub const CHOICE_YES_PROJECT: &str = "Yes, and don't ask again in this project";
pub const CHOICE_SHOW_REST: &str = "Show the rest";
pub const CHOICE_NO: &str = "No";
pub const APPROVED_FOR_SESSION: &str = "approved earlier in this session";
pub const APPROVED_FOR_PROJECT: &str = "approved for this project";
pub const APPROVALS_HEADER: &str = "\
# Commands Lokal runs without asking, by project: the project's folder, a tab,
# then the exact command. Delete a line to be asked again.
";
pub const NO_TERMINAL: &str = "Skipped: there's no terminal to ask you on.";
pub const GIT_SETTINGS_UNREAD: &str =
    "Lokal couldn't read this repository's git settings, so it asks first.";

pub fn git_runs_program(setting: &str) -> String {
    format!(
        "This repository's git settings name a program for git to run ({setting}), so Lokal asks first."
    )
}

pub const STEP_READ: &str = "read";
pub const STEP_CREATE: &str = "create";
pub const STEP_EDIT: &str = "edit";
pub const STEP_REMEMBER: &str = "remember";
pub const STEP_SESSIONS: &str = "sessions";
pub const STEP_SEND: &str = "send";
pub const ALREADY_REMEMBERED: &str = "Already there, so nothing changes.";
pub const NO_SESSIONS: &str = "no others open";
pub const WORKING: &str = "working";
pub const MESSAGE_KEYS: &str = "Enter passes it to the model, Esc drops it.";

/// Above a message from another session: "From api-3f2a, in ~/sites/api:".
pub fn message_from(name: &str, folder: &str) -> String {
    format!("From {name}, in {folder}:")
}

/// At the right of the `>`, while messages from other sessions wait.
pub fn messages_waiting(count: usize) -> String {
    match count {
        1 => "1 message waits".into(),
        count => format!("{count} messages wait"),
    }
}

pub fn passed_on(count: usize, senders: &[String]) -> String {
    match count {
        1 => format!(
            "Passed the message from {} on to the model.",
            listed(senders)
        ),
        count => format!(
            "Passed {count} messages on to the model, from {}.",
            listed(senders)
        ),
    }
}

pub fn dropped(count: usize, senders: &[String]) -> String {
    match count {
        1 => format!("Dropped the message from {}.", listed(senders)),
        count => format!("Dropped {count} messages, from {}.", listed(senders)),
    }
}

/// "a", "a and b", "a, b and c"
fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

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

pub fn uninstalled(path: &str) -> String {
    format!("Removed {path}.")
}

pub fn not_installed(path: &str) -> String {
    format!("There's no Lokal at {path} to remove.")
}

pub fn cant_uninstall(error: &str) -> String {
    format!("Couldn't remove Lokal: {error}")
}

pub const YOUR_FILES: &str = "Lokal kept these for you:";
pub const REMOVE_FILES_QUESTION: &str = "Remove them too?";
pub const KEEP_THEM: &str = "Keep them";
pub const REMOVE_THEM: &str = "Remove them";
pub const REMOVED_FILES: &str = "Removed them.";
pub const NOT_ASKED: &str = "There's no terminal to ask you on, so they stay.";

pub fn kept_files(folders: &[String]) -> String {
    format!("Kept them, in {}.", folders.join(" and "))
}

pub fn cant_remove(paths: &[String]) -> String {
    format!("Couldn't remove {}.", paths.join(", "))
}

pub fn folder_stays(folder: &str) -> String {
    format!("{folder} stays: it holds files Lokal didn't make.")
}

pub const THINKING: &str = "thinking";
pub const ASKING_AGAIN: &str =
    "The model ran out of room while reasoning. Asking it again, to keep it short.";
pub const NO_ANSWER: &str =
    "The model stopped without an answer. Ask again, or /new to start fresh.";
pub const RAN_OUT: &str = "The model ran out of room before answering, most likely reasoning in circles. /new starts fresh.";
pub const CUT_SHORT: &str = "The model ran out of room in the middle of a step, so it didn't run. Asking it again, to keep it short.";
pub const CUT_SHORT_AGAIN: &str = "The model ran out of room in the middle of a step again, so it didn't run. /new starts fresh; more context in LM Studio gives it more room.";

pub fn ran_out_of_room(context: u64) -> String {
    format!(
        "The model used up its whole context ({} tokens) before answering, most likely reasoning in circles. /new starts fresh; more context in LM Studio gives it more room.",
        number(context)
    )
}
pub const REASONING_SHOWN: &str = "The model's reasoning shows for the rest of this session, in a small window that clears itself.";
pub const REASONING_HIDDEN: &str = "The model's reasoning is hidden for the rest of this session.";

pub const FEEDBACK_KIND: &str = "What would you like to tell the developer?";
pub const FEEDBACK_BUG: &str = "Something doesn't work";
pub const FEEDBACK_IDEA: &str = "Something could be better";
pub const NEVER_MIND: &str = "Never mind";
pub const FEEDBACK_TITLE: &str = "  Title: ";
pub const FEEDBACK_WHAT_HAPPENED: &str = "  What happened? ";
pub const FEEDBACK_THE_IDEA: &str = "  The idea: ";
pub const FEEDBACK_CONTACT: &str =
    "  Where can you be answered? (optional, only if you'd like a reply) ";
pub const FEEDBACK_PREVIEW: &str = "This is everything that will be sent:";
pub const FEEDBACK_SEND_QUESTION: &str = "Send it?";
pub const FEEDBACK_SEND: &str = "Send";
pub const FEEDBACK_SEND_LESS: &str = "Send without the system and the details";
pub const FEEDBACK_DONT_SEND: &str = "Don't send";
pub const FEEDBACK_KEPT: &str = "Not sent. /feedback keeps what you typed.";
pub const FEEDBACK_REFUSED: &str =
    "It couldn't be sent. That's Lokal's fault or the service's, not yours.";
pub const FEEDBACK_UNREACHABLE: &str =
    "Couldn't reach issues.guimadaleno.com, so nothing was sent. /feedback keeps what you typed.";

pub fn feedback_sent(reference: &str) -> String {
    format!("Sent, thank you! Its reference is {reference}.")
}

pub fn feedback_field(field: &str) -> String {
    format!(
        "The service won't take the {field} as it is. /feedback keeps what you typed, to fix it."
    )
}

pub fn feedback_later(seconds: u32) -> String {
    let when = match seconds {
        0..=89 => "a minute".to_string(),
        90..=5399 => format!("{} minutes", (seconds + 30) / 60),
        _ => format!("{} hours", (seconds + 1800) / 3600),
    };
    format!(
        "Not sent: too many reports right now. Try again in about {when}; /feedback keeps what you typed."
    )
}

pub const INFO_VERSION: &str = "version";
pub const INFO_MODEL: &str = "model";
pub const INFO_SERVER: &str = "server";
pub const INFO_IN_USE: &str = "in use";
pub const INFO_ABOUT_ME: &str = "about me";
pub const INFO_SETTINGS: &str = "settings";
pub const INFO_PROJECT: &str = "project";
pub const INFO_SCRATCH: &str = "scratch";
pub const INFO_SESSION: &str = "session";
pub const INFO_NOTES: &str = "notes";
pub const NO_NOTES: &str = "none yet (/memory starts a LOKAL.md)";
pub const NOTES_CHANGED: &str =
    "The project's notes changed, so the model gets the new ones from here.";
pub const NOTES_GONE: &str = "The project has no notes now.";

pub const RECON_HOST: &str = "host";
pub const RECON_TOOLS: &str = "tools";
pub const RECON_DOCKER: &str = "docker";
pub const RECON_PROJECT: &str = "project";
pub const RECON_SET_UP: &str = "configured";
pub const RECON_NONE: &str = "none of the usual ones";
pub const RECON_DOCKER_DOWN: &str = "installed, but its daemon doesn't answer";
pub const RECON_NO_BRANCH: &str = "no branch checked out";
pub const RECON_NOT_A_REPOSITORY: &str = "not a git repository";
pub const RECON_DONE: &str = "The model knows this from your next message on.";

/// "daemon 29.5.3 · 2 containers running"
pub fn recon_docker(version: &str, running: usize) -> String {
    match running {
        0 => format!("daemon {version} · no containers running"),
        1 => format!("daemon {version} · 1 container running"),
        running => format!("daemon {version} · {running} containers running"),
    }
}

/// The line at the start that says which notes go along, and their cost.
pub fn notes_taken(names: &[&str], tokens: u64, cut: bool) -> String {
    let cut = if cut {
        ", cut to fit; the model reads the rest when it needs it"
    } else {
        ""
    };
    format!(
        "The project's notes go along with every message: {} · {}{cut}",
        names.join(", "),
        about_tokens(tokens)
    )
}

/// One note in /info: "src/AGENTS.md · ~1k tokens".
pub fn note_size(name: &str, tokens: u64, cut: bool) -> String {
    match (tokens, cut) {
        (0, true) => format!("{name} · didn't fit"),
        (_, true) => format!("{name} · {}, cut to fit", about_tokens(tokens)),
        _ => format!("{name} · {}", about_tokens(tokens)),
    }
}

pub fn scratch_folder(path: Option<&str>) -> String {
    match path {
        Some(path) => format!("{path} · temporary files, deleted when Lokal quits"),
        None => "none · no private temporary folder could be made".into(),
    }
}

/// This session's name among the others, and the others open.
pub fn session_names(name: Option<&str>, others: &[String]) -> String {
    match (name, others) {
        (None, _) => "none · other sessions can't reach this one".into(),
        (Some(name), []) => format!("{name} · {NO_SESSIONS}"),
        (Some(name), others) => format!("{name} · others open: {}", others.join(", ")),
    }
}

pub fn project_approvals(folder: &str, count: usize) -> String {
    match count {
        0 => format!("{folder} · no commands approved here"),
        1 => format!("{folder} · 1 command approved here"),
        count => format!("{folder} · {count} commands approved here"),
    }
}
pub const FOUND_ON_ITS_OWN: &str = "found on its own";
pub const FROM_SETTINGS: &str = "from your settings";
pub const NOTHING_YET: &str = "nothing yet";
pub const THIS_SESSION: &str = "this session";
pub const UPDATE_FAILED: &str = "Couldn't check for updates right now.";

pub fn update_standing(standing: &crate::update::Standing) -> &'static str {
    use crate::update::Standing;
    match standing {
        Standing::Daily => "checks for updates once a day",
        Standing::SwitchedOff => "updates off in your settings",
        Standing::NotARelease => "a build that doesn't update itself",
        Standing::CantReplace => "can't update itself from where it runs",
    }
}

pub fn cant_update(standing: &crate::update::Standing) -> String {
    use crate::update::Standing;
    match standing {
        Standing::SwitchedOff => "Updates are off in your settings (updates = off).".into(),
        Standing::NotARelease => {
            "This build doesn't update itself: only one installed from a release does.".into()
        }
        _ => "This copy runs from somewhere Lokal can't write, so it can't update itself.".into(),
    }
}

pub fn ready_next_time(version: &str) -> String {
    format!("{version} is ready for next time.")
}

pub fn newest() -> String {
    format!("You have the newest version, {VERSION}.")
}

pub fn context_size(tokens: u64) -> String {
    format!("{} tokens of context", number(tokens))
}

pub fn small_context(context: u64, most: u64) -> String {
    format!(
        "The model is loaded with {} tokens of context, so long files and outputs get cut. It takes up to {}: raise Context Length in LM Studio, if memory allows.",
        number(context),
        number(most)
    )
}

pub fn tokens_in_use(tokens: u64, context: u64) -> String {
    format!(
        "{} tokens ({}%)",
        number(tokens),
        (tokens * 100 / context.max(1)).min(100)
    )
}

pub fn file_lines(path: &str, lines: usize) -> String {
    match lines {
        1 => format!("{path} · 1 line"),
        lines => format!("{path} · {lines} lines"),
    }
}

pub fn settings_file(path: &str, exists: bool) -> String {
    if exists {
        path.to_string()
    } else {
        format!("{path} · none yet (/config writes one)")
    }
}

/// 65,536
pub fn number(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// What the model is writing, while it writes a tool call. The size shows
/// once some of it has arrived.
pub fn writing_tool(name: &str, path: Option<&str>, bytes: usize) -> String {
    let size = if bytes == 0 {
        String::new()
    } else {
        format!(" · {}", size(bytes))
    };
    match (name, path) {
        ("write_file", Some(path)) => format!("writing {path}{size}"),
        ("edit_file", Some(path)) => format!("editing {path}{size}"),
        ("write_file" | "edit_file", None) => format!("writing a file{size}"),
        ("read_file", Some(path)) => format!("about to read {path}"),
        ("run_command", _) => "writing a command".into(),
        ("remember", _) => "writing a note".into(),
        _ => format!("preparing a step{size}"),
    }
}

/// "~41k of 65k tokens": how much of the context the reply has reached.
pub fn tokens_of(used: u64, context: u64) -> String {
    format!("~{} of {} tokens", short(used), short(context))
}

/// "24 tok/s": how fast the model generated on the last turn.
pub fn speed(tokens_per_second: u64) -> String {
    format!("{tokens_per_second} tok/s")
}

/// "~1k tokens"
fn about_tokens(tokens: u64) -> String {
    format!("~{} tokens", short(tokens))
}

/// 812, 41k
fn short(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else {
        format!("{}k", (n + 500) / 1000)
    }
}

/// 812 B, 14 KB, 2.3 MB
pub fn size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", (bytes + 512) / 1024)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
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
        "user.md is getting long ({lines} lines). It goes along with every conversation, so trimming it saves room. /prefs opens it."
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

    #[test]
    fn numbers_get_thousands_separators() {
        assert_eq!(number(7), "7");
        assert_eq!(number(1743), "1,743");
        assert_eq!(number(65536), "65,536");
        assert_eq!(number(1_000_000), "1,000,000");
    }

    #[test]
    fn sizes_read_like_a_person_says_them() {
        assert_eq!(size(812), "812 B");
        assert_eq!(size(14 * 1024 + 100), "14 KB");
        assert_eq!(size(2_411_724), "2.3 MB");
    }
}
