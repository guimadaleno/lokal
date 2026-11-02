//! The run_command tool, and the person's own `!` commands: who decides
//! whether one runs, and what the model hears of it.

use serde_json::Value;

use super::{CONTEXT_FULL, LEAST_ROOM, OUTPUT_LIMIT, Outcome, clip, limit, refused, text_arg};
use crate::gitconfig::{self, WriteProgram};
use crate::input::Input;
use crate::judge::{self, Verdict};
use crate::shell::{self, Run};
use crate::{allowlist, interrupt, layout, mode, style, text};

/// Runs a command, asking first unless it's read-only. In auto, `judge`
/// holds what the judge reads, and it decides what would open on Yes.
pub(super) fn run_command(
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
        Run::Finished {
            output,
            hidden_lines,
            code,
        } => Ok(Outcome::Done(command_result(
            &output,
            hidden_lines,
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
        Ok(Run::Finished {
            output,
            hidden_lines,
            code,
        }) => Ok(command_result(
            &output,
            hidden_lines,
            code,
            limit(room, OUTPUT_LIMIT),
        )),
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

/// What the model hears of a command: its output, cut to `limit`, and how
/// it ended. `hidden_lines` is how many line breaks the shell left out of
/// the output's middle.
fn command_result(output: &str, hidden_lines: usize, code: Option<i32>, limit: usize) -> String {
    let mut result = if output.trim().is_empty() {
        "(no output)".to_string()
    } else if limit < LEAST_ROOM {
        format!(
            "The command ran, but there's no room left in the context for its output. {CONTEXT_FULL}"
        )
    } else {
        clip(output, limit, hidden_lines)
    };
    match code {
        Some(0) => {}
        Some(code) => result.push_str(&format!("\n[exit code {code}]")),
        None => result.push_str("\n[ended by a signal]"),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_result_notes_failures() {
        assert_eq!(command_result("", 0, Some(0), OUTPUT_LIMIT), "(no output)");
        assert_eq!(
            command_result("oops\n", 0, Some(2), OUTPUT_LIMIT),
            "oops\n\n[exit code 2]"
        );
    }

    #[test]
    fn command_result_without_room_still_says_it_ran() {
        let result = command_result("lots of output\n", 0, Some(1), 10);
        assert!(result.starts_with("The command ran, but there's no room left"));
        assert!(result.ends_with("[exit code 1]"));
    }
}
