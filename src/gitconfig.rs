//! What a repository's own git settings would have git run. Some run even
//! while git only reads: core.fsmonitor on git status, a filter or a diff
//! driver on git diff, log -p and blame, gpg.program on git log. Those
//! settings are files in .git that anything could have written, so a git
//! command runs unasked only where the repository names none of them, with
//! fsmonitor off and kept out of submodules, whose settings are their own.
//! As git writes, it also runs the repository's hooks, and the programs
//! settings like an alias or an editor name, which auto's judge can't see:
//! there, a git command asks when the repository has any.

use std::fs;
use std::io::{ErrorKind, Read};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::system;

/// Set for a git command that runs unasked. A submodule's settings are out
/// of `program_setting`'s reach, and they apply inside it: git stays out,
/// and fsmonitor is off in case it goes in anyway.
pub const UNASKED: [(&str, &str); 2] = [
    ("core.fsmonitor", "false"),
    ("diff.ignoreSubmodules", "all"),
];

/// Settings that name a program git runs while reading.
const PROGRAMS: &str = r"^(core\.fsmonitor|diff\.external|diff\..+\.(command|textconv)|filter\..+\.(clean|smudge|process)|gpg\.(.+\.)?program)$";
/// Settings that have git run a program as it writes or reaches a remote:
/// where its hooks are, what it runs to connect or to ask for a password,
/// an editor, a merge driver, the `ext::` transport, and an alias, which can
/// be a shell command or bring options of its own.
const WRITE_PROGRAMS: &str = r"^(core\.(hookspath|sshcommand|askpass|editor)|sequence\.editor|credential\.(.+\.)?helper|merge\..+\.driver|protocol\.(.+\.)?allow|alias\..+|remote\..+\.(uploadpack|receivepack))$";
/// core.fsmonitor as a yes or a no: git's own daemon, or none.
const SWITCHES: &[&str] = &["", "true", "false", "yes", "no", "on", "off", "1", "0"];
/// Reading settings is quick; a file they include could be one that never ends.
const READ_TIME: Duration = Duration::from_secs(2);

/// The first setting of the working folder's repository, its own or its
/// worktree's, that names a program git runs while reading. The person's
/// global settings are their own, so they don't count. Err when git
/// couldn't say, in time or at all: a git before 2.26 can't tell where a
/// setting comes from.
pub fn program_setting() -> Result<Option<String>, ()> {
    repository_setting(PROGRAMS)
}

/// What of the repository's own git would run as it does more than read:
/// a hook in `.git/hooks`, or a setting that names a program. Auto's judge
/// sees only the command, so a git command it would let through is asked
/// about instead. Err when git couldn't say.
pub fn write_program() -> Result<Option<WriteProgram>, ()> {
    if let Some(setting) = repository_setting(WRITE_PROGRAMS)? {
        return Ok(Some(WriteProgram::Setting(setting)));
    }
    let paths = match git(&["rev-parse", "--git-common-dir", "--git-path", "hooks"])? {
        Some((0, paths)) => paths,
        // Without git or outside a repository, there are no hooks.
        _ => return Ok(None),
    };
    let mut lines = paths.lines();
    let (Some(common), Some(hooks)) = (lines.next(), lines.next()) else {
        return Err(());
    };
    // Elsewhere, they come from the person's own core.hooksPath.
    if Path::new(common).join("hooks") != Path::new(hooks) {
        return Ok(None);
    }
    Ok(active_hook(Path::new(hooks)).map(WriteProgram::Hook))
}

pub enum WriteProgram {
    Setting(String),
    Hook(String),
}

/// The first hook in `folder` that git would run: executable, and not one of
/// the `.sample` files git puts there.
fn active_hook(folder: &Path) -> Option<String> {
    let mut hooks: Vec<String> = fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| {
            // fs::metadata follows a link, as git does.
            fs::metadata(entry.path())
                .is_ok_and(|file| file.is_file() && file.permissions().mode() & 0o111 != 0)
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.ends_with(".sample"))
        .collect();
    hooks.sort();
    hooks.into_iter().next()
}

/// The first setting matching `pattern` that the repository sets itself.
fn repository_setting(pattern: &str) -> Result<Option<String>, ()> {
    match git(&["config", "--show-scope", "--get-regexp", pattern])? {
        // Without git, a git command runs nothing at all.
        None => Ok(None),
        // Nothing matched.
        Some((1, _)) => Ok(None),
        Some((0, listing)) => Ok(first_of_the_repository(&listing)),
        Some(_) => Err(()),
    }
}

