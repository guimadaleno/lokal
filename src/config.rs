//! Lokal's files: the optional settings, user.md, and the last conversation.

use std::env;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::text;

/// Settings that pin what Lokal would otherwise find on its own.
#[derive(Default, Debug)]
pub struct Config {
    pub server: Option<String>,
    pub model: Option<String>,
    /// `updates = off`: the daily update check is the one thing Lokal sends
    /// on its own, so it can be switched off.
    pub updates_off: bool,
}

pub fn load() -> Result<Config, String> {
    let path = settings_path();
    match fs::read_to_string(&path) {
        Ok(contents) => parse(&contents, &shown(&path)),
        Err(_) => Ok(Config::default()),
    }
}

fn parse(contents: &str, path: &str) -> Result<Config, String> {
    let mut config = Config::default();
    for (index, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            return Err(text::bad_config_line(path, index + 1));
        };
        let value = Some(value.trim().to_string());
        match name.trim() {
            "server" => config.server = value,
            "model" => config.model = value,
            "updates" => match value.as_deref() {
                Some("on") => config.updates_off = false,
                Some("off") => config.updates_off = true,
                _ => return Err(text::bad_updates_setting(path, index + 1)),
            },
            other => return Err(text::unknown_setting(path, index + 1, other)),
        }
    }
    Ok(config)
}

pub fn approvals_path() -> PathBuf {
    config_dir().join("approved")
}

pub fn settings_path() -> PathBuf {
    config_dir().join("config")
}

pub fn user_md_path() -> PathBuf {
    user_md_in(&config_dir())
}

/// user.md in `dir`. The file under its old name, about-me.md, moves there
/// the first time it's needed; when it can't, the old one stays in use, so
/// the person's notes are never lost.
fn user_md_in(dir: &Path) -> PathBuf {
    let path = dir.join("user.md");
    let old = dir.join("about-me.md");
    if !path.exists() && old.exists() && fs::rename(&old, &path).is_err() {
        return old;
    }
    path
}

/// The person's preferences and facts, or None before the first launch has asked.
pub fn read_user_md() -> Option<String> {
    fs::read_to_string(user_md_path()).ok()
}

pub fn write_user_md(contents: &str) -> io::Result<()> {
    write(&user_md_path(), contents)
}

pub fn last_conversation_path() -> PathBuf {
    state_dir().join("last-conversation.json")
}

pub fn last_update_check_path() -> PathBuf {
    state_dir().join("last-update-check")
}

pub fn write(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)
}

pub fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// A path as the person would write it: ~/.config/lokal rather than /home/ana/.config/lokal.
pub fn shown(path: &Path) -> String {
    match path.strip_prefix(home()) {
        Ok(rest) if !home().as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

/// One of Lokal's own files: its settings, user.md, its approvals, its
/// saved conversation. The model never writes to them, even with a yes: it
/// must not change what it may do, where its words are sent, or what every
/// conversation starts with.
pub fn is_lokal_own(path: &Path) -> bool {
    let path = resolved(path);
    [config_dir(), state_dir()]
        .iter()
        .any(|dir| path.starts_with(resolved(dir)))
}

/// The path the kernel would reach, so no spelling of it (`..`, a link, a
/// relative path) slips past. Each part that exists is resolved before the
/// next is applied; parts that don't exist yet can't be links.
fn resolved(path: &Path) -> PathBuf {
    let mut out = if path.is_absolute() {
        PathBuf::from("/")
    } else {
        env::current_dir().unwrap_or_default()
    };
    for part in path.components() {
        match part {
            Component::RootDir | Component::Prefix(_) => out = PathBuf::from("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => {
                out.push(name);
                if let Ok(real) = fs::canonicalize(&out) {
                    out = real;
                }
            }
        }
    }
    out
}

fn config_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

fn state_dir() -> PathBuf {
    xdg_dir("XDG_STATE_HOME", ".local/state")
}

fn xdg_dir(variable: &str, fallback: &str) -> PathBuf {
    match env::var_os(variable) {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("lokal"),
        _ => home().join(fallback).join("lokal"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn reads_settings_and_skips_comments() {
        let config = parse(
            "# pinned\nserver = 192.0.2.10:1234\n\nmodel=some-model\nupdates = off\n",
            "config",
        )
        .unwrap();
        assert_eq!(config.server.as_deref(), Some("192.0.2.10:1234"));
        assert_eq!(config.model.as_deref(), Some("some-model"));
        assert!(config.updates_off);
        assert!(!parse("", "config").unwrap().updates_off);
        assert!(parse("updates = maybe", "config").is_err());
    }

    #[test]
    fn rejects_unknown_settings_and_bad_lines() {
        assert!(parse("sever = x", "config").unwrap_err().contains("line 1"));
        assert!(
            parse("server\n", "config")
                .unwrap_err()
                .contains("name = value")
        );
    }

    #[test]
    fn lokal_own_files_are_found_however_they_are_written() {
        let home = std::env::temp_dir().join(format!("lokal-own-{}", std::process::id()));
        let config = home.join(".config/lokal");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(home.join("work")).unwrap();
        std::os::unix::fs::symlink(&config, home.join("work/link")).unwrap();
        // SAFETY of the test: the variables only point this process at a scratch folder.
        unsafe {
            env::set_var("XDG_CONFIG_HOME", home.join(".config"));
        }
        assert!(is_lokal_own(&config.join("config")));
        assert!(is_lokal_own(&config.join("user.md")));
        assert!(is_lokal_own(&home.join("work/../.config/lokal/config")));
        assert!(is_lokal_own(&home.join("work/link/config")));
        assert!(is_lokal_own(&home.join("work/link/../lokal/user.md")));
        assert!(is_lokal_own(
            &home.join("missing/../.config/lokal/new-file")
        ));
        assert!(!is_lokal_own(&home.join("work/notes.md")));
        assert!(!is_lokal_own(&home.join(".config/lokal-other/config")));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn about_me_md_moves_to_user_md_once() {
        let dir = std::env::temp_dir().join(format!("lokal-user-md-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(user_md_in(&dir), dir.join("user.md"));

        fs::write(dir.join("about-me.md"), "- short answers\n").unwrap();
        assert_eq!(user_md_in(&dir), dir.join("user.md"));
        assert_eq!(
            fs::read_to_string(dir.join("user.md")).unwrap(),
            "- short answers\n"
        );
        assert!(!dir.join("about-me.md").exists());

        // Both there: user.md wins, and the old one is left alone.
        fs::write(dir.join("about-me.md"), "- old\n").unwrap();
        assert_eq!(user_md_in(&dir), dir.join("user.md"));
        assert!(dir.join("about-me.md").exists());

        // A move that fails keeps the old one in use.
        fs::remove_file(dir.join("user.md")).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        assert_eq!(user_md_in(&dir), dir.join("about-me.md"));
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
}
