//! The tools the model can call: run a command, read a file, write a file,
//! edit a file, and, in a conversation, list the person's other sessions and
//! mail one. Anything that changes something asks first, but where the
//! person chose a mode that asks less (`mode.rs`).

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::chat::ToolCall;
use crate::input::{Approval, Input};
use crate::judge;
use crate::mode::Mode;
use crate::peers::{self, Peers, Then, Undelivered};
use crate::{config, diff, layout, style, text};

mod command;
mod files;

pub use command::run_own;

/// How much command output the model gets; longer output loses its middle.
pub const OUTPUT_LIMIT: usize = 12_000;
/// With less room than this left in the context, an output would be too
/// little to use.
const LEAST_ROOM: usize = 1_000;
/// Ends what the model is told when the context has no room left: a clear
/// stop, or it keeps asking for less and less.
const CONTEXT_FULL: &str = "Answer with what you have, or, if that isn't enough, tell the user \
    the context is full: /new starts a fresh conversation.";
/// The longest line `remember` adds to user.md.
const NOTE_LIMIT: usize = 200;
pub enum Outcome {
    /// The result for the model.
    Done(String),
    /// A file of the project's was written: the result for the model, and
    /// the path as the model gave it, for auto's judge.
    Wrote { result: String, path: PathBuf },
    /// The person said no: the result for the model, and the turn ends so
    /// they can say what to do instead.
    Declined(String),
    /// The person pressed Ctrl+C.
    Stopped,
}

/// The tools, with the ones for other sessions when this one listens: the
/// same for the whole session, so the server's prompt cache holds.
pub fn definitions(sessions: bool) -> Value {
    let mut tools = vec![
        tool(
            "run_command",
            "Run a bash command. Returns its output (stdout and stderr together) and, if it failed, its exit code.",
            json!({
                "command": {"type": "string", "description": "The command line to run."},
                "explanation": {
                    "type": "string",
                    "description": "One short sentence in the user's language, shown when they're asked to approve: \
                        what the command will do, plainly, including anything big it does \
                        (downloads, installs, deletes, and how much)."
                }
            }),
            &["command", "explanation"],
        ),
        tool(
            "read_file",
            "Read a text file. A long file comes back cut short, saying the line to read on from; \
             read a part with from_line and to_line.",
            json!({
                "path": {"type": "string"},
                "from_line": {"type": "integer", "description": "First line to read, counting from 1."},
                "to_line": {"type": "integer", "description": "Last line to read."}
            }),
            &["path"],
        ),
        tool(
            "write_file",
            "Create a file, or replace everything in it. Missing folders are created.",
            json!({"path": {"type": "string"}, "content": {"type": "string"}}),
            &["path", "content"],
        ),
        tool(
            "edit_file",
            "Replace one piece of text in a file. old_text must appear exactly once in the file: \
             copy it exactly, with enough surrounding lines to make it unique.",
            json!({"path": {"type": "string"}, "old_text": {"type": "string"}, "new_text": {"type": "string"}}),
            &["path", "old_text", "new_text"],
        ),
        tool(
            "remember",
            "Add one short line to the notes every conversation starts with (user.md), or update \
             a line already there with replaces. Only when the user asks you to remember something; \
             they approve the change first. A note already there isn't added again. \
             Something only about this project goes in its LOKAL.md instead, changed like any file.",
            json!({
                "note": {"type": "string", "description": "The fact or preference, in one short line."},
                "replaces": {
                    "type": "string",
                    "description": "A line already in user.md that the note updates, copied exactly; \
                        its copies go too. Leave it out to add a new line."
                }
            }),
            &["note"],
        ),
    ];
    if sessions {
        tools.push(tool(
            "list_sessions",
            "List the user's other Lokal sessions open on this computer: each one's name, folder, \
             branch and what it's doing.",
            json!({}),
            &[],
        ));
        tools.push(tool(
            "mail",
            &format!(
                "Send a message to another of the user's Lokal sessions. Only what it needs: a \
                 question, an answer, a decision; never just thanks. At most {} characters.",
                text::number(peers::MESSAGE_LIMIT as u64)
            ),
            json!({
                "to": {"type": "string", "description": "The session's name, from list_sessions."},
                "text": {"type": "string"},
                "now": {
                    "type": "boolean",
                    "description": "true when it should take this up right away: a question, a request, \
                        an answer it waits for. false when it can wait for its next turn: news, thanks."
                }
            }),
            &["to", "text", "now"],
        ));
    }
    Value::Array(tools)
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {"type": "object", "properties": properties, "required": required}
        }
    })
}

