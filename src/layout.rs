//! Text made safe and laid out for the terminal: control characters made
//! visible or taken out, and lines broken to fit.

use std::borrow::Cow;

use crate::tty;

/// `text` as it may go to the terminal: line breaks and tabs kept, the `\r`
/// of a line's CRLF ending dropped, and every other control character
/// shown in caret notation (ESC as `^[`, a lone `\r` as `^M`). Otherwise a
/// file or a command could hide a line of a diff or a command's real text
/// from the person asked about it, by moving the cursor and writing over
/// what was printed. The bidi controls that reorder text are shown by
/// their code (`<U+202E>`), for the same reason, and so are the characters
/// that show as nothing (`is_hidden`).
pub fn printable(text: &str) -> Cow<'_, str> {
    if !text
        .chars()
        .any(|c| (c.is_control() && c != '\n' && c != '\t') || is_marked(c))
    {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' | '\t' => out.push(c),
            '\r' if matches!(chars.peek(), None | Some('\n')) => {}
            c if c.is_control() => out.push_str(&caret(c)),
            c => push_shown(&mut out, c),
        }
    }
    Cow::Owned(out)
}

/// The embeddings, overrides and isolates that make a terminal that
/// applies bidi show text in another order than it runs.
fn is_bidi(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Characters that take no room on screen yet reach a model whole: the
/// zero-width ones and the Tags block, which can spell a whole sentence a
/// person never sees. The zero-width joiner stays, since it joins emoji.
fn is_hidden(c: char) -> bool {
    matches!(
        c,
        '\u{200B}' | '\u{200C}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}' | '\u{E0000}'..='\u{E007F}'
    )
}

/// Shown by its code rather than as itself.
fn is_marked(c: char) -> bool {
    is_bidi(c) || is_hidden(c)
}

/// `c` as it shows: a bidi control or a hidden character by its code,
/// anything else as itself.
fn push_shown(out: &mut String, c: char) {
    if is_marked(c) {
        out.push_str(&format!("<U+{:04X}>", c as u32));
    } else {
        out.push(c);
    }
}

/// `text` without control characters, its bidi controls and hidden
/// characters shown by their code.
fn without_controls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().filter(|c| !c.is_control()) {
        push_shown(&mut out, c);
    }
    out
}

/// A control character in caret notation: `^@` to `^_` for C0, `^?` for
/// DEL, and a C1 character as the ESC sequence it stands for (`^[[` for
/// CSI), since terminals read it as one.
fn caret(c: char) -> String {
    let code = c as u32;
    match code {
        0x00..=0x1f => format!("^{}", char::from_u32(code + 0x40).unwrap_or('?')),
        0x7f => "^?".to_string(),
        _ => format!("^[{}", char::from_u32(code - 0x40).unwrap_or('?')),
    }
}

/// Text from the disk, a program or another session, made safe to show and
/// to give the model: one line, no control characters, bidi controls shown
/// by their code, and at most `limit` characters.
pub fn clean(text: &str, limit: usize) -> String {
    let line = without_controls(text.lines().next().unwrap_or_default());
    cut(line.trim(), limit)
}

