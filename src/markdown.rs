//! Light Markdown in the answer, styled for a terminal as it streams: bold,
//! inline code, headings, code blocks and links. Everything else shows as
//! written. Styles never outlive their line, so a stray marker can't paint
//! the rest of the screen. Only the screen is styled: the conversation keeps
//! the model's own text.

const BOLD_ON: &str = "\x1b[1m";
const BOLD_OFF: &str = "\x1b[22m";
/// A soft blue-gray: orange already marks the person's lines and Lokal acting.
const CODE_ON: &str = "\x1b[38;5;110m";
const CODE_OFF: &str = "\x1b[39m";
const FENCE: &str = "```";
/// A `[` that hasn't become a link within this many bytes is just a bracket.
const LONGEST_LINK: usize = 300;

enum LineKind {
    /// Not known yet: the line may still turn out to be a heading or a fence.
    Undecided,
    Fence,
    /// A heading, whose marker takes this many bytes of the line.
    Heading(usize),
    Plain,
}

#[derive(Default)]
pub struct Markdown {
    /// The start of a line, held until its kind is known.
    held: String,
    mid_line: bool,
    /// Dropping the rest of a fence line: the ``` and its language.
    in_fence_line: bool,
    in_block: bool,
    block_line_open: bool,
    heading: bool,
    bold: bool,
    code: bool,
    /// A `*` that may be the first half of `**`.
    star: bool,
    /// Held from a `[` that may become a link.
    link: Option<String>,
    last: char,
}

impl Markdown {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the next piece of the answer and returns what to print now,
    /// its control characters shown as marks, so only Lokal's own styles
    /// reach the terminal.
    pub fn push(&mut self, piece: &str) -> String {
        let mut out = String::new();
        for c in crate::layout::printable(piece).chars() {
            self.feed(c, &mut out);
        }
        out
    }

