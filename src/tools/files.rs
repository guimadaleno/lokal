//! The file tools: read_file, write_file and edit_file, and where a change
//! may be put to the person on Yes.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{ErrorKind, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use super::{
    Asking, CONTEXT_FULL, LEAST_ROOM, Outcome, everyday_folders, limit, propose, shown, text_arg,
};
use crate::context::CHARS_PER_TOKEN;
use crate::image::{self, Image};
use crate::input::Input;
use crate::judge::{Case, Change};
use crate::mode::Mode;
use crate::peers::Peers;
use crate::sandbox::{NOTES, PROTECTED};
use crate::{config, style, text};

/// How much of a file one read gives the model; a longer one says where to read on.
pub(super) const FILE_LIMIT: usize = 30_000;
/// The most of a file read into memory, for a part of it or a change to it.
const READ_CAP: u64 = 64 * 1024 * 1024;
/// A file change that changes nothing, which isn't put to the person.
const UNCHANGED: &str = "Nothing to do: the file already has that content.";

pub(super) fn read_file(
    args: &Value,
    room: Option<usize>,
    sees: Option<bool>,
) -> Result<Outcome, String> {
    let path = path_arg(args)?;
    let from = args["from_line"].as_u64().map(|line| line.max(1) as usize);
    let to = args["to_line"].as_u64().map(|line| line as usize);
    let range = match (from, to) {
        (None, None) => String::new(),
        _ => format!(
            " {}-{}",
            from.unwrap_or(1),
            to.map_or(String::new(), |to| to.to_string())
        ),
    };
    let limit = limit(room, FILE_LIMIT);
    if limit < LEAST_ROOM {
        print_read(&path, &range);
        return Ok(Outcome::Done(format!(
            "Not read: there's no room left in the context for it. {CONTEXT_FULL}"
        )));
    }
    let bytes = read_bytes(&path);
    // Text is anything that reads as UTF-8, whatever it starts with.
    let image = match &bytes {
        Ok(Some(bytes)) if std::str::from_utf8(bytes).is_err() => image::read(bytes),
        _ => None,
    };
    match &image {
        Some(image) => print_read(
            &path,
            &format!(" {}", text::image_size(image.width, image.height)),
        ),
        None => print_read(&path, &range),
    }
    let bytes = bytes?.ok_or_else(|| no_such_file(&path))?;
    if let Some(image) = image {
        return look_at(&path, &image, &bytes, room, sees);
    }
    let contents = String::from_utf8(bytes).map_err(|_| match sees {
        Some(true) => format!(
            "{} isn't a text file, nor a PNG, JPEG or WebP image.",
            path.display()
        ),
        _ => format!("{} isn't a text file.", path.display()),
    })?;
    read_part(&contents, from, to, limit).map(Outcome::Done)
}

fn print_read(path: &Path, detail: &str) {
    eprintln!(
        "{} {}{}",
        style::accent(text::STEP_READ),
        shown(path),
        style::gray(detail)
    );
}

/// An image, for a model that sees and has room for it. One too big for
/// the room is turned down, saying so: Lokal can't shrink it.
fn look_at(
    path: &Path,
    image: &Image,
    bytes: &[u8],
    room: Option<usize>,
    sees: Option<bool>,
) -> Result<Outcome, String> {
    match sees {
        Some(true) => {}
        Some(false) => {
            return Err(format!(
                "{} is an image, and the model loaded doesn't take images.",
                path.display()
            ));
        }
        None => {
            return Err(format!(
                "{} is an image, and this server doesn't say whether its model takes images, \
                 so Lokal doesn't send them.",
                path.display()
            ));
        }
    }
    if bytes.len() as u64 > image::MOST_BYTES {
        return Err(format!(
            "{} is over {} MB, too big to send to the model. A smaller copy would do: \
             make one with a command first.",
            path.display(),
            image::MOST_BYTES / 1024 / 1024
        ));
    }
    let tokens = image.tokens();
    // Where the context's size isn't known, as much as a read of text.
    let room = (room.unwrap_or(FILE_LIMIT) / CHARS_PER_TOKEN) as u64;
    if tokens > room {
        return Ok(Outcome::Done(format!(
            "Not read: the image would take about {tokens} tokens of the context, and there's \
             room for about {room}. A smaller copy would fit: make one with a command, if that's \
             fine, or answer with what you have."
        )));
    }
    Ok(Outcome::Images {
        text: format!(
            "{}: a {}×{} {} image.",
            path.display(),
            image.width,
            image.height,
            image.kind.name()
        ),
        urls: vec![image::data_url(image.kind, bytes)],
    })
}

