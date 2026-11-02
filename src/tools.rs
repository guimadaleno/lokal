//! The tools the model can call: run a command, read a file, write a file,
//! edit a file, and, in a conversation, list the person's other sessions and
//! mail one. Anything that changes something asks first.

use std::env;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::chat::ToolCall;
use crate::gitconfig::WriteProgram;
use crate::input::{Approval, Input};
use crate::judge::{self, Verdict};
use crate::mode::{self, Mode};
use crate::peers::{self, Peers, Then, Undelivered};
use crate::shell::{self, Run};
use crate::{allowlist, config, diff, gitconfig, interrupt, layout, style, text};

/// How much command output the model gets; longer output loses its middle.
const OUTPUT_LIMIT: usize = 12_000;
/// How much of a file one read gives the model; a longer one says where to read on.
const FILE_LIMIT: usize = 30_000;
/// The most of a file read into memory, for a part of it or a change to it.
const READ_CAP: u64 = 64 * 1024 * 1024;
/// With less room than this left in the context, an output would be too
/// little to use.
const LEAST_ROOM: usize = 1_000;
/// Ends what the model is told when the context has no room left: a clear
/// stop, or it keeps asking for less and less.
const CONTEXT_FULL: &str = "Answer with what you have, or, if that isn't enough, tell the user \
    the context is full: /new starts a fresh conversation.";
/// The longest line `remember` adds to user.md.
const NOTE_LIMIT: usize = 200;
/// How a file change's result starts, for the model and for `written_path`.
const WROTE: &str = "Wrote";

