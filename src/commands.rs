//! The slash commands, and what they print: the session's facts, the
//! other sessions, the help, and the files they open in the editor.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::context;
use crate::conversation::{Compacted, Conversation};
use crate::input::Input;
use crate::{
    config, feedback, image, layout, memory, mode, peers, recon, server, spinner, style, system,
    text, tty, update,
};

/// Past this many lines, user.md costs noticeable context on every turn.
const LONG_USER_MD: usize = 30;

pub fn print_header(server: &server::Server) {
    eprintln!(
        "{} {}  {}",
        style::bold(text::NAME),
        text::VERSION,
        style::gray(&server.model)
    );
}

/// What the slash commands need to know about the running session.
pub struct Session {
    pub started: Instant,
    pub updates_off: bool,
    pub mail: config::Mail,
    pub mail_turns: usize,
    /// What /feedback was given and couldn't send yet.
    pub feedback_draft: Option<feedback::Draft>,
}

/// `/prefs` is a command; `/etc/hosts is broken` is a request.
pub fn is_command(line: &str) -> bool {
    argument(line, "/compact").is_some()
        || argument(line, "/context").is_some()
        || (line.starts_with('/') && !line.contains(' ') && !line[1..].contains('/'))
}

/// What follows `command` on the line, for the commands that take a word
/// or more: `/compact` what to focus on, `/context raw`.
fn argument<'a>(line: &'a str, command: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(command)?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

/// Runs a slash command. False when it's time to quit.
pub fn run_command(
    command: &str,
    conversation: &mut Conversation,
    input: &mut Input,
    session: &mut Session,
) -> bool {
    if let Some(focus) = argument(command, "/compact") {
        compact(conversation, focus);
        return true;
    }
    if let Some(what) = argument(command, "/context") {
        eprintln!();
        if what == "raw" {
            print_raw_request(conversation);
        } else {
            print_context(conversation);
        }
        eprintln!();
        return true;
    }
    match command {
        "/new" => {
            conversation.clear();
            // A fresh start looks like one: the screen cleared, the header again.
            // What was on screen stays in the scrollback.
            if style::enabled() {
                eprint!("\x1b[2J\x1b[H");
            }
            print_header(&conversation.server);
            eprintln!("{}", style::gray(text::NEW_CONVERSATION));
        }
        "/prefs" => {
            let path = config::user_md_path();
            if !path.exists() {
                let _ = config::write_user_md("");
            }
            open_in_editor(&[&path]);
            warn_if_user_md_is_long();
            conversation.refresh_system_prompt();
        }
        "/memory" => {
            // The notes there are, or a first LOKAL.md, made only if it's saved.
            let mut paths: Vec<PathBuf> = conversation
                .notes()
                .iter()
                .map(|note| note.path.clone())
                .collect();
            if paths.is_empty() {
                paths.push(memory::first_path(&env::current_dir().unwrap_or_default()));
            }
            let paths: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
            open_in_editor(&paths);
            if conversation.refresh_notes() {
                if conversation.notes().is_empty() {
                    eprintln!("{}", style::gray(text::NOTES_GONE));
                } else {
                    print_notes(conversation);
                }
            }
        }
        "/config" => edit_settings(),
        "/sysprompt" => {
            // Plain and whole, just as the model gets it.
            let (prompt, tools, tokens) = conversation.system_prompt();
            eprintln!("\n{}\n", prompt.trim_end());
            eprintln!("{}\n", style::gray(&text::sysprompt_cost(tokens, &tools)));
        }
        "/sessions" => {
            eprintln!();
            print_sessions(conversation, session);
            eprintln!();
        }
        "/info" => {
            eprintln!();
            print_info(conversation, input, session);
            eprintln!();
        }
        "/recon" => {
            let rows = gather_recon();
            eprintln!();
            print_rows(&rows);
            eprintln!();
            eprintln!("{}", style::gray(text::RECON_DONE));
            conversation.set_recon(rows);
        }
        "/update" => check_for_update(session),
        "/reasoning" => {
            let shown = conversation.toggle_reasoning();
            let message = if shown {
                text::REASONING_SHOWN
            } else {
                text::REASONING_HIDDEN
            };
            eprintln!("{}", style::gray(message));
        }
        "/feedback" => {
            eprintln!();
            let draft = session.feedback_draft.take();
            session.feedback_draft =
                feedback::run(input, &conversation.server, session.updates_off, draft);
            eprintln!();
        }
        "/help" => print_help(),
        "/about" => {
            eprintln!();
            print_about();
            eprintln!();
        }
        "/exit" | "/quit" => return false,
        other => eprintln!("{}", text::unknown_command(other)),
    }
    true
}

