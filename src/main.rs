mod allowlist;
mod answer;
mod approvals;
mod chat;
mod config;
mod conversation;
mod diff;
mod feedback;
mod http;
mod input;
mod install;
mod interrupt;
mod keys;
mod markdown;
mod memory;
mod menu;
mod prompt;
mod reasoning;
// guimadaleno/release's client/release.rs, copied unchanged: no formatting or
// lint fixes may touch its bytes.
#[rustfmt::skip]
#[allow(clippy::all)]
mod release;
// guimadaleno/issues' client/report.rs, copied unchanged, like release.rs.
#[rustfmt::skip]
#[allow(clippy::all)]
mod report;
mod scratch;
mod server;
mod shell;
mod spinner;
mod style;
mod system;
mod text;
mod tools;
mod update;

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use conversation::Conversation;
use input::{Input, Line};

/// Past this many lines, about-me.md costs noticeable context on every turn.
const LONG_ABOUT_ME: usize = 30;
/// Below this, a long file or output no longer fits in one read. LM Studio
/// loads models with 8K unless told otherwise.
const SMALL_CONTEXT: u64 = 16_384;

fn main() -> ExitCode {
    let started = Instant::now();
    interrupt::install();
    let mut resume = false;
    let mut words = Vec::new();
    for arg in env::args().skip(1) {
        if !words.is_empty() {
            words.push(arg);
            continue;
        }
        match arg.as_str() {
            "-r" | "--resume" => resume = true,
            "-h" | "--help" => {
                println!("{}", text::USAGE);
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("lokal {}", text::VERSION);
                return ExitCode::SUCCESS;
            }
            "--install" => {
                return match install::install() {
                    Ok(message) => {
                        eprintln!("{message}");
                        ExitCode::SUCCESS
                    }
                    Err(message) => {
                        eprintln!("{}", style::red(&message));
                        ExitCode::FAILURE
                    }
                };
            }
            option if option.starts_with('-') => {
                eprintln!("{}", text::unknown_option(option));
                return ExitCode::from(2);
            }
            _ => words.push(arg),
        }
    }
    let request = words.join(" ");

    let config = match config::load() {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{}", style::red(&message));
            return ExitCode::FAILURE;
        }
    };
    let server = match server::find(&config) {
        Ok(server) => server,
        Err(message) => {
            eprintln!("{}", style::red(&message));
            return ExitCode::FAILURE;
        }
    };
    let folder = env::current_dir().unwrap_or_default();
    let project = approvals::ProjectApprovals::load(approvals::project_root(&folder));
    let mut input = Input::new(project);
    // Conversations only: a one-shot request often runs in a script, and
    // shouldn't reach the network on its own.
    let mut update = None;
    let mut first_command = None;
    if request.is_empty() {
        update = update::start(config.updates_off);
        print_header(&server);
        warn_if_context_is_small(&server);
        first_command = ask_first_question(&mut input);
        warn_if_about_me_is_long();
    }
    // Lives until Lokal quits, and takes its folder with it.
    let scratch = scratch::Scratch::create();
    let mut facts = system::gather();
    facts.scratch = scratch
        .as_ref()
        .map(|scratch| scratch.path().display().to_string());
    let mut conversation = Conversation::new(server, facts);
    if request.is_empty() && !conversation.notes().is_empty() {
        print_notes(&conversation);
    }
    if resume {
        if !conversation.resume() {
            eprintln!("{}", style::gray(text::NOTHING_TO_RESUME));
        } else if request.is_empty() {
            eprintln!("{}", style::gray(text::RESUMED));
        }
    }
    if request.is_empty() {
        let mut session = Session {
            started,
            updates_off: config.updates_off,
            feedback_draft: None,
        };
        talk(&mut conversation, &mut input, &mut session, first_command);
        let updated_to = update.and_then(|check| check.installed());
        let goodbye = text::lasted(started.elapsed(), updated_to.as_deref());
        eprintln!("{}", style::gray(&goodbye));
    } else {
        conversation.send(&request, &mut input);
    }
    ExitCode::SUCCESS
}

fn print_header(server: &server::Server) {
    eprintln!(
        "{} {}  {}",
        style::bold("Lokal"),
        text::VERSION,
        style::gray(&server.model)
    );
}

/// What the slash commands need to know about the running session.
struct Session {
    started: Instant,
    updates_off: bool,
    /// What /feedback was given and couldn't send yet.
    feedback_draft: Option<feedback::Draft>,
}

