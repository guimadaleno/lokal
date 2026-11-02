//! The tools for the person's other sessions: list_sessions and mail, both
//! shown as steps, so the person sees what this model tells the others.

use std::path::Path;

use serde_json::Value;

use super::{Outcome, text_arg};
use crate::peers::{self, Peers, Then, Undelivered};
use crate::{config, layout, style, text};

/// The other sessions open, on screen and for the model.
pub(super) fn list_sessions(peers: &Peers) -> Outcome {
    eprintln!("{}", style::accent(text::STEP_SESSIONS));
    let sessions = peers.others();
    if sessions.is_empty() {
        eprintln!("  {}", style::gray(text::NO_SESSIONS));
        return Outcome::Done("No other Lokal session is open.".into());
    }
    // The names in a column, and the folders after them, a long one going
    // on under itself.
    let names = sessions
        .iter()
        .map(|s| s.name.chars().count())
        .max()
        .unwrap_or(0);
    let lead = 2 + names + 2;
    let room = style::terminal_width()
        .unwrap_or(80)
        .saturating_sub(lead + 1)
        .max(20);
    let mut result = String::new();
    for session in &sessions {
        let mut row = config::shown(Path::new(&session.folder));
        let mut line = format!("- {}, in {}", session.name, session.folder);
        if !session.branch.is_empty() {
            row.push_str(&format!(" · {}", session.branch));
            line.push_str(&format!(", branch {}", session.branch));
        }
        row.push_str(&format!(" · {}", peers::session_state(session)));
        // A folder's or a branch's name can hold control characters.
        let row = layout::printable(&row);
        line.push_str(match (session.busy, session.asking) {
            (true, true) => {
                ", waiting for its user to approve a step: mail waits until its turn is done"
            }
            (true, false) => ", working on something now: mail waits until that's done",
            (false, _) => ", free",
        });
        if !session.request.is_empty() {
            line.push_str(&format!(
                "; the user last asked it: \"{}\"",
                session.request
            ));
        }
        if !session.files.is_empty() {
            line.push_str(&format!("; it changed {}", session.files.join(", ")));
        }
        for (index, part) in layout::broken(&row, room).iter().enumerate() {
            let name = if index == 0 {
                session.name.as_str()
            } else {
                ""
            };
            eprintln!("  {}", style::gray(&format!("{name:<names$}  {part}")));
        }
        result.push_str(&line);
        result.push('\n');
    }
    Outcome::Done(result)
}

/// Sends a mail unasked, shown as a step, whole: the person sees what this
/// model tells the other.
pub(super) fn mail(args: &Value, peers: &Peers) -> Result<Outcome, String> {
    let to = text_arg(args, "to")?.trim().trim_start_matches('@');
    let message = text_arg(args, "text")?.trim();
    // Left out, it's taken up at once: an answer it waits for mustn't stall.
    let now = args["now"].as_bool().unwrap_or(true);
    if message.is_empty() {
        return Err("the text is empty.".into());
    }
    if message.chars().count() > peers::MESSAGE_LIMIT {
        return Err(too_long());
    }
    if to == peers.name() {
        return Err("that's this session's own name; list_sessions lists the others.".into());
    }
    eprintln!("{}", style::mention(&layout::printable(&format!("@{to}"))));
    let width = style::terminal_width().unwrap_or(80);
    for line in layout::indented(&layout::printable(message), 2, width).lines() {
        eprintln!("{}", style::faint_italic(line));
    }
    const NO_WAITING: &str = "Any answer comes as mail: end your turn rather than wait for it.";
    match peers.send(to, message, now, false) {
        Ok(Then::Now) => Ok(Outcome::Done(format!(
            "Sent. {to}'s model takes it up now. {NO_WAITING}"
        ))),
        Ok(Then::AfterTurn) => Ok(Outcome::Done(format!(
            "Sent. {to} is busy; its model takes it up once it's free. {NO_WAITING}"
        ))),
        Ok(Then::Later) => Ok(Outcome::Done(format!(
            "Sent. {to}'s model reads it with its next turn."
        ))),
        Ok(Then::User) => Ok(Outcome::Done(format!(
            "Sent. It waits in {to}'s terminal for its user to pass it on, so an answer may take a \
             while or not come."
        ))),
        Ok(Then::Held) => Ok(Outcome::Done(format!(
            "Sent, but {to}'s user stopped the last turn mail started there, so this one waits for \
             their next request. An answer may take a while; tell your user if it matters."
        ))),
        Ok(Then::Limit) => Ok(Outcome::Done(format!(
            "Sent, but {to} has taken up as much mail on its own in a row as it may, so this one \
             waits there for its user. Tell your user, so they can step in."
        ))),
        Err(Undelivered::NoSuch) => Err(format!(
            "there's no session called {to} open; list_sessions lists them."
        )),
        Err(Undelivered::Full) => Err(format!(
            "{to} has {} mails waiting already, as many as it takes. Tell the user, or try again later.",
            peers::WAITING_LIMIT
        )),
        Err(Undelivered::TooLong) => Err(too_long()),
    }
}

fn too_long() -> String {
    format!(
        "a mail takes at most {} characters: shorten it.",
        text::number(peers::MESSAGE_LIMIT as u64)
    )
}