fn print_about() {
    eprintln!(
        "  {} {}  {}",
        style::bold(text::NAME),
        text::VERSION,
        style::gray(text::DESCRIPTION)
    );
    eprintln!("  {} {}", style::gray(text::MADE_BY), text::AUTHOR);
    eprintln!("  {}", text::WEBSITE);
}

/// Everything about this session, for when something seems off.
fn print_info(conversation: &Conversation, input: &Input, session: &Session) {
    let server = &conversation.server;
    let updates = text::update_standing(&update::standing_of(session.updates_off));
    let context = server.context.map_or_else(String::new, |tokens| {
        format!(" · {}", text::context_size(tokens))
    });
    let found = if server.pinned {
        text::FROM_SETTINGS
    } else {
        text::FOUND_ON_ITS_OWN
    };
    let in_use = match (conversation.tokens(), server.context) {
        (Some(tokens), Some(context)) => text::tokens_in_use(tokens, context),
        _ => text::NOTHING_YET.to_string(),
    };
    let session_time = text::duration(session.started.elapsed());
    let user_md = config::shown(&config::user_md_path());
    // One that doesn't read, saved in another encoding, never reaches the model.
    let user_md = match config::read_user_md() {
        Some(contents) => text::file_lines(&user_md, contents.lines().count()),
        None if config::user_md_path().exists() => text::unreadable_file(&user_md),
        None => text::file_lines(&user_md, 0),
    };
    let settings = config::settings_path();
    let mut rows = vec![
        (text::INFO_VERSION, format!("{} · {updates}", text::VERSION)),
        (text::INFO_MODEL, format!("{}{context}", server.model)),
        (text::INFO_SERVER, format!("{} · {found}", server.address)),
        (
            text::INFO_IN_USE,
            format!("{in_use} · {} {session_time}", text::THIS_SESSION),
        ),
        (text::INFO_USER_MD, user_md),
        (
            text::INFO_SETTINGS,
            text::settings_file(&config::shown(&settings), settings.exists()),
        ),
        (
            text::INFO_PROJECT,
            text::project_approvals(
                &config::shown(input.project().project()),
                input.project().count(),
            ),
        ),
    ];
    // One row a note, the label on the first.
    let notes = conversation.notes();
    if notes.is_empty() {
        rows.push((text::INFO_NOTES, text::NO_NOTES.to_string()));
    }
    for (index, note) in notes.iter().enumerate() {
        let label = if index == 0 { text::INFO_NOTES } else { "" };
        let tokens = context::tokens_in(&note.text);
        rows.push((label, text::note_size(&note.name, tokens, note.cut)));
    }
    if let Some(mcp) = conversation.mcp() {
        for (index, server) in mcp.summary().into_iter().enumerate() {
            let label = if index == 0 { text::INFO_MCP } else { "" };
            rows.push((label, server));
        }
    }
    rows.push((
        text::INFO_SCRATCH,
        text::scratch_folder(conversation.scratch()),
    ));
    let peers = conversation.peers();
    let others: Vec<String> = peers
        .map(|peers| {
            peers
                .others()
                .into_iter()
                .map(|session| session.name)
                .collect()
        })
        .unwrap_or_default();
    rows.push((
        text::INFO_SESSION,
        text::session_names(
            peers.map(peers::Peers::name),
            session.mail,
            session.mail_turns,
            &others,
        ),
    ));
    print_rows(&rows);
}

