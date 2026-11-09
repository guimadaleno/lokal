//! The mail waiting in this session, and when it starts a turn: at once, at
//! the `>`, after the person's next request, or never on its own.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use super::show::{notice, spaced};
use super::{REQUEST_LIMIT, SEEN_AFTER, WAITING_LIMIT};
use crate::config::Mail;
use crate::layout::clean;
use crate::text;

pub struct Message {
    pub from: String,
    pub folder: String,
    pub text: String,
    /// Typed by the person at the other session's `>`, as `@name …`.
    pub by_user: bool,
    /// To take up right away, rather than with the next turn.
    pub now: bool,
    /// When it showed on screen.
    pub(super) shown: Option<Instant>,
}

/// What becomes of a mail taken, as its sender hears it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Then {
    /// A turn starts on it now.
    Now,
    /// A turn starts on it when the session is free.
    AfterTurn,
    /// It waits for Enter there: `mail = ask`.
    User,
    /// It waits for the person's next request there: they stopped the last
    /// turn mail started.
    Held,
    /// It waits for the person's next request there: mail started as many
    /// turns in a row as it may.
    Limit,
    /// It goes along with the session's next turn.
    Later,
}

impl Then {
    pub(super) fn word(self) -> &'static str {
        match self {
            Then::Now => "now",
            Then::AfterTurn => "after_turn",
            Then::User => "user",
            Then::Held => "held",
            Then::Limit => "limit",
            Then::Later => "later",
        }
    }

    pub(super) fn from_word(word: &str) -> Option<Then> {
        [
            Then::Now,
            Then::AfterTurn,
            Then::User,
            Then::Held,
            Then::Limit,
            Then::Later,
        ]
        .into_iter()
        .find(|then| then.word() == word)
    }
}

pub(super) struct State {
    mail: Mail,
    turns_limit: usize,
    pub(super) waiting: Vec<Message>,
    /// A turn is running.
    pub(super) busy: bool,
    /// An approval question is open.
    pub(super) asking: bool,
    /// The person is at the `>`: mail prints above it at once.
    pub(super) at_prompt: bool,
    /// Mail printed at this prompt already, with a blank line after it.
    printed: bool,
    /// The `>` line isn't empty: mail doesn't take it away.
    typing: bool,
    /// Mail waits for the person's next request: they stopped a turn mail
    /// started, or said no in one. Otherwise a mail would start the next
    /// turn at once, and two models could go on with nobody able to stop
    /// them.
    held: bool,
    /// Turns mail started since the person last asked something.
    pub(super) turns: usize,
    /// Mail ended the `>` to start a turn.
    woken: bool,
    /// That the model takes mail up on its own was said once, so it isn't
    /// again.
    told_taken_up: bool,
    /// What the others see this session doing.
    pub(super) request: String,
    pub(super) files: Vec<String>,
}

impl State {
    /// What becomes of a mail, leaving aside whether the session is free.
    fn fate(&self, message: &Message) -> Then {
        if self.mail == Mail::Ask {
            return Then::User;
        }
        if !message.by_user {
            if !message.now {
                return Then::Later;
            }
            if self.held {
                return Then::Held;
            }
            if self.turns >= self.turns_limit {
                return Then::Limit;
            }
        }
        Then::Now
    }

    /// The same, now: one that would start waits while the session is busy,
    /// or the person is away from the `>` or typing at it.
    fn then(&self, message: &Message) -> Then {
        match self.fate(message) {
            Then::Now if self.busy || !self.at_prompt || self.typing => Then::AfterTurn,
            fate => fate,
        }
    }

    /// Prints the mail not shown yet, each saying what `then` says becomes
    /// of it, and spaced as one block. True when any printed.
    fn show_unshown(&mut self, then: impl Fn(&State, &Message) -> Then) -> bool {
        let mut printed = false;
        for index in 0..self.waiting.len() {
            if self.waiting[index].shown.is_none() {
                let fate = then(self, &self.waiting[index]);
                let footer = self.footer(fate);
                let notice = notice(&self.waiting[index], footer.as_deref());
                eprint!("{}", spaced(&notice, printed));
                self.waiting[index].shown = Some(Instant::now());
                printed = true;
            }
        }
        printed
    }

