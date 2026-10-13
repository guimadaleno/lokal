//! Which part of the streamed answer the person sees. Some servers send the
//! model's reasoning inside the answer as <think>…</think>; that never shows.
//! Blank space at the start and end of the answer doesn't either.

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

enum State {
    /// Nothing visible yet: the answer might still open with <think>.
    Start,
    Thinking,
    Answer,
}

pub struct AnswerFilter {
    state: State,
    held: String,
}

impl AnswerFilter {
    pub fn new() -> Self {
        AnswerFilter {
            state: State::Start,
            held: String::new(),
        }
    }

    /// Takes the next piece of the stream and returns what can be shown now.
    pub fn push(&mut self, piece: &str) -> String {
        self.held.push_str(piece);
        loop {
            match self.state {
                State::Start => {
                    let start = self.held.trim_start();
                    if let Some(rest) = start.strip_prefix(OPEN) {
                        self.held = rest.to_string();
                        self.state = State::Thinking;
                    } else if OPEN.starts_with(start) {
                        return String::new();
                    } else {
                        self.held = start.to_string();
                        self.state = State::Answer;
                    }
                }
                State::Thinking => match self.held.find(CLOSE) {
                    Some(end) => {
                        self.held.drain(..end + CLOSE.len());
                        self.state = State::Start;
                    }
                    None => {
                        // Keep just enough to spot a </think> split across pieces.
                        let cut = self
                            .held
                            .floor_char_boundary(self.held.len().saturating_sub(CLOSE.len()));
                        self.held.drain(..cut);
                        return String::new();
                    }
                },
                State::Answer => {
                    // Trailing blank space waits: it shows only if more text follows.
                    let shown = self.held.trim_end().len();
                    let rest = self.held.split_off(shown);
                    return std::mem::replace(&mut self.held, rest);
                }
            }
        }
    }

    /// What's left to show once the stream ends.
    pub fn finish(&mut self) -> String {
        match self.state {
            State::Start => std::mem::take(&mut self.held).trim().to_string(),
            State::Thinking | State::Answer => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(pieces: &[&str]) -> String {
        let mut filter = AnswerFilter::new();
        let mut shown: String = pieces.iter().map(|piece| filter.push(piece)).collect();
        shown.push_str(&filter.finish());
        shown
    }

    #[test]
    fn hides_reasoning_even_when_tags_are_split() {
        assert_eq!(
            run(&["<thi", "nk>plan", " things</th", "ink>\n\nHello", "!"]),
            "Hello!"
        );
    }

    #[test]
    fn trims_blank_space_around_the_answer() {
        assert_eq!(run(&["\n\n", "Hey", "\n", "there", "\n\n"]), "Hey\nthere");
    }

    #[test]
    fn keeps_text_that_only_looks_like_a_tag() {
        assert_eq!(run(&["<b>bold</b>"]), "<b>bold</b>");
        assert_eq!(run(&["<thi"]), "<thi");
    }

    #[test]
    fn unclosed_reasoning_shows_nothing() {
        assert_eq!(run(&["<think>still going"]), "");
    }

    #[test]
    fn handles_multibyte_text_while_thinking() {
        assert_eq!(run(&["<think>ação é ótimo ✻✻", "✻</think>ok"]), "ok");
    }
}
