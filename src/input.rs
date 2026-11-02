//! Reading from the person: the `>` prompt, with slash-command hints and
//! completion, the status of the last turn, Shift+Tab for the mode, Ctrl+C
//! twice to quit, and the mail from other sessions above it; and the yes/no
//! questions before anything changes.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashSet;
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
    EventContext, EventHandler, ExternalPrinter, KeyCode, KeyEvent, Modifiers, Movement,
    RepeatCount,
};

use crate::approvals::ProjectApprovals;
use crate::peers::{Directory, Inbox};
use crate::tty::{self, discard_typed_ahead};
use crate::{config, diff, menu, mode, style, text};

pub const PROMPT: &str = "> ";
/// How long after Esc another key makes it part of one, like an arrow
/// key's. Without it, rustyline waits for the next key, and a lone Esc does
/// nothing until then.
const ESC_WAIT_MS: u16 = 50;

pub enum Line {
    Text(String),
    /// A turn starts on mail.
    Mail,
    /// Esc on an empty line, with mail waiting.
    DropMessages,
    /// Ctrl+C twice, or Ctrl+D, on an empty line.
    Quit,
}

pub enum Approval {
    Yes,
    /// Yes, and the same command runs without asking for the rest of the session.
    YesForSession,
    /// Yes, and the same command runs without asking in this project, from now on.
    YesForProject,
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
    /// The same, kept for the project.
    project: ProjectApprovals,
    /// Mail from the person's other sessions, when this one listens.
    inbox: Option<Inbox>,
}

impl Input {
    pub fn new(project: ProjectApprovals) -> Self {
        let quit_armed = Arc::new(AtomicBool::new(false));
        // Answers piped into stdin don't count as approvals.
        let has_terminal = tty::open().is_some();
        if !has_terminal {
            return Input {
                editor: None,
                quit_armed,
                approved: HashSet::new(),
                project,
                inbox: None,
            };
        }
        let config = Config::builder()
            .completion_type(CompletionType::List)
            .completion_show_all_if_ambiguous(true)
            // Ask on the terminal even when stdin or stdout are redirected.
            .behavior(Behavior::PreferTerm)
            .keyseq_timeout(Some(ESC_WAIT_MS))
            .build();
        let editor = Editor::with_config(config).ok().map(|mut editor| {
            editor.set_helper(Some(Helper {
                status: None,
                quit_armed: quit_armed.clone(),
                accented: false,
                inbox: None,
                sessions: None,
                command_hint: Cell::new(false),
            }));
            editor
        });
        Input {
            editor,
            quit_armed,
            approved: HashSet::new(),
            project,
            inbox: None,
        }
    }

    /// What prints a mail above the `>` while the person types, from
    /// another thread. None without a terminal.
    pub fn printer(&mut self) -> Option<Box<dyn ExternalPrinter + Send>> {
        let printer = self.editor.as_mut()?.create_external_printer().ok()?;
        Some(Box::new(printer))
    }

    /// Mail shows at the `>` from now on, and Tab completes the other
    /// sessions' names after an `@`.
    pub fn receive(&mut self, inbox: Inbox, sessions: Directory) {
        if let Some(helper) = self.editor.as_mut().and_then(Editor::helper_mut) {
            helper.inbox = Some(inbox.clone());
            helper.sessions = Some(sessions);
        }
        self.inbox = Some(inbox);
    }

    /// The terminal's width right now, if there's a terminal.
    pub fn width(&self) -> Option<usize> {
        self.editor.as_ref()?;
        style::terminal_width()
    }

