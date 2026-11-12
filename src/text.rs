//! Everything Lokal says to the person, in one place.

use std::time::Duration;

use crate::server::Kind;

/// The product's name, as the header and /about show it.
pub const NAME: &str = "Lokal";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// What `lokal --version` prints: the command's name, then the version.
pub const VERSION_LINE: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"));
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
pub const AUTHOR: &str = env!("CARGO_PKG_AUTHORS");
pub const WEBSITE: &str = env!("CARGO_PKG_HOMEPAGE");
pub const MADE_BY: &str = "Made by";

pub const USAGE: &str = "\
Usage:
  lokal              start a conversation
  lokal <request>    ask one thing, get the answer
  lokal -r           pick up the last conversation
  lokal -r <request> add to the last conversation
  lokal --install    install to ~/.local/bin, where it keeps itself up to date
  lokal --uninstall  remove it, and your files if you say so

Settings live in ~/.config/lokal/config (server = 192.0.2.10:1234, model = name, updates = off,
mail = ask).";

/// The slash commands, with what each does.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/new", "start fresh"),
    ("/compact", "sum up the conversation to free context"),
    ("/prefs", "edit what Lokal knows about you"),
    ("/memory", "edit the project's notes"),
    ("/config", "edit Lokal's settings"),
    ("/info", "show the model, server and context in use"),
    ("/recon", "tell the model about this machine and project"),
    ("/update", "check for updates"),
    ("/reasoning", "show or hide the model's reasoning"),
    ("/sysprompt", "show the system prompt"),
    ("/sessions", "list your other sessions"),
    ("/feedback", "report a problem or an idea"),
    ("/help", "show commands and keys"),
    ("/about", "who made Lokal"),
    ("/exit", "quit"),
];

pub const HELP_KEYS: &str = "\
Tab completes a command. Esc or Ctrl+C stops what's running; at the >,
Ctrl+C clears the line, and twice on an empty one quits.
Typing while Lokal works starts a message: Enter queues it, and the model
reads it after its current step. ↑ brings what's queued back to edit.
!command runs your own command; its output goes with your next message.
Shift+Tab cycles: asking for everything, accept edits (file changes in the
working folder go through), auto (the model, asked apart, approves the rest).
What can't be undone or leaves your machine always asks.
Mail from your other sessions shows above the >: Enter passes it on, Esc
drops it. @name mails a session.
Ctrl+V or Alt+V pastes an image as [Image #1], for a model that sees.
MCP servers (mcp.<name> in /config) offer their tools; each call asks.
lokal -r picks up the last conversation.";

pub const SETTINGS_TEMPLATE: &str = "\
# Lokal's settings. Each one is optional: remove the # to use it.

# The model server, when Lokal shouldn't look for it on its own:
# server = 192.0.2.10:1234

# The model, when it shouldn't take the one LM Studio has loaded, or on
# Ollama the first that uses tools, a loaded one before the rest:
# model = name-of-a-model

# The daily update check:
# updates = off

# Mail between your Lokal sessions on this computer. auto: it starts a turn
# on its own. ask: it waits for Enter. off: no mail.
# mail = auto

# How many turns in a row mail may start on its own before it waits for you:
# mail_turns = 64

# MCP servers, one line each: a name of your choice and the command that
# starts it. It starts in your home folder, and every call asks first.
# mcp.files = npx -y @modelcontextprotocol/server-filesystem /home/you/notes
# Only some of a server's tools, to save context:
# mcp.files.tools = read_text_file, list_directory
";
pub const SETTINGS_NEXT_START: &str = "Saved. It applies the next time Lokal starts.";

pub const FIRST_QUESTION: &str = "How do you like your answers? (Enter skips)";
pub const PREFS_HEADING: &str = "How I like answers:";
pub const NEW_CONVERSATION: &str = "Fresh start.";
pub const RESUMED: &str = "Picking up where we left off.";
pub const NOTHING_TO_RESUME: &str = "Nothing to pick up yet.";
pub const STOPPED: &str = "Stopped.";
pub const QUIT_AGAIN: &str = "Ctrl+C again to quit";

pub const RUN_QUESTION: &str = "Run it?";
pub const WRITE_QUESTION: &str = "Write it?";
pub const CALL_QUESTION: &str = "Call it?";
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
pub const NO_TERMINAL: &str = "Skipped: no terminal to ask on.";

/// The mode, at the right of the `>` and beside the spinner. Manual has
/// none: it's how Lokal always works.
pub fn mode_name(mode: crate::mode::Mode) -> Option<&'static str> {
    use crate::mode::Mode;
    match mode {
        Mode::Manual => None,
        Mode::AcceptEdits => Some("accept edits"),
        Mode::Auto => Some("auto"),
    }
}

