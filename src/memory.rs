//! The project's notes for the model: LOKAL.md, and the AGENTS.md many
//! coding tools read, in the project's root and in every folder down to the
//! working directory. They go along with every request, so they get a share
//! of the context and no more.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::approvals;

/// In each folder, LOKAL.md comes first.
const NAMES: [&str; 2] = ["LOKAL.md", "AGENTS.md"];

#[derive(PartialEq)]
pub struct Note {
    pub path: PathBuf,
    /// The path from the project's root, as the person reads it.
    pub name: String,
    /// What of it goes along: all of it, or the lines that fit.
    pub text: String,
    pub cut: bool,
}

/// The notes for `folder`, the root's first, in at most `room` characters
/// together. One that doesn't fit at all still says it's there.
pub fn load(folder: &Path, room: usize) -> Vec<Note> {
    let root = approvals::project_root(folder);
    let mut left = room;
    let mut notes = Vec::new();
    for path in files(folder, &root) {
        let Some((text, whole)) = read_start(&path, left) else {
            continue;
        };
        if whole && text.trim().is_empty() {
            continue;
        }
        left -= text.len();
        let name = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        notes.push(Note {
            path,
            name,
            text,
            cut: !whole,
        });
    }
    notes
}

/// The project's root, where a first LOKAL.md goes.
pub fn first_path(folder: &Path) -> PathBuf {
    approvals::project_root(folder).join(NAMES[0])
}

/// LOKAL.md and AGENTS.md from `root` down to `folder`, each file once
/// however it's reached. Only those that stay inside the project: a link out
/// of it would hand the model any file of the person's, unasked.
fn files(folder: &Path, root: &Path) -> Vec<PathBuf> {
    let Ok(real_root) = fs::canonicalize(root) else {
        return Vec::new();
    };
    let mut folders = Vec::new();
    for ancestor in folder.ancestors() {
        folders.push(ancestor);
        if ancestor == root {
            break;
        }
    }
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for dir in folders.into_iter().rev() {
        for name in NAMES {
            let path = dir.join(name);
            if let Ok(real) = fs::canonicalize(&path)
                && real.starts_with(&real_root)
                && seen.insert(real)
            {
                files.push(path);
            }
        }
    }
    files
}

/// The file's text, whole when it fits in `limit` bytes, or the lines that
/// do. None when it can't be read or isn't text.
fn read_start(path: &Path, limit: usize) -> Option<(String, bool)> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    let whole = bytes.len() <= limit;
    bytes.truncate(limit);
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        // Cut in the middle of a character: up to it.
        Err(error) if error.utf8_error().error_len().is_none() => {
            let valid = error.utf8_error().valid_up_to();
            String::from_utf8(error.into_bytes()[..valid].to_vec()).ok()?
        }
        Err(_) => return None,
    };
    if whole {
        return Some((text, true));
    }
    // Whole lines only, so none goes along half.
    let end = text.rfind('\n').map_or(0, |newline| newline + 1);
    Some((text[..end].to_string(), false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("lokal-memory-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("repo/.git")).unwrap();
        fs::create_dir_all(base.join("repo/src/deep")).unwrap();
        base
    }

    #[test]
    fn notes_come_from_the_root_down_to_the_folder() {
        let base = folder("order");
        fs::write(base.join("AGENTS.md"), "outside the project\n").unwrap();
        fs::write(base.join("repo/AGENTS.md"), "root agents\n").unwrap();
        fs::write(base.join("repo/LOKAL.md"), "root lokal\n").unwrap();
        fs::write(base.join("repo/src/LOKAL.md"), "src lokal\n").unwrap();
        fs::write(base.join("repo/src/deep/AGENTS.md"), " \n").unwrap();
        let notes = load(&base.join("repo/src/deep"), 1000);
        let names: Vec<&str> = notes.iter().map(|note| note.name.as_str()).collect();
        assert_eq!(names, ["LOKAL.md", "AGENTS.md", "src/LOKAL.md"]);
        assert_eq!(notes[0].text, "root lokal\n");
        assert!(notes.iter().all(|note| !note.cut));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_file_reached_twice_or_leading_outside_goes_once_or_not_at_all() {
        let base = folder("links");
        fs::write(base.join("repo/LOKAL.md"), "the notes\n").unwrap();
        fs::write(base.join("secret"), "a key\n").unwrap();
        std::os::unix::fs::symlink("LOKAL.md", base.join("repo/AGENTS.md")).unwrap();
        std::os::unix::fs::symlink("../../secret", base.join("repo/src/LOKAL.md")).unwrap();
        let notes = load(&base.join("repo/src"), 1000);
        let names: Vec<&str> = notes.iter().map(|note| note.name.as_str()).collect();
        assert_eq!(names, ["LOKAL.md"]);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn notes_past_the_room_are_cut_at_a_line() {
        let base = folder("room");
        fs::write(base.join("repo/LOKAL.md"), "first line\nsecond line\n").unwrap();
        fs::write(base.join("repo/AGENTS.md"), "more\n").unwrap();
        let notes = load(&base.join("repo"), 15);
        assert_eq!(notes[0].text, "first line\n");
        assert!(notes[0].cut);
        // No room left: it still says it's there.
        assert_eq!(notes[1].text, "");
        assert!(notes[1].cut);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_cut_never_splits_a_character_and_binary_is_skipped() {
        let base = folder("text");
        fs::write(base.join("repo/LOKAL.md"), "ação\nmais\n").unwrap();
        fs::write(base.join("repo/AGENTS.md"), [0xff, 0xfe, 0x00]).unwrap();
        // The fourth byte is half of "ã".
        assert_eq!(
            read_start(&base.join("repo/LOKAL.md"), 4),
            Some((String::new(), false))
        );
        assert_eq!(
            read_start(&base.join("repo/LOKAL.md"), 7),
            Some(("ação\n".into(), false))
        );
        assert_eq!(read_start(&base.join("repo/AGENTS.md"), 100), None);
        fs::remove_dir_all(&base).unwrap();
    }
}
