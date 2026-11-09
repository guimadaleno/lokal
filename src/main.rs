mod allowlist;
mod answer;
mod approvals;
mod chat;
mod commands;
mod config;
mod context;
mod conversation;
mod diff;
mod feedback;
mod gitconfig;
mod http;
mod image;
mod input;
mod install;
mod interrupt;
mod judge;
mod keys;
mod layout;
mod markdown;
mod memory;
mod menu;
mod mode;
mod peers;
mod progress;
mod prompt;
mod reasoning;
mod recon;
// guimadaleno/base/release's client/release.rs, copied unchanged: no formatting or
// lint fixes may touch its bytes.
#[rustfmt::skip]
#[allow(clippy::all)]
mod release;
// guimadaleno/base/issues' client/report.rs, copied unchanged, like release.rs.
#[rustfmt::skip]
#[allow(clippy::all)]
mod report;
mod sandbox;
mod scratch;
mod server;
mod shell;
mod spinner;
mod style;
mod system;
mod text;
mod tools;
mod tty;
mod update;

use std::env;
use std::process::ExitCode;
use std::time::Instant;

use commands::{Session, is_command, run_command};
use conversation::Conversation;
use input::{Input, Line};

/// Below this, a long file or output no longer fits in one read. LM Studio
/// loads models with 8K unless told otherwise.
const SMALL_CONTEXT: u64 = 16_384;

/// What the command line asks for.
#[derive(Debug, PartialEq)]
enum Args {
    Help,
    Version,
    Install,
    Uninstall,
    /// An option Lokal doesn't know.
    Unknown(String),
    /// A conversation when `request` is empty, otherwise a one-shot request.
    Run {
        resume: bool,
        request: String,
    },
}

/// Options come first; from the first word on, everything is the request,
/// so `lokal explain -r` asks about `-r`.
fn parse_args(args: impl Iterator<Item = String>) -> Args {
    let mut resume = false;
    let mut words = Vec::new();
    for arg in args {
        if !words.is_empty() {
            words.push(arg);
            continue;
        }
        match arg.as_str() {
            "-r" | "--resume" => resume = true,
            "-h" | "--help" => return Args::Help,
            "-V" | "--version" => return Args::Version,
            "--install" => return Args::Install,
            "--uninstall" => return Args::Uninstall,
            option if option.starts_with('-') => return Args::Unknown(option.to_string()),
            _ => words.push(arg),
        }
    }
    Args::Run {
        resume,
        request: words.join(" "),
    }
}

