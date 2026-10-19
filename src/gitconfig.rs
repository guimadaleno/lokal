//! What a repository's own git settings would have git run. Some run even
//! while git only reads: core.fsmonitor on git status, a filter or a diff
//! driver on git diff, log -p and blame, gpg.program on git log. Those
//! settings are files in .git that anything could have written, so a git
//! command runs unasked only where the repository names none of them, with
//! fsmonitor off and kept out of submodules, whose settings are their own.

use std::io::{ErrorKind, Read};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Set for a git command that runs unasked. A submodule's settings are out
/// of `program_setting`'s reach, and they apply inside it: git stays out,
/// and fsmonitor is off in case it goes in anyway.
pub const UNASKED: [(&str, &str); 2] = [
    ("core.fsmonitor", "false"),
    ("diff.ignoreSubmodules", "all"),
];

/// Settings that name a program git runs while reading.
const PROGRAMS: &str = r"^(core\.fsmonitor|diff\.external|diff\..+\.(command|textconv)|filter\..+\.(clean|smudge|process)|gpg\.(.+\.)?program)$";
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
    let mut command = Command::new("git");
    command
        .args(["config", "--show-scope", "--get-regexp", PROGRAMS])
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
        // Without git, a git command runs nothing at all.
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
    match status.code() {
        // Nothing matched.
        Some(1) => Ok(None),
        Some(0) => {
            let mut listing = String::new();
            child
                .stdout
                .take()
                .ok_or(())?
                .read_to_string(&mut listing)
                .map_err(|_| ())?;
            Ok(first_of_the_repository(&listing))
        }
        _ => Err(()),
    }
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
}
