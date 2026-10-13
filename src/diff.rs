//! The colored diff shown before a file changes.

use similar::{ChangeTag, TextDiff};

use crate::style;

pub fn show(old: &str, new: &str) {
    let diff = TextDiff::from_lines(old, new);
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        eprintln!("  {}", style::gray(&hunk.header().to_string()));
        for change in hunk.iter_changes() {
            let line = change.value().trim_end_matches(['\n', '\r']);
            let shown = match change.tag() {
                ChangeTag::Delete => style::red(&format!("-{line}")),
                ChangeTag::Insert => style::green(&format!("+{line}")),
                ChangeTag::Equal => style::gray(&format!(" {line}")),
            };
            eprintln!("  {shown}");
        }
    }
}