/// A file's bytes, or None when there's no such file. Anything but a
/// regular file is refused before it's opened: a device or a pipe could
/// never end, or never begin. So is one too big to hold in memory.
fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    // A device is never opened: opening one can act on it.
    match fs::metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Ok(meta) if !meta.is_file() => {
            return Err(format!("{} isn't a regular file.", path.display()));
        }
        _ => {}
    }
    // Opened without waiting, then checked again, so a pipe put in the file's
    // place after the first check can't block either.
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("can't read {}: {error}", path.display())),
        Ok(file) => file,
    };
    if !file.metadata().is_ok_and(|meta| meta.is_file()) {
        return Err(format!("{} isn't a regular file.", path.display()));
    }
    let mut bytes = Vec::new();
    file.take(READ_CAP + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("can't read {}: {error}", path.display()))?;
    if bytes.len() as u64 > READ_CAP {
        return Err(format!(
            "{} is over {} MB, too big to read here. Use a command on it instead: \
             grep -n, head or tail.",
            path.display(),
            READ_CAP / 1024 / 1024
        ));
    }
    Ok(Some(bytes))
}

/// A text file's contents, or None when there's no such file.
fn read_text(path: &Path) -> Result<Option<String>, String> {
    let Some(bytes) = read_bytes(path)? else {
        return Ok(None);
    };
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| format!("{} isn't a text file.", path.display()))
}

fn no_such_file(path: &Path) -> String {
    format!("can't read {}: there's no such file.", path.display())
}

/// The lines from `from` to `to`, or the whole file, as many whole lines as
/// fit in `limit` characters. A part cut short says where to read on, so the
/// model reads the rest instead of guessing at it.
fn read_part(
    contents: &str,
    from: Option<usize>,
    to: Option<usize>,
    limit: usize,
) -> Result<String, String> {
    if from.is_none() && to.is_none() && contents.len() <= limit {
        return Ok(contents.to_string());
    }
    let lines: Vec<&str> = contents.lines().collect();
    let total = lines.len();
    let from = from.unwrap_or(1);
    if from > total {
        return Err(format!(
            "the file has {total} lines, so line {from} is past its end."
        ));
    }
    let to = to.unwrap_or(total).min(total);
    if to < from {
        return Err(format!("to_line {to} comes before from_line {from}."));
    }
    let part = &lines[from - 1..to];
    let mut size = 0;
    let fitting = part
        .iter()
        .take_while(|line| {
            size += line.len() + 1;
            size <= limit
        })
        .count();
    if fitting == part.len() {
        return Ok(format!(
            "Lines {from}-{to} of {total}:\n{}",
            part.join("\n")
        ));
    }
    if fitting == 0 {
        let start = &part[0][..part[0].floor_char_boundary(limit)];
        return Ok(format!(
            "Line {from} of {total} is too long to show whole; here's its start:\n{start}"
        ));
    }
    let last = from + fitting - 1;
    Ok(format!(
        "Lines {from}-{last} of {total}; the rest doesn't fit in one read. These lines get dropped \
         as the context fills and your reasoning isn't kept: write down in your reply the facts \
         from them your answer needs, then read on from line {}:\n{}",
        last + 1,
        part[..fitting].join("\n")
    ))
}

pub(super) fn write_file(
    args: &Value,
    input: &mut Input,
    mode: Mode,
    judge: Option<&Case>,
    scratch: Option<&Path>,
    peers: Option<&Peers>,
) -> Result<Outcome, String> {
    let path = writable_path(args)?;
    let content = text_arg(args, "content")?;
    let old = read_text(&path)?;
    if old.as_deref() == Some(content) {
        return Ok(Outcome::Done(UNCHANGED.into()));
    }
    let (step, change) = match &old {
        Some(old) => (text::STEP_EDIT, Change::Replace(old.lines().count())),
        None => (text::STEP_CREATE, Change::Create),
    };
    propose(
        &path,
        step,
        old.as_deref().unwrap_or(""),
        content,
        input,
        asking(&path, scratch, mode, judge, change),
        peers,
    )
}