/// The other sessions open, each with its folder, branch, what it's doing
/// and what it was asked: the facts the model hears of the ones working
/// where it does, without them showing then.
fn print_sessions(conversation: &Conversation, session: &Session) {
    let Some(peers) = conversation.peers() else {
        let off = session.mail == config::Mail::Off;
        eprintln!("  {}", style::gray(text::sessions_unreachable(off)));
        return;
    };
    let sessions = peers.others();
    if sessions.is_empty() {
        eprintln!("  {}", style::gray(&text::no_other_sessions(peers.name())));
        return;
    }
    for row in peers::session_rows(&sessions, true) {
        eprintln!("{row}");
    }
}

/// Labels in one column, values in the next. A value too long for the
/// terminal goes on under itself, broken after a separator, so it never
/// runs under the labels.
fn print_rows(rows: &[(&str, String)]) {
    // The indent and the label's column.
    let room = tty::width().map_or(0, |width| width.saturating_sub(13));
    for (label, value) in rows {
        for (index, line) in layout::wrapped(value, room).iter().enumerate() {
            let label = if index == 0 { *label } else { "" };
            eprintln!("  {}{line}", style::gray(&format!("{label:<10}")));
        }
    }
}

/// `/help`: the commands in alphabetical order, then the keys, what's
/// typed, the modes and the rest, all in one pair of columns: the name,
/// then what it does in gray. The modes show in their own colors, as at
/// the `>`.
fn print_help() {
    let mut commands = text::COMMANDS.to_vec();
    commands.sort_by_key(|(name, _)| *name);
    let blocks = [
        text::HELP_KEYS,
        text::HELP_TYPED,
        text::HELP_MODES,
        text::HELP_MORE,
    ];
    // The longest name and two spaces, so every description lines up.
    let width = commands
        .iter()
        .chain(blocks.iter().flat_map(|block| block.iter()))
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let row = |name: String, shown: usize, about: &str| {
        let pad = " ".repeat(width.saturating_sub(shown));
        eprintln!("  {name}{pad}{}", style::gray(about));
    };
    eprintln!();
    for (name, about) in &commands {
        row(name.to_string(), name.chars().count(), about);
    }
    for block in blocks {
        eprintln!();
        let modes = std::ptr::eq(block, text::HELP_MODES);
        for (index, (name, about)) in block.iter().enumerate() {
            let shown = if modes {
                let mode = [
                    mode::Mode::Manual,
                    mode::Mode::AcceptEdits,
                    mode::Mode::Auto,
                ][index];
                style::mode(name, mode)
            } else {
                name.to_string()
            };
            row(shown, name.chars().count(), about);
        }
        if modes {
            eprintln!("  {}", style::gray(text::HELP_ALWAYS_ASKS));
        }
    }
    eprintln!();
}