pub enum Outcome {
    /// The result for the model.
    Done(String),
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
            ("run_command", _) => run_command(&args, input, room, judge),
            ("read_file", _) => read_file(&args, room),
            ("write_file", _) => write_file(&args, input, mode, scratch, peers),
            ("edit_file", _) => edit_file(&args, input, mode, scratch, peers),
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

/// Runs a command, asking first unless it's read-only. In auto, `judge`
/// holds what the judge reads, and it decides what would open on Yes.
fn run_command(
    args: &Value,
    input: &mut Input,
    room: Option<usize>,
    judge: Option<&judge::Case>,
) -> Result<Outcome, String> {
    let command = text_arg(args, "command")?;
    print_command(command);
    // git also runs what the repository's own settings name: then even a
    // read is asked about, on No, whatever was approved before.
    let git_warning = if allowlist::git_elsewhere(command) {
        Some(text::GIT_ELSEWHERE.to_string())
    } else if allowlist::uses_git(command) {
        match gitconfig::program_setting() {
            Ok(None) => None,
            Ok(Some(setting)) => Some(text::git_runs_program(&setting)),
            Err(()) => Some(text::GIT_SETTINGS_UNREAD.to_string()),
        }
    } else {
        None
    };
    let mut unasked = allowlist::is_read_only(command) && git_warning.is_none();
    if !unasked {
        eprintln!();
        // A setting's name can hold any character but a line break.
        if let Some(warning) = &git_warning {
            eprintln!("  {}", layout::printable(warning));
        }
        if let Some(note) = input
            .approved_before(command)
            .filter(|_| git_warning.is_none())
        {
            eprintln!("  {}", style::faint(note));
        } else {
            let mut on_yes = allowlist::opens_on_yes(command) && git_warning.is_none();
            // The judge sees the command, never the hooks or programs the
            // repository has git run as it writes: those are the person's call.
            if on_yes
                && judge.is_some()
                && allowlist::uses_git(command)
                && let Some(warning) = write_warning()
            {
                eprintln!("  {}", layout::printable(&warning));
                on_yes = false;
            }
            // What the model says the command does, where the person decides on it.
            let explanation = args["explanation"].as_str().map_or("", str::trim);
            if !explanation.is_empty() {
                eprintln!("  {}", style::gray(&layout::printable(explanation)));
            }
            // The judge only ever sees what would open on Yes.
            let verdict = match judge.filter(|_| on_yes) {
                Some(case) => match judged(command, explanation, case) {
                    Some(verdict) => Some(verdict),
                    None => return Ok(Outcome::Stopped),
                },
                None => None,
            };
            if let Some(Verdict::Yes(reason)) = &verdict {
                print_indented(&text::judge_ran(reason), style::faint);
                unasked = true;
            } else {
                // The judge's no, or a verdict it couldn't read, asks on No.
                let approval = input.approve(command, on_yes && verdict.is_none());
                input.keep_approval(&approval, command);
                if let Some(outcome) = refused(approval) {
                    return Ok(outcome);
                }
            }
        }
    }
    // The output is a block of its own, after the command and any question.
    match shell::run(command, unasked).map_err(|error| format!("couldn't start bash: {error}"))? {
        Run::Finished { output, code } => Ok(Outcome::Done(command_result(
            &output,
            code,
            limit(room, OUTPUT_LIMIT),
        ))),
        Run::Stopped => Ok(Outcome::Stopped),
    }
}

/// Why a git command auto would judge is asked instead: a hook or a
/// program the repository has git run as it writes. None when it has none.
fn write_warning() -> Option<String> {
    match gitconfig::write_program() {
        Ok(None) => None,
        Ok(Some(WriteProgram::Setting(setting))) => Some(text::git_runs_program(&setting)),
        Ok(Some(WriteProgram::Hook(hook))) => Some(text::git_runs_hook(&hook)),
        Err(()) => Some(text::GIT_SETTINGS_UNREAD.to_string()),
    }
}

/// Auto's judge on a command, with auto's warning the first time it acts,
/// and why it asks when it does. None when the person stopped it.
fn judged(command: &str, explanation: &str, case: &judge::Case) -> Option<Verdict> {
    if mode::warn_of_auto() {
        print_indented(text::AUTO_WARNING, style::gray);
    }
    let verdict = judge::ask(command, explanation, case)?;
    match &verdict {
        Verdict::No(reason) => print_indented(&text::judge_asks(reason), style::gray),
        Verdict::Unread => print_indented(text::JUDGE_UNREAD, style::gray),
        Verdict::Yes(_) => {}
    }
    Some(verdict)
}

/// The `$` line a command's step starts with, its later lines indented.
fn print_command(command: &str) {
    let command = layout::printable(command).replace('\n', "\n  ");
    eprintln!("{} {command}", style::accent("$"));
}

/// A path the model gave, as the person reads it.
fn shown(path: &Path) -> String {
    layout::printable(&config::shown(path)).into_owned()
}

/// A line in a step, going on under its own start where it doesn't fit,
/// each row painted on its own.
fn print_indented(text: &str, paint: fn(&str) -> String) {
    let width = style::terminal_width().unwrap_or(80);
    for line in layout::indented(text, 2, width).lines() {
        eprintln!("{}", paint(line));
    }
}

/// A command the person typed at the `>` after a `!`. It runs without a
/// question, since typing it was their yes, and without `gitconfig::UNASKED`,
/// which guards only what runs unasked. What it gave, for the model; None
/// when there's nothing to give, as when the person stopped it.
pub fn run_own(command: &str, room: Option<usize>) -> Option<String> {
    print_command(command);
    interrupt::clear();
    let result = match shell::run(command, false) {
        Ok(Run::Finished { output, code }) => {
            Ok(command_result(&output, code, limit(room, OUTPUT_LIMIT)))
        }
        Ok(Run::Stopped) => Err(None),
        Err(error) => Err(Some(text::cant_start_bash(&error.to_string()))),
    };
    if let Err(Some(message)) = &result {
        eprintln!("  {}", style::red(message));
    }
    // A blank line after the step, as after the model's.
    eprintln!();
    if let Err(None) = result {
        eprintln!("{}", style::gray(text::STOPPED));
    }
    result.ok()
}

fn command_result(output: &str, code: Option<i32>, limit: usize) -> String {
    let mut result = if output.trim().is_empty() {
        "(no output)".to_string()
    } else if limit < LEAST_ROOM {
        format!(
            "The command ran, but there's no room left in the context for its output. {CONTEXT_FULL}"
        )
    } else {
        clip(output, limit)
    };
    match code {
        Some(0) => {}
        Some(code) => result.push_str(&format!("\n[exit code {code}]")),
        None => result.push_str("\n[ended by a signal]"),
    }
    result
}

fn read_file(args: &Value, room: Option<usize>) -> Result<Outcome, String> {
    let path = path_arg(args)?;
    let from = args["from_line"].as_u64().map(|line| line.max(1) as usize);
    let to = args["to_line"].as_u64().map(|line| line as usize);
    let range = match (from, to) {
        (None, None) => String::new(),
        _ => format!(
            " {}-{}",
            from.unwrap_or(1),
            to.map_or(String::new(), |to| to.to_string())
        ),
    };
    eprintln!(
        "{} {}{}",
        style::accent(text::STEP_READ),
        shown(&path),
        style::gray(&range)
    );
    let limit = limit(room, FILE_LIMIT);
    if limit < LEAST_ROOM {
        return Ok(Outcome::Done(format!(
            "Not read: there's no room left in the context for it. {CONTEXT_FULL}"
        )));
    }
    let contents = read_text(&path)?.ok_or_else(|| no_such_file(&path))?;
    read_part(&contents, from, to, limit).map(Outcome::Done)
}

/// A text file's contents, or None when there's no such file. Anything but
/// a regular file is refused before it's opened: a device or a pipe could
/// never end, or never begin. So is one too big to hold in memory.
fn read_text(path: &Path) -> Result<Option<String>, String> {
    match fs::metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("can't read {}: {error}", path.display())),
        Ok(file) if !file.is_file() => {
            return Err(format!("{} isn't a regular file.", path.display()));
        }
        Ok(_) => {}
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(READ_CAP + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("can't read {}: {error}", path.display()))?;
    if bytes.len() as u64 > READ_CAP {
        return Err(format!(
            "{} is over {} MB, too big to read here. Use a command on it instead: \
             grep -n, head or tail.",
            path.display(),
            READ_CAP / 1024 / 1024
        ));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| format!("{} isn't a text file.", path.display()))
}

