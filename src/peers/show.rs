//! What the person and the model see of mail and of the other sessions:
//! the notices above the `>`, the mail framed for the model, and the words
//! for what a session is doing.

use std::path::Path;

use super::{Message, Session};
use unicode_width::UnicodeWidthStr;

use crate::layout::{broken, indented, right_padding};
use crate::screen::Layout;
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
/// last asked and the files it changed. Lines ready to print at `width`.
pub fn session_rows(sessions: &[Session], asked: bool, width: usize) -> Vec<String> {
    let names = sessions
        .iter()
        .map(|session| session.name.chars().count())
        .max()
        .unwrap_or(0);
    let room = width.saturating_sub(names + 5).max(20);
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
/// gray, broken to fit the terminal under its own start, and `time` faint
/// at the right of its first row where it has room (#46).
pub fn notice_line(text: &str, width: usize, time: Option<&str>) -> String {
    let mut out = String::new();
    for (index, part) in first_parts(text, width).iter().enumerate() {
        let lead = if index == 0 {
            format!("  {} ", style::faint("·"))
        } else {
            " ".repeat(4)
        };
        out.push_str(&format!("{lead}{}", style::gray(part)));
        if index == 0
            && let Some(time) = time
            && let Some(spaces) = right_padding(lead_and(part), time.width(), width)
        {
            out.push_str(&format!("{}{}", " ".repeat(spaces), style::faint(time)));
        }
        out.push('\n');
    }
    out
}

/// `text` broken to fit under the lead at `width`.
fn first_parts(text: &str, width: usize) -> Vec<String> {
    broken(text, width.saturating_sub(5).max(20))
}

/// Where a notice's first row ends: its lead is four columns wide, and a
/// name may hold wide characters.
fn lead_and(part: &str) -> usize {
    4 + part.width()
}

/// A mail as it shows at any width, so it's laid out again when the width
/// changes (#49): who sent it and from where, the mail as it came,
/// `footer`, what becomes of it, and a blank line after it and before it,
/// unless one printed just before already left one. When it came is
/// decided now, at the width now: a time shows only when its minute
/// changed (#46).
pub(super) fn notice(message: &Message, footer: Option<&str>, after_another: bool) -> Layout {
    let width = tty::width().unwrap_or(80);
    let from = if message.by_user {
        text::MAIL_FROM_YOU.to_string()
    } else {
        text::mail_from(&message.from)
    };
    let first = first_parts(&from, width)
        .first()
        .cloned()
        .unwrap_or_default();
    let time = clock::shown(message.arrived, |time| {
        right_padding(lead_and(&first), time, width).is_some()
    });
    let text = message.text.clone();
    let footer = footer.unwrap_or_default().to_string();
    Box::new(move |width| {
        let mut notice = notice_line(&from, width, time.as_deref());
        notice.push_str(&indented(&text, 4, width));
        for line in indented(&footer, 4, width).lines() {
            notice.push_str(&format!("{}\n", style::faint(line)));
        }
        spaced(&notice, after_another)
    })
}

/// A notice with a blank line after it, and before it too unless one
/// printed just before already left one.
fn spaced(notice: &str, after_another: bool) -> String {
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
            arrived: std::time::SystemTime::now(),
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
        let line = notice_line("api-1:", 40, Some("10:40"));
        assert_eq!(line, format!("  · api-1:{}10:40\n", " ".repeat(24)));
        assert_eq!(line.trim_end_matches('\n').width(), 39);
        // Without room, the row stays as it was.
        assert_eq!(notice_line("api-1:", 12, Some("10:40")), "  · api-1:\n");
        // A name of wide characters takes their columns, not their count.
        let wide = notice_line("日本語-1:", 40, Some("10:40"));
        assert_eq!(wide.trim_end_matches('\n').width(), 39, "{wide}");
    }

    #[test]
    fn a_mail_is_laid_out_again_at_another_width() {
        let message = Message {
            from: "api-1".into(),
            folder: "/".into(),
            text: "The tests pass now, all of them, on both machines.".into(),
            by_user: false,
            now: true,
            shown: None,
            arrived: std::time::SystemTime::now(),
        };
        let layout = notice(&message, Some("Goes with the next turn."), false);
        let wide = layout(80);
        assert!(wide.starts_with("\n  · api-1:"), "{wide}");
        assert!(wide.contains("    The tests pass now, all of them, on both machines.\n"));
        let narrow = layout(30);
        assert!(narrow.contains("    The tests pass now, all\n"), "{narrow}");
        assert!(narrow.ends_with("Goes with the next turn.\n\n"));
    }
}
