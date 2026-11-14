//! Reading from the person: the `>` prompt, with slash-command hints and
//! completion, the status of the last turn, Shift+Tab for the mode, Ctrl+C
//! twice to quit, and the mail from other sessions above it; and the yes/no
//! questions before anything changes.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
use crate::tty::{self, RawMode, discard_typed_ahead};
use crate::{clipboard, clock, config, diff, layout, menu, mode, queue, style, text};

pub const PROMPT: &str = "> ";
/// How long after Esc another key makes it part of one, like an arrow
/// key's. Without it, rustyline waits for the next key, and a lone Esc does
/// nothing until then.
const ESC_WAIT_MS: u16 = 50;
/// A draft, or a key into one this recently, means the person is typing a
/// message (`queue`) when an approval appears...
const TYPING_JUST_NOW: Duration = Duration::from_secs(1);
/// ...and the approval then takes only Esc and Ctrl+C for this long.
const DEAF_WHILE_TYPING: Duration = Duration::from_millis(600);

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
    /// Calls to MCP tools the person said not to ask about again, this
    /// session: apart from commands, so a call's text can't let a command
    /// run unasked.
    approved_calls: HashSet<String>,
    /// Mail from the person's other sessions, when this one listens.
    inbox: Option<Inbox>,
    /// The images pasted this session, `[Image #1]` first, as data URLs.
    pasted: Arc<Mutex<Vec<String>>>,
    /// Whether the model takes images; None when the server doesn't say.
    vision: Option<bool>,
    /// A word on a paste that took nothing, for the line it was pressed on.
    notice: Notice,
    /// The terminal's pastes, for the empty one Windows Terminal sends.
    watch: PasteWatch,
    /// A printer kept for the session, mail or not: while one lives,
    /// rustyline waits for a key in `select` and reads only once one came,
    /// so a reader in `read` is one in the middle of a paste, which
    /// `WatchPaste` waits out. Mail's printer is this one's twin.
    _printer: Option<Box<dyn ExternalPrinter + Send>>,
}

/// A terminal paste in progress, and one that came empty. With only an
/// image on the clipboard, Windows Terminal answers Ctrl+V with an empty
/// paste rather than the key, and rustyline inserts that nothing without a
/// redraw. So a paste that no redraw follows within `EMPTY_PASTE_WAIT`, once
/// rustyline waits for keys again, was empty, and stands for the image: Lokal
/// ends the line, as mail does, reads the clipboard and gives the line back
/// with the marker in. A paste of text that comes slowly, over a slow link,
/// is still being read, and waited for.
#[derive(Clone, Default)]
struct PasteWatch {
    /// The line and cursor a paste started at, until a redraw shows it landed.
    started: Arc<Mutex<Option<(String, usize)>>>,
    /// The same, once none did: the paste was empty.
    empty: Arc<Mutex<Option<(String, usize)>>>,
}

/// Long enough for a paste of text to land and redraw the line, and how
/// often a slow one is looked at again.
const EMPTY_PASTE_WAIT: Duration = Duration::from_millis(300);

/// What a reading of the line gave: a line, an empty paste to act on, or
/// auto chosen for the first time, whose warning shows above the line.
enum Read {
    Line(Line),
    EmptyPaste(String, usize),
    AutoChosen(String, usize),
}