fn no_such_file(path: &Path) -> String {
    format!("can't read {}: there's no such file.", path.display())
}

/// The lines from `from` to `to`, or the whole file, as many whole lines as
/// fit in `limit` characters. A part cut short says where to read on, so the
/// model reads the rest instead of guessing at it.
fn read_part(
    contents: &str,
    from: Option<usize>,
    to: Option<usize>,
    limit: usize,
) -> Result<String, String> {
    if from.is_none() && to.is_none() && contents.len() <= limit {
        return Ok(contents.to_string());
    }
    let lines: Vec<&str> = contents.lines().collect();
    let total = lines.len();
    let from = from.unwrap_or(1);
    if from > total {
        return Err(format!(
            "the file has {total} lines, so line {from} is past its end."
        ));
    }
    let to = to.unwrap_or(total).min(total);
    if to < from {
        return Err(format!("to_line {to} comes before from_line {from}."));
    }
    let part = &lines[from - 1..to];
    let mut size = 0;
    let fitting = part
        .iter()
        .take_while(|line| {
            size += line.len() + 1;
            size <= limit
        })
        .count();
    if fitting == part.len() {
        return Ok(format!(
            "Lines {from}-{to} of {total}:\n{}",
            part.join("\n")
        ));
    }
    if fitting == 0 {
        let start = &part[0][..part[0].floor_char_boundary(limit)];
        return Ok(format!(
            "Line {from} of {total} is too long to show whole; here's its start:\n{start}"
        ));
    }
    let last = from + fitting - 1;
    Ok(format!(
        "Lines {from}-{last} of {total}; the rest doesn't fit in one read. These lines get dropped \
         as the context fills and your reasoning isn't kept: write down in your reply the facts \
         from them your answer needs, then read on from line {}:\n{}",
        last + 1,
        part[..fitting].join("\n")
    ))
}

fn write_file(
    args: &Value,
    input: &mut Input,
    mode: Mode,
    scratch: Option<&Path>,
    peers: Option<&Peers>,
) -> Result<Outcome, String> {
    let path = writable_path(args)?;
    let content = text_arg(args, "content")?;
    let old = read_text(&path)?;
    let step = if old.is_some() {
        text::STEP_EDIT
    } else {
        text::STEP_CREATE
    };
    propose(
        &path,
        step,
        old.as_deref().unwrap_or(""),
        content,
        input,
        asking(write_opens_on_yes(&path, scratch), mode),
        peers,
    )
}

