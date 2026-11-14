//! The remember tool: the one way the model changes user.md, a line at a
//! time, shown as a diff and written only with a yes.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde_json::Value;

use super::{Asking, Outcome, propose, text_arg};
use crate::input::Input;
use crate::{config, style, text};

/// The longest line `remember` adds to user.md.
const NOTE_LIMIT: usize = 200;

/// The one way the model changes user.md: a line the person asked it to
/// remember, shown as a diff and written only with a yes.
pub(super) fn remember(args: &Value, input: &mut Input) -> Result<Outcome, String> {
    let note = text_arg(args, "note")?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if note.is_empty() {
        return Err("the note is empty.".into());
    }
    if note.chars().count() > NOTE_LIMIT {
        return Err(format!("keep the note under {NOTE_LIMIT} characters."));
    }
    let replaces = args["replaces"]
        .as_str()
        .filter(|line| !line.trim().is_empty());
    let path = config::user_md_path();
    let old = current_notes(&path)?;
    let Some(new) = remembered(&old, &note, replaces)? else {
        showln!(
            "{} {}",
            style::accent(text::STEP_REMEMBER),
            config::shown(&path)
        );
        showln!("  {}", style::gray(text::ALREADY_REMEMBERED));
        return Ok(Outcome::Done(
            "It's already in user.md, so nothing changed.".into(),
        ));
    };
    // On No, in every mode: it changes what every conversation starts with.
    let outcome = propose(
        &path,
        text::STEP_REMEMBER,
        &old,
        &new,
        input,
        Asking::OnNo,
        None,
    )?;
    // user.md is Lokal's, not among the files the turn wrote.
    Ok(match outcome {
        Outcome::Wrote { result, .. } => Outcome::Done(result),
        other => other,
    })
}

/// What user.md holds now, or nothing when it isn't there yet. One Lokal
/// can't read, such as one saved in another encoding, is never taken for
/// empty: the change would then put one line in place of all of them.
fn current_notes(path: &Path) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(old) => Ok(old),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!(
            "can't read {} ({error}), so nothing changed. Tell the user: they can fix it \
             with /prefs.",
            path.display()
        )),
    }
}

/// user.md with `note` added at its end, or in place of the line `replaces`
/// names, whose copies go too. None when that changes nothing, as for a
/// note already there.
fn remembered(old: &str, note: &str, replaces: Option<&str>) -> Result<Option<String>, String> {
    let line = format!("- {note}\n");
    let new = match replaces {
        None if old.lines().any(|existing| same_note(existing, note)) => return Ok(None),
        None => {
            let mut new = old.to_string();
            if !new.is_empty() && !new.ends_with('\n') {
                new.push('\n');
            }
            new + &line
        }
        Some(replaces) => {
            if !old.lines().any(|existing| same_note(existing, replaces)) {
                return Err(format!(
                    "user.md has no line \"{replaces}\". Copy the line exactly, or leave \
                     replaces out to add a new one."
                ));
            }
            let mut new = String::new();
            let mut placed = false;
            for existing in old.lines() {
                if !same_note(existing, replaces) {
                    new.push_str(existing);
                    new.push('\n');
                } else if !placed {
                    new.push_str(&line);
                    placed = true;
                }
            }
            new
        }
    };
    Ok((new != old).then_some(new))
}

/// Two lines say the same when they differ only in a leading "- " or "* ",
/// in spacing or in case.
fn same_note(line: &str, note: &str) -> bool {
    fn plain(text: &str) -> String {
        let text = text.trim();
        let text = text
            .strip_prefix("- ")
            .or_else(|| text.strip_prefix("* "))
            .unwrap_or(text);
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }
    plain(line) == plain(note)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn remember_adds_a_note_once() {
        let old = "How I like answers: short\n- uses vim\n";
        assert_eq!(
            remembered(old, "works in Rust", None).unwrap().unwrap(),
            "How I like answers: short\n- uses vim\n- works in Rust\n"
        );
        assert_eq!(remembered(old, "Uses  Vim", None).unwrap(), None);
        assert_eq!(
            remembered("- uses vim", "works in Rust", None)
                .unwrap()
                .unwrap(),
            "- uses vim\n- works in Rust\n"
        );
        assert_eq!(
            remembered("", "uses vim", None).unwrap().unwrap(),
            "- uses vim\n"
        );
    }

    #[test]
    fn remember_never_takes_a_user_md_it_cant_read_for_empty() {
        let dir = env::temp_dir().join(format!("lokal-notes-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("user.md");
        assert_eq!(current_notes(&path).unwrap(), "");
        fs::write(&path, b"- caf\xe9 com leite\n").unwrap();
        assert!(
            current_notes(&path)
                .unwrap_err()
                .contains("nothing changed")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remember_replaces_a_line_and_its_copies() {
        let old = "- uses vim\n- works in Rust\n- uses vim\n";
        assert_eq!(
            remembered(old, "uses helix", Some("- uses vim"))
                .unwrap()
                .unwrap(),
            "- uses helix\n- works in Rust\n"
        );
        // The same note in place of itself only drops the copies.
        assert_eq!(
            remembered(old, "uses vim", Some("uses vim"))
                .unwrap()
                .unwrap(),
            "- uses vim\n- works in Rust\n"
        );
        assert_eq!(
            remembered("- uses vim\n", "uses vim", Some("uses vim")).unwrap(),
            None
        );
        assert!(
            remembered(old, "uses helix", Some("uses emacs"))
                .unwrap_err()
                .contains("no line \"uses emacs\"")
        );
    }
}