pub(super) fn edit_file(
    args: &Value,
    input: &mut Input,
    mode: Mode,
    judge: Option<&Case>,
    scratch: Option<&Path>,
    peers: Option<&Peers>,
) -> Result<Outcome, String> {
    let path = writable_path(args)?;
    let old_text = text_arg(args, "old_text")?;
    let new_text = text_arg(args, "new_text")?;
    if old_text.is_empty() {
        return Err("old_text is empty.".into());
    }
    let old = read_text(&path)?.ok_or_else(|| no_such_file(&path))?;
    match old.matches(old_text).count() {
        0 => {
            return Err(format!(
                "old_text isn't in {}. Read the file again and copy the text exactly.",
                path.display()
            ));
        }
        1 => {}
        count => {
            return Err(format!(
                "old_text appears {count} times in {}. Include more surrounding lines so it appears once.",
                path.display()
            ));
        }
    }
    let new = old.replacen(old_text, new_text, 1);
    if new == old {
        return Ok(Outcome::Done(UNCHANGED.into()));
    }
    let asking = asking(&path, scratch, mode, judge, Change::Edit);
    propose(&path, text::STEP_EDIT, &old, &new, input, asking, peers)
}

/// How a change to `path` is put to the person in `mode`, by Lokal's own
/// rule first. A protected place asks on No in every mode, and auto's judge
/// never decides it. In accept edits and auto, the working and scratch
/// folders take a change unasked. Elsewhere, auto's judge decides, and
/// manual and accept edits ask on Yes.
fn asking<'a>(
    path: &Path,
    scratch: Option<&Path>,
    mode: Mode,
    judge: Option<&'a Case<'a>>,
    change: Change,
) -> Asking<'a> {
    if is_protected(path) {
        Asking::OnNo
    } else if mode != Mode::Manual
        && is_everyday_place(&config::resolved(path), &everyday_folders(scratch))
    {
        Asking::Not
    } else {
        match judge {
            Some(case) => Asking::Judge(case, change),
            None => Asking::OnYes,
        }
    }
}

/// Where a change opens on No in every mode, and auto's judge never
/// decides: what programs run or read as they start, the box's own list
/// (`sandbox::PROTECTED`: git's hooks and settings, shell start-up, editors'
/// and package managers' settings) with the project's notes and CI's
/// workflows; keys; the person's own settings, hidden right in the home
/// folder; and a folder on the PATH, where a file could pass for a program.
fn is_protected(path: &Path) -> bool {
    let path = config::resolved(path);
    let parts: Vec<&OsStr> = path
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    let listed = PROTECTED
        .iter()
        .chain(NOTES)
        .chain(&[".ssh", ".gnupg", ".github/workflows"])
        .any(|entry| {
            let entry: Vec<&OsStr> = Path::new(entry).iter().collect();
            parts.windows(entry.len()).any(|window| window == entry)
        });
    let home = config::resolved(&config::home());
    let home_settings = path
        .strip_prefix(&home)
        .ok()
        .and_then(|inside| inside.iter().next())
        .is_some_and(|first| first.as_encoded_bytes().starts_with(b"."));
    let on_path = path.parent().is_some_and(|folder| {
        env::var_os("PATH").is_some_and(|value| {
            env::split_paths(&value)
                .filter(|entry| entry.is_absolute())
                .any(|entry| config::resolved(&entry) == folder)
        })
    });
    listed || home_settings || on_path
}

/// Inside one of `folders`, with no hidden file or folder on the way there.
/// Hidden ones are mostly settings that change what other programs run
/// (git's config and hooks, CI, a shell's start-up, an editor's tasks), or
/// they hold secrets, like .env.
fn is_everyday_place(path: &Path, folders: &[PathBuf]) -> bool {
    folders.iter().any(|folder| {
        path.strip_prefix(folder).is_ok_and(|inside| {
            !inside
                .components()
                .any(|part| part.as_os_str().as_encoded_bytes().starts_with(b"."))
        })
    })
}

/// The path argument, when it's somewhere the model may write.
fn writable_path(args: &Value) -> Result<PathBuf, String> {
    let path = path_arg(args)?;
    if config::is_lokal_own(&path) {
        return Err(format!(
            "{} is one of Lokal's own files (its settings, user.md, its saved state), \
             which you can't change. To keep something the user asked you to remember, use \
             remember; the user edits these files with /prefs and /config.",
            path.display()
        ));
    }
    Ok(path)
}

