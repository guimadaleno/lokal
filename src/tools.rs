//! The tools the model can call: run a command, read a file, write a file,
//! edit a file. Anything that changes something asks first.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::chat::ToolCall;
use crate::input::{Approval, Input};
use crate::shell::{self, Run};
use crate::{allowlist, config, diff, style, text};

/// How much command output the model gets; longer output loses its middle.
const OUTPUT_LIMIT: usize = 12_000;
/// The same for files.
const FILE_LIMIT: usize = 30_000;
/// The longest line `remember` adds to about-me.md.
const NOTE_LIMIT: usize = 200;

pub enum Outcome {
    /// The result for the model.
    Done(String),
    /// The person said no: the result for the model, and the turn ends so
    /// they can say what to do instead.
    Declined(String),
    /// The person pressed Ctrl+C.
    Stopped,
}

pub fn definitions() -> Value {
    json!([
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
            "Read a text file. A long file comes back without its middle; read a part with from_line and to_line.",
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
            "Add one short line to the notes every conversation starts with (about-me.md). \
             Only when the user asks you to remember something; they approve the line first.",
            json!({"note": {"type": "string", "description": "The fact or preference, in one short line."}}),
            &["note"],
        ),
    ])
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

pub fn run(call: &ToolCall, input: &mut Input) -> Outcome {
    let result = match serde_json::from_str::<Value>(&call.arguments) {
        Ok(args) => match call.name.as_str() {
            "run_command" => run_command(&args, input),
            "read_file" => read_file(&args),
            "write_file" => write_file(&args, input),
            "edit_file" => edit_file(&args, input),
            "remember" => remember(&args, input),
            other => Err(format!("there's no tool called {other}.")),
        },
        Err(_) => Err(format!(
            "the arguments aren't valid JSON: {}",
            call.arguments
        )),
    };
    result.unwrap_or_else(|error| Outcome::Done(format!("Error: {error}")))
}

fn run_command(args: &Value, input: &mut Input) -> Result<Outcome, String> {
    let command = text_arg(args, "command")?;
    eprintln!("{} {}", style::accent("$"), command.replace('\n', "\n  "));
    if !allowlist::is_read_only(command) {
        eprintln!();
        if let Some(note) = input.approved_before(command) {
            eprintln!("  {}", style::faint(note));
        } else {
            // What the model says the command does, where the person decides on it.
            if let Some(explanation) = args["explanation"]
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                eprintln!("  {}", style::gray(explanation));
            }
            let approval = input.approve(command);
            input.keep_approval(&approval, command);
            if let Some(outcome) = refused(approval) {
                return Ok(outcome);
            }
        }
    }
    // The output is a block of its own, after the command and any question.
    match shell::run(command).map_err(|error| format!("couldn't start bash: {error}"))? {
        Run::Finished { output, code } => Ok(Outcome::Done(command_result(&output, code))),
        Run::Stopped => Ok(Outcome::Stopped),
    }
}

fn command_result(output: &str, code: Option<i32>) -> String {
    let mut result = if output.trim().is_empty() {
        "(no output)".to_string()
    } else {
        clip(output, OUTPUT_LIMIT)
    };
    match code {
        Some(0) => {}
        Some(code) => result.push_str(&format!("\n[exit code {code}]")),
        None => result.push_str("\n[ended by a signal]"),
    }
    result
}

fn read_file(args: &Value) -> Result<Outcome, String> {
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
        config::shown(&path),
        style::gray(&range)
    );
    let bytes =
        fs::read(&path).map_err(|error| format!("can't read {}: {error}", path.display()))?;
    let contents =
        String::from_utf8(bytes).map_err(|_| format!("{} isn't a text file.", path.display()))?;
    if from.is_none() && to.is_none() {
        return Ok(Outcome::Done(clip(&contents, FILE_LIMIT)));
    }
    let from = from.unwrap_or(1);
    let total = contents.lines().count();
    let to = to.unwrap_or(total).min(total);
    let part: Vec<&str> = contents
        .lines()
        .skip(from - 1)
        .take((to + 1).saturating_sub(from))
        .collect();
    let part = part.join("\n");
    Ok(Outcome::Done(format!(
        "Lines {from}-{to} of {total}:\n{}",
        clip(&part, FILE_LIMIT)
    )))
}

fn write_file(args: &Value, input: &mut Input) -> Result<Outcome, String> {
    let path = writable_path(args)?;
    let content = text_arg(args, "content")?;
    let old = match fs::read_to_string(&path) {
        Ok(old) => Some(old),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(format!("can't read {}: {error}", path.display())),
    };
    let step = if old.is_some() {
        text::STEP_EDIT
    } else {
        text::STEP_CREATE
    };
    propose(&path, step, old.as_deref().unwrap_or(""), content, input)
}

fn edit_file(args: &Value, input: &mut Input) -> Result<Outcome, String> {
    let path = writable_path(args)?;
    let old_text = text_arg(args, "old_text")?;
    let new_text = text_arg(args, "new_text")?;
    if old_text.is_empty() {
        return Err("old_text is empty.".into());
    }
    let old = fs::read_to_string(&path)
        .map_err(|error| format!("can't read {}: {error}", path.display()))?;
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
    propose(&path, text::STEP_EDIT, &old, &new, input)
}

/// The one way the model adds to about-me.md: a line the person asked it to
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
    let old = config::read_about_me().unwrap_or_default();
    let mut new = old.clone();
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(&format!("- {note}\n"));
    propose(
        &config::about_me_path(),
        text::STEP_REMEMBER,
        &old,
        &new,
        input,
    )
}

/// Shows the change as a diff and writes it if the person agrees.
fn propose(
    path: &Path,
    step: &str,
    old: &str,
    new: &str,
    input: &mut Input,
) -> Result<Outcome, String> {
    if old == new && step == text::STEP_EDIT {
        return Ok(Outcome::Done(
            "Nothing to do: the file already has that content.".into(),
        ));
    }
    eprintln!("{} {}", style::accent(step), config::shown(path));
    let rest = diff::show(old, new);
    if let Some(outcome) = refused(input.approve_write(rest)) {
        return Ok(outcome);
    }
    config::write(path, new).map_err(|error| format!("can't write {}: {error}", path.display()))?;
    Ok(Outcome::Done(format!("Wrote {}.", path.display())))
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
            "{} is one of Lokal's own files (its settings, about-me.md, its saved state), \
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
        assert_eq!(command_result("", Some(0)), "(no output)");
        assert_eq!(command_result("oops\n", Some(2)), "oops\n\n[exit code 2]");
    }
}