/// `/context`: what fills the context, part by part, in estimated tokens,
/// with each part's share; then what's free and what's kept for replies.
fn print_context(conversation: &Conversation) {
    let used = conversation.context_use();
    let context = conversation.server.context;
    let prompt: u64 = used.prompt.iter().map(|(_, tokens)| tokens).sum();
    let tools: u64 = used.tools.iter().map(|(_, tokens)| tokens).sum();
    let total = prompt + tools + used.conversation;
    let share = |tokens: u64| {
        context.map_or_else(String::new, |context| {
            format!("{:>4}%", (tokens * 100).div_ceil(context.max(1)))
        })
    };
    let row = |label: &str, tokens: u64, indent: bool| {
        let (label, tokens) = if indent {
            (
                format!("  {label}"),
                style::gray(&format!("{:>7}", text::about(tokens))),
            )
        } else {
            (
                label.to_string(),
                format!("{:>7}{}", text::about(tokens), style::gray(&share(tokens))),
            )
        };
        eprintln!("  {}{tokens}", style::gray(&format!("{label:<18}")));
    };
    eprintln!(
        "  {}\n",
        text::context_total(&conversation.server.model, total, context)
    );
    row(text::CONTEXT_PROMPT, prompt, false);
    for (name, tokens) in &used.prompt {
        row(name, *tokens, true);
    }
    row(text::CONTEXT_TOOLS, tools, false);
    for (name, tokens) in &used.tools {
        row(&layout::printable(name), *tokens, true);
    }
    row(text::CONTEXT_CONVERSATION, used.conversation, false);
    if used.outputs > 0 {
        row(text::CONTEXT_OUTPUTS, used.outputs, true);
    }
    if let Some(context) = context {
        // A quarter stays for the model's reasoning and reply (`context::budget`).
        let kept = context / 4;
        row(
            text::CONTEXT_FREE,
            context.saturating_sub(total + kept),
            false,
        );
        row(text::CONTEXT_REPLY, kept, false);
    }
    eprintln!();
    eprintln!(
        "{}",
        style::gray(&text::context_estimated(conversation.tokens()))
    );
    eprintln!("{}", style::gray(text::CONTEXT_RAW_HINT));
}

/// `/context raw`: the next request as the server would get it, each
/// message under its role, its text as the model gets it. Control
/// characters show as marks, and an image as its size, not its data.
fn print_raw_request(conversation: &Conversation) {
    let mut request = conversation.next_request();
    let messages = request["messages"].take();
    let tools = request["tools"].take();
    if let Some(fields) = request.as_object_mut() {
        fields.retain(|_, value| !value.is_null());
    }
    eprintln!("{}", style::faint(&request.to_string()));
    for message in messages.as_array().into_iter().flatten() {
        let role = message["role"].as_str().unwrap_or("?");
        eprintln!("\n{}", style::faint(&format!("── {role} ──")));
        match &message["content"] {
            serde_json::Value::String(content) if !content.is_empty() => {
                eprintln!("{}", layout::printable(content));
            }
            serde_json::Value::Array(parts) => {
                for part in parts {
                    match part["text"].as_str() {
                        Some(text) => eprintln!("{}", layout::printable(text)),
                        None => eprintln!(
                            "{}",
                            style::faint(&format!(
                                "[image, {}]",
                                text::about(
                                    image::tokens_in_url(
                                        part["image_url"]["url"].as_str().unwrap_or_default()
                                    )
                                    .unwrap_or(0)
                                )
                            ))
                        ),
                    }
                }
            }
            _ => {}
        }
        for call in message["tool_calls"].as_array().into_iter().flatten() {
            let name = call["function"]["name"].as_str().unwrap_or("?");
            let arguments = call["function"]["arguments"].as_str().unwrap_or_default();
            eprintln!("{}", style::faint(&format!("→ {name}")));
            eprintln!("{}", layout::printable(arguments));
        }
        // The system prompt's place: the tools follow it in the request.
        if role == "system" {
            eprintln!("\n{}", style::faint("── tools ──"));
            let tools = serde_json::to_string_pretty(&tools).unwrap_or_default();
            eprintln!("{}", layout::printable(&tools));
        }
    }
}

/// `/compact`: the summary the model now keeps, then what it freed. Stopped
/// or failed, nothing changed, and it says so.
fn compact(conversation: &mut Conversation, focus: &str) {
    eprintln!();
    match conversation.compact(focus) {
        Compacted::Nothing => eprintln!("{}", style::gray(text::NOTHING_TO_COMPACT)),
        Compacted::Stopped => eprintln!(
            "{}",
            style::gray(&format!("{} {}", text::STOPPED, text::NOTHING_CHANGED))
        ),
        Compacted::Failed(Some(message)) => {
            layout::print_fitted(&message, style::red);
            eprintln!("{}", style::gray(text::NOTHING_CHANGED));
        }
        Compacted::Failed(None) => eprintln!("{}", style::gray(text::NO_SUMMARY)),
        Compacted::Done {
            before,
            after,
            summary,
        } => {
            // The model's words, as the model now remembers the conversation,
            // without the bold markers a gray block would show raw; the
            // conversation keeps them as written.
            let width = tty::width().unwrap_or(80);
            let shown = layout::printable(&summary).replace("**", "");
            for line in layout::indented(&shown, 2, width).lines() {
                eprintln!("{}", style::gray(line));
            }
            eprintln!();
            eprintln!("{}", style::gray(&text::compacted(before, after)));
        }
    }
    eprintln!();
}

