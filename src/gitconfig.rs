//! What a repository's own git settings would have git run. Some run even
//! while git only reads: core.fsmonitor on git status, a filter or a diff
//! driver on git diff, log -p and blame, gpg.program on git log. Those
//! settings are files in .git that anything could have written, so a git
//! command runs unasked only where the repository names none of them, with
//! fsmonitor off and kept out of submodules, whose settings are their own.
//! As git writes, it also runs the repository's hooks, and programs that a
//! long and growing list of settings can name (an alias, an editor, a merge
//! tool…), which auto's judge can't see. So in auto, a git command asks when
//! the repository has a hook, or sets anything beyond the plain settings
//! git writes itself.

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
/// The settings a repository holds that can't have git run anything: what
/// `git init`, `clone`, `remote`, `branch` and `submodule` write, and a few
/// choices among git's own behaviors. Anything else is asked about in auto,
/// since the settings that name a program have no fixed end.
const PLAIN_CORE: &[&str] = &[
    "repositoryformatversion",
    "filemode",
    "bare",
    "logallrefupdates",
    "ignorecase",
    "precomposeunicode",
    "symlinks",
    "autocrlf",
    "eol",
    "safecrlf",
    "sparsecheckout",
    "sparsecheckoutcone",
    "untrackedcache",
    "quotepath",
    "longpaths",
    // Where a submodule's work tree is, from inside it.
    "worktree",
];
const PLAIN_REMOTE: &[&str] = &[
    "url",
    "pushurl",
    "fetch",
    "push",
    "tagopt",
    "prune",
    "mirror",
    "promisor",
    "partialclonefilter",
    // What `gh repo clone` marks a fork's upstream with.
    "gh-resolved",
];
/// `vscode-merge-base` is what VS Code writes on a branch it makes: a ref.
const PLAIN_BRANCH: &[&str] = &[
    "remote",
    "merge",
    "rebase",
    "pushremote",
    "description",
    "vscode-merge-base",
];
/// What Git LFS writes itself; others in its section name programs it runs.
const PLAIN_LFS: &[&str] = &["repositoryformatversion", "url", "access", "locksverify"];
const PLAIN_SUBMODULE: &[&str] = &["url", "active", "branch", "shallow", "ignore"];
/// A submodule's `update` can also be `!command`.
const SUBMODULE_UPDATES: &[&str] = &["checkout", "rebase", "merge", "none"];
const PLAIN_OTHER: &[&str] = &[
    // `git clone --recurse-submodules` sets it.
    "submodule.active",
    "pull.rebase",
    "pull.ff",
    "push.default",
    "push.autosetupremote",
    "fetch.prune",
    "init.defaultbranch",
    "merge.ff",
    "rebase.autostash",
    "commit.gpgsign",
    "tag.gpgsign",
];
/// Sections whose every setting is plain: who commits, git's own format
/// extensions, colors.
const PLAIN_SECTIONS: &[&str] = &["user", "extensions", "color"];
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

/// What of the repository's own could have git run a program as it does
/// more than read: a hook in `.git/hooks`, or a setting beyond the plain
/// ones. Auto's judge sees only the command, so a git command it would let
/// through is asked about instead. Err when git couldn't say.
pub fn write_program() -> Result<Option<WriteProgram>, ()> {
    let listing = match git(&["config", "--show-scope", "--get-regexp", "."])? {
        None | Some((1, _)) => String::new(),
        Some((0, listing)) => listing,
        Some(_) => return Err(()),
    };
    if let Some(setting) = first_beyond_the_plain(&listing) {
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

/// From the same listing, the first setting of the repository's own that
/// isn't a plain one.
fn first_beyond_the_plain(listing: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let (scope, setting) = line.split_once('\t')?;
        let (name, value) = setting.split_once(' ').unwrap_or((setting, ""));
        (matches!(scope, "local" | "worktree") && !is_plain(name, value)).then(|| name.to_string())
    })
}