/// After the mode at the right of the `>`, when there's room.
pub const MODE_KEYS: &str = "(Shift+Tab to cycle)";

/// Under a diff that went through without a question.
pub const WRITTEN_UNASKED: &str = "written: accept edits is on";

/// Once a session, as auto is chosen.
pub const AUTO_WARNING: &str = "Auto lets the model approve its own steps, and it'll get some \
    wrong: use it at your own risk. Deletes outside the folder, sudo, uploads and your settings \
    still ask.";
/// Beside the spinner while the judge decides.
pub const JUDGING: &str = "auto · asking the model apart";
/// Under a file change auto made.
pub fn judge_wrote(reason: &str) -> String {
    if reason.is_empty() {
        "auto wrote it".into()
    } else {
        format!("auto wrote it: {reason}")
    }
}
/// Under a command auto ran.
pub fn judge_ran(reason: &str) -> String {
    if reason.is_empty() {
        "auto ran it".into()
    } else {
        format!("auto ran it: {reason}")
    }
}
/// Over the question, when the judge said no.
pub fn judge_asks(reason: &str) -> String {
    if reason.is_empty() {
        "Auto asks you instead.".into()
    } else {
        format!("Auto asks you instead: {reason}")
    }
}
pub const JUDGE_UNREAD: &str = "Auto asks you instead: the model's verdict couldn't be read.";

/// Under a `!command` that couldn't start.
pub fn cant_start_bash(error: &str) -> String {
    format!("Couldn't start bash: {error}")
}
pub const GIT_SETTINGS_UNREAD: &str =
    "Couldn't read this repository's git settings, so Lokal asks.";
pub const GIT_ELSEWHERE: &str =
    "git may run in another repository or with other settings here, so Lokal asks.";

/// From `lokal --box`, when the box couldn't be set up and nothing ran.
pub fn box_unavailable(step: &str) -> String {
    format!("Lokal's box couldn't be set up ({step}), so nothing ran.")
}

/// In auto, before a git command the judge would otherwise see.
pub fn git_unusual_setting(setting: &str) -> String {
    format!("This repository sets {setting}, which may make git run a program, so Lokal asks.")
}

pub fn git_runs_program(setting: &str) -> String {
    format!("This repository's git settings name a program to run ({setting}), so Lokal asks.")
}

/// In auto, before a git command the judge would otherwise see.
pub fn git_runs_hook(hook: &str) -> String {
    format!("This repository has a git hook ({hook}), so Lokal asks.")
}

pub const STEP_READ: &str = "read";

/// After the path of an image the model looks at.
pub fn image_size(width: u32, height: u32) -> String {
    format!("{width}×{height}")
}
pub const STEP_CREATE: &str = "create";
pub const STEP_EDIT: &str = "edit";
pub const STEP_REMEMBER: &str = "remember";
pub const STEP_SESSIONS: &str = "sessions";
pub const ALREADY_REMEMBERED: &str = "Already there.";
pub const NO_SESSIONS: &str = "no others open";
pub const WORKING: &str = "working";
pub const SESSION_ASKING: &str = "needs you";
pub const SESSION_IDLE: &str = "idle";
pub const MAIL_KEYS: &str = "Enter passes it on, Esc drops it.";
pub const MAIL_TAKEN_UP: &str = "Taken up on its own; Esc stops it.";
pub const MAIL_LATER: &str = "Goes with the next turn.";
pub const MAIL_AFTER_YOU: &str = "Waits until you're done; Enter on an empty line passes it on.";
pub const MAIL_HELD: &str = "On hold since you stopped the last mail turn; Enter passes it on.";