    /// What a notice says under a mail: what becomes of it.
    fn footer(&mut self, then: Then) -> Option<String> {
        match then {
            Then::Now if self.told_taken_up => None,
            Then::Now => {
                self.told_taken_up = true;
                Some(text::MAIL_TAKEN_UP.to_string())
            }
            Then::Later => Some(text::MAIL_LATER.to_string()),
            Then::Limit => Some(text::mail_limit(self.turns_limit)),
            Then::Held => Some(text::MAIL_HELD.to_string()),
            // At the `>`, only while the person types: it goes once they're done.
            Then::AfterTurn => Some(text::MAIL_AFTER_YOU.to_string()),
            Then::User => Some(text::MAIL_KEYS.to_string()),
        }
    }
}

/// The mail waiting, shared by the listener and the `>` prompt.
#[derive(Clone)]
pub struct Inbox(Arc<Mutex<State>>);

/// A mail taken: what becomes of it, and what to print above the `>`.
pub(super) struct Arrival {
    pub(super) then: Then,
    pub(super) print: Option<String>,
}

impl Inbox {
    pub(super) fn new(mail: Mail, turns_limit: usize) -> Inbox {
        Inbox(Arc::new(Mutex::new(State {
            mail,
            turns_limit,
            waiting: Vec::new(),
            busy: false,
            asking: false,
            at_prompt: false,
            printed: false,
            typing: false,
            held: false,
            turns: 0,
            woken: false,
            told_taken_up: false,
            request: String::new(),
            files: Vec::new(),
        })))
    }

    pub fn count(&self) -> usize {
        self.lock().waiting.len()
    }

    /// The person is at the `>` from now on, and what came meanwhile shows;
    /// `typing` when it opens on a line already begun, which mail waits for.
    /// True when a turn starts on mail at once instead; then the `>` isn't
    /// shown.
    pub fn enter_prompt(&self, typing: bool) -> bool {
        let mut guard = self.lock();
        let state = &mut *guard;
        state.at_prompt = true;
        state.typing = typing;
        let start = state
            .waiting
            .iter()
            .any(|message| state.then(message) == Then::Now);
        state.printed = state.show_unshown(State::then);
        state.at_prompt = !start;
        start
    }

    /// The person left the `>`. True when mail ended it.
    pub fn leave_prompt(&self) -> bool {
        let mut state = self.lock();
        state.at_prompt = false;
        state.typing = false;
        std::mem::take(&mut state.woken)
    }

    pub fn set_typing(&self, typing: bool) {
        self.lock().typing = typing;
    }

    /// An approval question opened or closed, for the others to see.
    pub fn set_asking(&self, asking: bool) {
        self.lock().asking = asking;
    }

    /// Mail that ended the `>`, shown before its turn starts.
    pub fn show_arrived(&self) {
        self.lock().show_unshown(|_, _| Then::Now);
    }

    /// Takes the mail that has been on screen a moment, for Enter to pass
    /// on or Esc to drop.
    pub fn take(&self) -> Vec<Message> {
        self.take_where(|_, message| message.shown.is_some_and(|at| at.elapsed() >= SEEN_AFTER))
    }

    /// Takes the mail a turn starts on, with what goes along with the next
    /// turn.
    pub fn take_for_turn(&self) -> Vec<Message> {
        self.take_where(|state, message| {
            message.shown.is_some() && matches!(state.fate(message), Then::Now | Then::Later)
        })
    }

    /// Takes what goes along with the next turn, for one the person asked.
    pub fn take_later(&self) -> Vec<Message> {
        self.take_where(|state, message| {
            message.shown.is_some() && state.fate(message) == Then::Later
        })
    }

