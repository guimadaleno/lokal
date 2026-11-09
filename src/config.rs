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
    /// Mail between the person's sessions on this machine (`peers.rs`).
    pub mail: Mail,
    /// How many turns in a row mail may start on its own.
    pub mail_turns: Option<usize>,
    /// The MCP servers the person set up, in the order they come.
    pub mcp: Vec<McpServer>,
}

/// An MCP server: `mcp.<name> = <command line>`, and optionally
/// `mcp.<name>.tools = a, b` for the only tools of its the model gets.
#[derive(Clone, Debug, PartialEq)]
pub struct McpServer {
    pub name: String,
    /// The program and its arguments, split as a shell would.
    pub command: Vec<String>,
    pub tools: Option<Vec<String>>,
}

/// The longest name a server may have: it starts every tool's name, which
/// the chat API keeps under 64 characters.
const MCP_NAME_MOST: usize = 20;

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub enum Mail {
    /// A mail starts a turn on its own.
    #[default]
    Auto,
    /// A mail waits for Enter.
    Ask,
    /// No mail: no socket, no tools.
    Off,
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
            "mail" => {
                config.mail = match value.as_deref() {
                    Some("auto") => Mail::Auto,
                    Some("ask") => Mail::Ask,
                    Some("off") => Mail::Off,
                    _ => return Err(text::bad_mail_setting(path, index + 1)),
                }
            }
            "mail_turns" => match value.as_deref().map(str::parse) {
                Some(Ok(turns)) => config.mail_turns = Some(turns),
                _ => return Err(text::bad_mail_turns_setting(path, index + 1)),
            },
            other if other.starts_with("mcp.") => {
                set_mcp(&mut config.mcp, other, value.as_deref().unwrap_or(""))
                    .map_err(|error| text::bad_mcp_setting(path, index + 1, &error))?;
            }
            other => return Err(text::unknown_setting(path, index + 1, other)),
        }
    }
    if let Some(server) = config.mcp.iter().find(|server| server.command.is_empty()) {
        return Err(text::mcp_tools_without_server(path, &server.name));
    }
    Ok(config)
}

/// One `mcp.<name>` or `mcp.<name>.tools` line. Err says what's wrong.
fn set_mcp(servers: &mut Vec<McpServer>, setting: &str, value: &str) -> Result<(), String> {
    let rest = &setting["mcp.".len()..];
    let (name, field) = match rest.split_once('.') {
        Some((name, field)) => (name, Some(field)),
        None => (rest, None),
    };
    let valid = !name.is_empty()
        && name.len() <= MCP_NAME_MOST
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Err(text::bad_mcp_name(name, MCP_NAME_MOST));
    }
    let index = match servers.iter().position(|server| server.name == name) {
        Some(index) => index,
        None => {
            servers.push(McpServer {
                name: name.to_string(),
                command: Vec::new(),
                tools: None,
            });
            servers.len() - 1
        }
    };
    let server = &mut servers[index];
    match field {
        None if !server.command.is_empty() => Err(text::mcp_set_twice(name)),
        None => {
            server.command = words(value);
            if server.command.is_empty() {
                return Err(text::mcp_no_command(name));
            }
            Ok(())
        }
        Some("tools") => {
            let tools = value
                .split(',')
                .map(str::trim)
                .filter(|tool| !tool.is_empty())
                .map(String::from)
                .collect();
            server.tools = Some(tools);
            Ok(())
        }
        Some(other) => Err(text::unknown_mcp_field(name, other)),
    }
}

/// A value split as a shell would: quotes and a backslash keep a space in
/// a word ("/mnt/c/Program Files/…").
pub fn words(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\\', _) => word.extend(chars.next()),
            ('"' | '\'', None) => quote = Some(c),
            (c, Some(open)) if c == open => quote = None,
            (c, None) if c.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            (c, _) => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
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

/// The MCP tool lists the person has seen, a hash a server.
pub fn mcp_seen_path() -> PathBuf {
    state_dir().join("mcp-seen")
}

pub fn last_update_check_path() -> PathBuf {
    state_dir().join("last-update-check")
}

/// Every file Lokal keeps for the person, user.md under both its names:
/// what `lokal --uninstall` offers to remove, and nothing else.
pub fn own_files() -> Vec<PathBuf> {
    vec![
        settings_path(),
        config_dir().join("user.md"),
        config_dir().join("about-me.md"),
        approvals_path(),
        last_conversation_path(),
        last_update_check_path(),
        mcp_seen_path(),
    ]
}

/// The folders those files live in.
pub fn own_folders() -> [PathBuf; 2] {
    [config_dir(), state_dir()]
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
pub fn resolved(path: &Path) -> PathBuf {
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
    fn reads_mcp_servers_and_their_tools() {
        let config = parse(
            "mcp.github = npx -y \"@scope/server github\"\n\
             mcp.github.tools = create_issue, search_issues\n\
             mcp.fs-2 = /opt/fs --root /srv\n",
            "config",
        )
        .unwrap();
        assert_eq!(
            config.mcp,
            [
                McpServer {
                    name: "github".into(),
                    command: vec!["npx".into(), "-y".into(), "@scope/server github".into()],
                    tools: Some(vec!["create_issue".into(), "search_issues".into()]),
                },
                McpServer {
                    name: "fs-2".into(),
                    command: vec!["/opt/fs".into(), "--root".into(), "/srv".into()],
                    tools: None,
                },
            ]
        );
    }

    #[test]
    fn a_wrong_mcp_setting_says_what_and_where() {
        let error = |contents: &str| parse(contents, "config").unwrap_err();
        assert!(error("mcp.GitHub = x").contains("line 1: `GitHub` can't name an MCP server"));
        assert!(error("mcp. = x").contains("can't name an MCP server"));
        assert!(error("mcp.a = x\nmcp.a = y").contains("line 2: `mcp.a` is set twice."));
        assert!(error("mcp.a =").contains("needs the command"));
        assert!(error("mcp.a.env = X=1").contains("`mcp.a.env` isn't a setting"));
        assert!(error("mcp.a.tools = x").contains("`mcp.a.tools` is set, but not `mcp.a`"));
    }

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
    fn mail_is_on_its_own_unless_the_settings_say() {
        let config = parse("", "config").unwrap();
        assert_eq!((config.mail, config.mail_turns), (Mail::Auto, None));
        let config = parse("mail = ask\nmail_turns = 8", "config").unwrap();
        assert_eq!((config.mail, config.mail_turns), (Mail::Ask, Some(8)));
        assert_eq!(parse("mail = off", "config").unwrap().mail, Mail::Off);
        assert!(parse("mail = sometimes", "config").is_err());
        assert!(parse("mail_turns = lots", "config").is_err());
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