/// Above a mail: "api-1:".
pub fn mail_from(name: &str) -> String {
    format!("{name}:")
}

/// Above a mail the person wrote at another session's `>`.
pub const MAIL_FROM_YOU: &str = "You:";

/// Under a mail that waits since mail started as many turns as it may.
pub fn mail_limit(turns: usize) -> String {
    format!(
        "Mail started {turns} turns in a row, the limit, so this one waits; Enter passes it on."
    )
}

/// At the right of the `>`, while mail waits.
pub fn mail_waiting(count: usize) -> String {
    match count {
        1 => "1 mail".into(),
        count => format!("{count} mails"),
    }
}

/// After what's queued, under the spinner.
pub const EDIT_QUEUED: &str = "↑ to edit";

/// Under the spinner, beside what Enter queued while Lokal works.
pub fn queued(count: usize) -> String {
    match count {
        1 => "queued".into(),
        count => format!("{count} queued"),
    }
}

/// Esc on an empty line, for mail on screen: otherwise nothing would say
/// it went.
pub const DROPPED: &str = "Dropped.";

/// After `@name …` at the `>`, when it didn't go: otherwise the person
/// would think it had.
pub fn mail_full(to: &str) -> String {
    format!("Not sent: {to} is full.")
}

pub fn mail_too_long(limit: usize) -> String {
    format!("Not sent: over {} characters.", number(limit as u64))
}

/// Under a session in /sessions: "last asked "fold the menu" · changed
/// src/menu.rs".
pub fn session_doing(request: &str, files: &[String]) -> String {
    let mut parts = Vec::new();
    if !request.is_empty() {
        parts.push(format!("last asked \"{request}\""));
    }
    if !files.is_empty() {
        parts.push(format!("changed {}", files.join(", ")));
    }
    parts.join(" · ")
}

/// /sessions with no other session open.
pub fn no_other_sessions(name: &str) -> String {
    format!("No other sessions open. This one is {name}.")
}

/// /sessions where this session doesn't listen.
pub fn sessions_unreachable(mail_off: bool) -> &'static str {
    if mail_off {
        "Mail is off, so sessions don't see each other."
    } else {
        "Other sessions can't reach this one: no private folder for it."
    }
}

pub const NO_HTTPS: &str = "`server` can't be https://: Lokal speaks plain HTTP to it.";
pub const CONNECTION_DROPPED: &str = "The server hung up mid-answer.";
pub const NO_MODEL_LOADED: &str =
    "No model loaded in LM Studio. Load one, or set `model` in ~/.config/lokal/config.";
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
    format!("Already installed, at {path}.")
}

pub fn cant_install(error: &str) -> String {
    format!("Couldn't install: {error}")
}

pub fn uninstalled(path: &str) -> String {
    format!("Removed {path}.")
}

pub fn not_installed(path: &str) -> String {
    format!("No Lokal at {path}.")
}

pub fn cant_uninstall(error: &str) -> String {
    format!("Couldn't remove Lokal: {error}")
}

pub const YOUR_FILES: &str = "Lokal kept these for you:";
pub const REMOVE_FILES_QUESTION: &str = "Remove them too?";
pub const KEEP_THEM: &str = "Keep them";
pub const REMOVE_THEM: &str = "Remove them";
pub const REMOVED_FILES: &str = "Removed them.";
pub const NOT_ASKED: &str = "No terminal to ask on, so they stay.";

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
pub const NOTHING_TO_COMPACT: &str = "Nothing to compact yet.";
pub const NOTHING_CHANGED: &str = "Nothing changed.";
pub const NO_SUMMARY: &str = "The model wrote no summary. Nothing changed.";

/// Beside the spinner while `/compact` waits: "compacting · 12s · 240 tokens".
pub fn compacting(time: Duration, tokens: u64) -> String {
    match tokens {
        0 => format!("compacting · {}", duration(time)),
        tokens => format!("compacting · {} · {tokens} tokens", duration(time)),
    }
}