fn edit_file(
    args: &Value,
    input: &mut Input,
    mode: Mode,
    scratch: Option<&Path>,
    peers: Option<&Peers>,
) -> Result<Outcome, String> {
    let path = writable_path(args)?;
    let old_text = text_arg(args, "old_text")?;
    let new_text = text_arg(args, "new_text")?;
    if old_text.is_empty() {
        return Err("old_text is empty.".into());
    }
    let old = read_text(&path)?.ok_or_else(|| no_such_file(&path))?;
    match old.matches(old_text).count() {
        0 => {
            return Err(format!(
                "old_text isn't in {}. Read the file again and copy the text exactly.",
                path.display()
            ));
        }
        1 => {}
        count => {
            return Err(format!(
                "old_text appears {count} times in {}. Include more surrounding lines so it appears once.",
                path.display()
            ));
        }
    }
    let new = old.replacen(old_text, new_text, 1);
    let asking = asking(write_opens_on_yes(&path, scratch), mode);
    propose(&path, text::STEP_EDIT, &old, &new, input, asking, peers)
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
    propose(
        &path,
        text::STEP_REMEMBER,
        &old,
        &new,
        input,
        Asking::OnNo,
        None,
    )
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
    if old == new && step == text::STEP_EDIT {
        return Ok(Outcome::Done(
            "Nothing to do: the file already has that content.".into(),
        ));
    }
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
    Ok(Outcome::Done(format!("{WROTE} {}.", path.display())))
}

/// The file a call wrote, if it wrote one, as auto's judge hears of it.
pub fn written_path(call: &ToolCall, outcome: &Outcome) -> Option<String> {
    let Outcome::Done(result) = outcome else {
        return None;
    };
    if !matches!(call.name.as_str(), "write_file" | "edit_file") || !result.starts_with(WROTE) {
        return None;
    }
    let args = serde_json::from_str::<Value>(&call.arguments).ok()?;
    path_arg(&args).ok().map(|path| path.display().to_string())
}

/// Whether a file change's question may open on Yes: only for a file in the
/// working folder or the scratch folder. A working folder that is the home
/// folder or / would hold everything, so then it doesn't count.
fn write_opens_on_yes(path: &Path, scratch: Option<&Path>) -> bool {
    let home = config::resolved(&config::home());
    let working = env::current_dir()
        .ok()
        .map(|folder| config::resolved(&folder))
        .filter(|folder| *folder != home && folder.parent().is_some());
    let folders: Vec<PathBuf> = working
        .into_iter()
        .chain(scratch.map(config::resolved))
        .collect();
    is_everyday_place(&config::resolved(path), &folders)
}

