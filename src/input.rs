//! Reading from the person: the `>` prompt, with slash-command hints and
//! completion, the status of the last turn and Ctrl+C twice to quit; and the
//! yes/no questions before anything changes.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustyline::completion::Completer;
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{
    Behavior, Cmd, CompletionType, ConditionalEventHandler, Config, Context, Editor, Event,
    EventContext, EventHandler, KeyEvent, Movement, RepeatCount,
};

use crate::{menu, style, text};

const PROMPT: &str = "> ";

pub enum Line {
    Text(String),
    /// Ctrl+C twice, or Ctrl+D, on an empty line.
    Quit,
}

pub enum Approval {
    Yes,
    /// Yes, and the same command runs without asking for the rest of the session.
    YesForSession,
    /// The turn ends here, so the person can say what to do instead.
    No,
    /// Ctrl+C: stop the whole turn.
    Stop,
    /// There's no terminal to ask on.
    Unavailable,
}

pub struct Input {
    /// None without a terminal: then nothing is asked, so nothing is approved.
    editor: Option<Editor<Helper, DefaultHistory>>,
    /// Set by a first Ctrl+C on an empty line; the next one quits.
    quit_armed: Arc<AtomicBool>,
    /// Commands the person said not to ask about again. Exact text, this
    /// session only, never saved: a pattern would let in commands nobody saw.
    approved: HashSet<String>,
}

impl Input {
    pub fn new() -> Self {
        let quit_armed = Arc::new(AtomicBool::new(false));
        // Answers piped into stdin don't count as approvals.
        let has_terminal = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .is_ok();
        if !has_terminal {
            return Input {
                editor: None,
                quit_armed,
                approved: HashSet::new(),
            };
        }
        let config = Config::builder()
            .completion_type(CompletionType::List)
            .completion_show_all_if_ambiguous(true)
            // Ask on the terminal even when stdin or stdout are redirected.
            .behavior(Behavior::PreferTerm)
            .build();
        let editor = Editor::with_config(config).ok().map(|mut editor| {
            editor.set_helper(Some(Helper {
                status: None,
                width: 0,
                quit_armed: quit_armed.clone(),
                accented: false,
            }));
            editor
        });
        Input {
            editor,
            quit_armed,
            approved: HashSet::new(),
        }
    }

    /// The terminal's width right now, if there's a terminal.
    pub fn width(&mut self) -> Option<usize> {
        let (columns, _) = self.editor.as_mut()?.dimensions()?;
        Some(columns as usize)
    }

    /// Reads a request at the `>` prompt. `status` (the model's time and
    /// the share of the context in use) shows at the far right of the line.
    pub fn request(&mut self, status: Option<String>) -> Line {
        let width = self.width().unwrap_or(0);
        let Some(editor) = &mut self.editor else {
            return Line::Quit;
        };
        if let Some(helper) = editor.helper_mut() {
            helper.status = status;
            helper.width = width;
            helper.accented = true;
        }
        // Only here: at an approval, one Ctrl+C stops the turn.
        let ctrl_c = QuitOnSecondCtrlC {
            armed: self.quit_armed.clone(),
        };
        editor.bind_sequence(
            KeyEvent::ctrl('C'),
            EventHandler::Conditional(Box::new(ctrl_c)),
        );
        let line = editor.readline(PROMPT);
        editor.unbind_sequence(KeyEvent::ctrl('C'));
        // A reminder left over must not show at the next question.
        self.quit_armed.store(false, Ordering::SeqCst);
        match line {
            Ok(line) => {
                if !line.trim().is_empty() {
                    let _ = editor.add_history_entry(line.as_str());
                }
                Line::Text(line)
            }
            Err(_) => Line::Quit,
        }
    }

    /// Asks a yes/no question; Enter alone means no.
    /// Asks before something changes. The choice starts on No, so Enter
    /// alone never approves. `for_session` offers not asking again.
    pub fn approve(&mut self, question: &str, for_session: bool) -> Approval {
        if self.editor.is_none() {
            return Approval::Unavailable;
        }
        discard_typed_ahead();
        let mut options = vec![(text::CHOICE_YES, 'y', Approval::Yes)];
        if for_session {
            options.push((text::CHOICE_YES_SESSION, 'a', Approval::YesForSession));
        }
        options.push((text::CHOICE_NO, 'n', Approval::No));
        let items: Vec<menu::Item> = options
            .iter()
            .map(|(label, key, _)| menu::Item { label, key: *key })
            .collect();
        match menu::choose(question, &items, items.len() - 1) {
            Some(menu::Choice::Picked(index)) => options.swap_remove(index).2,
            Some(menu::Choice::Interrupted) => Approval::Stop,
            None => Approval::Unavailable,
        }
    }

    pub fn is_approved_for_session(&self, command: &str) -> bool {
        self.approved.contains(command)
    }

    pub fn approve_for_session(&mut self, command: &str) {
        self.approved.insert(command.to_string());
    }

    /// Asks a free question; None if it was skipped.
    pub fn answer(&mut self, prompt: &str) -> Option<String> {
        let answer = self.ask(prompt)?.ok()?;
        let answer = answer.trim();
        (!answer.is_empty()).then(|| answer.to_string())
    }

    fn ask(&mut self, prompt: &str) -> Option<Result<String, ReadlineError>> {
        let editor = self.editor.as_mut()?;
        if let Some(helper) = editor.helper_mut() {
            helper.status = None;
            helper.accented = false;
        }
        Some(editor.readline(prompt))
    }
}