    /// What's left once the answer ends, with every style closed.
    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        if !self.mid_line && !self.held.is_empty() {
            self.mid_line = true;
            for c in std::mem::take(&mut self.held).chars() {
                self.feed_text(c, &mut out);
            }
        }
        if self.star {
            self.star = false;
            out.push('*');
        }
        if let Some(link) = self.link.take() {
            out.push_str(&link);
        }
        self.close_line_styles(&mut out);
        *self = Self::default();
        out
    }

    fn feed(&mut self, c: char, out: &mut String) {
        if self.in_fence_line {
            if c == '\n' {
                self.in_fence_line = false;
            }
            return;
        }
        if self.mid_line {
            self.feed_text(c, out);
            return;
        }
        if c == '\n' {
            out.push_str(&std::mem::take(&mut self.held));
            out.push('\n');
            return;
        }
        self.held.push(c);
        match self.line_kind() {
            LineKind::Undecided => {}
            LineKind::Fence => {
                self.held.clear();
                self.in_block = !self.in_block;
                self.in_fence_line = true;
            }
            LineKind::Heading(marker) => {
                let rest = self.held.split_off(marker);
                self.held.clear();
                self.mid_line = true;
                self.heading = true;
                out.push_str(BOLD_ON);
                for c in rest.chars() {
                    self.feed_text(c, out);
                }
            }
            LineKind::Plain => {
                self.mid_line = true;
                for c in std::mem::take(&mut self.held).chars() {
                    self.feed_text(c, out);
                }
            }
        }
    }

    fn line_kind(&self) -> LineKind {
        let start = self.held.trim_start_matches(' ');
        if start.is_empty() || (FENCE.starts_with(start) && start.len() < FENCE.len()) {
            return LineKind::Undecided;
        }
        if start.starts_with(FENCE) {
            return LineKind::Fence;
        }
        if self.in_block {
            return LineKind::Plain;
        }
        let hashes = start.len() - start.trim_start_matches('#').len();
        if hashes == 0 || hashes > 6 {
            return LineKind::Plain;
        }
        match start[hashes..].chars().next() {
            None => LineKind::Undecided,
            Some(' ') => LineKind::Heading(self.held.len()),
            Some(_) => LineKind::Plain,
        }
    }

    fn feed_text(&mut self, c: char, out: &mut String) {
        if self.in_block {
            if c == '\n' {
                if self.block_line_open {
                    out.push_str(CODE_OFF);
                }
                self.block_line_open = false;
                self.end_line(out);
            } else {
                if !self.block_line_open {
                    out.push_str(CODE_ON);
                    self.block_line_open = true;
                }
                out.push(c);
            }
            return;
        }
        if self.link.is_some() {
            self.feed_link(c, out);
            return;
        }
        if self.code {
            match c {
                '`' => {
                    out.push_str(CODE_OFF);
                    self.code = false;
                }
                '\n' => self.end_line(out),
                _ => out.push(c),
            }
            return;
        }
        if self.star {
            self.star = false;
            if c == '*' {
                self.toggle_bold(out);
                return;
            }
            out.push('*');
            self.last = '*';
        }
        match c {
            '*' => self.star = true,
            '`' => {
                out.push_str(CODE_ON);
                self.code = true;
            }
            '[' => self.link = Some(String::from("[")),
            '\n' => self.end_line(out),
            _ => {
                out.push(c);
                self.last = c;
            }
        }
    }

    /// `**` opens bold, unless it sits right after a letter or digit, as in
    /// `2**8`; inside a heading, the whole line is bold already.
    fn toggle_bold(&mut self, out: &mut String) {
        if self.heading {
            return;
        }
        if self.bold {
            out.push_str(BOLD_OFF);
            self.bold = false;
        } else if self.last.is_alphanumeric() {
            out.push_str("**");
            self.last = '*';
        } else {
            out.push_str(BOLD_ON);
            self.bold = true;
        }
    }

    /// `[text](url)` becomes `text (url)`; anything else stays as written.
    fn feed_link(&mut self, c: char, out: &mut String) {
        let Some(link) = self.link.as_mut() else {
            return;
        };
        link.push(c);
        let mut done = None;
        if c == '\n' || link.len() > LONGEST_LINK {
            done = Some(false);
        } else if let Some(close) = link.find(']') {
            let after = &link[close + 1..];
            if !after.is_empty() {
                if !after.starts_with('(') || c.is_whitespace() {
                    done = Some(false);
                } else if c == ')' {
                    done = Some(true);
                }
            }
        }
        match done {
            Some(true) => {
                let link = self.link.take().unwrap_or_default();
                let close = link.find(']').unwrap_or(0);
                let text = &link[1..close];
                let url = &link[close + 2..link.len() - 1];
                out.push_str(&format!("{text} ({url})"));
                self.last = ')';
            }
            Some(false) => {
                // Not a link: the bracket is plain text, and the rest is read again.
                let held = self.link.take().unwrap_or_default();
                out.push('[');
                self.last = '[';
                for c in held[1..].chars() {
                    self.feed_text(c, out);
                }
            }
            None => {}
        }
    }

    fn end_line(&mut self, out: &mut String) {
        self.close_line_styles(out);
        out.push('\n');
        self.mid_line = false;
        self.last = '\n';
    }

    fn close_line_styles(&mut self, out: &mut String) {
        if self.star {
            self.star = false;
            out.push('*');
        }
        if self.code || self.block_line_open {
            out.push_str(CODE_OFF);
        }
        if self.bold || self.heading {
            out.push_str(BOLD_OFF);
        }
        self.code = false;
        self.block_line_open = false;
        self.bold = false;
        self.heading = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(pieces: &[&str]) -> String {
        let mut markdown = Markdown::new();
        let mut out: String = pieces.iter().map(|piece| markdown.push(piece)).collect();
        out.push_str(&markdown.finish());
        out
    }

    fn bold(text: &str) -> String {
        format!("{BOLD_ON}{text}{BOLD_OFF}")
    }

    fn code(text: &str) -> String {
        format!("{CODE_ON}{text}{CODE_OFF}")
    }

    #[test]
    fn bold_even_when_split_across_pieces() {
        assert_eq!(
            render(&["um **dois** três"]),
            format!("um {} três", bold("dois"))
        );
        assert_eq!(
            render(&["um *", "*do", "is*", "* três"]),
            format!("um {} três", bold("dois"))
        );
    }

    #[test]
    fn a_power_is_not_bold() {
        assert_eq!(render(&["2**8 é 256"]), "2**8 é 256");
        assert_eq!(render(&["glob *.rs aqui"]), "glob *.rs aqui");
    }

    #[test]
    fn inline_code_loses_its_backticks() {
        assert_eq!(
            render(&["use `ls -la` aqui"]),
            format!("use {} aqui", code("ls -la"))
        );
        assert_eq!(render(&["`a **b**`"]), code("a **b**"));
    }

    #[test]
    fn headings_are_bold_lines() {
        assert_eq!(
            render(&["## Título\ntexto"]),
            format!("{}\ntexto", bold("Título"))
        );
        assert_eq!(render(&["#", "# Dividido"]), bold("Dividido"));
        assert_eq!(render(&["#hashtag"]), "#hashtag");
    }

    #[test]
    fn code_blocks_lose_their_fences() {
        assert_eq!(
            render(&["Rode:\n```bash\nls **x**\n", "pwd\n```\nfim"]),
            format!("Rode:\n{}\n{}\nfim", code("ls **x**"), code("pwd"))
        );
    }

    #[test]
    fn links_show_their_address() {
        assert_eq!(
            render(&["veja [docs](https://exam", "ple.com)."]),
            "veja docs (https://example.com)."
        );
        assert_eq!(render(&["array[0] = 1"]), "array[0] = 1");
        assert_eq!(render(&["[1] nota"]), "[1] nota");
    }

    #[test]
    fn styles_end_with_their_line() {
        assert_eq!(
            render(&["**sem fim\nnormal"]),
            format!("{}\nnormal", bold("sem fim"))
        );
        assert_eq!(
            render(&["`aberto\nnormal"]),
            format!("{}\nnormal", code("aberto"))
        );
    }

    #[test]
    fn control_characters_show_as_marks() {
        assert_eq!(render(&["ok\x1b]52;c;eA==\x07\n"]), "ok^[]52;c;eA==^G\n");
    }

    #[test]
    fn everything_else_is_as_written() {
        let text = "- um\n- dois\n\n1. três\n| a | b |";
        assert_eq!(render(&[text]), text);
    }
}