/// Inside one of `folders`, with no hidden file or folder on the way there.
/// Hidden ones are mostly settings that change what other programs run
/// (git's config and hooks, CI, a shell's start-up, an editor's tasks), or
/// they hold secrets, like .env.
fn is_everyday_place(path: &Path, folders: &[PathBuf]) -> bool {
    folders.iter().any(|folder| {
        path.strip_prefix(folder).is_ok_and(|inside| {
            !inside
                .components()
                .any(|part| part.as_os_str().as_encoded_bytes().starts_with(b"."))
        })
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

/// The path argument, when it's somewhere the model may write.
fn writable_path(args: &Value) -> Result<PathBuf, String> {
    let path = path_arg(args)?;
    if config::is_lokal_own(&path) {
        return Err(format!(
            "{} is one of Lokal's own files (its settings, user.md, its saved state), \
             which you can't change. To keep something the user asked you to remember, use \
             remember; the user edits these files with /prefs and /config.",
            path.display()
        ));
    }
    Ok(path)
}

/// The path argument, with ~ expanded. Relative paths start at the working directory.
fn path_arg(args: &Value) -> Result<PathBuf, String> {
    let path = text_arg(args, "path")?;
    Ok(match path.strip_prefix("~/") {
        Some(rest) => config::home().join(rest),
        None if path == "~" => config::home(),
        None => PathBuf::from(path),
    })
}

/// What an output may take: the tool's own limit, or the room left in the
/// context when that's less.
fn limit(room: Option<usize>, own: usize) -> usize {
    room.map_or(own, |room| room.min(own))
}

/// Keeps the start and the end of a long text, cut at line breaks: that's
/// where the useful part of an output usually is.
fn clip(text: &str, limit: usize) -> String {
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
    let last_cut = first_cut + text[head..tail].lines().count().max(1) - 1;
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
        let clipped = clip(&text, 2000);
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
        assert_eq!(clip("short\n", 100), "short\n");
    }

    #[test]
    fn command_result_notes_failures() {
        assert_eq!(command_result("", Some(0), OUTPUT_LIMIT), "(no output)");
        assert_eq!(
            command_result("oops\n", Some(2), OUTPUT_LIMIT),
            "oops\n\n[exit code 2]"
        );
    }

    #[test]
    fn command_result_without_room_still_says_it_ran() {
        let result = command_result("lots of output\n", Some(1), 10);
        assert!(result.starts_with("The command ran, but there's no room left"));
        assert!(result.ends_with("[exit code 1]"));
    }

    fn numbered(count: usize) -> String {
        (1..=count).map(|n| format!("line {n}\n")).collect()
    }

    #[test]
    fn read_part_gives_a_file_that_fits_as_it_is() {
        let file = numbered(3);
        assert_eq!(read_part(&file, None, None, 1000).unwrap(), file);
        assert_eq!(
            read_part(&file, Some(2), None, 1000).unwrap(),
            "Lines 2-3 of 3:\nline 2\nline 3"
        );
    }

    #[test]
    fn read_part_cuts_at_a_line_and_says_where_to_read_on() {
        // Each line is 7 or 8 characters with its newline: 25 fit in 200.
        let file = numbered(100);
        let part = read_part(&file, None, None, 200).unwrap();
        let (header, body) = part.split_once('\n').unwrap();
        let last = body.lines().count();
        assert!(header.starts_with(&format!("Lines 1-{last} of 100; the rest doesn't fit")));
        assert!(header.ends_with(&format!("read on from line {}:", last + 1)));
        assert_eq!(body.lines().last().unwrap(), format!("line {last}"));
        assert!(body.len() <= 200);
        let next = read_part(&file, Some(last + 1), None, 200).unwrap();
        assert!(next.starts_with(&format!("Lines {}-", last + 1)));
    }

    #[test]
    fn read_part_shows_the_start_of_a_line_too_long_for_the_room() {
        let file = format!("{}\nshort\n", "x".repeat(500));
        let part = read_part(&file, None, None, 100).unwrap();
        assert_eq!(
            part,
            format!(
                "Line 1 of 2 is too long to show whole; here's its start:\n{}",
                "x".repeat(100)
            )
        );
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
    fn only_a_regular_file_is_read() {
        assert!(read_text(Path::new("/dev/zero")).is_err());
        let dir = env::temp_dir().join(format!("lokal-read-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let pipe = dir.join("pipe");
        let name = std::ffi::CString::new(pipe.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: a valid C string; mkfifo only creates the pipe.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        // Opened, a pipe with no writer would block for good.
        assert!(
            read_text(&pipe)
                .unwrap_err()
                .contains("isn't a regular file")
        );
        assert_eq!(read_text(&dir.join("missing")).unwrap(), None);
        fs::write(dir.join("a.txt"), "hi\n").unwrap();
        assert_eq!(
            read_text(&dir.join("a.txt")).unwrap().as_deref(),
            Some("hi\n")
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
    fn writes_open_on_yes_only_in_plain_files_of_the_folders() {
        let folders = [
            PathBuf::from("/home/ana/project"),
            PathBuf::from("/tmp/lokal-1000/42"),
        ];
        for path in [
            "/home/ana/project/src/main.rs",
            "/home/ana/project/README.md",
            "/tmp/lokal-1000/42/download.html",
        ] {
            assert!(is_everyday_place(Path::new(path), &folders), "{path}");
        }
        for path in [
            "/home/ana/.bashrc",
            "/home/ana/project-other/x",
            "/home/ana/project/.env",
            "/home/ana/project/.git/config",
            "/home/ana/project/.github/workflows/ci.yml",
            "/etc/hosts",
        ] {
            assert!(!is_everyday_place(Path::new(path), &folders), "{path}");
        }
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

    #[test]
    fn read_part_refuses_lines_past_the_end() {
        let file = numbered(125);
        assert_eq!(
            read_part(&file, Some(400), Some(750), 1000).unwrap_err(),
            "the file has 125 lines, so line 400 is past its end."
        );
        assert!(read_part(&file, Some(10), Some(5), 1000).is_err());
        assert_eq!(
            read_part(&file, Some(124), Some(750), 1000).unwrap(),
            "Lines 124-125 of 125:\nline 124\nline 125"
        );
    }
}