/// `/recon`'s probes, with the spinner meanwhile: Docker can take a moment.
fn gather_recon() -> Vec<(&'static str, String)> {
    let folder = env::current_dir().unwrap_or_default();
    spinner::while_running(move || recon::gather(&folder)).unwrap_or_default()
}

/// `/update`: asks for the newest version now, with the spinner meanwhile.
fn check_for_update(session: &Session) {
    let updates_off = session.updates_off;
    let message = match spinner::while_running(move || update::check_now(updates_off)) {
        Ok(update::Asked::Installed(version)) => text::ready_next_time(&version),
        Ok(update::Asked::Newest) => text::newest(),
        Ok(update::Asked::Unavailable(standing)) => text::cant_update(&standing),
        Ok(update::Asked::Failed) | Err(_) => text::UPDATE_FAILED.to_string(),
    };
    eprintln!("{}", style::gray(&message));
}

/// Opens the settings in the person's editor, written out with every setting
/// commented when there are none yet, and says whether what was saved reads.
fn edit_settings() {
    let path = config::settings_path();
    if !path.exists()
        && let Err(error) = config::write(&path, text::SETTINGS_TEMPLATE)
    {
        eprintln!(
            "{}",
            style::red(&text::cant_save(&config::shown(&path), &error.to_string()))
        );
        return;
    }
    // The approvals per project are settings too, opened beside them once there are any.
    let approvals = config::approvals_path();
    if approvals.exists() {
        open_in_editor(&[&path, &approvals]);
    } else {
        open_in_editor(&[&path]);
    }
    match config::load() {
        Ok(_) => eprintln!("{}", style::gray(text::SETTINGS_NEXT_START)),
        Err(message) => eprintln!("{}", style::red(&message)),
    }
}

fn open_in_editor(paths: &[&Path]) {
    let editor = env::var("VISUAL")
        .or_else(|_| env::var("EDITOR"))
        .unwrap_or_else(|_| "nano".into());
    // Through sh, so an editor set with arguments ("code --wait") works. Both
    // come from the PATH's absolute folders: the working folder may be a
    // project's, and a `./nano` of its own isn't the person's editor.
    let Some(sh) = system::program("sh") else {
        eprintln!("{}", style::red(&text::no_editor(&editor)));
        return;
    };
    let path = env::var("PATH").unwrap_or_default();
    let status = Command::new(sh)
        .env("PATH", system::absolute_only(&path))
        .arg("-c")
        .arg(format!("{editor} \"$@\""))
        .arg("sh")
        .args(paths)
        .status();
    if !status.is_ok_and(|status| status.success()) {
        eprintln!("{}", style::red(&text::no_editor(&editor)));
    }
}

/// Which of the project's notes go along, and what they cost: what's in
/// every request is never a surprise.
pub fn print_notes(conversation: &Conversation) {
    let notes = conversation.notes();
    let names: Vec<&str> = notes.iter().map(|note| note.name.as_str()).collect();
    let tokens = notes
        .iter()
        .map(|note| context::tokens_in(&note.text))
        .sum();
    let cut = notes.iter().any(|note| note.cut);
    eprintln!("{}", style::gray(&text::notes_taken(&names, tokens, cut)));
}

pub fn warn_if_user_md_is_long() {
    let lines = config::read_user_md().map_or(0, |contents| contents.lines().count());
    if lines > LONG_USER_MD {
        eprintln!("{}", style::gray(&text::long_prefs(lines)));
    }
}
