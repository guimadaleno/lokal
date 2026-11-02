//! The colored diff shown before a file changes. A long one shows only its
//! start, so a new file doesn't flood the conversation; the rest is one
//! choice away in the approval menu.

use similar::{ChangeTag, TextDiff};

use crate::{layout, style, text};

/// A diff up to this many lines shows whole.
const LONG: usize = 30;
/// How many lines a longer one shows.
const START: usize = 20;

/// Prints the change, or only its start when it's long, and returns the
/// lines left out.
pub fn show(old: &str, new: &str) -> Vec<String> {
    let mut lines = lines(old, new);
    let rest = if lines.len() > LONG {
        lines.split_off(START)
    } else {
        Vec::new()
    };
    print(&lines);
    if !rest.is_empty() {
        eprintln!("  {}", style::gray(&text::more_lines(rest.len())));
    }
    rest
}

pub fn print(lines: &[String]) {
    for line in lines {
        eprintln!("  {line}");
    }
}

fn lines(old: &str, new: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let diff = TextDiff::from_lines(old, new);
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        lines.push(style::gray(&hunk.header().to_string()));
        for change in hunk.iter_changes() {
            let line = layout::printable(change.value().trim_end_matches(['\n', '\r']));
            lines.push(match change.tag() {
                ChangeTag::Delete => style::red(&format!("-{line}")),
                ChangeTag::Insert => style::green(&format!("+{line}")),
                ChangeTag::Equal => style::gray(&format!(" {line}")),
            });
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(count: usize) -> String {
        (1..=count).map(|n| format!("line {n}\n")).collect()
    }

    #[test]
    fn a_long_diff_holds_back_all_but_its_start() {
        // A header and one line per insert.
        assert!(show("", &numbered(LONG - 1)).is_empty());
        let rest = show("", &numbered(100));
        assert_eq!(rest.len(), 101 - START);
        assert!(rest.last().unwrap().contains("+line 100"));
    }

    #[test]
    fn a_line_cant_hide_itself() {
        let shown = lines("", "rm -rf ~ \x1b[2K\x1b[1Gls\n");
        assert!(shown[1].contains("+rm -rf ~ ^[[2K^[[1Gls"), "{shown:?}");
        // A lone \r ends a line of the diff, so what follows it can't cover it.
        let shown = lines("", "rm -rf ~\rls\n");
        assert!(shown[1].contains("+rm -rf ~") && shown[2].contains("+ls"));
    }
}
