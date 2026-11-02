//! `/feedback`: a problem or an idea, filed as an issue for the developer
//! through issues.guimadaleno.com, as every app here does it. guimadaleno/
//! base/issues' README is the authority; report.rs is its client. Nothing goes
//! unless the person asks, sees all of it and says send. The conversation is
//! never part of it: it can hold files and secrets the model read.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::input::Input;
use crate::menu::{self, Item};
use crate::report::{self, Answer, Field, Kind, Report};
use crate::server::Server;
use crate::{spinner, style, system, text, update};

const APP: &str = "lokal";

/// What the person typed, kept until it's sent, so a failed send loses nothing.
#[derive(Default, Clone)]
pub struct Draft {
    title: String,
    description: String,
    contact: String,
}

/// Asks for a report, shows it whole, and sends it if the person says so.
/// Returns what to keep for next time: the draft, unless it was sent.
pub fn run(
    input: &mut Input,
    server: &Server,
    updates_off: bool,
    draft: Option<Draft>,
) -> Option<Draft> {
    let kinds = [
        Item {
            label: text::FEEDBACK_BUG,
            key: 'b',
        },
        Item {
            label: text::FEEDBACK_IDEA,
            key: 'i',
        },
        Item {
            label: text::NEVER_MIND,
            key: 'n',
        },
    ];
    let kind = match menu::choose(text::FEEDBACK_KIND, &kinds, 2, 2)? {
        menu::Choice::Picked(0) => Kind::Bug,
        menu::Choice::Picked(1) => Kind::Idea,
        _ => return draft,
    };
    let mut typed = draft.unwrap_or_default();
    // Leaving the title or the description empty is how to give up; what
    // was typed before stays for next time.
    let Some(title) = input.answer_with(text::FEEDBACK_TITLE, &typed.title) else {
        return Some(typed);
    };
    typed.title = title;
    let asked = if kind == Kind::Bug {
        text::FEEDBACK_WHAT_HAPPENED
    } else {
        text::FEEDBACK_THE_IDEA
    };
    let Some(description) = input.answer_with(asked, &typed.description) else {
        return Some(typed);
    };
    typed.description = description;
    typed.contact = input
        .answer_with(text::FEEDBACK_CONTACT, &typed.contact)
        .unwrap_or_default();

    let title = report::line(&typed.title);
    let description = report::text(&typed.description);
    let contact = report::line(&typed.contact);
    let system = report::line(&system_line());
    let details = details(server, updates_off);
    let details: Vec<(&str, &str)> = details
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let mut report = Report {
        kind,
        title: &title,
        description: &description,
        version: text::VERSION,
        system: Some(&system),
        contact: (!contact.is_empty()).then_some(contact.as_str()),
        details: &details,
        log: None,
    };
    if let Err(field) = report.check() {
        eprintln!("  {}", style::red(&text::feedback_field(field_name(field))));
        return Some(typed);
    }

    show(&report);
    let sends = [
        Item {
            label: text::FEEDBACK_SEND,
            key: 's',
        },
        Item {
            label: text::FEEDBACK_SEND_LESS,
            key: 'w',
        },
        Item {
            label: text::FEEDBACK_DONT_SEND,
            key: 'n',
        },
    ];
    match menu::choose(text::FEEDBACK_SEND_QUESTION, &sends, 2, 2)? {
        menu::Choice::Picked(0) => {}
        menu::Choice::Picked(1) => {
            report.system = None;
            report.details = &[];
        }
        _ => {
            eprintln!("  {}", style::gray(text::FEEDBACK_KEPT));
            return Some(typed);
        }
    }
    if is_sent(&report) { None } else { Some(typed) }
}

/// Sends the report and says how it went. False keeps the draft.
fn is_sent(report: &Report) -> bool {
    let Ok(body) = report.body() else {
        eprintln!("  {}", style::red(text::FEEDBACK_REFUSED));
        return false;
    };
    let Some((status, answer)) = send(&body) else {
        eprintln!("  {}", style::red(text::FEEDBACK_UNREACHABLE));
        return false;
    };
    match report::answer(status, &answer) {
        Answer::Sent(reference) => {
            eprintln!("  {}", style::gray(&text::feedback_sent(&reference)));
            true
        }
        Answer::Invalid(field) => {
            eprintln!("  {}", style::red(&text::feedback_field(field_name(field))));
            false
        }
        Answer::Later(seconds) => {
            eprintln!("  {}", style::gray(&text::feedback_later(seconds)));
            false
        }
        Answer::Refused => {
            eprintln!("  {}", style::red(text::FEEDBACK_REFUSED));
            false
        }
    }
}