/// What git printed with `args` and the code it ended with; None without
/// git. Err when it couldn't run or didn't end in time.
fn git(args: &[&str]) -> Result<Option<(i32, String)>, ()> {
    // Without git, a git command runs nothing at all.
    let Some(git) = system::program("git") else {
        return Ok(None);
    };
    let mut command = Command::new(git);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: setsid is safe to call between fork and exec.
    // A session of its own, like the model's commands: no terminal to wait on.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|_| ())? {
            break status;
        }
        if started.elapsed() > READ_TIME {
            let _ = child.kill();
            let _ = child.wait();
            return Err(());
        }
        thread::sleep(Duration::from_millis(5));
    };
    let mut printed = String::new();
    child
        .stdout
        .take()
        .ok_or(())?
        .read_to_string(&mut printed)
        .map_err(|_| ())?;
    Ok(Some((status.code().ok_or(())?, printed)))
}

/// From `git config --show-scope --get-regexp` ("local<tab>name value"),
/// the first name set by the repository itself to a program.
fn first_of_the_repository(listing: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let (scope, setting) = line.split_once('\t')?;
        let (name, value) = setting.split_once(' ').unwrap_or((setting, ""));
        let switch = name == "core.fsmonitor"
            && SWITCHES.contains(&value.trim().to_ascii_lowercase().as_str());
        (matches!(scope, "local" | "worktree") && !switch).then(|| name.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_repository_settings_count() {
        let listing = "global\tfilter.lfs.clean git-lfs clean -- %f\n\
                       local\tdiff.Evil.textconv /tmp/x\n\
                       worktree\tgpg.program /tmp/y\n";
        assert_eq!(
            first_of_the_repository(listing).as_deref(),
            Some("diff.Evil.textconv")
        );
        assert_eq!(first_of_the_repository("system\tgpg.program gpg2\n"), None);
        assert_eq!(first_of_the_repository(""), None);
    }

    #[test]
    fn fsmonitor_counts_only_as_a_program() {
        assert_eq!(
            first_of_the_repository("local\tcore.fsmonitor False\nlocal\tcore.fsmonitor\n"),
            None
        );
        assert_eq!(
            first_of_the_repository("local\tcore.fsmonitor .git/hooks/query-watchman\n").as_deref(),
            Some("core.fsmonitor")
        );
    }

    #[test]
    fn settings_that_run_a_program_as_git_writes_are_found() {
        let dir = std::env::temp_dir().join(format!("lokal-write-programs-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config");
        fs::write(
            &file,
            "[core]\n\thooksPath = h\n\taskPass = a\n\teditor = e\n\tbare = false\n\
             [alias]\n\tst = !sh x\n[credential \"https://example.com\"]\n\thelper = c\n\
             [merge \"ours\"]\n\tdriver = d\n[protocol \"ext\"]\n\tallow = always\n\
             [remote \"origin\"]\n\turl = u\n\tuploadpack = p\n[user]\n\tname = Ana\n",
        )
        .unwrap();
        let output = Command::new("git")
            .args(["config", "--file"])
            .arg(&file)
            .args(["--get-regexp", WRITE_PROGRAMS])
            .output()
            .unwrap();
        let listing = String::from_utf8(output.stdout).unwrap();
        let keys: Vec<&str> = listing
            .lines()
            .filter_map(|line| line.split(' ').next())
            .collect();
        assert_eq!(
            keys,
            [
                "core.hookspath",
                "core.askpass",
                "core.editor",
                "alias.st",
                "credential.https://example.com.helper",
                "merge.ours.driver",
                "protocol.ext.allow",
                "remote.origin.uploadpack",
            ]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_hook_is_an_executable_file_but_a_sample() {
        let folder = std::env::temp_dir().join(format!("lokal-hooks-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let file = |name: &str, mode: u32| {
            let path = folder.join(name);
            fs::write(&path, "#!/bin/sh\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        };
        file("pre-commit.sample", 0o755);
        file("pre-push", 0o644);
        assert_eq!(active_hook(&folder), None);
        file("post-merge", 0o755);
        assert_eq!(active_hook(&folder).as_deref(), Some("post-merge"));
        fs::remove_dir_all(&folder).unwrap();
    }
}