    /// Reads a request at the `>` prompt. `status` (the model's time and
    /// the share of the context in use) shows at the far right of the line.
    /// Mail that starts a turn returns `Line::Mail`, whether it waited for
    /// the `>` or ended it.
    pub fn request(&mut self, status: Option<String>) -> Line {
        let Some(editor) = &mut self.editor else {
            return Line::Quit;
        };
        if self.inbox.as_ref().is_some_and(Inbox::enter_prompt) {
            return Line::Mail;
        }
        if let Some(helper) = editor.helper_mut() {
            helper.status = status;
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
        let shift_tab = KeyEvent(KeyCode::BackTab, Modifiers::NONE);
        editor.bind_sequence(shift_tab, EventHandler::Conditional(Box::new(NextMode)));
        let esc = KeyEvent(KeyCode::Esc, Modifiers::NONE);
        let dropping = Arc::new(AtomicBool::new(false));
        if let Some(inbox) = &self.inbox {
            let drop = DropOnEsc {
                inbox: inbox.clone(),
                pressed: dropping.clone(),
            };
            editor.bind_sequence(esc, EventHandler::Conditional(Box::new(drop)));
        }
        let line = editor.readline(PROMPT);
        let woken = self.inbox.as_ref().is_some_and(Inbox::leave_prompt);
        if self.inbox.is_some() {
            editor.unbind_sequence(esc);
        }
        editor.unbind_sequence(shift_tab);
        editor.unbind_sequence(KeyEvent::ctrl('C'));
        // A reminder left over must not show at the next question.
        self.quit_armed.store(false, Ordering::SeqCst);
        match line {
            Err(ReadlineError::Interrupted) if woken => {
                // Mail ended an empty line, which rustyline leaves with the
                // status on it: it goes, and the mail shows in its place.
                if style::enabled() {
                    eprint!("\x1b[1A\r\x1b[2K");
                }
                Line::Mail
            }
            Ok(_) if dropping.load(Ordering::SeqCst) => Line::DropMessages,
            Ok(line) => {
                if !line.trim().is_empty() {
                    let _ = editor.add_history_entry(line.as_str());
                }
                Line::Text(line)
            }
            Err(_) => Line::Quit,
        }
    }

    /// Asks before a command runs, and offers not to ask again about this
    /// exact command. The choice starts on Yes only where Lokal's rule says
    /// a slip costs little (`allowlist::opens_on_yes`), and Esc is always No.
    pub fn approve(&mut self, command: &str, opens_on_yes: bool) -> Approval {
        if self.editor.is_none() {
            return Approval::Unavailable;
        }
        discard_typed_ahead();
        let mut options = vec![
            (text::CHOICE_YES, 'y', Approval::Yes),
            (text::CHOICE_YES_SESSION, 'a', Approval::YesForSession),
        ];
        if self.project.can_keep(command) {
            options.push((text::CHOICE_YES_PROJECT, 'p', Approval::YesForProject));
        }
        options.push((text::CHOICE_NO, 'n', Approval::No));
        let items: Vec<menu::Item> = options
            .iter()
            .map(|(label, key, _)| menu::Item { label, key: *key })
            .collect();
        let no = items.len() - 1;
        let start = if opens_on_yes { 0 } else { no };
        match self.choose(text::RUN_QUESTION, &items, start, no) {
            Some(menu::Choice::Picked(index)) => options.swap_remove(index).2,
            Some(menu::Choice::Interrupted) => Approval::Stop,
            None => Approval::Unavailable,
        }
    }

    /// Asks before a file changes, starting on Yes only when `opens_on_yes`,
    /// with Esc always No; a file change never gets "don't ask again". When
    /// the diff showed only its start, `rest` holds the lines left out, and
    /// a choice between Yes and No prints them and asks again.
    pub fn approve_write(&mut self, mut rest: Vec<String>, opens_on_yes: bool) -> Approval {
        if self.editor.is_none() {
            return Approval::Unavailable;
        }
        loop {
            // Again after the rest: keys pressed while it printed came
            // before the question.
            discard_typed_ahead();
            let mut items = vec![menu::Item {
                label: text::CHOICE_YES,
                key: 'y',
            }];
            if !rest.is_empty() {
                items.push(menu::Item {
                    label: text::CHOICE_SHOW_REST,
                    key: 's',
                });
            }
            items.push(menu::Item {
                label: text::CHOICE_NO,
                key: 'n',
            });
            let no = items.len() - 1;
            let start = if opens_on_yes { 0 } else { no };
            match self.choose(text::WRITE_QUESTION, &items, start, no) {
                Some(menu::Choice::Picked(0)) => return Approval::Yes,
                Some(menu::Choice::Picked(index)) if index < no => {
                    diff::print(&std::mem::take(&mut rest));
                }
                Some(menu::Choice::Picked(_)) => return Approval::No,
                Some(menu::Choice::Interrupted) => return Approval::Stop,
                None => return Approval::Unavailable,
            }
        }
    }

    /// An approval question; the other sessions see this one waits for a
    /// yes meanwhile.
    fn choose(
        &self,
        question: &str,
        items: &[menu::Item],
        start: usize,
        safe: usize,
    ) -> Option<menu::Choice> {
        if let Some(inbox) = &self.inbox {
            inbox.set_asking(true);
        }
        let choice = menu::choose(question, items, start, safe);
        if let Some(inbox) = &self.inbox {
            inbox.set_asking(false);
        }
        choice
    }

    /// Why a command may run without asking again, if it may.
    pub fn approved_before(&self, command: &str) -> Option<&'static str> {
        if self.approved.contains(command) {
            Some(text::APPROVED_FOR_SESSION)
        } else if self.project.contains(command) {
            Some(text::APPROVED_FOR_PROJECT)
        } else {
            None
        }
    }

