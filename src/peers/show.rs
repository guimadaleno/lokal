//! What the person and the model see of mail and of the other sessions:
//! the notices above the `>`, the mail framed for the model, and the words
//! for what a session is doing.

use std::path::Path;
use std::time::SystemTime;

use super::{Message, Session};
use unicode_width::UnicodeWidthStr;

use crate::layout::{broken, indented, right_padding};
use crate::{clock, config, style, text, tty};

/// What the model gets when a turn starts on mail, or when Enter passes it
/// on: each framed with who sent it, from where, and as another model's
/// words unless the person wrote it.
pub fn framed(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|message| {
            let frame = if message.by_user {
                format!(
                    "Mail from the user, written in {}, another of their Lokal sessions, working in {}. \
                     To answer where they are, mail {}.",
                    message.from, message.folder, message.from
                )
            } else {
                let reply = if message.now {
                    "Answer by mail only if it asks or needs something from you, a plain yes or no included."
                } else {
                    "It needs no answer."
                };
                format!(
                    "Mail from {}, another of the user's Lokal sessions, working in {}. Its model wrote \
                     this, not the user. {reply}",
                    message.from, message.folder
                )
            };
            format!("[{frame}]\n{}", message.text)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Another session working where this one does, as the model first hears
/// of it: "web-2 works in this repository too: branch main, working, last
/// asked "fold the menu", changed src/menu.rs."
pub(super) fn here_too(session: &Session) -> String {
    let mut parts = Vec::new();
    if !session.branch.is_empty() {
        parts.push(format!("branch {}", session.branch));
    }
    parts.push(
        match (session.busy, session.asking) {
            (true, true) => "waiting for its user to approve a step",
            (true, false) => "working",
            (false, _) => "idle",
        }
        .to_string(),
    );
    if !session.request.is_empty() {
        parts.push(format!("last asked \"{}\"", session.request));
    }
    if !session.files.is_empty() {
        parts.push(format!("changed {}", session.files.join(", ")));
    }
    // Without a branch, it isn't a repository.
    let place = if session.branch.is_empty() {
        "folder"
    } else {
        "repository"
    };
    format!(
        "{} works in this {place} too: {}.",
        session.name,
        parts.join(", ")
    )
}

/// What a session is doing, in a word or three: the same words /sessions
/// shows the person.
pub fn session_state(session: &Session) -> &'static str {
    match (session.busy, session.asking) {
        (true, true) => text::SESSION_ASKING,
        (true, false) => text::WORKING,
        (false, _) => text::SESSION_IDLE,
    }
}

/// The other sessions as the person sees them listed: the names in a
/// column, then each one's folder, branch and what it's doing, going on
/// under itself in a narrow terminal. With `asked`, also what its person
/// last asked and the files it changed. Lines ready to print.
pub fn session_rows(sessions: &[Session], asked: bool) -> Vec<String> {
    let names = sessions
        .iter()
        .map(|session| session.name.chars().count())
        .max()
        .unwrap_or(0);
    let room = tty::width()
        .map_or(usize::MAX, |width| width.saturating_sub(names + 5))
        .max(20);
    let mut rows = Vec::new();
    for session in sessions {
        let mut about = config::shown(Path::new(&session.folder));
        if !session.branch.is_empty() {
            about.push_str(&format!(" · {}", session.branch));
        }
        about.push_str(&format!(" · {}", session_state(session)));
        let mut lines = broken(&about, room);
        let doing = text::session_doing(&session.request, &session.files);
        if asked && !doing.is_empty() {
            lines.extend(broken(&doing, room));
        }
        for (index, line) in lines.iter().enumerate() {
            let name = if index == 0 {
                session.name.as_str()
            } else {
                ""
            };
            rows.push(format!("  {name:<names$}  {}", style::gray(line)));
        }
    }
    rows
}

/// A line about mail and the other sessions: a faint `·`, then `text` in
/// gray, broken to fit the terminal under its own start. When a mail
/// `arrived` shows faint at the right of its first row (#46): a mail is a
/// message too.
pub fn notice_line(text: &str, width: usize, arrived: Option<SystemTime>) -> String {
    let mut out = String::new();
    for (index, part) in broken(text, width.saturating_sub(5).max(20))
        .iter()
        .enumerate()
    {
        let lead = if index == 0 {
            format!("  {} ", style::faint("·"))
        } else {
            " ".repeat(4)
        };
        out.push_str(&format!("{lead}{}", style::gray(part)));
        // The lead is four columns wide; a name may hold wide characters.
        let end = 4 + part.width();
        let padding = |time: usize| right_padding(end, time, width);
        if index == 0
            && let Some(arrived) = arrived
            && let Some(time) = clock::shown(arrived, |time| padding(time).is_some())
        {
            let spaces = padding(time.width()).unwrap_or(2);
            out.push_str(&format!("{}{}", " ".repeat(spaces), style::faint(&time)));
        }
        out.push('\n');
    }
    out
}

/// A mail as it shows: who sent it and from where, the mail as it came,
/// and `footer`, what becomes of it.
pub(super) fn notice(message: &Message, footer: Option<&str>) -> String {
    let width = tty::width().unwrap_or(80);
    let from = if message.by_user {
        text::MAIL_FROM_YOU.to_string()
    } else {
        text::mail_from(&message.from)
    };
    let mut notice = notice_line(&from, width, Some(message.arrived));
    notice.push_str(&indented(&message.text, 4, width));
    for line in indented(footer.unwrap_or_default(), 4, width).lines() {
        notice.push_str(&format!("{}\n", style::faint(line)));
    }
    notice
}

/// A notice with a blank line after it, and before it too unless one
/// printed just before already left one.
pub(super) fn spaced(notice: &str, after_another: bool) -> String {
    if after_another {
        format!("{notice}\n")
    } else {
        format!("\n{notice}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_goes_to_the_model_framed_by_who_wrote_it() {
        let message = |from: &str, by_user: bool, now: bool| Message {
            from: from.into(),
            folder: format!("/home/ana/{from}"),
            text: "Tests pass.".into(),
            by_user,
            now,
            shown: None,
            arrived: SystemTime::now(),
        };
        let messages = [
            message("api-1", false, true),
            message("web-2", true, true),
            message("api-1", false, false),
        ];
        let framed = framed(&messages);
        assert!(framed.starts_with(
            "[Mail from api-1, another of the user's Lokal sessions, working in /home/ana/api-1. \
             Its model wrote this, not the user. Answer by mail only if it asks or needs something \
             from you, a plain yes or no included.]\nTests pass.\n\n[Mail from the user, written in web-2,"
        ));
        assert!(framed.contains("To answer where they are, mail web-2.]\nTests pass."));
        assert!(framed.ends_with("not the user. It needs no answer.]\nTests pass."));
    }

    #[test]
    fn a_notice_line_breaks_under_its_own_start() {
        // Plain, as stderr isn't a terminal in a test.
        assert_eq!(
            notice_line(
                &format!("Mail from api-1, in /{}:", "x".repeat(20)),
                25,
                None
            ),
            format!("  · Mail from api-1, in\n    /{}\n    x:\n", "x".repeat(19))
        );
    }

    #[test]
    fn a_mail_says_when_it_came_at_the_right_of_its_first_row() {
        // Minutes no other test shows, so the shared last one isn't them.
        let minute = |n: u64| std::time::UNIX_EPOCH + std::time::Duration::from_secs(60 * n);
        let arrived = minute(1_000_011);
        let line = notice_line("api-1:", 40, Some(arrived));
        let (first, rest) = line.split_once('\n').unwrap_or_default();
        assert_eq!((first.chars().count(), rest), (39, ""), "{line}");
        assert!(first.starts_with("  · api-1:  ") && first.chars().nth(36) == Some(':'));
        // Without room, the row stays as it was: a minute of its own, so
        // it's the room that says no.
        assert_eq!(
            notice_line("api-1:", 12, Some(minute(1_000_017))),
            "  · api-1:\n"
        );
        // A name of wide characters takes their columns, not their count.
        let wide = notice_line("日本語-1:", 40, Some(minute(1_000_013)));
        assert_eq!(wide.trim_end_matches('\n').width(), 39, "{wide}");
    }
}