/// Keys pressed while Lokal was working were pressed before the question was
/// on screen, so they can't be its answer: a "y" typed early would approve a
/// command nobody has seen.
fn discard_typed_ahead() {
    if let Ok(terminal) = OpenOptions::new().read(true).open("/dev/tty") {
        // SAFETY: a plain call on a descriptor that stays open for it.
        unsafe {
            libc::tcflush(terminal.as_raw_fd(), libc::TCIFLUSH);
        }
    }
}

/// Ctrl+C at the `>` prompt: clears what's typed; on an empty line, it asks
/// for a second press, and that one quits.
struct QuitOnSecondCtrlC {
    armed: Arc<AtomicBool>,
}

impl ConditionalEventHandler for QuitOnSecondCtrlC {
    fn handle(
        &self,
        _event: &Event,
        _count: RepeatCount,
        _positive: bool,
        context: &EventContext,
    ) -> Option<Cmd> {
        if !context.line().is_empty() {
            self.armed.store(false, Ordering::SeqCst);
            return Some(Cmd::Kill(Movement::WholeLine));
        }
        if self.armed.swap(true, Ordering::SeqCst) {
            // Quit the way Ctrl+D does, which also clears the reminder from the line.
            Some(Cmd::EndOfFile)
        } else {
            // Redraw, so the reminder shows.
            Some(Cmd::Repaint)
        }
    }
}

struct Helper {
    status: Option<String>,
    width: usize,
    quit_armed: Arc<AtomicBool>,
    /// The `>` line is in the accent color, so the person's messages stand
    /// out in the conversation. Questions aren't.
    accented: bool,
}

impl Helper {
    /// The reminder after a first Ctrl+C, and the status at the right edge.
    fn line_hint(&self, line: &str) -> Option<Hint> {
        let armed = line.is_empty() && self.quit_armed.load(Ordering::SeqCst);
        let mut display = if armed {
            text::QUIT_AGAIN.to_string()
        } else {
            String::new()
        };
        if let Some(status) = &self.status {
            let used = PROMPT.len()
                + line.chars().count()
                + display.chars().count()
                + status.chars().count();
            // Stay off the last column, or the terminal wraps the line.
            if let Some(padding) = self
                .width
                .checked_sub(used + 1)
                .filter(|padding| *padding >= 2)
            {
                display.push_str(&" ".repeat(padding));
                display.push_str(status);
            }
        }
        (!display.is_empty()).then_some(Hint {
            display,
            completion: None,
        })
    }

    /// The reminder and the status are the dimmest things on screen.
    fn is_faint(&self, hint: &str) -> bool {
        hint.starts_with(text::QUIT_AGAIN)
            || self
                .status
                .as_ref()
                .is_some_and(|status| hint.ends_with(status.as_str()))
    }
}

/// The commands matching what's typed. For the only match: the rest of its
/// name, then what it does.
fn command_hint(line: &str, pos: usize) -> Option<Hint> {
    if pos < line.len() {
        return None;
    }
    let matches: Vec<&(&str, &str)> = text::COMMANDS
        .iter()
        .filter(|(name, _)| name.starts_with(line))
        .collect();
    match matches.as_slice() {
        [] => None,
        [(name, about)] => {
            let rest = &name[line.len()..];
            Some(Hint {
                display: format!("{rest}  {about}"),
                completion: Some(rest.to_string()),
            })
        }
        several => {
            let names: Vec<&str> = several.iter().map(|(name, _)| *name).collect();
            Some(Hint {
                display: format!("   {}", names.join("  ")),
                completion: None,
            })
        }
    }
}

struct Hint {
    display: String,
    completion: Option<String>,
}

impl rustyline::hint::Hint for Hint {
    fn display(&self) -> &str {
        &self.display
    }

    fn completion(&self) -> Option<&str> {
        self.completion.as_deref()
    }
}

impl Hinter for Helper {
    type Hint = Hint;

    fn hint(&self, line: &str, pos: usize, _context: &Context<'_>) -> Option<Hint> {
        // Typing anything means the person isn't quitting after all.
        if !line.is_empty() {
            self.quit_armed.store(false, Ordering::SeqCst);
        }
        if line.starts_with('/') {
            command_hint(line, pos)
        } else {
            self.line_hint(line)
        }
    }
}

impl Highlighter for Helper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        if self.accented {
            Cow::Owned(style::accent(prompt))
        } else {
            Cow::Borrowed(prompt)
        }
    }

    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if self.accented {
            Cow::Owned(style::accent(line))
        } else {
            Cow::Borrowed(line)
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        if self.is_faint(hint) {
            Cow::Owned(style::faint(hint))
        } else {
            Cow::Owned(style::gray(hint))
        }
    }

    /// Without this, a typed character is drawn as is, without the color.
    fn highlight_char(&self, _line: &str, _pos: usize, _kind: CmdKind) -> bool {
        self.accented
    }
}

impl Completer for Helper {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _context: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        let typed = &line[..pos];
        if !typed.starts_with('/') || typed.contains(' ') {
            return Ok((0, Vec::new()));
        }
        let names = text::COMMANDS
            .iter()
            .map(|(name, _)| name.to_string())
            .filter(|name| name.starts_with(typed))
            .collect();
        Ok((0, names))
    }
}

impl Validator for Helper {}

impl rustyline::Helper for Helper {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_alone_lists_every_command() {
        let hint = command_hint("/", 1).unwrap();
        assert_eq!(hint.display, "   /new  /prefs  /help  /about  /exit");
        assert_eq!(hint.completion, None);
    }

    #[test]
    fn one_match_completes_and_explains() {
        let hint = command_hint("/pr", 3).unwrap();
        assert_eq!(hint.display, "efs  edit what Lokal knows about you");
        assert_eq!(hint.completion.as_deref(), Some("efs"));
        assert!(command_hint("/xyz", 4).is_none());
    }
}