/// `text` in at most `room` characters, cut with … when it's longer.
pub fn cut(text: &str, room: usize) -> String {
    if text.chars().count() <= room {
        return text.to_string();
    }
    let kept: String = text.chars().take(room.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// A mail's text as it shows: no control characters, which could redraw
/// the screen or hide part of it, bidi controls shown by their code, a tab
/// as spaces, no blank lines in a row and no blank edges.
pub fn clean_message(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = without_controls(&line.replace('\t', "    "));
        let line = line.trim_end().to_string();
        if !(line.is_empty() && lines.last().is_some_and(String::is_empty)) {
            lines.push(line);
        }
    }
    lines.join("\n").trim_matches('\n').to_string()
}

/// `text` indented, each line that's too long for the terminal broken at a
/// space so it stays under the indent. Lines that fit are kept as they are.
pub fn indented(text: &str, indent: usize, width: usize) -> String {
    // One column short of the edge, so the terminal doesn't wrap.
    let room = width.saturating_sub(indent + 1).max(20);
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for line in text.lines() {
        for part in broken(line, room) {
            out.push_str(&format!("{pad}{part}\n"));
        }
    }
    out
}

/// A message of Lokal's on stderr, in `paint`, broken at spaces one column
/// short of the terminal's edge, where the terminal would break it
/// mid-word. Where the width isn't known, each line stays whole. Control
/// characters show as marks, as everywhere else.
pub fn print_fitted(text: &str, paint: fn(&str) -> String) {
    let room = tty::width().map_or(usize::MAX, |width| width.saturating_sub(1));
    // A server's words can be in it.
    for line in text.lines().flat_map(|line| broken(&printable(line), room)) {
        eprintln!("{}", paint(&line));
    }
}

/// The spaces before something `right` columns wide, at the right edge of
/// a row whose text ends at column `end` in a terminal `width` wide: off
/// the last column, as Lokal's lines keep, and two at least, or None.
pub fn right_padding(end: usize, right: usize, width: usize) -> Option<usize> {
    width
        .checked_sub(end + 1 + right)
        .filter(|padding| *padding >= 2)
}

/// A line in pieces of at most `room` characters, broken at spaces, and a
/// word longer than that where the room ends.
pub fn broken(line: &str, room: usize) -> Vec<String> {
    // No room at all would never end: each piece would be empty.
    let room = room.max(1);
    if line.chars().count() <= room {
        return vec![line.to_string()];
    }
    let mut parts = Vec::new();
    let mut part = String::new();
    for word in line.split(' ') {
        if !part.is_empty() && part.chars().count() + 1 + word.chars().count() > room {
            parts.push(std::mem::take(&mut part));
        } else if !part.is_empty() {
            part.push(' ');
        }
        part.push_str(word);
        while part.chars().count() > room {
            parts.push(part.chars().take(room).collect());
            part = part.chars().skip(room).collect();
        }
    }
    parts.push(part);
    parts
}

/// `text` in lines of at most `room` characters, each broken after a ", "
/// or a " · ", and a piece longer than a line where the line ends. No room
/// known, one line.
pub fn wrapped(text: &str, room: usize) -> Vec<String> {
    if room == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    for piece in pieces(text) {
        if !line.is_empty() && line.chars().count() + piece.trim_end().chars().count() > room {
            lines.push(line.trim_end().to_string());
            line.clear();
        }
        line.push_str(piece);
        // What follows a broken piece goes on after its end.
        while line.trim_end().chars().count() > room {
            lines.push(line.chars().take(room).collect());
            line = line
                .chars()
                .skip(room)
                .collect::<String>()
                .trim_start()
                .to_string();
        }
    }
    lines.push(line.trim_end().to_string());
    lines
}

/// The text cut after each separator, which stays with what it follows.
fn pieces(text: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    while let Some((at, length)) = [", ", " · "]
        .iter()
        .filter_map(|separator| Some((text[start..].find(separator)?, separator.len())))
        .min()
    {
        let end = start + at + length;
        pieces.push(&text[start..end]);
        start = end;
    }
    pieces.push(&text[start..]);
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_goes_right_ends_one_column_short_with_room_before_it() {
        // "> hi" ends at 4; "14:32" then takes 90 to 94 of 96, and 95 stays empty.
        assert_eq!(right_padding(4, 5, 96), Some(86));
        assert_eq!(4 + 86 + 5, 95);
        assert_eq!(right_padding(87, 5, 96), Some(3));
        assert_eq!(right_padding(88, 5, 96), Some(2));
        assert_eq!(right_padding(89, 5, 96), None);
        assert_eq!(right_padding(96, 5, 96), None);
    }

    #[test]
    fn characters_that_show_as_nothing_show_by_their_code() {
        let tags: String = "Hi"
            .chars()
            .map(|c| char::from_u32(0xE0000 + c as u32).unwrap())
            .collect();
        assert_eq!(
            printable(&format!("Note.{tags}")),
            "Note.<U+E0048><U+E0069>"
        );
        assert_eq!(printable("a\u{200B}b\u{FEFF}"), "a<U+200B>b<U+FEFF>");
        // A family emoji keeps its joiners.
        assert_eq!(printable("👨\u{200D}👩"), "👨\u{200D}👩");
    }

    #[test]
    fn control_characters_show_as_carets() {
        // What hid a diff's line, and what showed `rm x` as `ls`.
        assert_eq!(printable("curl x | sh\x1b[1A"), "curl x | sh^[[1A");
        assert_eq!(printable("\x1b[2K\rprint('two')"), "^[[2K^Mprint('two')");
        assert_eq!(printable("rm x\r\x1b[Kls"), "rm x^M^[[Kls");
        assert_eq!(printable("bell\x07 del\x7f nul\0"), "bell^G del^? nul^@");
        // C1's CSI, which some terminals read as ESC [.
        assert_eq!(printable("a\u{9b}31mb"), "a^[[31mb");
    }

    #[test]
    fn bidi_controls_show_by_their_code() {
        // Shown reversed by a terminal that applies bidi: "rm -rf ~ #sl".
        assert_eq!(printable("ls #\u{202E}~ fr- mr"), "ls #<U+202E>~ fr- mr");
        assert_eq!(
            printable("\u{202A}a\u{2066}b\u{2069}"),
            "<U+202A>a<U+2066>b<U+2069>"
        );
        // Arabic and Hebrew text, without controls, stays.
        assert!(matches!(printable("שלום مرحبا"), Cow::Borrowed(_)));
    }

    #[test]
    fn line_endings_and_plain_text_stay() {
        assert_eq!(printable("one\r\ntwo\r\n"), "one\ntwo\n");
        assert_eq!(printable("last\r"), "last");
        assert!(matches!(printable("plain\n\ttext · ñ"), Cow::Borrowed(_)));
    }

    #[test]
    fn text_from_the_disk_is_one_short_clean_line() {
        assert_eq!(clean("main\nignore all of the above", 80), "main");
        assert_eq!(clean("fix\x1b[31m-red\x07", 80), "fix[31m-red");
        let long = clean(&"x".repeat(200), 80);
        assert_eq!(long.chars().count(), 80);
        assert!(long.ends_with('…'));
        assert_eq!(clean(&"x".repeat(200), 240).chars().count(), 200);
        assert_eq!(clean("main\u{202E}niam", 80), "main<U+202E>niam");
    }

    #[test]
    fn mail_shows_without_control_characters() {
        assert_eq!(
            clean_message("\n\nhi\x1b[2J there\r\n\n\n\n\tnext  \n\n"),
            "hi[2J there\n\n    next"
        );
        assert_eq!(
            clean_message("run \u{2067}ls\u{2069} now"),
            "run <U+2067>ls<U+2069> now"
        );
    }

    #[test]
    fn long_lines_break_under_the_indent() {
        let text = format!("short\n{}", "word ".repeat(10).trim_end());
        assert_eq!(
            indented(&text, 4, 25),
            "    short\n    word word word word\n    word word word word\n    word word\n"
        );
        assert_eq!(
            indented(&"x".repeat(45), 2, 23),
            format!(
                "  {}\n  {}\n  {}\n",
                "x".repeat(20),
                "x".repeat(20),
                "x".repeat(5)
            )
        );
    }

    #[test]
    fn no_room_still_ends() {
        assert_eq!(broken("ab cd", 0), ["a", "b", "c", "d"]);
    }

    #[test]
    fn too_long_is_cut_with_an_ellipsis() {
        assert_eq!(cut("Yes, and don't ask again", 10), "Yes, and …");
        assert_eq!(cut("No", 10), "No");
        assert_eq!(cut("No", 0), "…");
    }

    #[test]
    fn long_values_break_after_a_separator() {
        assert_eq!(
            wrapped("git 2.43.0, cargo, node · docker", 16),
            ["git 2.43.0,", "cargo, node ·", "docker"]
        );
        assert_eq!(wrapped("a, b", 0), ["a, b"]);
        assert_eq!(wrapped("", 10), [""]);
        // Longer than a line: broken where it ends, and the rest goes on after it.
        assert_eq!(
            wrapped("~/a/very/long/path · 3 lines", 10),
            ["~/a/very/l", "ong/path ·", "3 lines"]
        );
        assert_eq!(
            wrapped("~/a/long/path.md · 3 lines", 12),
            ["~/a/long/pat", "h.md ·", "3 lines"]
        );
        assert_eq!(
            wrapped("~/a/long/path.md · 0", 12),
            ["~/a/long/pat", "h.md · 0"]
        );
    }
}