/// What shows after the line until it changes: the line, and the word.
type Notice = Arc<Mutex<Option<(String, String)>>>;

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
                approved_calls: HashSet::new(),
                project,
                inbox: None,
                pasted: Arc::default(),
                vision: None,
                notice: Arc::default(),
                watch: PasteWatch::default(),
                _printer: None,
            };
        }
        let config = Config::builder()
            .completion_type(CompletionType::List)
            .completion_show_all_if_ambiguous(true)
            // Ask on the terminal even when stdin or stdout are redirected.
            .behavior(Behavior::PreferTerm)
            .keyseq_timeout(Some(ESC_WAIT_MS))
            .build();
        let notice: Notice = Arc::default();
        let watch = PasteWatch::default();
        let mut printer = None;
        let editor = Editor::with_config(config).ok().map(|mut editor| {
            editor.set_helper(Some(Helper {
                status: None,
                quit_armed: quit_armed.clone(),
                accented: false,
                inbox: None,
                sessions: None,
                command_hint: Cell::new(false),
                notice: notice.clone(),
                paste_started: watch.started.clone(),
            }));
            printer = editor
                .create_external_printer()
                .ok()
                .map(|printer| Box::new(printer) as Box<dyn ExternalPrinter + Send>);
            editor
        });
        Input {
            editor,
            quit_armed,
            approved: HashSet::new(),
            approved_calls: HashSet::new(),
            project,
            inbox: None,
            pasted: Arc::default(),
            vision: None,
            notice,
            watch,
            _printer: printer,
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
        tty::width()
    }

    /// Reads a request at the `>` prompt, starting from `typed`: what the
    /// person typed while Lokal worked. `status` (the model's time and the
    /// share of the context in use) shows at the far right of the line.
    /// Mail that starts a turn returns `Line::Mail`, whether it waited for
    /// the `>` or ended it.
    pub fn request(&mut self, status: Option<String>, typed: String) -> Line {
        let mut initial = (typed, String::new());
        let mut first = true;
        loop {
            match self.read_line(status.clone(), &initial, first) {
                Read::Line(line) => return line,
                Read::EmptyPaste(line, pos) => {
                    first = false;
                    // Lokal ended the line to read the image. It stays on
                    // screen as it was while the clipboard is read, which can
                    // take seconds, its cursor hidden: rustyline left it for
                    // the next row, so the cursor goes back up, and the next
                    // reading writes the line over it at once, marker in.
                    if style::enabled() {
                        eprint!("\x1b[{}A\r\x1b[?25l", rows(&line));
                    }
                    let (left, right) = line.split_at(pos.min(line.len()));
                    // Keys typed meanwhile wait for the next reading, unechoed,
                    // and a paste among them comes bracketed, as at the `>`.
                    let terminal = tty::open();
                    let raw = terminal.as_ref().and_then(RawMode::enter);
                    if style::enabled() {
                        eprint!("\x1b[?2004h");
                    }
                    let pasted = paste(&self.pasted, self.vision);
                    drop(raw);
                    initial = match pasted {
                        Ok(marker) => (format!("{left}{marker}"), right.to_string()),
                        Err(why) => {
                            if let Ok(mut notice) = self.notice.lock() {
                                *notice = Some((line.clone(), why));
                            }
                            (left.to_string(), right.to_string())
                        }
                    };
                    if style::enabled() {
                        eprint!("\x1b[?25h");
                    }
                }
                Read::AutoChosen(line, pos) => {
                    first = false;
                    // Lokal ended the line for auto's warning: the line goes
                    // from the screen, rustyline having left the cursor on
                    // the row after it, the warning takes its place, and the
                    // next reading writes the line under it as it was.
                    if style::enabled() {
                        eprint!("\x1b[{}A\r\x1b[J", rows(&line));
                    }
                    mode::show_warning();
                    let (left, right) = line.split_at(pos.min(line.len()));
                    initial = (left.to_string(), right.to_string());
                }
            }
        }
    }

    /// One reading of the `>`, starting from `initial`'s text on either side
    /// of the cursor.
    fn read_line(
        &mut self,
        status: Option<String>,
        initial: &(String, String),
        first: bool,
    ) -> Read {
        let Some(editor) = &mut self.editor else {
            return Read::Line(Line::Quit);
        };
        // What an earlier reading's watch left is no longer this line's.
        for slot in [&self.watch.started, &self.watch.empty] {
            if let Ok(mut slot) = slot.lock() {
                slot.take();
            }
        }
        // A line given back is one being typed, which mail waits for.
        let typing = !initial.0.is_empty() || !initial.1.is_empty();
        if self
            .inbox
            .as_ref()
            .is_some_and(|inbox| inbox.enter_prompt(typing))
        {
            return Read::Line(Line::Mail);
        }
        if let Some(helper) = editor.helper_mut() {
            helper.status = status;
            helper.accented = true;
        }
        // A paste's word belongs to the prompt it was pressed at, but for
        // one the empty paste's reading left for this very line.
        if first && let Ok(mut notice) = self.notice.lock() {
            notice.take();
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
        let auto_chosen = Arc::new(Mutex::new(None));
        let next_mode = NextMode {
            auto_chosen: auto_chosen.clone(),
        };
        editor.bind_sequence(shift_tab, EventHandler::Conditional(Box::new(next_mode)));
        // Ctrl+V, and Alt+V for terminals that keep Ctrl+V for their paste.
        for key in [KeyEvent::ctrl('V'), KeyEvent::alt('v')] {
            let paste = PasteImage {
                pasted: self.pasted.clone(),
                vision: self.vision,
                notice: self.notice.clone(),
                inbox: self.inbox.clone(),
            };
            editor.bind_sequence(key, EventHandler::Conditional(Box::new(paste)));
        }
        // An empty paste stands for an image, as Ctrl+V does: it's how
        // Windows Terminal sends that key. So it says why none can go, too.
        let paste_start = KeyEvent(KeyCode::BracketedPasteStart, Modifiers::NONE);
        let watch = WatchPaste {
            watch: self.watch.clone(),
        };
        editor.bind_sequence(paste_start, EventHandler::Conditional(Box::new(watch)));
        let esc = KeyEvent(KeyCode::Esc, Modifiers::NONE);
        let dropping = Arc::new(AtomicBool::new(false));
        if let Some(inbox) = &self.inbox {
            let drop = DropOnEsc {
                inbox: inbox.clone(),
                pressed: dropping.clone(),
            };
            editor.bind_sequence(esc, EventHandler::Conditional(Box::new(drop)));
        }
        let line = editor.readline_with_initial(PROMPT, (&initial.0, &initial.1));
        let woken = self.inbox.as_ref().is_some_and(Inbox::leave_prompt);
        // A paste the line left with is no longer watched.
        if let Ok(mut started) = self.watch.started.lock() {
            started.take();
        }
        let empty = self
            .watch
            .empty
            .lock()
            .ok()
            .and_then(|mut empty| empty.take());
        editor.unbind_sequence(paste_start);
        if self.inbox.is_some() {
            editor.unbind_sequence(esc);
        }
        editor.unbind_sequence(shift_tab);
        editor.unbind_sequence(KeyEvent::ctrl('C'));
        editor.unbind_sequence(KeyEvent::ctrl('V'));
        editor.unbind_sequence(KeyEvent::alt('v'));
        // A reminder left over must not show at the next question.
        self.quit_armed.store(false, Ordering::SeqCst);
        let auto_chosen = auto_chosen.lock().ok().and_then(|mut at| at.take());
        match line {
            Err(ReadlineError::Interrupted) if !woken && empty.is_some() => {
                let (line, pos) = empty.unwrap_or_default();
                Read::EmptyPaste(line, pos)
            }
            Err(ReadlineError::Interrupted) if !woken && auto_chosen.is_some() => {
                let (line, pos) = auto_chosen.unwrap_or_default();
                Read::AutoChosen(line, pos)
            }
            Err(ReadlineError::Interrupted) if woken => {
                // Mail ended an empty line, which rustyline leaves with the
                // status on it: it goes, and the mail shows in its place.
                if style::enabled() {
                    eprint!("\x1b[1A\r\x1b[2K");
                }
                Read::Line(Line::Mail)
            }
            Ok(_) if dropping.load(Ordering::SeqCst) => Read::Line(Line::DropMessages),
            Ok(line) => {
                if !line.trim().is_empty() {
                    let _ = editor.add_history_entry(line.as_str());
                }
                Read::Line(Line::Text(line))
            }
            Err(_) => Read::Line(Line::Quit),
        }
    }

    /// A line queued while Lokal worked, shown as the `>` shows one typed
    /// there, with the time Enter `queued` it at the right of its last row
    /// (#46), and kept in the history as one would be.
    pub fn show_queued(&mut self, line: &str, queued: SystemTime) {
        let shown = layout::printable(line);
        let time = tty::width()
            .filter(|_| style::enabled())
            .and_then(|width| time_beside(&shown, queued, width));
        let rows: Vec<&str> = shown.lines().collect();
        for (index, row) in rows.iter().enumerate() {
            let prompt = if index == 0 { PROMPT } else { "" };
            let after = match &time {
                Some((padding, time)) if index + 1 == rows.len() => {
                    format!("{}{}", " ".repeat(*padding), style::faint(time))
                }
                _ => String::new(),
            };
            eprintln!("{}{after}", style::accent(&format!("{prompt}{row}")));
        }
        if let Some(editor) = &mut self.editor {
            let _ = editor.add_history_entry(line);
        }
    }

    /// When the person sent `line` at the `>`, faint at the right of its
    /// last row, where the status sat while they typed it (#46). Enter
    /// leaves the line right above the cursor.
    pub fn date_sent(&self, line: &str, at: SystemTime) {
        let Some(width) = tty::width().filter(|_| style::enabled()) else {
            return;
        };
        // rustyline draws the line as typed, so it's laid out as typed.
        if let Some((_, time)) = time_beside(line, at, width) {
            // Up to that row, out to where the time starts, and back down.
            eprint!(
                "\x1b[1A\x1b[{}G{}\r\n",
                width - time.width(),
                style::faint(&time)
            );
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
    pub fn approve_write(&mut self, rest: Vec<String>, opens_on_yes: bool) -> Approval {
        self.approve_shown(text::WRITE_QUESTION, rest, diff::print, false, opens_on_yes)
    }

    /// Asks `question` about what was just shown, starting on Yes only when
    /// `opens_on_yes`, with Esc always No. `session` offers "don't ask
    /// again" for the rest of the session. When only the start was shown,
    /// `rest` holds the lines left out, and a choice before No prints them
    /// with `print` and asks again.
    pub fn approve_shown(
        &mut self,
        question: &str,
        mut rest: Vec<String>,
        print: fn(&[String]),
        session: bool,
        opens_on_yes: bool,
    ) -> Approval {
        if self.editor.is_none() {
            return Approval::Unavailable;
        }
        loop {
            // Again after the rest: keys pressed while it printed came
            // before the question.
            discard_typed_ahead();
            let mut options = vec![(text::CHOICE_YES, 'y')];
            if session {
                options.push((text::CHOICE_YES_SESSION, 'a'));
            }
            if !rest.is_empty() {
                options.push((text::CHOICE_SHOW_REST, 's'));
            }
            options.push((text::CHOICE_NO, 'n'));
            let items: Vec<menu::Item> = options
                .iter()
                .map(|&(label, key)| menu::Item { label, key })
                .collect();
            let no = items.len() - 1;
            let start = if opens_on_yes { 0 } else { no };
            let picked = match self.choose(question, &items, start, no) {
                Some(menu::Choice::Picked(index)) => options[index].1,
                Some(menu::Choice::Interrupted) => return Approval::Stop,
                None => return Approval::Unavailable,
            };
            match picked {
                'y' => return Approval::Yes,
                'a' => return Approval::YesForSession,
                's' => print(&std::mem::take(&mut rest)),
                _ => return Approval::No,
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
        // Typing a message when the question came, the person's next keys
        // were meant for the message: an Enter to queue it would answer a
        // question they haven't read.
        let deaf = if queue::typing(TYPING_JUST_NOW) {
            DEAF_WHILE_TYPING
        } else {
            Duration::ZERO
        };
        let choice = menu::choose(question, items, start, safe, deaf);
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

    /// Whether the model takes images, for a paste at the `>` to say so.
    pub fn set_vision(&mut self, vision: Option<bool>) {
        self.vision = vision;
    }

    /// The pasted images whose marker is whole in `line`, in its order, with
    /// their numbers: what goes with the request is what the line shows.
    pub fn images_in(&self, line: &str) -> Vec<(usize, String)> {
        let Ok(pasted) = self.pasted.lock() else {
            return Vec::new();
        };
        markers(line)
            .into_iter()
            .filter_map(|number| Some((number, pasted.get(number.checked_sub(1)?).cloned()?)))
            .collect()
    }

    /// Whether there's a terminal to ask on: without one, nothing is.
    pub fn can_ask(&self) -> bool {
        self.editor.is_some()
    }

    /// Whether this exact MCP call was approved for the session.
    pub fn call_approved_before(&self, call: &str) -> bool {
        self.approved_calls.contains(call)
    }

    pub fn keep_call_approval(&mut self, approval: &Approval, call: &str) {
        if let Approval::YesForSession = approval {
            self.approved_calls.insert(call.to_string());
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
            // All of it: a paste or what was queued can hold several lines.
            return Some(Cmd::Kill(Movement::WholeBuffer));
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

/// Ctrl+V or Alt+V at the `>`: the clipboard's image, as `[Image #N]`
/// where the cursor is, or a word on why there's none.
struct PasteImage {
    pasted: Arc<Mutex<Vec<String>>>,
    vision: Option<bool>,
    notice: Notice,
    /// Told the person is typing while the clipboard is read, so mail can't
    /// end the line under the marker about to go in.
    inbox: Option<Inbox>,
}

impl ConditionalEventHandler for PasteImage {
    fn handle(
        &self,
        _event: &Event,
        _count: RepeatCount,
        _positive: bool,
        context: &EventContext,
    ) -> Option<Cmd> {
        // The next hint sets it again from the line.
        if let Some(inbox) = &self.inbox {
            inbox.set_typing(true);
        }
        match paste(&self.pasted, self.vision) {
            Ok(marker) => Some(Cmd::Insert(1, marker)),
            Err(why) => {
                if let Ok(mut notice) = self.notice.lock() {
                    *notice = Some((context.line().to_string(), why));
                }
                Some(Cmd::Repaint)
            }
        }
    }
}

/// The rows a line takes at the `>`, wrapped at the terminal's width.
fn rows(line: &str) -> usize {
    rows_at(line, tty::width().unwrap_or(80))
}

/// The same at `width` columns.
fn rows_at(line: &str, width: usize) -> usize {
    laid_out(line, width).0
}

/// The rows a line takes at the `>` at `width` columns, and the column it
/// ends at on the last, laid out as rustyline does: a line break a paste
/// kept starts a row at the left edge, a character that doesn't fit moves
/// whole to the next row, a tab reaches the next stop, and a row filled
/// exactly wraps only at the end of the line.
fn laid_out(line: &str, width: usize) -> (usize, usize) {
    let width = width.max(1);
    let (mut row, mut column) = (0, PROMPT.width());
    for c in line.chars() {
        if c == '\n' {
            row += 1;
            column = 0;
            continue;
        }
        let wide = if c == '\t' {
            TAB_STOP - column % TAB_STOP
        } else {
            c.width().unwrap_or(0)
        };
        column += wide;
        if column > width {
            row += 1;
            column = wide;
        }
    }
    if column == width {
        row += 1;
        column = 0;
    }
    (row + 1, column)
}

/// rustyline's tab stop, which Lokal leaves as it comes.
const TAB_STOP: usize = 8;

/// When a line `drawn` after the prompt was sent, and the spaces before it
/// at the right of the line's last row; None without room (#46).
fn time_beside(drawn: &str, at: SystemTime, width: usize) -> Option<(usize, String)> {
    let time = clock::shown(at, |time| time_padding(drawn, time, width).is_some())?;
    Some((time_padding(drawn, time.width(), width)?, time))
}

/// The spaces before something `right` columns wide at the right of the
/// last row of a line `drawn` after the prompt, or None. A last row the
/// line filled has no room: past it is an empty row rustyline adds, or
/// none at all for a line printed whole.
fn time_padding(drawn: &str, right: usize, width: usize) -> Option<usize> {
    let (_, end) = laid_out(drawn, width);
    layout::right_padding(end, right, width).filter(|_| end > 0)
}

/// The clipboard's image, kept as the session's next, and its marker; or
/// why there's none.
fn paste(pasted: &Mutex<Vec<String>>, vision: Option<bool>) -> Result<String, String> {
    let url = match vision {
        Some(true) => clipboard::image().map(|(_, url)| url)?,
        Some(false) => return Err(text::PASTE_BLIND.to_string()),
        None => return Err(text::PASTE_UNSAID.to_string()),
    };
    let mut images = pasted
        .lock()
        .map_err(|_| text::CLIPBOARD_NO_IMAGE.to_string())?;
    images.push(url);
    Ok(text::image_marker(images.len()))
}

/// The start of a terminal's paste: watched, so an empty one is known by
/// the redraw that doesn't follow. rustyline reads the paste as ever.
struct WatchPaste {
    watch: PasteWatch,
}

impl ConditionalEventHandler for WatchPaste {
    fn handle(
        &self,
        _event: &Event,
        _count: RepeatCount,
        _positive: bool,
        context: &EventContext,
    ) -> Option<Cmd> {
        let at = (context.line().to_string(), context.pos());
        if let Ok(mut started) = self.watch.started.lock() {
            *started = Some(at);
        }
        let watch = self.watch.clone();
        // The thread reading the paste: this one.
        // SAFETY: gettid has no arguments and can't fail.
        let reader = unsafe { libc::syscall(libc::SYS_gettid) };
        thread::spawn(move || {
            // A paste that comes in pieces is still being read, however
            // slowly: only once its reader waits for keys again, with no
            // redraw, was it empty.
            loop {
                thread::sleep(EMPTY_PASTE_WAIT);
                let open = watch.started.lock().is_ok_and(|s| s.is_some());
                if !open || !reading(reader) {
                    break;
                }
            }
            let Some(at) = watch.started.lock().ok().and_then(|mut s| s.take()) else {
                return;
            };
            if let Ok(mut empty) = watch.empty.lock() {
                *empty = Some(at);
            }
            // SAFETY: a plain call, signalling this process; readline
            // returns Interrupted on it, as for mail.
            unsafe {
                libc::kill(libc::getpid(), libc::SIGINT);
            }
        });
        None
    }
}

/// Whether the thread `tid` of this process is still at a paste: waiting in
/// `read`, as rustyline's does in the middle of one, or running between two
/// reads. Only a wait in another call, `select` at the `>` since `Input` made
/// a printer, says it's done. Linux says so in `/proc`; where it can't be
/// read, done.
fn reading(tid: libc::c_long) -> bool {
    std::fs::read_to_string(format!("/proc/self/task/{tid}/syscall"))
        .is_ok_and(|call| still_reading(&call))
}

/// What `/proc/<pid>/task/<tid>/syscall` says, read as above: a call's
/// number and its arguments, or `running`.
fn still_reading(call: &str) -> bool {
    match call
        .split_whitespace()
        .next()
        .map(str::parse::<libc::c_long>)
    {
        Some(Ok(number)) => number == libc::SYS_read || number < 0,
        Some(Err(_)) => true,
        None => false,
    }
}

/// The numbers of the `[Image #N]` markers whole in `line`, in its order,
/// each once.
fn markers(line: &str) -> Vec<usize> {
    let mut numbers = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("[Image #") {
        rest = &rest[start + "[Image #".len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        // Written as Lokal writes it, and each image once.
        if !digits.is_empty()
            && !digits.starts_with('0')
            && rest[digits.len()..].starts_with(']')
            && let Ok(number) = digits.parse()
            && !numbers.contains(&number)
        {
            numbers.push(number);
        }
    }
    numbers
}

/// Shift+Tab at the `>`: the next mode, which shows at the right at once.
/// Auto, the first time this session, ends the line with what's typed and
/// where the cursor is, so its warning can show above it.
struct NextMode {
    auto_chosen: Arc<Mutex<Option<(String, usize)>>>,
}

impl ConditionalEventHandler for NextMode {
    fn handle(
        &self,
        _event: &Event,
        _count: RepeatCount,
        _positive: bool,
        context: &EventContext,
    ) -> Option<Cmd> {
        mode::next();
        if mode::warning_due()
            && let Ok(mut at) = self.auto_chosen.lock()
        {
            *at = Some((context.line().to_string(), context.pos()));
            return Some(Cmd::Interrupt);
        }
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
    /// A paste's word, shown until the line changes.
    notice: Notice,
    /// A paste in progress, cleared by the redraw of one that landed.
    paste_started: Arc<Mutex<Option<(String, usize)>>>,
}

impl Helper {
    /// What shows at the right edge: the mail waiting, at the `>` only,
    /// where Enter and Esc act on it, and the mode, where Shift+Tab changes
    /// it, with the key when there's room (`keys`); then the status.
    fn status(&self, keys: bool) -> Option<String> {
        let waiting = self
            .inbox
            .as_ref()
            .filter(|_| self.accented)
            .map(Inbox::count)
            .filter(|count| *count > 0)
            .map(text::mail_waiting);
        let mode = text::mode_name(mode::current())
            .filter(|_| self.accented)
            .map(|name| {
                if keys {
                    format!("{name} {}", text::MODE_KEYS)
                } else {
                    name.to_string()
                }
            });
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
        let notice = self.notice.lock().ok().and_then(|mut notice| {
            match notice.as_ref() {
                Some((at, why)) if at == line => Some(why.clone()),
                // Typing ends it.
                _ => notice.take().and(None),
            }
        });
        let mut display = if armed {
            text::QUIT_AGAIN.to_string()
        } else if let Some(why) = notice {
            format!("  {why}")
        } else {
            String::new()
        };
        // Read on every redraw, so the hint follows the window as it's
        // resized. It stays on the line's last row and off its last column:
        // wrapped, it takes a row an empty paste's redraw doesn't count.
        let width = tty::width();
        let (rows, end) = width.map_or((1, 0), |width| laid_out(line, width));
        let room = width.map_or(usize::MAX, |width| width.saturating_sub(end + 1));
        if display.width() > room {
            display = if room > 3 {
                layout::cut(&display, room)
            } else {
                String::new()
            };
        }
        // The status only beside a line on one row, at the right edge. The
        // mode's key gives way first.
        let fits = |status: &String| {
            room.checked_sub(display.width() + status.width())
                .filter(|padding| *padding >= 2)
        };
        if rows == 1
            && width.is_some()
            && let Some((status, padding)) = [true, false]
                .into_iter()
                .filter_map(|keys| self.status(keys))
                .find_map(|status| fits(&status).map(|padding| (status, padding)))
        {
            display.push_str(&" ".repeat(padding));
            display.push_str(&status);
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
    // One column short of the edge, so the terminal doesn't wrap it.
    let room = width.map_or(usize::MAX, |width| {
        width.saturating_sub(PROMPT.len() + line.chars().count() + 1)
    });
    match matches.as_slice() {
        [] => None,
        [(name, about)] => {
            let rest = &name[line.len()..];
            Some(Hint {
                display: layout::cut(&format!("{rest}  {about}"), room),
                completion: Some(rest.to_string()),
            })
        }
        several => {
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
        // A paste whose text went in redraws the line: it wasn't empty.
        if let Ok(mut started) = self.paste_started.lock()
            && started
                .as_ref()
                .is_some_and(|(at, at_pos)| at != line || *at_pos != pos)
        {
            started.take();
        }
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
            command_hint(line, pos, tty::width())
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
    fn each_line_break_in_a_line_starts_a_row() {
        assert_eq!(rows_at("hello", 80), 1);
        assert_eq!(rows_at(&"x".repeat(78), 80), 2);
        assert_eq!(rows_at(&"x".repeat(77), 80), 1);
        assert_eq!(rows_at("first line one\nline two\nline three", 80), 3);
        assert_eq!(rows_at(&format!("a\n{}", "x".repeat(80)), 80), 3);
        assert_eq!(rows_at(&"漢".repeat(40), 80), 2);
        // A row filled exactly before a break doesn't wrap: only the end does.
        assert_eq!(rows_at(&format!("{}\nend", "x".repeat(78)), 80), 2);
        assert_eq!(rows_at(&format!("a\n{}\nb", "x".repeat(160)), 80), 4);
        assert_eq!(rows_at("\n\n", 80), 3);
        // A wide character that doesn't fit moves whole to the next row.
        let straddling = format!("{}漢{}", "x".repeat(77), "漢".repeat(39));
        assert_eq!(rows_at(&straddling, 80), 3);
        assert_eq!(rows_at(&format!("{}\tx", "x".repeat(69)), 80), 1);
        assert_eq!(rows_at(&format!("{}\tx", "x".repeat(77)), 80), 2);
        // Where the line ends, for the hint after it.
        assert_eq!(laid_out("hello", 80), (1, 7));
        assert_eq!(laid_out("olá 漢字 ok", 80), (1, 13));
        assert_eq!(laid_out(&"x".repeat(78), 80), (2, 0));
        assert_eq!(laid_out("a\nbc", 80), (2, 2));
    }

    #[test]
    fn a_time_goes_beside_the_last_row_only_where_it_has_room() {
        // "> hi" ends at 4: the time ends one column short of the edge.
        assert_eq!(time_padding("hi", 5, 40), Some(30));
        // Wide characters take their columns.
        assert_eq!(time_padding("日本", 5, 20), Some(8));
        // A last row the line filled has none; a row after it has its own.
        assert_eq!(time_padding(&"a".repeat(18), 5, 20), None);
        assert_eq!(
            time_padding(&format!("{}\nab", "a".repeat(18)), 5, 20),
            Some(12)
        );
        // A hidden character takes no room as rustyline draws it: this row
        // ends at 76 of 80, with no room for a time.
        let pasted = format!("{}\u{200B}", "x".repeat(74));
        assert_eq!(time_padding(&pasted, 5, 80), None);
        assert_eq!(laid_out(&pasted, 80), (1, 76));
    }

    #[test]
    fn a_reader_running_or_in_read_is_still_at_its_paste() {
        let read = format!("{} 0x3 0x5a 0x400 0x0 0x0 0x0 0x7f 0x7f", libc::SYS_read);
        assert!(still_reading(&read));
        assert!(still_reading("running"));
        assert!(still_reading("-1 0x7ffc 0x7f2e"));
        assert!(!still_reading(
            "270 0x9 0x7ffc 0x0 0x0 0x0 0x0 0x7ffc 0x7f2e"
        ));
        assert!(!still_reading(""));
    }

    #[test]
    fn only_whole_markers_name_images_in_the_line_s_order() {
        assert_eq!(markers("see [Image #2] then [Image #1]"), [2, 1]);
        assert_eq!(
            markers("[Image #3 cut [Image #] [Image #x] [Image #4]"),
            [4]
        );
        assert_eq!(markers("no images"), Vec::<usize>::new());
        assert_eq!(markers("[Image #1] and [Image #1] again"), [1]);
        assert_eq!(markers("[Image #01] [Image #0] [Image #10]"), [10]);
    }

    #[test]
    fn slash_alone_lists_every_command() {
        let hint = command_hint("/", 1, None).unwrap();
        assert_eq!(
            hint.display,
            "   /new  /compact  /context  /prefs  /memory  /config  /info  /recon  /update  /reasoning  /sysprompt  /sessions  /feedback  /help  /about  /exit"
        );
        assert_eq!(hint.completion, None);
    }

    #[test]
    fn a_narrow_terminal_lists_the_commands_that_fit() {
        let hint = command_hint("/", 1, Some(40)).unwrap();
        assert_eq!(hint.display, "   /new  /compact  /context  …");
        // With the prompt and the slash, within one column short of the edge.
        assert!("> /".len() + hint.display.chars().count() <= 39);
    }

    #[test]
    fn one_match_is_cut_to_a_narrow_terminal() {
        let hint = command_hint("/inf", 4, Some(30)).unwrap();
        // With the prompt and the line, one column short of the edge.
        assert_eq!("> /inf".len() + hint.display.chars().count(), 29);
        assert!(hint.display.starts_with("o  ") && hint.display.ends_with('…'));
        assert_eq!(hint.completion.as_deref(), Some("o"));
    }

    #[test]
    fn one_match_completes_and_explains() {
        let hint = command_hint("/pr", 3, None).unwrap();
        assert_eq!(hint.display, "efs  edit what Lokal knows about you");
        assert_eq!(hint.completion.as_deref(), Some("efs"));
        assert!(command_hint("/xyz", 4, None).is_none());
    }
}