/// After `/compact`: "Compacted: ~41k → ~3k tokens."
pub fn compacted(before: u64, after: u64) -> String {
    format!("Compacted: ~{} → ~{} tokens.", short(before), short(after))
}
pub const ASKING_AGAIN: &str = "Out of room while reasoning. Asking again, briefer.";
pub const WITHOUT_THE_IMAGE: &str = "Asking again without the image.";
pub const NO_ANSWER: &str = "No answer from the model. Ask again, or /new.";
pub const RAN_OUT: &str =
    "Out of room before an answer, likely thinking in circles. /new starts fresh.";
pub const CUT_SHORT: &str = "Out of room mid-step, so it didn't run. Asking again, briefer.";
pub const CUT_SHORT_AGAIN: &str = "Out of room mid-step again, so it didn't run. /new starts fresh, \
    or load the model with more context.";

pub fn ran_out_of_room(context: u64) -> String {
    format!(
        "The model used its whole context ({} tokens) before answering, likely thinking in circles. /new starts fresh, or load it with more context.",
        number(context)
    )
}
pub const REASONING_SHOWN: &str = "Reasoning on for this session.";
pub const REASONING_HIDDEN: &str = "Reasoning off.";

pub const FEEDBACK_KIND: &str = "What would you like to tell the developer?";
pub const FEEDBACK_BUG: &str = "Something doesn't work";
pub const FEEDBACK_IDEA: &str = "Something could be better";
pub const NEVER_MIND: &str = "Never mind";
pub const FEEDBACK_TITLE: &str = "  Title: ";
pub const FEEDBACK_WHAT_HAPPENED: &str = "  What happened? ";
pub const FEEDBACK_THE_IDEA: &str = "  The idea: ";
pub const FEEDBACK_CONTACT: &str = "  Where to reply? (optional) ";
pub const FEEDBACK_PREVIEW: &str = "Everything that will be sent:";
pub const FEEDBACK_SEND_QUESTION: &str = "Send it?";
pub const FEEDBACK_SEND: &str = "Send";
pub const FEEDBACK_SEND_LESS: &str = "Send without the system and the details";
pub const FEEDBACK_DONT_SEND: &str = "Don't send";
pub const FEEDBACK_KEPT: &str = "Not sent. /feedback keeps what you typed.";
pub const FEEDBACK_REFUSED: &str = "Couldn't send it: Lokal's fault or the service's, not yours.";
pub const FEEDBACK_UNREACHABLE: &str =
    "Couldn't reach issues.guimadaleno.com; nothing sent. /feedback keeps what you typed.";

pub fn feedback_sent(reference: &str) -> String {
    format!("Sent, thanks! Reference: {reference}.")
}

pub fn feedback_field(field: &str) -> String {
    format!("The service won't take the {field} as is. /feedback keeps what you typed.")
}

pub fn feedback_later(seconds: u32) -> String {
    let when = match seconds {
        0..=89 => "a minute".to_string(),
        90..=5399 => format!("{} minutes", (seconds + 30) / 60),
        _ => format!("{} hours", (seconds + 1800) / 3600),
    };
    format!(
        "Not sent: too many reports. Try again in about {when}; /feedback keeps what you typed."
    )
}

pub const INFO_VERSION: &str = "version";
pub const INFO_MODEL: &str = "model";
pub const INFO_SERVER: &str = "server";
pub const INFO_IN_USE: &str = "in use";
pub const INFO_USER_MD: &str = "user.md";
pub const INFO_SETTINGS: &str = "settings";
pub const INFO_PROJECT: &str = "project";
pub const INFO_SCRATCH: &str = "scratch";
pub const INFO_MCP: &str = "mcp";
pub const INFO_SESSION: &str = "session";
pub const INFO_NOTES: &str = "notes";
pub const NO_NOTES: &str = "none yet (/memory starts a LOKAL.md)";
pub const NOTES_CHANGED: &str = "Project notes changed; the model has the new ones.";
pub const NOTES_GONE: &str = "No project notes now.";