/// The path argument, with ~ expanded. Relative paths start at the working directory.
fn path_arg(args: &Value) -> Result<PathBuf, String> {
    let path = text_arg(args, "path")?;
    Ok(match path.strip_prefix("~/") {
        Some(rest) => config::home().join(rest),
        None if path == "~" => config::home(),
        None => PathBuf::from(path),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(count: usize) -> String {
        (1..=count).map(|n| format!("line {n}\n")).collect()
    }

    #[test]
    fn read_part_gives_a_file_that_fits_as_it_is() {
        let file = numbered(3);
        assert_eq!(read_part(&file, None, None, 1000).unwrap(), file);
        assert_eq!(
            read_part(&file, Some(2), None, 1000).unwrap(),
            "Lines 2-3 of 3:\nline 2\nline 3"
        );
    }

    #[test]
    fn read_part_cuts_at_a_line_and_says_where_to_read_on() {
        // Each line is 7 or 8 characters with its newline: 25 fit in 200.
        let file = numbered(100);
        let part = read_part(&file, None, None, 200).unwrap();
        let (header, body) = part.split_once('\n').unwrap();
        let last = body.lines().count();
        assert!(header.starts_with(&format!("Lines 1-{last} of 100; the rest doesn't fit")));
        assert!(header.ends_with(&format!("read on from line {}:", last + 1)));
        assert_eq!(body.lines().last().unwrap(), format!("line {last}"));
        assert!(body.len() <= 200);
        let next = read_part(&file, Some(last + 1), None, 200).unwrap();
        assert!(next.starts_with(&format!("Lines {}-", last + 1)));
    }

    #[test]
    fn read_part_shows_the_start_of_a_line_too_long_for_the_room() {
        let file = format!("{}\nshort\n", "x".repeat(500));
        let part = read_part(&file, None, None, 100).unwrap();
        assert_eq!(
            part,
            format!(
                "Line 1 of 2 is too long to show whole; here's its start:\n{}",
                "x".repeat(100)
            )
        );
    }

    fn folder(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("lokal-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(path: &Path, room: usize, sees: Option<bool>) -> Result<Outcome, String> {
        read_file(&serde_json::json!({"path": path}), Some(room), sees)
    }

    #[test]
    fn a_model_that_sees_gets_an_image() {
        let dir = folder("image");
        let path = dir.join("shot.png");
        fs::write(&path, image::png_start(640, 480)).unwrap();
        let Ok(Outcome::Images { text, urls }) = read(&path, 100_000, Some(true)) else {
            panic!("no image");
        };
        assert_eq!(text, format!("{}: a 640×480 PNG image.", path.display()));
        assert!(urls[0].starts_with("data:image/png;base64,iVBORw0KGgo"));
        let no = read(&path, 100_000, Some(false)).err().unwrap();
        assert!(no.ends_with("is an image, and the model loaded doesn't take images."));
        let unknown = read(&path, 100_000, None).err().unwrap();
        assert!(unknown.contains("this server doesn't say whether its model takes images"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_image_too_big_for_the_room_is_turned_down_with_its_estimate() {
        let dir = folder("big-image");
        let path = dir.join("photo.png");
        fs::write(&path, image::png_start(4000, 3000)).unwrap();
        let Ok(Outcome::Done(result)) = read(&path, 30_000, Some(true)) else {
            panic!("not turned down");
        };
        assert!(result.starts_with("Not read: the image would take about 15370 tokens"));
        assert!(result.contains("room for about 10000"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn text_stays_text_whatever_it_starts_with() {
        let dir = folder("not-image");
        let text = dir.join("notes.txt");
        fs::write(&text, "GIF89a is how a GIF starts.").unwrap();
        let Ok(Outcome::Done(result)) = read(&text, 100_000, Some(true)) else {
            panic!("not read as text");
        };
        assert_eq!(result, "GIF89a is how a GIF starts.");
        let binary = dir.join("data.bin");
        fs::write(&binary, [0x00, 0xFF, 0xFE, 0x80]).unwrap();
        let seeing = read(&binary, 100_000, Some(true)).err().unwrap();
        assert!(seeing.ends_with("isn't a text file, nor a PNG, JPEG or WebP image."));
        let blind = read(&binary, 100_000, Some(false)).err().unwrap();
        assert!(blind.ends_with("isn't a text file."));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_a_regular_file_is_read() {
        assert!(read_text(Path::new("/dev/zero")).is_err());
        let dir = env::temp_dir().join(format!("lokal-read-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let pipe = dir.join("pipe");
        let name = std::ffi::CString::new(pipe.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: a valid C string; mkfifo only creates the pipe.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        // Opened, a pipe with no writer would block for good.
        assert!(
            read_text(&pipe)
                .unwrap_err()
                .contains("isn't a regular file")
        );
        assert_eq!(read_text(&dir.join("missing")).unwrap(), None);
        fs::write(dir.join("a.txt"), "hi\n").unwrap();
        assert_eq!(
            read_text(&dir.join("a.txt")).unwrap().as_deref(),
            Some("hi\n")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn protected_places_are_where_programs_start_keys_and_settings() {
        let home = config::home();
        for path in [
            "/work/p/.git/hooks/pre-commit",
            "/work/p/.git/config",
            "/work/p/.github/workflows/ci.yml",
            "/work/p/AGENTS.md",
            "/work/p/sub/LOKAL.md",
            "/work/p/.vscode/tasks.json",
            "/work/p/.envrc",
            "/work/p/gradle/wrapper/gradle-wrapper.properties",
            "/work/p/keys/.ssh/id_ed25519",
            "/usr/bin/ls",
        ] {
            assert!(is_protected(Path::new(path)), "{path}");
        }
        for inside in [".bashrc", ".config/autostart/x.desktop", ".local/bin/ls"] {
            assert!(is_protected(&home.join(inside)), "~/{inside}");
        }
        for path in [
            "/work/p/index.html",
            "/work/p/.gitignore",
            "/work/p/.env",
            "/work/p/.github/dependabot.yml",
            "/work/p/docs/agents.html",
            "/mnt/c/Users/ana/Desktop/site/index.html",
        ] {
            assert!(!is_protected(Path::new(path)), "{path}");
        }
        for inside in ["notes.txt", "projects/site/.prettierrc"] {
            assert!(!is_protected(&home.join(inside)), "~/{inside}");
        }
    }

    #[test]
    fn a_change_asks_by_the_mode_and_the_place() {
        let server = crate::server::Server {
            address: String::new(),
            model: String::new(),
            context: None,
            max_context: None,
            vision: None,
            kind: crate::server::Kind::Other,
            context_settled: true,
            pinned: false,
        };
        let case = Case {
            server: &server,
            asked: &[],
            folder: "/work/p",
            scratch: None,
            written: &[],
        };
        let scratch = Path::new("/tmp/lokal-1000/42");
        let in_scratch = Path::new("/tmp/lokal-1000/42/page.html");
        let elsewhere = Path::new("/mnt/c/Users/ana/Desktop/site/index.html");
        let protected = Path::new("/work/p/.git/hooks/pre-commit");
        let how = |path, mode, judge| match asking(path, Some(scratch), mode, judge, Change::Create)
        {
            Asking::OnNo => "no",
            Asking::OnYes => "yes",
            Asking::Not => "not",
            Asking::Judge(..) => "judge",
        };
        assert_eq!(how(elsewhere, Mode::Manual, None), "yes");
        assert_eq!(how(in_scratch, Mode::Manual, None), "yes");
        assert_eq!(how(protected, Mode::Manual, None), "no");
        assert_eq!(how(in_scratch, Mode::AcceptEdits, None), "not");
        assert_eq!(how(elsewhere, Mode::AcceptEdits, None), "yes");
        assert_eq!(how(protected, Mode::AcceptEdits, None), "no");
        assert_eq!(how(in_scratch, Mode::Auto, Some(&case)), "not");
        assert_eq!(how(elsewhere, Mode::Auto, Some(&case)), "judge");
        assert_eq!(how(protected, Mode::Auto, Some(&case)), "no");
    }

    #[test]
    fn edits_are_accepted_only_in_plain_files_of_the_folders() {
        let folders = [
            PathBuf::from("/home/ana/project"),
            PathBuf::from("/tmp/lokal-1000/42"),
        ];
        for path in [
            "/home/ana/project/src/main.rs",
            "/home/ana/project/README.md",
            "/tmp/lokal-1000/42/download.html",
        ] {
            assert!(is_everyday_place(Path::new(path), &folders), "{path}");
        }
        for path in [
            "/home/ana/.bashrc",
            "/home/ana/project-other/x",
            "/home/ana/project/.env",
            "/home/ana/project/.git/config",
            "/home/ana/project/.github/workflows/ci.yml",
            "/etc/hosts",
        ] {
            assert!(!is_everyday_place(Path::new(path), &folders), "{path}");
        }
    }

    #[test]
    fn read_part_refuses_lines_past_the_end() {
        let file = numbered(125);
        assert_eq!(
            read_part(&file, Some(400), Some(750), 1000).unwrap_err(),
            "the file has 125 lines, so line 400 is past its end."
        );
        assert!(read_part(&file, Some(10), Some(5), 1000).is_err());
        assert_eq!(
            read_part(&file, Some(124), Some(750), 1000).unwrap(),
            "Lines 124-125 of 125:\nline 124\nline 125"
        );
    }
}
