//! The tools the model can call: run a command, read a file, write a file,
//! edit a file, and, in a conversation, list the person's other sessions and
//! mail one. Anything that changes something asks first, but where the
//! person chose a mode that asks less (`mode.rs`).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::chat::ToolCall;
use crate::input::{Approval, Input};
use crate::judge::{self, Verdict};
use crate::mode::Mode;
use crate::peers::{self, Peers};
use crate::{config, diff, layout, mode, style, text, tty};
use files::FILE_LIMIT;

mod command;
mod files;
mod mail;
mod mcp;
mod remember;

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
pub enum Outcome {
    /// The result for the model.
    Done(String),
    /// Images for a model that sees: the text that goes with them, and
    /// their data URLs.
    Images { text: String, urls: Vec<String> },
    /// A file of the project's was written: the result for the model, and
    /// the path as the model gave it, for auto's judge.
    Wrote { result: String, path: PathBuf },
    /// The person said no: the result for the model, and the turn ends so
    /// they can say what to do instead.
    Declined(String),
    /// The person pressed Ctrl+C.
    Stopped,
}

/// The tools, with the ones for other sessions when this one listens, and
/// images when the model sees: the same for the whole session, so the
/// server's prompt cache holds.
pub fn definitions(sessions: bool, images: bool) -> Value {
    let read = if images {
        "Read a text file, or look at an image (PNG, JPEG or WebP). "
    } else {
        "Read a text file. "
    };
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
            &format!(
                "{read}A long file comes back cut short, saying the line to read on from; \
                 read a part with from_line and to_line."
            ),
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
/// size is known; `scratch` is the session's folder, `peers` the other
/// sessions, when this one listens, and `servers` the MCP servers whose
/// tools are offered.
#[allow(clippy::too_many_arguments)]
pub fn run(
    call: &ToolCall,
    input: &mut Input,
    mode: Mode,
    case: &judge::Case,
    room: Option<usize>,
    scratch: Option<&Path>,
    peers: Option<&Peers>,
    servers: Option<&mut crate::mcp::Mcp>,
) -> Outcome {
    let judge = (mode == Mode::Auto).then_some(case);
    let result = match serde_json::from_str::<Value>(&call.arguments) {
        Ok(args) => match (call.name.as_str(), peers) {
            ("run_command", _) => {
                command::run_command(&args, input, room, judge, &everyday_folders(scratch))
            }
            ("read_file", _) => files::read_file(&args, room, case.server.vision),
            ("write_file", _) => files::write_file(&args, input, mode, judge, scratch, peers),
            ("edit_file", _) => files::edit_file(&args, input, mode, judge, scratch, peers),
            ("remember", _) => remember::remember(&args, input),
            ("list_sessions", Some(peers)) => Ok(mail::list_sessions(peers)),
            ("mail", Some(peers)) => mail::mail(&args, peers),
            (other, _) => match servers {
                Some(servers) => match servers.find(other) {
                    Some(index) => {
                        mcp::call(servers, index, &args, input, room, case.server.vision)
                    }
                    None => Err(format!("there's no tool called {other}.")),
                },
                None => Err(format!("there's no tool called {other}.")),
            },
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

/// The folders where a step is everyday: the working folder, unless it's
/// the home folder or /, which would hold every file, and the scratch folder.
fn everyday_folders(scratch: Option<&Path>) -> Vec<PathBuf> {
    let home = config::resolved(&config::home());
    let working = std::env::current_dir()
        .ok()
        .map(|folder| config::resolved(&folder))
        .filter(|folder| *folder != home && folder.parent().is_some());
    working
        .into_iter()
        .chain(scratch.map(config::resolved))
        .collect()
}

/// Whether a file change asks, and where its question starts.
enum Asking<'a> {
    OnNo,
    OnYes,
    /// Where edits are accepted: in the working or scratch folder.
    Not,
    /// In auto, elsewhere: the judge decides, and its no asks on No.
    Judge(&'a judge::Case<'a>, judge::Change),
}

/// Auto's judge on a step, with auto's warning the first time it acts, and
/// why it asks when it does. None when the person stopped it.
fn judged(ask: impl FnOnce() -> Option<Verdict>) -> Option<Verdict> {
    if mode::warn_of_auto() {
        print_indented(text::AUTO_WARNING, style::gray);
    }
    let verdict = ask()?;
    match &verdict {
        Verdict::No(reason) => print_indented(&text::judge_asks(reason), style::gray),
        Verdict::Unread => print_indented(text::JUDGE_UNREAD, style::gray),
        Verdict::Yes(_) => {}
    }
    Some(verdict)
}

/// A line in a step, going on under its own start where it doesn't fit,
/// each row painted on its own.
fn print_indented(text: &str, paint: fn(&str) -> String) {
    let width = tty::width().unwrap_or(80);
    for line in layout::indented(text, 2, width).lines() {
        eprintln!("{}", paint(line));
    }
}

/// Shows the change as a diff and writes it if the person agrees, or
/// without asking when `asking` says so, or auto's judge does. The other sessions, when given,
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
    let (asking, ran) = match asking {
        Asking::Judge(case, change) => {
            let path = config::resolved(path).display().to_string();
            match judged(|| judge::ask_write(&path, change, case)) {
                None => return Ok(Outcome::Stopped),
                Some(Verdict::Yes(reason)) => (Asking::Not, Some(reason)),
                Some(_) => (Asking::OnNo, None),
            }
        }
        asking => (asking, None),
    };
    if !matches!(asking, Asking::Not)
        && let Some(outcome) = refused(input.approve_write(rest, matches!(asking, Asking::OnYes)))
    {
        return Ok(outcome);
    }
    config::write(path, new).map_err(|error| format!("can't write {}: {error}", path.display()))?;
    match ran {
        Some(reason) => print_indented(&text::judge_wrote(&reason), style::faint),
        None if matches!(asking, Asking::Not) => {
            eprintln!("  {}", style::faint(text::WRITTEN_UNASKED));
        }
        None => {}
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
}
