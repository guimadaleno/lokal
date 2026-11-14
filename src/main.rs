#[macro_use]
mod screen;
mod allowlist;
mod answer;
mod approvals;
mod chat;
mod clipboard;
mod clock;
mod commands;
mod compact;
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
mod mcp;
mod memory;
mod menu;
mod mode;
mod peers;
mod progress;
mod prompt;
mod protected;
mod queue;
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
use std::time::{Instant, SystemTime};

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
    if config.times_off {
        clock::switch_off();
    }
    let server = match server::find(&config) {
        Ok(server) => server,
        Err(message) => {
            layout::print_fitted(&message, style::red);
            return ExitCode::FAILURE;
        }
    };
    let folder = env::current_dir().unwrap_or_default();
    let project = approvals::ProjectApprovals::load(approvals::project_root(&folder));
    let mut input = Input::new(project);
    input.set_vision(server.vision);
    // MCP servers in a conversation at a terminal only, where each call can
    // be asked about; started now, so they're up by the first request.
    let mcp = if talking && input.can_ask() {
        mcp::Mcp::start(&config.mcp)
    } else {
        None
    };
    // Conversations only: a one-shot request often runs in a script, and
    // shouldn't reach the network on its own.
    let mut update = None;
    let mut first_line = None;
    if talking {
        update = update::start(config.updates_off);
        // From here on the screen is kept, to lay out again at a new width.
        screen::keep();
        commands::print_header(&server);
        warn_if_context_is_small(&server, false);
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
    let mut conversation = Conversation::new(server, facts, peers, mcp);
    if talking && !conversation.notes().is_empty() {
        commands::print_notes(&conversation);
    }
    if resume {
        if !conversation.resume() {
            showln!("{}", style::gray(text::NOTHING_TO_RESUME));
        } else if talking {
            showln!("{}", style::gray(text::RESUMED));
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
        showln!("{}", style::gray(&goodbye));
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
    // What's typed while Lokal works is kept from here (#3).
    queue::open();
    // Not dated: what start-up printed after it may stand between.
    if let Some(line) = first
        && !take_line(&line, None, conversation, input, session)
    {
        return;
    }
    loop {
        // Queued while the last turn ran, and it ended on its own: it goes
        // as typed. Otherwise what was typed waits at the `>`.
        if let Some((line, queued)) = queue::take_ready() {
            input.show_queued(&line, queued);
            if !take_line(&line, None, conversation, input, session) {
                break;
            }
            continue;
        }
        let line = match input.request(conversation.status(), queue::take_typed()) {
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
        if !take_line(&line, Some(SystemTime::now()), conversation, input, session) {
            break;
        }
    }
}

/// What the person typed at the `>`: a slash command, a command of their
/// own, mail for another session or a request. A line typed at the `>` is
/// `sent` then, and is kept as it shows, with that time beside it but for
/// a slash command or an empty line; one queued or answered at a question
/// is None, as what showed it kept it. False when it's time to quit.
fn take_line(
    typed: &str,
    sent: Option<SystemTime>,
    conversation: &mut Conversation,
    input: &mut Input,
    session: &mut Session,
) -> bool {
    let line = typed.trim();
    if let Some(sent) = sent {
        let dated = !line.is_empty() && !is_command(line);
        input.sent(typed, dated.then_some(sent));
    }
    if line.is_empty() {
        pass_on_messages(conversation, input);
        return true;
    }
    if is_command(line) {
        return run_command(line, conversation, input, session);
    }
    if let Some(command) = own_command(line) {
        // A blank line between the line and what follows, as after a request.
        showln!();
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
    showln!();
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
    let end = conversation.send(request, input, asked);
    // Stopped, said no to or failed: what the person queued meanwhile
    // waits at the `>`, since they may want it changed.
    if end != conversation::Turn::Answered {
        queue::hold();
    }
    if end == conversation::Turn::SteppedIn
        && asked.is_none()
        && let Some(peers) = conversation.peers()
    {
        peers.inbox().hold();
    }
    if conversation.take_learned_context() {
        warn_if_context_is_small(&conversation.server, true);
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
        showln!("{}", style::gray(&failed));
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
    showln!();
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
        showln!("{}", style::gray(text::DROPPED));
    }
}

/// A faint line between turns, as wide as the terminal, ending in when the
/// turn ended (#46). Laid out again when the width changes (#49).
fn print_separator(input: &mut Input) {
    match input.width() {
        Some(width) if width > 1 => {
            let time = clock::shown(SystemTime::now(), |time| has_room(width, time));
            screen::fitted(Box::new(move |width| {
                format!("{}\n", style::faint(&separator(width, time.as_deref())))
            }));
        }
        // On stderr, like the line: stdout carries only answers.
        _ => showln!(),
    }
}

/// The least of the rule that shows beside a time.
const RULE_BESIDE_TIME: usize = 4;

/// Whether the rule at `width` has room for a time `time` columns wide.
fn has_room(width: usize, time: usize) -> bool {
    width.saturating_sub(1) >= RULE_BESIDE_TIME + 1 + time
}

/// The line between turns at `width`, one column short of the edge so the
/// terminal doesn't wrap it, ending in `time` where it has room.
fn separator(width: usize, time: Option<&str>) -> String {
    let rule = width.saturating_sub(1).max(1);
    match time.filter(|time| has_room(width, time.chars().count())) {
        Some(time) => format!("{} {time}", "─".repeat(rule - 1 - time.chars().count())),
        None => "─".repeat(rule),
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
    showln!("{}", text::FIRST_QUESTION);
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
        Ok(()) if !contents.is_empty() => showln!("{}", style::gray(&text::saved_prefs(&path))),
        Ok(()) => {}
        Err(error) => showln!(
            "{}",
            style::red(&text::cant_save(&path, &error.to_string()))
        ),
    }
    None
}

/// Only when the model takes more: otherwise there's nothing to raise.
/// `apart` puts a blank line before it, to set it off from an answer.
fn warn_if_context_is_small(server: &server::Server, apart: bool) {
    if let (Some(context), Some(most)) = (server.context, server.max_context)
        && context < SMALL_CONTEXT
        && most > context
    {
        if apart {
            showln!();
        }
        layout::print_fitted(
            &text::small_context(context, most, server.kind),
            style::gray,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Args {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn the_rule_ends_in_its_time_where_there_is_room() {
        let rule = separator(40, Some("10:40"));
        assert_eq!(rule.chars().count(), 39);
        assert!(rule.starts_with("────") && rule.ends_with("─ 10:40"));
        // The same rule at another width: it's laid out again there.
        assert_eq!(separator(20, Some("10:40")).chars().count(), 19);
        // Without room for the time and a bit of rule, only the rule.
        assert_eq!(separator(10, Some("10:40")), "─".repeat(9));
        assert_eq!(separator(9, None), "─".repeat(8));
        assert_eq!(separator(1, None), "─");
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