/// Everything that will be sent, as it will be sent.
fn show(report: &Report) {
    let kind = if report.kind == Kind::Bug {
        "bug"
    } else {
        "idea"
    };
    let details: Vec<String> = report
        .details
        .iter()
        .map(|(name, value)| format!("{name} {value}"))
        .collect();
    let mut rows = vec![
        ("kind", kind.to_string()),
        ("title", report.title.to_string()),
    ];
    rows.push((
        "description",
        report.description.replace('\n', "\n               "),
    ));
    if let Some(contact) = report.contact {
        rows.push(("contact", contact.to_string()));
    }
    rows.push(("version", report.version.to_string()));
    if let Some(system) = report.system {
        rows.push(("system", system.to_string()));
    }
    if !details.is_empty() {
        rows.push(("details", details.join(" · ")));
    }
    eprintln!();
    eprintln!("  {}", style::gray(text::FEEDBACK_PREVIEW));
    for (label, value) in rows {
        eprintln!("  {}{value}", style::gray(&format!("{label:<13}")));
    }
    eprintln!();
}

/// The system and its version, and the processor: nothing that names the machine.
fn system_line() -> String {
    let facts = system::gather();
    let wsl = if facts.wsl { ", WSL" } else { "" };
    format!("{}{wsl}, {}", facts.os, std::env::consts::ARCH)
}

/// How Lokal is set up, in kinds, never names: no server address, no model.
fn details(server: &Server, updates_off: bool) -> Vec<(&'static str, String)> {
    let locale = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    let server_kind = if server.context.is_some() {
        "lm-studio"
    } else {
        "openai-compatible"
    };
    let found = if server.pinned { "settings" } else { "found" };
    let updates = match update::standing_of(updates_off) {
        update::Standing::Daily => "daily",
        update::Standing::SwitchedOff => "off",
        update::Standing::NotARelease => "not-a-release",
        update::Standing::CantReplace => "cant-replace",
    };
    let mut details = vec![
        ("server", server_kind.to_string()),
        ("server_from", found.to_string()),
        ("updates", updates.to_string()),
    ];
    if !locale.is_empty() {
        details.insert(0, ("locale", locale));
    }
    details
}

/// The POST, through curl, as the updater fetches: HTTPS to the service and
/// nowhere else, no redirect followed, thirty seconds at most. None when no
/// answer came at all.
fn send(body: &[u8]) -> Option<(u16, Vec<u8>)> {
    let mut child = Command::new(system::program("curl")?)
        .args([
            "--silent",
            "--proto",
            "=https",
            "--max-time",
            "30",
            "--max-filesize",
            "65536",
        ])
        .args(["--user-agent", &format!("lokal/{}", text::VERSION)])
        .args([
            "--header",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
        ])
        .args(["--write-out", "\n%{http_code}"])
        .arg(report::address(APP))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(body).ok()?;
    // The answer is read while curl runs, so a long one can't fill the pipe and stall it.
    let output = spinner::while_running(move || child.wait_with_output())
        .ok()?
        .ok()?;
    let split = output.stdout.iter().rposition(|byte| *byte == b'\n')?;
    let status: u16 = std::str::from_utf8(&output.stdout[split + 1..])
        .ok()?
        .trim()
        .parse()
        .ok()?;
    // 000 is curl's way of saying nothing answered.
    (status != 0).then(|| (status, output.stdout[..split].to_vec()))
}

fn field_name(field: Field) -> &'static str {
    match field {
        Field::Title => "title",
        Field::Description => "description",
        Field::Contact => "contact",
        Field::System => "system",
        Field::Details => "details",
        Field::Version => "version",
        Field::Kind => "kind",
        Field::Log => "log",
        Field::Report => "report",
    }
}
