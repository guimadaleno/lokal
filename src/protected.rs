//! Places where a change opens on No in every mode, and auto's judge never
//! decides: what programs run or read as they start, keys, and the
//! person's own settings. A file the model writes there (`tools/files.rs`),
//! and a command that writes, moves into or `cd`s there (`allowlist/hard.rs`),
//! asks first.

use std::env;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use crate::config;
use crate::sandbox::{NOTES, PROTECTED};

/// Beyond the box's own list (`sandbox::PROTECTED`: git's hooks and
/// settings, shell start-up, editors' and package managers' settings) and
/// the project's notes: keys, CI, tools that run what their settings name,
/// and Windows' own places where programs start, written in lowercase.
const MORE: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".github/workflows",
    ".github/actions",
    ".gitlab-ci.yml",
    ".cargo/config",
    ".cargo/config.toml",
    "mise.toml",
    ".mise.toml",
    "appdata",
    "windowspowershell",
    "documents/powershell",
];

/// Whether `path` is protected: on the lists above, in any case (a
/// Windows drive under WSL ignores it), the person's settings hidden right
/// in the home folder, `~/bin`, a bare repository's hooks, or a folder on
/// the PATH, where a file could pass for a program.
pub fn is_protected(path: &Path) -> bool {
    let path = config::resolved(path);
    let parts: Vec<String> = path
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    let listed = PROTECTED.iter().chain(NOTES).chain(MORE).any(|entry| {
        let entry: Vec<String> = Path::new(entry)
            .iter()
            .map(|part| part.to_string_lossy().to_lowercase())
            .collect();
        parts.windows(entry.len()).any(|window| window == entry)
    });
    let bare_hooks = parts
        .windows(2)
        .any(|pair| pair[0].ends_with(".git") && pair[1] == "hooks");
    let home = config::resolved(&config::home());
    let in_home = path.strip_prefix(&home).ok();
    let home_settings = in_home
        .and_then(|inside| inside.iter().next())
        .is_some_and(|first| {
            first.as_encoded_bytes().starts_with(b".") || first == OsStr::new("bin")
        });
    let on_path = path
        .parent()
        .is_some_and(|folder| path_folders().iter().any(|entry| entry == folder));
    listed || bare_hooks || home_settings || on_path
}

/// The PATH's absolute folders, resolved once a session: under WSL it
/// holds Windows' own, and each one read on `/mnt/c` is slow.
fn path_folders() -> &'static [PathBuf] {
    static FOLDERS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    FOLDERS.get_or_init(|| {
        let Some(value) = env::var_os("PATH") else {
            return Vec::new();
        };
        env::split_paths(&value)
            .filter(|entry| entry.is_absolute())
            .map(|entry| config::resolved(&entry))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_programs_start_keys_and_settings_are_protected() {
        let home = config::home();
        for path in [
            "/work/p/.git/hooks/pre-commit",
            "/work/p/.git/config",
            "/work/p/.GIT/hooks/pre-commit",
            "/work/p/.github/workflows/ci.yml",
            "/work/p/.github/actions/build/action.yml",
            "/work/p/.gitlab-ci.yml",
            "/work/p/AGENTS.md",
            "/work/p/Agents.md",
            "/work/p/sub/LOKAL.md",
            "/work/p/.vscode/tasks.json",
            "/work/p/.envrc",
            "/work/p/.cargo/config.toml",
            "/work/p/mise.toml",
            "/work/p/gradle/wrapper/gradle-wrapper.properties",
            "/work/p/keys/.ssh/id_ed25519",
            "/srv/repos/x.git/hooks/post-receive",
            "/mnt/c/Users/ana/AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup/x.bat",
            "/mnt/c/Users/ana/Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1",
            "/mnt/c/Users/ana/Documents/PowerShell/Microsoft.PowerShell_profile.ps1",
            "/mnt/c/Users/ana/.SSH/authorized_keys",
            "/usr/bin/ls",
        ] {
            assert!(is_protected(Path::new(path)), "{path}");
        }
        for inside in [
            ".bashrc",
            ".config/autostart/x.desktop",
            ".local/bin/ls",
            "bin/ls",
        ] {
            assert!(is_protected(&home.join(inside)), "~/{inside}");
        }
        for path in [
            "/work/p/index.html",
            "/work/p/.gitignore",
            "/work/p/.env",
            "/work/p/.github/dependabot.yml",
            "/work/p/docs/agents.html",
            "/work/p/src/bin/main.rs",
            "/mnt/c/Users/ana/Desktop/site/index.html",
            "/mnt/c/Users/ana/Documents/notes.txt",
        ] {
            assert!(!is_protected(Path::new(path)), "{path}");
        }
        for inside in ["notes.txt", "projects/site/.prettierrc", "projects/bin/x"] {
            assert!(!is_protected(&home.join(inside)), "~/{inside}");
        }
    }
}