/// Runs one tool call in `mode`, with what auto's judge reads in `case`.
/// `room` is how many characters its output may take, when the context's
/// size is known; `scratch` is the session's folder, and `peers` the other
/// sessions, when this one listens.
pub fn run(
    call: &ToolCall,
    input: &mut Input,
    mode: Mode,
    case: &judge::Case,
    room: Option<usize>,
    scratch: Option<&Path>,
    peers: Option<&Peers>,
) -> Outcome {
    let judge = (mode == Mode::Auto).then_some(case);
    let result = match serde_json::from_str::<Value>(&call.arguments) {
        Ok(args) => match (call.name.as_str(), peers) {
            ("run_command", _) => command::run_command(&args, input, room, judge),
            ("read_file", _) => files::read_file(&args, room),
            ("write_file", _) => files::write_file(&args, input, mode, scratch, peers),
            ("edit_file", _) => files::edit_file(&args, input, mode, scratch, peers),
            ("remember", _) => remember(&args, input),
            ("list_sessions", Some(peers)) => Ok(list_sessions(peers)),
            ("mail", Some(peers)) => mail(&args, peers),
            (other, _) => Err(format!("there's no tool called {other}.")),
        },
        // Not repeated back: they can be a whole file.
        Err(error) => Err(format!("the arguments aren't valid JSON ({error}).")),
    };
    result.unwrap_or_else(|error| Outcome::Done(format!("Error: {error}")))
}

/// A path the model gave, as the person reads it.
fn shown(path: &Path) -> String {
    layout::printable(&config::shown(path)).into_owned()
}