pub const RECON_HOST: &str = "host";
pub const RECON_TOOLS: &str = "tools";
pub const RECON_DOCKER: &str = "docker";
pub const RECON_PROJECT: &str = "project";
pub const RECON_SET_UP: &str = "configured";
pub const RECON_NONE: &str = "none of the usual ones";
pub const RECON_DOCKER_DOWN: &str = "installed, but its daemon doesn't answer";
pub const RECON_NO_BRANCH: &str = "no branch checked out";
pub const RECON_NOT_A_REPOSITORY: &str = "not a git repository";
pub const RECON_DONE: &str = "The model knows this from now on.";

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
    let cut = if cut { ", cut to fit" } else { "" };
    format!(
        "Project notes, sent with every message: {} · {}{cut}",
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
        Some(path) => format!("{path} · temporary, gone when Lokal quits"),
        None => "none · couldn't make a private one".into(),
    }
}

/// This session's name among the others, and the others open.
pub fn session_names(
    name: Option<&str>,
    mail: crate::config::Mail,
    turns: usize,
    others: &[String],
) -> String {
    use crate::config::Mail;
    let Some(name) = name else {
        return match mail {
            Mail::Off => "none · mail is off in your settings".into(),
            _ => "none · other sessions can't reach this one".into(),
        };
    };
    let mail = match mail {
        Mail::Ask => "mail waits for you".to_string(),
        _ => format!("mail starts up to {turns} turns on its own"),
    };
    match others {
        [] => format!("{name} · {mail} · {NO_SESSIONS}"),
        others => format!("{name} · {mail} · others open: {}", others.join(", ")),
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
        Standing::NotARelease => "This build doesn't update itself; a release does.".into(),
        _ => "Can't update: Lokal can't write where this copy runs.".into(),
    }
}

pub fn ready_next_time(version: &str) -> String {
    format!("{version} is ready for next time.")
}

pub fn newest() -> String {
    format!("Up to date: {VERSION}.")
}

pub fn context_size(tokens: u64) -> String {
    format!("{} tokens of context", number(tokens))
}