/// The conversation, from `first`, a command typed at the first question.
fn talk(
    conversation: &mut Conversation,
    input: &mut Input,
    session: &mut Session,
    first: Option<String>,
) {
    if let Some(command) = first
        && !run_command(&command, conversation, input, session)
    {
        return;
    }
    while let Line::Text(line) = input.request(conversation.status()) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if is_command(line) {
            if !run_command(line, conversation, input, session) {
                break;
            }
            continue;
        }
        // A blank line between the request and what follows, to breathe.
        eprintln!();
        conversation.send(line, input);
        print_separator(input);
    }
}

/// A faint line between turns, as wide as the terminal is right now. Lines
/// already printed stay as they are when the window is resized: the
/// terminal owns those.
fn print_separator(input: &mut Input) {
    match input.width() {
        // One column short of the edge, so the terminal doesn't wrap it.
        Some(width) if width > 1 => eprintln!("{}", style::faint(&"─".repeat(width - 1))),
        _ => println!(),
    }
}

/// `/prefs` is a command; `/etc/hosts is broken` is a request.
fn is_command(line: &str) -> bool {
    line.starts_with('/') && !line.contains(' ') && !line[1..].contains('/')
}

/// Runs a slash command. False when it's time to quit.
fn run_command(
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
            let path = config::about_me_path();
            if !path.exists() {
                let _ = config::write_about_me("");
            }
            open_in_editor(&[&path]);
            warn_if_about_me_is_long();
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
        "/info" => {
            eprintln!();
            print_info(conversation, input, session);
            eprintln!();
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

/// On the first launch, asks one question; skipping it is fine. Either way
/// about-me.md then exists, so the question isn't asked again. A slash
/// command typed instead answers nothing: it's returned to run, and the
/// question comes back at the next launch.
fn ask_first_question(input: &mut Input) -> Option<String> {
    if config::read_about_me().is_some() {
        return None;
    }
    eprintln!("{}", text::FIRST_QUESTION);
    let contents = match input.answer("> ") {
        Some(answer) if is_command(&answer) => return Some(answer),
        Some(answer) => format!("{} {answer}\n", text::PREFS_HEADING),
        None => String::new(),
    };
    let path = config::shown(&config::about_me_path());
    match config::write_about_me(&contents) {
        Ok(()) if !contents.is_empty() => eprintln!("{}", style::gray(&text::saved_prefs(&path))),
        Ok(()) => {}
        Err(error) => eprintln!(
            "{}",
            style::red(&text::cant_save(&path, &error.to_string()))
        ),
    }
    None
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
    let about_me = config::about_me_path();
    let about_me_lines = config::read_about_me().map_or(0, |text| text.lines().count());
    let settings = config::settings_path();
    let mut rows = vec![
        (text::INFO_VERSION, format!("{} · {updates}", text::VERSION)),
        (text::INFO_MODEL, format!("{}{context}", server.model)),
        (text::INFO_SERVER, format!("{} · {found}", server.address)),
        (
            text::INFO_IN_USE,
            format!("{in_use} · {} {session_time}", text::THIS_SESSION),
        ),
        (
            text::INFO_ABOUT_ME,
            text::file_lines(&config::shown(&about_me), about_me_lines),
        ),
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
    for (label, value) in rows {
        eprintln!("  {}{value}", style::gray(&format!("{label:<10}")));
    }
}

/// `/update`: asks for the newest version now, with the spinner meanwhile.
fn check_for_update(session: &Session) {
    let updates_off = session.updates_off;
    let check = std::thread::spawn(move || update::check_now(updates_off));
    let mut spinner = spinner::Spinner::new();
    while !check.is_finished() {
        spinner.tick();
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    spinner.clear();
    let message = match check.join() {
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

/// Only when the model takes more: otherwise there's nothing to raise.
fn warn_if_context_is_small(server: &server::Server) {
    if let (Some(context), Some(most)) = (server.context, server.max_context)
        && context < SMALL_CONTEXT
        && most > context
    {
        eprintln!("{}", style::gray(&text::small_context(context, most)));
    }
}

/// Which of the project's notes go along, and what they cost: what's in
/// every request is never a surprise.
fn print_notes(conversation: &Conversation) {
    let notes = conversation.notes();
    let names: Vec<&str> = notes.iter().map(|note| note.name.as_str()).collect();
    let tokens = notes
        .iter()
        .map(|note| conversation::tokens_in(&note.text))
        .sum();
    let cut = notes.iter().any(|note| note.cut);
    eprintln!("{}", style::gray(&text::notes_taken(&names, tokens, cut)));
}

fn warn_if_about_me_is_long() {
    let lines = config::read_about_me().map_or(0, |contents| contents.lines().count());
    if lines > LONG_ABOUT_ME {
        eprintln!("{}", style::gray(&text::long_prefs(lines)));
    }
}