fn main() -> ExitCode {
    // The box sets itself up and becomes the command: nothing else of
    // Lokal's may start first, a thread least of all.
    if env::args().nth(1).as_deref() == Some("--box") {
        return sandbox::enter(env::args().skip(2));
    }
    let started = Instant::now();
    interrupt::install();
    match parse_args(env::args().skip(1)) {
        Args::Help => {
            println!("{}", text::USAGE);
            ExitCode::SUCCESS
        }
        Args::Version => {
            println!("{}", text::VERSION_LINE);
            ExitCode::SUCCESS
        }
        Args::Install => match install::install() {
            Ok(message) => {
                eprintln!("{message}");
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("{}", style::red(&message));
                ExitCode::FAILURE
            }
        },
        Args::Uninstall => {
            if install::uninstall() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Args::Unknown(option) => {
            eprintln!("{}", text::unknown_option(&option));
            ExitCode::from(2)
        }
        Args::Run { resume, request } => run(started, resume, &request),
    }
}

/// Start-up, then the conversation or the one-shot request.
fn run(started: Instant, resume: bool, request: &str) -> ExitCode {
    // A conversation at the `>`, rather than a one-shot request.
    let talking = request.is_empty();
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
    let mut first_line = None;
    if talking {
        update = update::start(config.updates_off);
        commands::print_header(&server);
        warn_if_context_is_small(&server);
        first_line = ask_first_question(&mut input);
        commands::warn_if_user_md_is_long();
    }
    // Lives until Lokal quits, and takes its folder with it.
    let scratch = scratch::Scratch::create();
    let mut facts = system::gather();
    facts.scratch = scratch
        .as_ref()
        .map(|scratch| scratch.path().display().to_string());
    // The person's other sessions, in a conversation at a terminal only:
    // there, someone sees mail arrive and can stop what it starts.
    let mail_turns = config.mail_turns.unwrap_or(peers::TURNS);
    let peers = scratch
        .as_ref()
        .filter(|_| talking && config.mail != config::Mail::Off)
        .and_then(|scratch| {
            let printer = input.printer()?;
            peers::Peers::start(
                scratch.base(),
                &folder,
                config.mail,
                mail_turns,
                Some(printer),
            )
        });
    if let Some(peers) = &peers {
        input.receive(peers.inbox(), peers.directory());
    }
    let mut conversation = Conversation::new(server, facts, peers);
    if talking && !conversation.notes().is_empty() {
        commands::print_notes(&conversation);
    }
    if resume {
        if !conversation.resume() {
            eprintln!("{}", style::gray(text::NOTHING_TO_RESUME));
        } else if talking {
            eprintln!("{}", style::gray(text::RESUMED));
        }
    }
    if talking {
        let mut session = Session {
            started,
            updates_off: config.updates_off,
            mail: config.mail,
            mail_turns,
            feedback_draft: None,
        };
        talk(&mut conversation, &mut input, &mut session, first_line);
        let updated_to = update.and_then(|check| check.installed());
        let goodbye = text::lasted(started.elapsed(), updated_to.as_deref());
        eprintln!("{}", style::gray(&goodbye));
    } else if conversation.send(request, &mut input, Some(request)) != conversation::Turn::Answered
    {
        // A script running a one-shot request can tell no answer came: the
        // server failed, or the person said no or stopped it.
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// The conversation, from `first`, a line typed at the first question that
/// wasn't an answer to it.
fn talk(
    conversation: &mut Conversation,
    input: &mut Input,
    session: &mut Session,
    first: Option<String>,
) {
    if let Some(line) = first
        && !take_line(&line, conversation, input, session)
    {
        return;
    }
    loop {
        let line = match input.request(conversation.status()) {
            Line::Text(line) => line,
            Line::Mail => {
                take_up_mail(conversation, input);
                continue;
            }
            Line::DropMessages => {
                drop_messages(conversation);
                continue;
            }
            Line::Quit => break,
        };
        if !take_line(&line, conversation, input, session) {
            break;
        }
    }
}

/// What the person typed at the `>`: a slash command, a command of their
/// own, mail for another session or a request. False when it's time to quit.
fn take_line(
    line: &str,
    conversation: &mut Conversation,
    input: &mut Input,
    session: &mut Session,
) -> bool {
    let line = line.trim();
    if line.is_empty() {
        pass_on_messages(conversation, input);
        return true;
    }
    if is_command(line) {
        return run_command(line, conversation, input, session);
    }
    if let Some(command) = own_command(line) {
        // A blank line between the line and what follows, as after a request.
        eprintln!();
        if let Some(peers) = conversation.peers() {
            peers.inbox().person_acted(None);
        }
        conversation.run_own(command);
        print_separator(input);
        return true;
    }
    if mail_from_person(line, conversation) {
        return true;
    }
    // A blank line between the request and what follows, to breathe.
    eprintln!();
    if let Some(peers) = conversation.peers() {
        peers.inbox().person_acted(Some(line));
    }
    let request = with_news(conversation, line);
    turn(conversation, input, &request, Some(line));
    true
}

/// One turn on `request`, which the person `asked` in their own words, or
/// which is mail when None. When the person stops a turn on mail, or says
/// no in one, mail waits for their next request: two models answering each
/// other would otherwise start the next turn at once.
fn turn(conversation: &mut Conversation, input: &mut Input, request: &str, asked: Option<&str>) {
    if conversation.send(request, input, asked) == conversation::Turn::SteppedIn
        && asked.is_none()
        && let Some(peers) = conversation.peers()
    {
        peers.inbox().hold();
    }
    print_separator(input);
}

/// The request with what came meanwhile: mail that needs no answer, shown
/// as it arrived, and news of the sessions working where this one does,
/// which doesn't show (`Peers::news`).
fn with_news(conversation: &Conversation, request: &str) -> String {
    let Some(peers) = conversation.peers() else {
        return request.to_string();
    };
    let mut request = request.to_string();
    let later = peers.inbox().take_later();
    if !later.is_empty() {
        request.push_str("\n\n");
        request.push_str(&peers::framed(&later));
    }
    if let Some(note) = peers.news() {
        request.push_str("\n\n");
        request.push_str(&note);
    }
    request
}

/// A turn on mail, started on its own.
fn take_up_mail(conversation: &mut Conversation, input: &mut Input) {
    let Some(peers) = conversation.peers() else {
        return;
    };
    let inbox = peers.inbox();
    inbox.show_arrived();
    let messages = inbox.take_for_turn();
    if messages.is_empty() {
        return;
    }
    inbox.turn_on_mail(&messages);
    let request = with_news(conversation, &peers::framed(&messages));
    turn(conversation, input, &request, None);
}

/// `@name …` at the `>`, for an open session: Lokal mails it what follows,
/// as the person's own words. False for anything else, which is a request.
fn mail_from_person(line: &str, conversation: &Conversation) -> bool {
    let Some(peers) = conversation.peers() else {
        return false;
    };
    let Some(rest) = line.strip_prefix('@') else {
        return false;
    };
    let (to, text) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let text = text.trim();
    if text.is_empty() || to == peers.name() {
        return false;
    }
    // Sent, the line itself says so; not sent, it's said, or the person
    // would think it went.
    let failed = match peers.send(to, text, true, true) {
        Err(peers::Undelivered::NoSuch) => return false,
        Ok(_) => None,
        Err(peers::Undelivered::Full) => Some(text::mail_full(to)),
        Err(peers::Undelivered::TooLong) => Some(text::mail_too_long(peers::MESSAGE_LIMIT)),
    };
    peers.inbox().person_acted(None);
    if let Some(failed) = failed {
        eprintln!("{}", style::gray(&failed));
    }
    true
}

/// Enter on an empty line: the mail on screen goes to the model, framed as
/// another session's, as the turn's request.
fn pass_on_messages(conversation: &mut Conversation, input: &mut Input) {
    let Some(peers) = conversation.peers() else {
        return;
    };
    let messages = peers.inbox().take();
    if messages.is_empty() {
        return;
    }
    peers.inbox().person_acted(None);
    // A blank line between the `>` and what follows, as after a request.
    eprintln!();
    let request = with_news(conversation, &peers::framed(&messages));
    turn(conversation, input, &request, None);
}

/// Esc on an empty line: the mail on screen goes, and the model never
/// hears of it.
fn drop_messages(conversation: &Conversation) {
    let Some(peers) = conversation.peers() else {
        return;
    };
    if !peers.inbox().take().is_empty() {
        eprintln!("{}", style::gray(text::DROPPED));
    }
}

/// A faint line between turns, as wide as the terminal is right now. Lines
/// already printed stay as they are when the window is resized: the
/// terminal owns those.
fn print_separator(input: &mut Input) {
    match input.width() {
        // One column short of the edge, so the terminal doesn't wrap it.
        Some(width) if width > 1 => eprintln!("{}", style::faint(&"─".repeat(width - 1))),
        // On stderr, like the line: stdout carries only answers.
        _ => eprintln!(),
    }
}

/// What follows the `!` of `!ls` or `! ls`: a command of the person's own.
fn own_command(line: &str) -> Option<&str> {
    line.strip_prefix('!')
        .map(str::trim)
        .filter(|command| !command.is_empty())
}

/// On the first launch, asks one question; skipping it is fine. Either way
/// user.md then exists, so the question isn't asked again. A slash
/// command, a `!` command or mail typed instead answers nothing: it's
/// returned to take as at the `>`, and the question comes back at the next
/// launch.
fn ask_first_question(input: &mut Input) -> Option<String> {
    // Whether it exists, not whether it reads: one saved in another
    // encoding doesn't, and the answer would replace the person's notes.
    if config::user_md_path().exists() {
        return None;
    }
    eprintln!("{}", text::FIRST_QUESTION);
    let contents = match input.answer(input::PROMPT) {
        Some(answer)
            if is_command(&answer) || own_command(&answer).is_some() || answer.starts_with('@') =>
        {
            return Some(answer);
        }
        Some(answer) => format!("{} {answer}\n", text::PREFS_HEADING),
        None => String::new(),
    };
    let path = config::shown(&config::user_md_path());
    match config::write_user_md(&contents) {
        Ok(()) if !contents.is_empty() => eprintln!("{}", style::gray(&text::saved_prefs(&path))),
        Ok(()) => {}
        Err(error) => eprintln!(
            "{}",
            style::red(&text::cant_save(&path, &error.to_string()))
        ),
    }
    None
}

/// Only when the model takes more: otherwise there's nothing to raise.
fn warn_if_context_is_small(server: &server::Server) {
    if let (Some(context), Some(most)) = (server.context, server.max_context)
        && context < SMALL_CONTEXT
        && most > context
    {
        let warning = text::small_context(context, most, server.kind);
        // Broken at spaces, one column short of the edge: the terminal
        // would break it mid-word.
        let room = tty::width().map_or(usize::MAX, |width| width.saturating_sub(1));
        for line in layout::broken(&warning, room) {
            eprintln!("{}", style::gray(&line));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Args {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn options_come_before_the_request() {
        let run = |resume, request: &str| Args::Run {
            resume,
            request: request.to_string(),
        };
        assert_eq!(parsed(&[]), run(false, ""));
        assert_eq!(parsed(&["-r"]), run(true, ""));
        assert_eq!(parsed(&["--resume", "fix", "it"]), run(true, "fix it"));
        // From the first word on, an option is part of the request.
        assert_eq!(parsed(&["explain", "-r"]), run(false, "explain -r"));
        assert_eq!(parsed(&["-V", "x"]), Args::Version);
        assert_eq!(parsed(&["--nope"]), Args::Unknown("--nope".into()));
    }
}