    fn take_where(&self, wanted: impl Fn(&State, &Message) -> bool) -> Vec<Message> {
        let mut guard = self.lock();
        let state = &mut *guard;
        let (taken, kept) = std::mem::take(&mut state.waiting)
            .into_iter()
            .partition(|message| wanted(state, message));
        state.waiting = kept;
        taken
    }

    /// The person asked something, or passed mail on: mail may start turns
    /// again, the count starting over. `request` is what the others see.
    pub fn person_acted(&self, request: Option<&str>) {
        let mut state = self.lock();
        state.held = false;
        state.turns = 0;
        if let Some(request) = request {
            state.request = clean(request, REQUEST_LIMIT);
        }
    }

    /// A turn starts on mail: one more in a row, unless the person wrote
    /// some of it at another session's `>`.
    pub fn turn_on_mail(&self, messages: &[Message]) {
        match messages.iter().find(|message| message.by_user) {
            Some(written) => self.person_acted(Some(&written.text)),
            None => self.lock().turns += 1,
        }
    }

    /// The person stopped a turn mail started, or said no in one: mail waits
    /// for their next request.
    pub fn hold(&self) {
        self.lock().held = true;
    }

    /// Mail that ended the `>` makes it return: rustyline ends `readline`
    /// on SIGINT. Sent while the state is held, so the person can't leave
    /// the `>` in between; and handled on this thread before `kill`
    /// returns. Should they have left a moment before, Lokal's own handler
    /// raises its flag instead, and the turn starting clears it first.
    pub(super) fn wake(&self) {
        let state = self.lock();
        if state.at_prompt && state.woken {
            // SAFETY: a plain call, signalling this process.
            unsafe {
                libc::kill(libc::getpid(), libc::SIGINT);
            }
        }
    }

    /// A mail checked and taken, or why not.
    pub(super) fn add(&self, mut message: Message) -> Result<Arrival, &'static str> {
        let mut state = self.lock();
        if state.waiting.len() >= WAITING_LIMIT {
            return Err("full");
        }
        let then = state.then(&message);
        let mut print = None;
        if then == Then::Now {
            // Shown by the `>` as it ends, before the turn.
            state.woken = true;
        } else if state.at_prompt {
            message.shown = Some(Instant::now());
            let footer = state.footer(then);
            print = Some(spaced(&notice(&message, footer.as_deref()), state.printed));
            state.printed = true;
        }
        state.waiting.push(message);
        Ok(Arrival { then, print })
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn mail_is_taken_once_it_has_been_on_screen_a_moment() {
        let inbox = Inbox::new(Mail::Auto, 3);
        let message = |shown: Option<Duration>, now: bool| Message {
            from: "api-1".into(),
            folder: "/".into(),
            text: "hi".into(),
            by_user: false,
            now,
            shown: shown.map(|ago| Instant::now() - ago),
        };
        let second = Some(Duration::from_secs(1));
        inbox.lock().waiting.extend([
            message(second, true),
            message(Some(Duration::ZERO), true),
            message(None, true),
            message(second, false),
        ]);
        assert_eq!(inbox.take_later().len(), 1);
        assert_eq!(inbox.take().len(), 1);
        inbox.person_acted(None);
        // A turn on mail takes what's on screen, however briefly.
        assert_eq!(inbox.take_for_turn().len(), 1);
        assert_eq!(inbox.count(), 1);
    }

    #[test]
    fn that_the_model_takes_mail_up_on_its_own_is_said_once() {
        let inbox = Inbox::new(Mail::Auto, 3);
        let mut state = inbox.lock();
        assert_eq!(
            state.footer(Then::Now).as_deref(),
            Some(text::MAIL_TAKEN_UP)
        );
        assert_eq!(state.footer(Then::Now), None);
        assert_eq!(state.footer(Then::User).as_deref(), Some(text::MAIL_KEYS));
    }
}