/// Whether a setting, as git lists it (section and key in lowercase, a
/// subsection between them as written), can't have git run anything.
fn is_plain(name: &str, value: &str) -> bool {
    let Some((section, rest)) = name.split_once('.') else {
        return false;
    };
    let (subsection, key) = match rest.rsplit_once('.') {
        Some((subsection, key)) => (Some(subsection), key),
        None => (None, rest),
    };
    match (section, subsection, key) {
        ("core", None, "fsmonitor") => {
            SWITCHES.contains(&value.trim().to_ascii_lowercase().as_str())
        }
        ("core", None, key) => PLAIN_CORE.contains(&key),
        ("remote", Some(_), key) => PLAIN_REMOTE.contains(&key),
        ("submodule", Some(_), "update") => SUBMODULE_UPDATES.contains(&value.trim()),
        ("submodule", Some(_), key) => PLAIN_SUBMODULE.contains(&key),
        ("branch", Some(_), key) => PLAIN_BRANCH.contains(&key),
        ("lfs", _, key) => PLAIN_LFS.contains(&key),
        (section, _, _) if PLAIN_SECTIONS.contains(&section) => true,
        (section, None, key) => PLAIN_OTHER.contains(&format!("{section}.{key}").as_str()),
        _ => false,
    }
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
    fn only_plain_settings_of_the_repository_pass() {
        // What `git init` and `git clone` write, and a submodule set up.
        let plain = "global\tcore.editor vim\n\
                     local\tcore.repositoryformatversion 0\n\
                     local\tcore.filemode true\n\
                     local\tcore.bare false\n\
                     local\tremote.origin.url https://example.com/repo.git\n\
                     local\tremote.origin.fetch +refs/heads/*:refs/remotes/origin/*\n\
                     local\tbranch.main.remote origin\n\
                     local\tbranch.main.merge refs/heads/main\n\
                     local\tsubmodule.lib.url https://example.com/lib.git\n\
                     local\tsubmodule.lib.active true\n\
                     local\tsubmodule.lib.update rebase\n\
                     local\tuser.email ana@example.com\n\
                     local\tpull.rebase true\n\
                     local\tsubmodule.active .\n\
                     local\tcore.worktree ../../../lib\n\
                     local\tlfs.repositoryformatversion 0\n\
                     local\tlfs.https://example.com/repo.git/info/lfs.access basic\n\
                     local\tremote.upstream.gh-resolved base\n\
                     local\tbranch.fix.vscode-merge-base origin/main\n";
        assert_eq!(first_beyond_the_plain(plain), None);
        for (setting, name) in [
            ("alias.st !sh x", "alias.st"),
            ("core.editor e", "core.editor"),
            ("core.hookspath h", "core.hookspath"),
            ("submodule.lib.update !sh u", "submodule.lib.update"),
            ("difftool.x.path /tmp/t", "difftool.x.path"),
            ("gpg.ssh.defaultkeycommand k", "gpg.ssh.defaultkeycommand"),
            ("man.x.cmd m", "man.x.cmd"),
            (
                "credential.https://example.com.helper c",
                "credential.https://example.com.helper",
            ),
            ("remote.origin.uploadpack p", "remote.origin.uploadpack"),
            ("protocol.ext.allow always", "protocol.ext.allow"),
            ("something.new value", "something.new"),
            (
                "lfs.customtransfer.x.path /bin/sh",
                "lfs.customtransfer.x.path",
            ),
            ("lfs.extension.x.smudge s", "lfs.extension.x.smudge"),
            (
                "branch.main.mergeoptions -s evil",
                "branch.main.mergeoptions",
            ),
        ] {
            let listing = format!("{plain}worktree\t{setting}\n");
            assert_eq!(first_beyond_the_plain(&listing).as_deref(), Some(name));
        }
        assert!(is_plain("core.fsmonitor", "false"));
        assert!(!is_plain("core.fsmonitor", "/tmp/watch"));
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