    /// Keeps a "don't ask again" for as long as the person chose.
    pub fn keep_approval(&mut self, approval: &Approval, command: &str) {
        match approval {
            Approval::YesForSession => {
                self.approved.insert(command.to_string());
            }
            Approval::YesForProject => {
                if let Err(error) = self.project.add(command) {
                    let path = config::shown(&config::approvals_path());
                    eprintln!(
                        "  {}",
                        style::red(&text::cant_save(&path, &error.to_string()))
                    );
                }
            }
            _ => {}
        }
    }

    pub fn project(&self) -> &ProjectApprovals {
        &self.project
    }

    /// Asks a free question; None if it was skipped.
    pub fn answer(&mut self, prompt: &str) -> Option<String> {
        self.answer_with(prompt, "")
    }

    /// The same, with the line already holding `typed`, to change or keep.
    pub fn answer_with(&mut self, prompt: &str, typed: &str) -> Option<String> {
        let editor = self.editor.as_mut()?;
        if let Some(helper) = editor.helper_mut() {
            helper.status = None;
            helper.accented = false;
        }
        let answer = editor.readline_with_initial(prompt, (typed, "")).ok()?;
        let answer = answer.trim();
        (!answer.is_empty()).then(|| answer.to_string())
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

/// Shift+Tab at the `>`: the next mode, which shows at the right at once.
struct NextMode;

impl ConditionalEventHandler for NextMode {
    fn handle(
        &self,
        _event: &Event,
        _count: RepeatCount,
        _positive: bool,
        _context: &EventContext,
    ) -> Option<Cmd> {
        mode::next();
        Some(Cmd::Repaint)
    }
}

/// Esc on an empty line, while mail from other sessions waits: ends the
/// line, so `request` says they're to be dropped. Otherwise Esc does what it
/// always did.
struct DropOnEsc {
    inbox: Inbox,
    pressed: Arc<AtomicBool>,
}

impl ConditionalEventHandler for DropOnEsc {
    fn handle(
        &self,
        _event: &Event,
        _count: RepeatCount,
        _positive: bool,
        context: &EventContext,
    ) -> Option<Cmd> {
        if !context.line().is_empty() || self.inbox.count() == 0 {
            return None;
        }
        self.pressed.store(true, Ordering::SeqCst);
        Some(Cmd::AcceptLine)
    }
}

struct Helper {
    status: Option<String>,
    quit_armed: Arc<AtomicBool>,
    /// The `>` line is in the accent color, so the person's messages stand
    /// out in the conversation. Questions aren't.
    accented: bool,
    inbox: Option<Inbox>,
    /// The other sessions, whose names Tab completes after an `@`.
    sessions: Option<Directory>,
    /// The hint showing is a command's, in gray, rather than the reminder or the status.
    command_hint: Cell<bool>,
}

impl Helper {
    /// What shows at the right edge: the mail waiting, at the `>` only,
    /// where Enter and Esc act on it, and the mode, where Shift+Tab changes
    /// it; then the status.
    fn status(&self) -> Option<String> {
        let waiting = self
            .inbox
            .as_ref()
            .filter(|_| self.accented)
            .map(Inbox::count)
            .filter(|count| *count > 0)
            .map(text::mail_waiting);
        let mode = text::mode_name(mode::current())
            .filter(|_| self.accented)
            .map(String::from);
        let parts: Vec<String> = waiting
            .into_iter()
            .chain(mode)
            .chain(self.status.clone())
            .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// The reminder after a first Ctrl+C, and the status at the right edge.
    fn line_hint(&self, line: &str) -> Option<Hint> {
        let armed = line.is_empty() && self.quit_armed.load(Ordering::SeqCst);
        let mut display = if armed {
            text::QUIT_AGAIN.to_string()
        } else {
            String::new()
        };
        if let Some(status) = &self.status() {
            let used = PROMPT.len()
                + line.chars().count()
                + display.chars().count()
                + status.chars().count();
            // Read on every redraw, so the status follows the window as it's
            // resized. Off the last column, or the terminal wraps the line.
            let width = style::terminal_width().unwrap_or(0);
            if let Some(padding) = width.checked_sub(used + 1).filter(|padding| *padding >= 2) {
                display.push_str(&" ".repeat(padding));
                display.push_str(status);
            }
        }
        (!display.is_empty()).then_some(Hint {
            display,
            completion: None,
        })
    }
}

/// The commands matching what's typed. For the only match: the rest of its
/// name, then what it does. Several are listed in the room `width` leaves,
/// whole names only and … when some don't fit, so the line never wraps.
fn command_hint(line: &str, pos: usize, width: Option<usize>) -> Option<Hint> {
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
            // One column short of the edge, so the terminal doesn't wrap it.
            let room = width.map_or(usize::MAX, |width| {
                width.saturating_sub(PROMPT.len() + line.chars().count() + 1)
            });
            let mut display = String::from(" ");
            for (index, (name, _)) in several.iter().enumerate() {
                // Room for the … stays, in case the next one doesn't fit.
                let ellipsis = if index + 1 < several.len() { 3 } else { 0 };
                if display.chars().count() + 2 + name.len() + ellipsis > room {
                    display.push_str("  …");
                    break;
                }
                display.push_str("  ");
                display.push_str(name);
            }
            Some(Hint {
                display,
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
        // Mail doesn't take away a line the person is writing.
        if let Some(inbox) = self.inbox.as_ref().filter(|_| self.accented) {
            inbox.set_typing(!line.is_empty());
        }
        // Kept for the hint's color: mail arriving redraws the hint shown,
        // though what it would say has changed.
        self.command_hint.set(line.starts_with('/'));
        if line.starts_with('/') {
            command_hint(line, pos, style::terminal_width())
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

    /// The reminder and the status are the dimmest things on screen; the
    /// mode among them has its own color.
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        if self.command_hint.get() {
            return Cow::Owned(style::gray(hint));
        }
        let mode = mode::current();
        if let Some(name) = text::mode_name(mode).filter(|_| self.accented)
            && let Some(at) = hint.find(name)
        {
            let (before, rest) = hint.split_at(at);
            let after = &rest[name.len()..];
            return Cow::Owned(format!(
                "{}{}{}",
                style::faint(before),
                style::mode(name, mode),
                style::faint(after)
            ));
        }
        Cow::Owned(style::faint(hint))
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
        if typed.contains(' ') {
            return Ok((0, Vec::new()));
        }
        let names = if typed.starts_with('/') {
            text::COMMANDS
                .iter()
                .map(|(name, _)| name.to_string())
                .collect()
        } else if let Some(sessions) = self.sessions.as_ref().filter(|_| typed.starts_with('@')) {
            sessions
                .names()
                .into_iter()
                .map(|name| format!("@{name}"))
                .collect()
        } else {
            Vec::new()
        };
        let names = names
            .into_iter()
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
        let hint = command_hint("/", 1, None).unwrap();
        assert_eq!(
            hint.display,
            "   /new  /prefs  /memory  /config  /info  /recon  /update  /reasoning  /sysprompt  /sessions  /feedback  /help  /about  /exit"
        );
        assert_eq!(hint.completion, None);
    }

    #[test]
    fn a_narrow_terminal_lists_the_commands_that_fit() {
        let hint = command_hint("/", 1, Some(40)).unwrap();
        assert_eq!(hint.display, "   /new  /prefs  /memory  /config  …");
        // With the prompt and the slash, one column short of the edge.
        assert_eq!("> /".len() + hint.display.chars().count(), 39);
    }

    #[test]
    fn one_match_completes_and_explains() {
        let hint = command_hint("/pr", 3, None).unwrap();
        assert_eq!(hint.display, "efs  edit what Lokal knows about you");
        assert_eq!(hint.completion.as_deref(), Some("efs"));
        assert!(command_hint("/xyz", 4, None).is_none());
    }
}