/// The one way the model changes user.md: a line the person asked it to
/// remember, shown as a diff and written only with a yes.
fn remember(args: &Value, input: &mut Input) -> Result<Outcome, String> {
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
        eprintln!(
            "{} {}",
            style::accent(text::STEP_REMEMBER),
            config::shown(&path)
        );
        eprintln!("  {}", style::gray(text::ALREADY_REMEMBERED));
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

/// The other sessions open, on screen and for the model.
fn list_sessions(peers: &Peers) -> Outcome {
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
fn mail(args: &Value, peers: &Peers) -> Result<Outcome, String> {
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

/// Whether a file change asks, and where its question starts.
#[derive(Clone, Copy, PartialEq)]
enum Asking {
    OnNo,
    OnYes,
    /// Where it would start on Yes, with edits accepted.
    Not,
}

/// How a file change is put to the person in `mode`, from whether its
/// question would open on Yes (`write_opens_on_yes`).
fn asking(opens_on_yes: bool, mode: Mode) -> Asking {
    match (opens_on_yes, mode) {
        (false, _) => Asking::OnNo,
        (true, Mode::Manual) => Asking::OnYes,
        (true, Mode::AcceptEdits | Mode::Auto) => Asking::Not,
    }
}

/// Shows the change as a diff and writes it if the person agrees, or
/// without asking when `asking` says so. The other sessions, when given,
/// hear which file changed.
fn propose(
    path: &Path,
    step: &str,
    old: &str,
    new: &str,
    input: &mut Input,
    asking: Asking,
    peers: Option<&Peers>,
) -> Result<Outcome, String> {
    eprintln!("{} {}", style::accent(step), shown(path));
    let rest = diff::show(old, new);
    if asking != Asking::Not
        && let Some(outcome) = refused(input.approve_write(rest, asking == Asking::OnYes))
    {
        return Ok(outcome);
    }
    config::write(path, new).map_err(|error| format!("can't write {}: {error}", path.display()))?;
    if asking == Asking::Not {
        eprintln!("  {}", style::faint(text::WRITTEN_UNASKED));
    }
    if let Some(peers) = peers {
        peers.wrote(path);
    }
    Ok(Outcome::Wrote {
        result: format!("Wrote {}.", path.display()),
        path: path.to_path_buf(),
    })
}

/// None if the person approved; otherwise what to tell the model.
fn refused(approval: Approval) -> Option<Outcome> {
    match approval {
        Approval::Yes | Approval::YesForSession | Approval::YesForProject => None,
        Approval::Stop => Some(Outcome::Stopped),
        Approval::No => Some(Outcome::Declined(
            "The user declined, and will say what to do instead.".into(),
        )),
        Approval::Unavailable => {
            eprintln!("  {}", style::gray(text::NO_TERMINAL));
            Some(Outcome::Done(
                "Not done. Lokal is running without a terminal, so it can't ask the user, and \
                 nothing beyond reading will be allowed in this run. Don't try other ways; tell \
                 the user what you would have done."
                    .into(),
            ))
        }
    }
}

fn text_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args[name]
        .as_str()
        .ok_or_else(|| format!("the \"{name}\" argument is missing."))
}

/// What an output may take: the tool's own limit, or the room left in the
/// context when that's less.
fn limit(room: Option<usize>, own: usize) -> usize {
    room.map_or(own, |room| room.min(own))
}

/// Keeps the start and the end of a long text, cut at line breaks: that's
/// where the useful part of an output usually is. `hidden_lines` line
/// breaks were already left out of its middle, and count among those cut.
fn clip(text: &str, limit: usize, hidden_lines: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let head = text.floor_char_boundary(limit / 2);
    let head = text[..head].rfind('\n').map_or(head, |newline| newline + 1);
    let tail = text.ceil_char_boundary(text.len() - limit / 2);
    let tail = text[tail..]
        .find('\n')
        .map_or(tail, |newline| tail + newline + 1);
    let first_cut = text[..head].lines().count() + 1;
    let last_cut = first_cut + text[head..tail].lines().count().max(1) - 1 + hidden_lines;
    format!(
        "{}[... lines {first_cut}-{last_cut} left out ...]\n{}",
        &text[..head],
        &text[tail..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn clip_keeps_head_and_tail_whole_lines() {
        let text: String = (1..=1000).map(|n| format!("line {n}\n")).collect();
        let clipped = clip(&text, 2000, 0);
        assert!(clipped.len() < 2100);
        assert!(clipped.starts_with("line 1\n"));
        assert!(clipped.ends_with("line 1000\n"));
        let marker = clipped
            .lines()
            .find(|line| line.starts_with("[..."))
            .unwrap();
        let first_kept_after: usize = clipped
            .split(marker)
            .nth(1)
            .unwrap()
            .lines()
            .nth(1)
            .unwrap()[5..]
            .parse()
            .unwrap();
        let last_before: usize = clipped
            .split(marker)
            .next()
            .unwrap()
            .lines()
            .last()
            .unwrap()[5..]
            .parse()
            .unwrap();
        assert_eq!(
            marker,
            format!(
                "[... lines {}-{} left out ...]",
                last_before + 1,
                first_kept_after - 1
            )
        );
    }

    #[test]
    fn clip_leaves_short_text_alone() {
        assert_eq!(clip("short\n", 100, 0), "short\n");
    }

    #[test]
    fn clip_counts_the_lines_the_shell_left_out() {
        let text: String = (1..=1000).map(|n| format!("line {n}\n")).collect();
        let whole = clip(&text, 2000, 0);
        // As if lines 401 to 600 had never been kept.
        let start = text.find("line 401\n").unwrap();
        let end = text.find("line 601\n").unwrap();
        let kept = format!("{}{}", &text[..start], &text[end..]);
        assert_eq!(clip(&kept, 2000, 200), whole);
    }

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

    #[test]
    fn accept_edits_skips_only_the_questions_that_open_on_yes() {
        assert!(asking(true, Mode::Manual) == Asking::OnYes);
        assert!(asking(true, Mode::AcceptEdits) == Asking::Not);
        assert!(asking(false, Mode::Manual) == Asking::OnNo);
        assert!(asking(false, Mode::AcceptEdits) == Asking::OnNo);
        assert!(asking(true, Mode::Auto) == Asking::Not);
        assert!(asking(false, Mode::Auto) == Asking::OnNo);
    }
}
