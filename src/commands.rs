//! The slash commands, and what they print: the session's facts, the
//! other sessions, the help, and the files they open in the editor.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::conversation::{self, Conversation};
use crate::input::Input;
use crate::{config, feedback, layout, memory, peers, recon, server, spinner, style, text, update};

/// Past this many lines, user.md costs noticeable context on every turn.
const LONG_USER_MD: usize = 30;

pub fn print_header(server: &server::Server) {
    eprintln!(
        "{} {}  {}",
        style::bold("Lokal"),
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
    line.starts_with('/') && !line.contains(' ') && !line[1..].contains('/')
}

/// Runs a slash command. False when it's time to quit.
pub fn run_command(
    command: &str,
    conversation: &mut Conversation,
    input: &mut Input,
    session: &mut Session,
) -> bool {
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
        "/help" => {
            eprintln!();
            // The longest name and two spaces, so every description lines up.
            let width = text::COMMANDS
                .iter()
                .map(|(name, _)| name.len())
                .max()
                .unwrap_or(0)
                + 2;
            for (name, about) in text::COMMANDS {
                eprintln!("  {name:<width$}{}", style::gray(about));
            }
            eprintln!("{}", style::gray(text::HELP_KEYS));
            eprintln!();
        }
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
        style::bold("Lokal"),
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
        let tokens = conversation::tokens_in(&note.text);
        rows.push((label, text::note_size(&note.name, tokens, note.cut)));
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
    // The names in a column, and the rest after them, going on under itself.
    let names = sessions
        .iter()
        .map(|other| other.name.chars().count())
        .max()
        .unwrap_or(0);
    let room = style::terminal_width()
        .map_or(usize::MAX, |width| width.saturating_sub(names + 5))
        .max(20);
    for other in &sessions {
        let mut about = config::shown(Path::new(&other.folder));
        if !other.branch.is_empty() {
            about.push_str(&format!(" · {}", other.branch));
        }
        about.push_str(&format!(" · {}", peers::session_state(other)));
        let mut lines = layout::broken(&about, room);
        let doing = text::session_doing(&other.request, &other.files);
        if !doing.is_empty() {
            lines.extend(layout::broken(&doing, room));
        }
        for (index, line) in lines.iter().enumerate() {
            let name = if index == 0 { other.name.as_str() } else { "" };
            eprintln!("  {name:<names$}  {}", style::gray(line));
        }
    }
}

/// Labels in one column, values in the next. A value too long for the
/// terminal goes on under itself, broken after a separator, so it never
/// runs under the labels.
fn print_rows(rows: &[(&str, String)]) {
    // The indent and the label's column.
    let room = style::terminal_width().map_or(0, |width| width.saturating_sub(13));
    for (label, value) in rows {
        for (index, line) in layout::wrapped(value, room).iter().enumerate() {
            let label = if index == 0 { *label } else { "" };
            eprintln!("  {}{line}", style::gray(&format!("{label:<10}")));
        }
    }
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
    // Through sh, so an editor set with arguments ("code --wait") works.
    let status = Command::new("sh")
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
        .map(|note| conversation::tokens_in(&note.text))
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
