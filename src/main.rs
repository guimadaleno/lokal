mod allowlist;
mod answer;
mod chat;
mod config;
mod conversation;
mod diff;
mod http;
mod input;
mod install;
mod interrupt;
mod menu;
mod prompt;
// guimadaleno/release's client/release.rs, copied unchanged: no formatting or
// lint fixes may touch its bytes.
#[rustfmt::skip]
#[allow(clippy::all)]
mod release;
mod server;
mod shell;
mod spinner;
mod style;
mod system;
mod text;
mod tools;
mod update;

use std::env;
use std::process::{Command, ExitCode};
use std::time::Instant;

use conversation::Conversation;
use input::{Input, Line};

/// Past this many lines, about-me.md costs noticeable context on every turn.
const LONG_ABOUT_ME: usize = 30;

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
    let mut input = Input::new();
    // Conversations only: a one-shot request often runs in a script, and
    // shouldn't reach the network on its own.
    let mut update = None;
    if request.is_empty() {
        update = update::start(config.updates_off);
        print_header(&server);
        ask_first_question(&mut input);
        warn_if_about_me_is_long();
    }
    let mut conversation = Conversation::new(server, system::gather());
    if resume {
        if !conversation.resume() {
            eprintln!("{}", style::gray(text::NOTHING_TO_RESUME));
        } else if request.is_empty() {
            eprintln!("{}", style::gray(text::RESUMED));
        }
    }
    if request.is_empty() {
        talk(&mut conversation, &mut input);
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

fn talk(conversation: &mut Conversation, input: &mut Input) {
    while let Line::Text(line) = input.request(conversation.status()) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if is_command(line) {
            if !run_command(line, conversation) {
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
fn run_command(command: &str, conversation: &mut Conversation) -> bool {
    match command {
        "/new" => {
            conversation.clear();
            eprintln!("{}", style::gray(text::NEW_CONVERSATION));
        }
        "/prefs" => {
            edit_about_me();
            warn_if_about_me_is_long();
            conversation.refresh_system_prompt();
        }
        "/help" => {
            for (name, about) in text::COMMANDS {
                eprintln!("  {name:<8}{}", style::gray(about));
            }
            eprintln!("{}", style::gray(text::HELP_KEYS));
        }
        "/about" => print_about(),
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
    eprintln!("  {}", text::REPOSITORY);
}

/// On the first launch, asks one question; skipping it is fine. Either way
/// about-me.md then exists, so the question isn't asked again.
fn ask_first_question(input: &mut Input) {
    if config::read_about_me().is_some() {
        return;
    }
    eprintln!("{}", text::FIRST_QUESTION);
    let contents = match input.answer("> ") {
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
}

fn edit_about_me() {
    let path = config::about_me_path();
    if !path.exists() {
        let _ = config::write_about_me("");
    }
    let editor = env::var("VISUAL")
        .or_else(|_| env::var("EDITOR"))
        .unwrap_or_else(|_| "nano".into());
    // Through sh, so an editor set with arguments ("code --wait") works.
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&path)
        .status();
    if !status.is_ok_and(|status| status.success()) {
        eprintln!("{}", style::red(&text::no_editor(&editor)));
    }
}

fn warn_if_about_me_is_long() {
    let lines = config::read_about_me().map_or(0, |contents| contents.lines().count());
    if lines > LONG_ABOUT_ME {
        eprintln!("{}", style::gray(&text::long_prefs(lines)));
    }
}