pub fn small_context(context: u64, most: u64, kind: Kind) -> String {
    let raise = match kind {
        Kind::Ollama => {
            "raise OLLAMA_CONTEXT_LENGTH where Ollama runs, or num_ctx in the model's Modelfile"
        }
        Kind::LmStudio | Kind::Other => "raise Context Length in LM Studio",
    };
    format!(
        "Loaded with {} tokens of context, so long files get cut. The model takes up to {}: {raise}, if memory allows.",
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

/// /info's user.md row, when the file isn't UTF-8.
pub fn unreadable_file(path: &str) -> String {
    format!("{path} · not UTF-8 text, so the model doesn't get it")
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

/// Under /sysprompt: what it and the tools cost, on every request.
pub fn sysprompt_cost(tokens: u64, tools: &[String]) -> String {
    format!(
        "That, with the tools ({}), is {} on every request.",
        tools.join(", "),
        about_tokens(tokens)
    )
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
    if count == 1 {
        "… +1 line".to_string()
    } else {
        format!("… +{count} lines")
    }
}

pub fn unknown_option(option: &str) -> String {
    format!("There's no {option} option. Try lokal --help.")
}

pub fn unknown_command(command: &str) -> String {
    format!("There's no {command}. Try /help.")
}

pub fn saved_prefs(path: &str) -> String {
    format!("Saved to {path}. /prefs changes it.")
}

pub fn long_prefs(lines: usize) -> String {
    format!("user.md is long ({lines} lines) and goes with every conversation. /prefs trims it.")
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
        "{path}, line {line}: there's no setting called `{name}`. The ones that exist are `server`, `model`, `updates`, `mail`, `mail_turns` and `mcp.<name>`."
    )
}

pub const MCP_SEEN_HEADER: &str = "# The MCP tool lists you've seen, a hash a server. Lokal writes this file:\n# a list that changes is shown again before the model gets it.\n";
pub const CLIPBOARD_NO_IMAGE: &str = "No image on the clipboard.";
pub const CLIPBOARD_NO_READER: &str = "Nothing here can read the clipboard's images.";
pub const CLIPBOARD_TOO_BIG: &str = "The clipboard's image is over 20 MB.";
pub const CLIPBOARD_SLOW: &str = "The clipboard took too long to read.";
pub const PASTE_BLIND: &str = "This model doesn't take images.";
pub const PASTE_UNSAID: &str = "This server doesn't say whether its model takes images.";
/// In the conversation, where a pasted image was once the model replied after it.
pub const PASTED_DROPPED: &str = "[A pasted image, dropped to free up context after you saw it.]";

/// To the model, in the request, for a pasted image the context had no room for.
pub fn pasted_left_out(image: usize) -> String {
    format!("[Image #{image} isn't attached: the context had no room for it, so you can't see it.]")
}
/// In the saved conversation, where a pasted image was.
pub const PASTED_NOT_KEPT: &str = "[A pasted image, not kept in a saved conversation.]";

pub fn clipboard_failed(why: &str) -> String {
    format!("Couldn't read the clipboard: {why}.")
}

pub fn pasted_no_room(image: usize, tokens: u64, room: u64) -> String {
    format!(
        "Image #{image} left out: it takes ~{} tokens, and there's room for ~{}.",
        number(tokens),
        number(room)
    )
}

pub fn image_marker(number: usize) -> String {
    format!("[Image #{number}]")
}

pub const MCP_NO_PROGRAM: &str = "there's no command to start it.";
pub const MCP_STARTING: &str = "starting the MCP servers";
pub const MCP_FAILED: &str = "it failed as it started.";
pub const MCP_COULDNT_START: &str = "couldn't start";
pub const MCP_NO_TOOLS: &str = "no tools";
pub const MCP_DECLINED: &str = "you said no to its tools";
pub const OFFER_QUESTION: &str = "Offer them to the model?";

pub fn mcp_couldnt_start(name: &str, reason: &str) -> String {
    format!("MCP server {name} didn't start: {reason}")
}

pub fn mcp_no_tools(name: &str) -> String {
    format!("MCP server {name} has no tools Lokal can offer.")
}

pub fn mcp_offers(name: &str, tools: usize, tokens: u64) -> String {
    let count = if tools == 1 {
        "1 tool".to_string()
    } else {
        format!("{tools} tools")
    };
    format!(
        "MCP server {name} offers {count}, ~{} tokens a request. All it tells the model:",
        number(tokens)
    )
}

pub fn mcp_offered(name: &str, tools: usize, tokens: u64) -> String {
    let count = if tools == 1 {
        "1 tool".to_string()
    } else {
        format!("{tools} tools")
    };
    format!("{name} · {count}, ~{} tokens a request", number(tokens))
}

pub fn mcp_starts_later(name: &str) -> String {
    format!("{name} · starting; its tools are shown with the first request")
}

pub fn mcp_no_such_tool(server: &str, name: &str) -> String {
    format!("The MCP server {server} has no tool called {name}, which mcp.{server}.tools names.")
}

pub const MCP_WANTS_INPUT: &str =
    "The tool asked for more from the user, which Lokal can't give it in this version.";
pub const MCP_IMAGE_UNREAD: &str =
    "[An image Lokal can't read, left out: only PNG, JPEG and WebP go to the model.]";
pub const MCP_IMAGE_BLIND: &str = "[An image, left out: the model loaded doesn't take images.]";
pub const MCP_IMAGE_UNSAID: &str =
    "[An image, left out: this server doesn't say whether its model takes images.]";
pub const MCP_AUDIO: &str = "[Audio, left out: Lokal gives the model none.]";
pub const MCP_TOOL_FAILED: &str = "The tool failed:";
pub const MCP_ARGUMENTS_NOT_OBJECT: &str =
    "the arguments must be a JSON object, as MCP requires. Nothing was called.";

pub fn mcp_image_too_big(width: u32, height: u32) -> String {
    format!("[An image of {width}×{height}, left out: it's over 20 MB.]")
}

pub fn mcp_binary(uri: &str) -> String {
    format!("[{uri}: binary data, left out.]")
}

pub fn mcp_link(name: &str, uri: &str) -> String {
    format!("[A link: {name} {uri}]")
}

pub fn mcp_unknown_part(kind: &str) -> String {
    format!("[A part of a kind Lokal doesn't know ({kind}), left out.]")
}

pub fn mcp_image_no_room(width: u32, height: u32, tokens: u64) -> String {
    format!(
        "[An image of {width}×{height}, left out: it would take about {tokens} tokens, more than the context has room for.]"
    )
}

pub fn mcp_image_shown(size: &str) -> String {
    format!("an image, {size}")
}

pub fn mcp_same_name(server: &str, name: &str) -> String {
    format!(
        "The MCP server {server}'s tool {name} is left out: another of its tools has that name already."
    )
}
pub const MCP_ENDED: &str = "the server ended.";

pub fn mcp_relative_program(program: &str) -> String {
    format!("`{program}` is a relative path: use an absolute one, or a program the PATH finds.")
}

pub fn mcp_cant_start(program: &str, error: &str) -> String {
    format!("couldn't start `{program}`: {error}.")
}

pub fn mcp_unknown_version(version: &str) -> String {
    format!("it speaks a version of MCP Lokal doesn't: {version}.")
}

pub fn mcp_no_answer(method: &str) -> String {
    format!("it didn't answer `{method}` in time.")
}

pub fn mcp_refused(message: &str) -> String {
    format!("the server said: {message}")
}

/// What went wrong with a server on its first line, then the last lines it
/// wrote to stderr.
pub fn mcp_with_stderr(message: &str, stderr: &[String]) -> String {
    let mut lines = vec![message.to_string()];
    lines.extend(stderr.iter().cloned());
    lines.join("\n")
}

pub fn bad_mcp_setting(path: &str, line: usize, error: &str) -> String {
    format!("{path}, line {line}: {error}")
}

pub fn bad_mcp_name(name: &str, most: usize) -> String {
    format!(
        "`{name}` can't name an MCP server: use lowercase letters, digits and dashes, at most {most}, as in `mcp.github = …`."
    )
}

pub fn mcp_set_twice(name: &str) -> String {
    format!("`mcp.{name}` is set twice.")
}

pub fn mcp_no_command(name: &str) -> String {
    format!(
        "`mcp.{name}` needs the command that starts the server, as in `mcp.{name} = npx -y <package>`."
    )
}

pub fn unknown_mcp_field(name: &str, field: &str) -> String {
    format!(
        "`mcp.{name}.{field}` isn't a setting: an MCP server takes `mcp.{name}` and `mcp.{name}.tools`."
    )
}

pub fn mcp_tools_without_server(path: &str, name: &str) -> String {
    format!("{path}: `mcp.{name}.tools` is set, but not `mcp.{name}`, the command that starts it.")
}

pub fn bad_mail_setting(path: &str, line: usize) -> String {
    format!("{path}, line {line}: `mail` is auto, ask or off.")
}

pub fn bad_mail_turns_setting(path: &str, line: usize) -> String {
    format!(
        "{path}, line {line}: `mail_turns` is a number: how many turns in a row mail may start on its own."
    )
}

pub fn no_server(tried: &[String]) -> String {
    let tried = match tried.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    };
    format!(
        "No model server found (tried {tried}). Is Ollama or LM Studio running? Or set `server` in ~/.config/lokal/config."
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
    format!("The server at {address} didn't send the JSON Lokal expects.")
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
    fn the_addresses_tried_read_as_a_list() {
        let tried = |n: usize| -> Vec<String> { (1..=n).map(|i| format!("h{i}:1")).collect() };
        assert!(no_server(&tried(1)).contains("(tried h1:1)"));
        assert!(no_server(&tried(2)).contains("(tried h1:1 and h2:1)"));
        assert!(no_server(&tried(4)).contains("(tried h1:1, h2:1, h3:1 and h4:1)"));
    }

    #[test]
    fn numbers_get_thousands_separators() {
        assert_eq!(number(7), "7");
        assert_eq!(number(1743), "1,743");
        assert_eq!(number(65536), "65,536");
        assert_eq!(number(1_000_000), "1,000,000");
    }

    #[test]
    fn a_session_shows_what_it_was_asked_and_what_it_changed() {
        assert_eq!(
            session_doing("fold it", &["a.rs".into(), "b.rs".into()]),
            "last asked \"fold it\" · changed a.rs, b.rs"
        );
        assert_eq!(session_doing("", &[]), "");
    }

    #[test]
    fn sizes_read_like_a_person_says_them() {
        assert_eq!(size(812), "812 B");
        assert_eq!(size(14 * 1024 + 100), "14 KB");
        assert_eq!(size(2_411_724), "2.3 MB");
    }
}
