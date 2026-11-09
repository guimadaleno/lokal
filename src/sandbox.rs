//! The box (`docs/sandbox.md`): a command run in it writes only where the
//! box allows, has no network, reaches no socket, runs nothing of
//! Windows', signals nothing outside, and doesn't see the person's keys.
//! The kernel enforces it: namespaces, Landlock and a seccomp filter, set
//! up by `lokal --box`, a process of its own with one thread, which then
//! becomes the command.

use std::ffi::CString;
use std::fs;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::{approvals, config, system, text};

/// What `lokal --box` exits with when the box couldn't be set up, before
/// anything ran.
pub const UNAVAILABLE: u8 = 125;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Profile {
    /// Writes only in the scratch folder and the box's own /tmp.
    Read,
    /// Also in the project, a git repository, but not in what would have
    /// something run or steer the model later.
    Work,
}

/// Where keys live, under the home folder: covered by empty ones.
const SECRET_FOLDERS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".config/gh",
    ".config/gcloud",
    ".config/rclone",
    ".config/op",
    ".password-store",
    ".local/share/keyrings",
    // Claude Code's own: its credentials and settings.
    ".claude",
];
const SECRET_FILES: &[&str] = &[
    ".netrc",
    ".git-credentials",
    ".config/git/credentials",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".npmrc",
    ".pypirc",
    ".claude.json",
];
/// In the work box, read-only at the project's root: what a tool runs or
/// reads on its own later, when the person opens the folder or runs it
/// outside the box. git's hooks, settings and submodules, and the hook
/// managers that keep them in the tree; editor tasks and agents' settings
/// (Claude Code's, MCP servers); what a shell runs on its own; package
/// managers' and build tools' settings and loaders; dev containers; the
/// project's notes for the model. Claude Code's sandbox protects the same,
/// and the list follows its. A name that isn't there yet can't be covered:
/// the caller checks for new ones after the command.
pub const PROTECTED: &[&str] = &[
    ".git",
    ".gitmodules",
    ".gitconfig",
    ".githooks",
    ".husky",
    ".pre-commit-config.yaml",
    "lefthook.yml",
    "lefthook.yaml",
    ".lefthook.yml",
    ".lefthook.yaml",
    ".vscode",
    ".idea",
    ".claude",
    ".claude.json",
    ".mcp.json",
    ".devcontainer",
    ".devcontainer.json",
    ".envrc",
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".bash_logout",
    ".bash_aliases",
    ".profile",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".zlogout",
    ".ripgreprc",
    ".npmrc",
    ".yarnrc",
    ".yarnrc.yml",
    ".pnp.cjs",
    ".pnp.loader.mjs",
    ".pnpmfile.cjs",
    "bunfig.toml",
    ".bunfig.toml",
    ".bazelrc",
    ".bazelversion",
    ".bazeliskrc",
    "gradle/wrapper/gradle-wrapper.properties",
    ".mvn/wrapper/maven-wrapper.properties",
    "pyrightconfig.json",
];
pub const NOTES: &[&str] = &["LOKAL.md", "AGENTS.md"];
/// Programs run only from here, the home folder and what the box may write:
/// never `/init`, WSL's door to Windows, nor a Windows drive.
const PROGRAM_FOLDERS: &[&str] = &[
    "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/opt", "/snap", "/nix",
];
/// Variables that point at what the box hides. Those that look like they
/// hold a secret go too (`secret_like`).
const DROPPED: &[&str] = &[
    "SSH_AUTH_SOCK",
    "SSH_AGENT_PID",
    "GPG_AGENT_INFO",
    "WSL_INTEROP",
    "DBUS_SESSION_BUS_ADDRESS",
    "DOCKER_HOST",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
];

/// `lokal --box <read|work> <folder> <scratch> <command>`: sets up the box
/// and becomes `bash -c <command>` in `folder`.
pub fn enter(args: impl Iterator<Item = String>) -> ExitCode {
    let args: Vec<String> = args.collect();
    let [profile, folder, scratch, command] = args.as_slice() else {
        eprintln!("{}", text::box_unavailable("usage"));
        return ExitCode::from(UNAVAILABLE);
    };
    let profile = match profile.as_str() {
        "read" => Profile::Read,
        "work" => Profile::Work,
        _ => {
            eprintln!("{}", text::box_unavailable("usage"));
            return ExitCode::from(UNAVAILABLE);
        }
    };
    let Some(bash) = system::program("bash") else {
        eprintln!("{}", text::box_unavailable("bash"));
        return ExitCode::from(UNAVAILABLE);
    };
    if let Err(step) = set_up(profile, Path::new(folder), Path::new(scratch)) {
        eprintln!("{}", text::box_unavailable(&step));
        return ExitCode::from(UNAVAILABLE);
    }
    let mut shell = Command::new(bash);
    shell.arg("-c").arg(command).current_dir(folder);
    for variable in DROPPED {
        shell.env_remove(variable);
    }
    for (name, _) in std::env::vars_os() {
        if secret_like(&name.to_string_lossy()) {
            shell.env_remove(name);
        }
    }
    // Nothing comes in but the command's own output: a descriptor open on
    // a file or a folder outside would be a way past every layer. And no
    // terminal to type into.
    unsafe {
        libc::syscall(libc::SYS_close_range, 3, u32::MAX, 0);
        libc::setsid();
    }
    let error = shell.exec();
    eprintln!("{}", text::box_unavailable(&format!("exec: {error}")));
    ExitCode::from(UNAVAILABLE)
}

/// Every layer, or an error naming the one that failed. Mounts come first:
/// once Landlock holds, the mount topology can't change.
fn set_up(profile: Profile, folder: &Path, scratch: &Path) -> Result<(), String> {
    if !cfg!(target_arch = "x86_64") {
        return Err("only on x86-64 so far".into());
    }
    let home = config::home();
    let project = approvals::project_root(folder);
    // Read while git still runs as it would outside. One that isn't there
    // yet can't be covered, and a command could make it.
    let named = if profile == Profile::Work {
        named_by_git(&project, &home)?
    } else {
        Vec::new()
    };
    if profile == Profile::Work
        && (!project.join(".git").exists()
            || home.starts_with(&project)
            || project == Path::new("/"))
    {
        return Err("the work box needs a git repository".into());
    }
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    check(
        unsafe {
            libc::unshare(
                libc::CLONE_NEWUSER | libc::CLONE_NEWNS | libc::CLONE_NEWNET | libc::CLONE_NEWIPC,
            )
        },
        "unshare",
    )?;
    write("/proc/self/setgroups", "deny")?;
    write("/proc/self/uid_map", &format!("{uid} {uid} 1"))?;
    write("/proc/self/gid_map", &format!("{gid} {gid} 1"))?;
    mount(None, "/", None, libc::MS_REC | libc::MS_PRIVATE, None)?;
    // Found before the box mounts anything of its own.
    // Where their files really are: through a link (/home to /var/home),
    // the path as written would find the wrong mount.
    let mut restricted = vec![home.clone()];
    restricted.extend(config::own_folders());
    if profile == Profile::Work {
        restricted.push(project.clone());
    }
    let restricted: Vec<PathBuf> = restricted
        .into_iter()
        .map(|path| fs::canonicalize(&path).unwrap_or(path))
        .collect();
    let restricted: Vec<&Path> = restricted.iter().map(PathBuf::as_path).collect();
    let doors = doors(&mounts()?, &restricted);
    if let Some(door) = doors
        .iter()
        .find(|door| folder.starts_with(door) || scratch.starts_with(door))
    {
        return Err(format!(
            "working through a second mount, {}",
            door.display()
        ));
    }
    // Under /tmp, the folder the command works in and the scratch folder
    // are taken before /tmp is covered, to be put back in the box's own:
    // the folder first, read-only in the read box, and the scratch folder
    // over it if it's inside.
    let working = if profile == Profile::Work {
        &project
    } else {
        folder
    };
    let mut kept = Vec::new();
    for path in [working, scratch] {
        if path.starts_with("/tmp") {
            kept.push((path.to_path_buf(), open_tree(path)?));
        }
    }

    // Sockets and other sessions live in /run and /tmp; /dev/shm is shared.
    tmpfs("/run", 0)?;
    if fs::symlink_metadata("/var/run").is_ok_and(|meta| meta.is_dir()) {
        tmpfs("/var/run", 0)?;
    }
    tmpfs("/tmp", 0)?;
    if Path::new("/dev/shm").is_dir() {
        tmpfs("/dev/shm", 0)?;
    }
    // The person's terminals: a pty there could be read, or set up anew.
    if Path::new("/dev/pts").is_dir() {
        tmpfs("/dev/pts", libc::MS_RDONLY)?;
    }
    // POSIX message queues of the box's own IPC namespace.
    if Path::new("/dev/mqueue").is_dir() {
        let _ = mount(
            Some(Path::new("mqueue")),
            "/dev/mqueue",
            Some("mqueue"),
            0,
            None,
        );
    }
    for (path, tree) in &kept {
        if !path.exists() {
            fs::create_dir_all(path).map_err(|error| format!("{}: {error}", path.display()))?;
        }
        move_mount(tree, path)?;
        if profile == Profile::Read && path == working && path != scratch {
            read_only(path)?;
        }
    }

    for name in SECRET_FOLDERS {
        let path = home.join(name);
        if path.is_dir() {
            tmpfs(&path, libc::MS_RDONLY)?;
        }
    }
    for name in SECRET_FILES {
        let path = home.join(name);
        if path.is_file() {
            mount(
                Some(Path::new("/dev/null")),
                &path,
                None,
                libc::MS_BIND,
                None,
            )?;
        }
    }
    for path in config::own_folders() {
        if path.is_dir() {
            tmpfs(&path, libc::MS_RDONLY)?;
        }
    }
    if profile == Profile::Work {
        for path in protected(&project, folder)
            .into_iter()
            .chain(named)
            .chain(nested_git(&project))
        {
            read_only(&path)?;
        }
    }
    shut(&doors)?;
    // A second door to Windows shut: WSL's interop is a binfmt_misc entry,
    // and a namespace's own instance starts empty. Landlock already keeps
    // /init from running, so a kernel without it loses nothing.
    let _ = mount(
        Some(Path::new("binfmt_misc")),
        "/proc/sys/fs/binfmt_misc",
        Some("binfmt_misc"),
        0,
        None,
    );
    // Only `lo` exists here; up, a project's tests can still listen on it.
    let _ = loopback_up();

    landlock(profile, &project, scratch, &home)?;
    seccomp()
}

/// What of the tree git's settings would have git run or read later,
/// outside the box, read with git itself (includes followed, every scope)
/// before the box is set up. Err when the work box can't vouch for them:
/// - the repository's own settings, which anything could have written,
///   may hold only the plain ones `gitconfig::is_plain` knows, since the
///   settings that name a program have no fixed end;
/// - the person's own (global, system) are theirs, but a path in them can
///   point into the tree (`core.hooksPath = .hooks`): each such path is
///   kept read-only, and one that isn't there yet refuses the box, since a
///   command could make it;
/// - git that can't list them refuses the box too.
fn named_by_git(project: &Path, home: &Path) -> Result<Vec<PathBuf>, String> {
    let git = system::program("git").ok_or("git isn't there to read its settings")?;
    let output = Command::new(git)
        .arg("-C")
        .arg(project)
        .args(["config", "--includes", "--show-scope", "--null", "--list"])
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("git config: {error}"))?;
    if !output.status.success() {
        return Err("git couldn't list the repository's settings".into());
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut fields = listing.split('\0');
    let mut named = Vec::new();
    while let (Some(scope), Some(setting)) = (fields.next(), fields.next()) {
        let (key, value) = setting.split_once('\n').unwrap_or((setting, ""));
        let key = key.to_ascii_lowercase();
        if matches!(scope, "local" | "worktree") {
            if !crate::gitconfig::is_plain(&key, value) {
                return Err(format!(
                    "the repository's git settings hold {key}, which the box can't vouch for"
                ));
            }
            continue;
        }
        for (path, surely) in paths_in(project, home, &key, value) {
            if path.exists() {
                named.push(path);
            } else if surely {
                return Err(format!(
                    "git's settings name {}, which isn't there yet",
                    path.display()
                ));
            }
        }
    }
    Ok(named)
}

/// The words of a setting's value that name a path inside the project, as
/// git would read them: `~/` is the home folder, a relative path is the
/// project's. A bare word is a program on the PATH, but for a hooks folder.
/// Each comes with whether it surely means a file of the project, so that
/// one not there yet refuses the box: a hooks folder, a word starting with
/// `./`, or one whose first folder is the project's (`tools/t.sh` where
/// `tools` is). `origin/main` in an alias is a ref, not a path. An include
/// of the person's counts only by a full path (`~/p/.gitconfig`): a
/// relative one is read from their own folder.
fn paths_in(project: &Path, home: &Path, key: &str, value: &str) -> Vec<(PathBuf, bool)> {
    let include = key.starts_with("include");
    config::words(value)
        .into_iter()
        .map(|word| word.trim_start_matches('!').to_string())
        .filter(|word| !word.is_empty() && !word.starts_with('-') && !word.contains("://"))
        .filter(|word| key == "core.hookspath" || word.contains('/') || word.starts_with('.'))
        .filter(|word| !include || word.starts_with('/') || word.starts_with("~/"))
        .filter_map(|word| {
            let path = lexical(&match word.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => project.join(&word),
            });
            let first = path.strip_prefix(project).ok()?.components().next()?;
            let surely = RUN_BY_GIT.contains(&key)
                || (key.starts_with("gpg.") && key.ends_with(".program"))
                || include
                || word.starts_with("./")
                || (word.contains('/') && project.join(first).is_dir());
            Some((path, surely))
        })
        .collect()
}

/// Settings whose program git runs by itself: a path in them always means
/// a file, even one that reads like a ref (`scripts/x`).
const RUN_BY_GIT: &[&str] = &[
    "core.hookspath",
    "core.fsmonitor",
    "core.pager",
    "core.editor",
    "sequence.editor",
    "core.sshcommand",
    "core.askpass",
    "diff.external",
    "gpg.program",
];

/// A variable whose name says it holds a secret: a token, a password, a
/// key, an account's.
fn secret_like(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "API_KEY",
        "PRIVATE_KEY",
    ]
    .iter()
    .any(|word| name.contains(word))
        || name.starts_with("AWS_")
        || name.ends_with("_KEY")
}

/// Repositories inside the project, each `.git` below the root: git would
/// run what their settings name when someone works there later. Links aren't
/// followed, and node_modules, a package manager's, is left out.
fn nested_git(project: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![project.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            if name == ".git" {
                if folder != project {
                    found.push(entry.path());
                }
            } else if kind.is_dir() && name != "node_modules" {
                folders.push(entry.path());
            }
        }
    }
    found
}

/// `path` with its `.` and `..` worked out on the text alone.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// A line of /proc/self/mountinfo: the file system's device, the folder of
/// it the mount shows, and where.
struct Mount {
    device: (u32, u32),
    root: PathBuf,
    at: PathBuf,
}

fn mounts() -> Result<Vec<Mount>, String> {
    let mountinfo = fs::read_to_string("/proc/self/mountinfo")
        .map_err(|error| format!("mountinfo: {error}"))?;
    Ok(mountinfo.lines().filter_map(mount_line).collect())
}

fn mount_line(line: &str) -> Option<Mount> {
    let mut fields = line.split(' ').skip(2);
    let (major, minor) = fields.next()?.split_once(':')?;
    Some(Mount {
        device: (major.parse().ok()?, minor.parse().ok()?),
        root: PathBuf::from(unescape(fields.next()?)),
        at: PathBuf::from(unescape(fields.next()?)),
    })
}

/// The mount `path` is reached through, the last of the deepest, and the
/// path inside its file system.
fn resolve<'a>(mounts: &'a [Mount], path: &Path) -> Option<(&'a Mount, PathBuf)> {
    let mount = mounts
        .iter()
        .filter(|mount| path.starts_with(&mount.at))
        .max_by_key(|mount| mount.at.components().count())?;
    Some((mount, mount.root.join(path.strip_prefix(&mount.at).ok()?)))
}

/// The same files can be reached through a second mount of their file
/// system: WSL mounts the whole distribution again at /mnt/wslg/distro,
/// and Docker Desktop binds folders of home under /mnt/wsl. Landlock's
/// rules follow the files, not the path, and the hidden keys are hidden
/// at one path only. So a mount is a door when it shows the files of a
/// `restricted` path somewhere else than that path's own mount does. A
/// mount of a folder at its own place, like /snap, isn't one, nor is a file
/// system mounted once, as a /home partition is.
fn doors(mounts: &[Mount], restricted: &[&Path]) -> Vec<PathBuf> {
    let mut doors = Vec::new();
    for path in restricted {
        let Some((own, inside)) = resolve(mounts, path) else {
            continue;
        };
        for other in mounts {
            if std::ptr::eq(other, own) || other.device != own.device {
                continue;
            }
            let overlaps = inside.starts_with(&other.root) || other.root.starts_with(&inside);
            let in_place = other
                .root
                .strip_prefix(&own.root)
                .is_ok_and(|rest| own.at.join(rest) == other.at);
            if overlaps && !in_place && !doors.contains(&other.at) {
                doors.push(other.at.clone());
            }
        }
    }
    doors.sort_by_key(|at| at.as_os_str().len());
    doors
}

/// Each door shut, the shortest first, as covering one covers what's
/// beneath it: a folder with an empty read-only tmpfs, a file with
/// /dev/null. One that can't be reached any more is shut already.
fn shut(doors: &[PathBuf]) -> Result<(), String> {
    let mut shut: Vec<&PathBuf> = Vec::new();
    for at in doors {
        if shut.iter().any(|done| at.starts_with(done)) {
            continue;
        }
        match fs::symlink_metadata(at) {
            Err(_) => continue,
            Ok(meta) if meta.is_dir() => tmpfs(at, libc::MS_RDONLY)?,
            Ok(_) => mount(Some(Path::new("/dev/null")), at, None, libc::MS_BIND, None)?,
        }
        shut.push(at);
    }
    Ok(())
}

/// mountinfo writes a space, a tab, a newline and a backslash as \040, \011, \012, \134.
fn unescape(field: &str) -> String {
    let mut out = String::new();
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let code: String = chars.by_ref().take(3).collect();
            if let Ok(byte) = u8::from_str_radix(&code, 8) {
                out.push(byte as char);
                continue;
            }
            out.push(c);
            out.push_str(&code);
        } else {
            out.push(c);
        }
    }
    out
}

/// What stays read-only in the work box: the protected names at the
/// project's root, and the notes from there down to `folder`.
fn protected(project: &Path, folder: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = PROTECTED.iter().map(|name| project.join(name)).collect();
    for dir in folder.ancestors() {
        paths.extend(NOTES.iter().map(|name| dir.join(name)));
        if dir == project {
            break;
        }
    }
    paths.retain(|path| path.exists());
    paths
}

// Landlock, from linux/landlock.h: libc doesn't carry it.
const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;
const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;
const EXECUTE: u64 = 1 << 0;
const WRITE_FILE: u64 = 1 << 1;
const REMOVE_DIR: u64 = 1 << 4;
const REMOVE_FILE: u64 = 1 << 5;
const MAKE_CHAR: u64 = 1 << 6;
const MAKE_DIR: u64 = 1 << 7;
const MAKE_REG: u64 = 1 << 8;
const MAKE_SOCK: u64 = 1 << 9;
const MAKE_FIFO: u64 = 1 << 10;
const MAKE_BLOCK: u64 = 1 << 11;
const MAKE_SYM: u64 = 1 << 12;
const REFER: u64 = 1 << 13;
const TRUNCATE: u64 = 1 << 14;
/// An ioctl on a device: a terminal's settings, or WSL's GPU channel to
/// Windows (/dev/dxg). ABI 5.
const IOCTL_DEV: u64 = 1 << 15;
const WRITES: u64 = WRITE_FILE
    | REMOVE_DIR
    | REMOVE_FILE
    | MAKE_CHAR
    | MAKE_DIR
    | MAKE_REG
    | MAKE_SOCK
    | MAKE_FIFO
    | MAKE_BLOCK
    | MAKE_SYM
    | REFER
    | TRUNCATE;
const SCOPE_ABSTRACT_UNIX_SOCKET: u64 = 1 << 0;
const SCOPE_SIGNAL: u64 = 1 << 1;
/// The scopes came with ABI 6 (Linux 6.12): without them, a command could
/// signal Lokal.
const LANDLOCK_ABI: i64 = 6;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

/// Writes only beneath the box's own folders, programs only from where
/// programs live, and no signal or abstract socket past the box. Reading
/// is left alone: the read-only list reads anywhere already.
fn landlock(profile: Profile, project: &Path, scratch: &Path, home: &Path) -> Result<(), String> {
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<RulesetAttr>(),
            0,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if abi < LANDLOCK_ABI {
        return Err(format!("landlock ABI {abi}, {LANDLOCK_ABI} needed"));
    }
    let attr = RulesetAttr {
        handled_access_fs: WRITES | EXECUTE | IOCTL_DEV,
        handled_access_net: 0,
        scoped: SCOPE_ABSTRACT_UNIX_SOCKET | SCOPE_SIGNAL,
    };
    let ruleset = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attr,
            std::mem::size_of::<RulesetAttr>(),
            0,
        )
    };
    check(ruleset as i32, "landlock")?;
    let ruleset = unsafe { OwnedFd::from_raw_fd(ruleset as i32) };

    // The scratch folder by name too: under a TMPDIR elsewhere, it isn't in /tmp.
    let mut writable = vec![
        PathBuf::from("/tmp"),
        PathBuf::from("/dev/shm"),
        scratch.to_path_buf(),
    ];
    if profile == Profile::Work {
        writable.push(project.to_path_buf());
    }
    for path in &writable {
        allow(&ruleset, path, WRITES | EXECUTE)?;
    }
    allow(
        &ruleset,
        Path::new("/dev/null"),
        WRITE_FILE | TRUNCATE | IOCTL_DEV,
    )?;
    for folder in PROGRAM_FOLDERS.iter().map(Path::new).chain([home]) {
        allow(&ruleset, folder, EXECUTE)?;
    }

    check(
        unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) },
        "no_new_privs",
    )?;
    let restricted =
        unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset.as_raw_fd(), 0) };
    check(restricted as i32, "landlock")
}

/// A rule for `path` and what's beneath it. A path that isn't there needs none.
fn allow(ruleset: &OwnedFd, path: &Path, access: u64) -> Result<(), String> {
    let Ok(target) = fs::File::options()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
        .open(path)
    else {
        return Ok(());
    };
    // A file takes only the rights a file can have.
    let access = if target.metadata().is_ok_and(|meta| meta.is_dir()) {
        access
    } else {
        access & (EXECUTE | WRITE_FILE | TRUNCATE | IOCTL_DEV)
    };
    let rule = PathBeneathAttr {
        allowed_access: access,
        parent_fd: target.as_raw_fd(),
    };
    let added = unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset.as_raw_fd(),
            LANDLOCK_RULE_PATH_BENEATH,
            &rule,
            0,
        )
    };
    check(
        added as i32,
        &format!("landlock rule for {}", path.display()),
    )
}

/// Sockets of the internet's kinds only, and netlink, which some programs
/// ask about the network with. A Unix socket outside could be reached by
/// its path (Landlock doesn't cover connecting to one), and vsock reaches
/// the Windows host under WSL past the network namespace. socketpair still
/// works. No io_uring, which could open a socket past this filter. On
/// x86-64, a 32-bit or x32 call is refused whole.
fn seccomp() -> Result<(), String> {
    const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
    const X32_SYSCALL_BIT: u32 = 0x4000_0000;
    const ARCH: u32 = 4;
    const NR: u32 = 0;
    const ARG0: u32 = 16;
    let deny = libc::SECCOMP_RET_ERRNO | libc::EPERM as u32;
    let allow = libc::SECCOMP_RET_ALLOW;
    let load = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
    let jeq = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
    let jge = (libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K) as u16;
    let ret = (libc::BPF_RET | libc::BPF_K) as u16;
    let at = |code, jt, jf, k| libc::sock_filter { code, jt, jf, k };
    let filter = [
        at(load, 0, 0, ARCH),
        at(jeq, 1, 0, AUDIT_ARCH_X86_64),
        at(ret, 0, 0, deny),
        at(load, 0, 0, NR),
        at(jge, 0, 1, X32_SYSCALL_BIT),
        at(ret, 0, 0, deny),
        at(jeq, 0, 1, libc::SYS_io_uring_setup as u32),
        at(ret, 0, 0, deny),
        // Not socket(): allowed, at 14.
        at(jeq, 0, 5, libc::SYS_socket as u32),
        at(load, 0, 0, ARG0),
        at(jeq, 3, 0, libc::AF_INET as u32),
        at(jeq, 2, 0, libc::AF_INET6 as u32),
        at(jeq, 1, 0, libc::AF_NETLINK as u32),
        at(ret, 0, 0, deny),
        at(ret, 0, 0, allow),
    ];
    let program = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_ptr().cast_mut(),
    };
    let set = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            0,
            &program,
        )
    };
    check(set as i32, "seccomp")
}

/// A detached copy of the mount at `path`, to attach elsewhere later.
fn open_tree(path: &Path) -> Result<OwnedFd, String> {
    let path = c_path(path)?;
    let fd = unsafe {
        libc::syscall(
            libc::SYS_open_tree,
            libc::AT_FDCWD,
            path.as_ptr(),
            libc::OPEN_TREE_CLONE | libc::OPEN_TREE_CLOEXEC | libc::AT_RECURSIVE as u32,
        )
    };
    check(fd as i32, "open_tree")?;
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

fn move_mount(tree: &OwnedFd, target: &Path) -> Result<(), String> {
    let empty = CString::default();
    let target = c_path(target)?;
    let moved = unsafe {
        libc::syscall(
            libc::SYS_move_mount,
            tree.as_raw_fd(),
            empty.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::MOVE_MOUNT_F_EMPTY_PATH,
        )
    };
    check(moved as i32, "move_mount")
}

fn tmpfs(target: impl AsRef<Path>, flags: libc::c_ulong) -> Result<(), String> {
    mount(
        Some(Path::new("tmpfs")),
        target,
        Some("tmpfs"),
        flags | libc::MS_NOSUID | libc::MS_NODEV,
        Some("mode=0700"),
    )
}

/// `path` bound onto itself, then made read-only, keeping the flags the
/// mount it's on already has: a namespace can't clear them.
fn read_only(path: &Path) -> Result<(), String> {
    mount(Some(path), path, None, libc::MS_BIND | libc::MS_REC, None)?;
    let held = mount_flags(path)?;
    mount(
        None,
        path,
        None,
        libc::MS_REMOUNT | libc::MS_BIND | libc::MS_RDONLY | held,
        None,
    )
}

fn mount_flags(path: &Path) -> Result<libc::c_ulong, String> {
    let c = c_path(path)?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    check(unsafe { libc::statvfs(c.as_ptr(), &mut stat) }, "statvfs")?;
    let pairs = [
        (libc::ST_NOSUID, libc::MS_NOSUID),
        (libc::ST_NODEV, libc::MS_NODEV),
        (libc::ST_NOEXEC, libc::MS_NOEXEC),
        (libc::ST_NOATIME, libc::MS_NOATIME),
        (libc::ST_NODIRATIME, libc::MS_NODIRATIME),
        // musl's libc lacks ST_RELATIME; it's Linux's 0x1000 for both.
        (0x1000, libc::MS_RELATIME),
    ];
    Ok(pairs
        .iter()
        .filter(|(st, _)| stat.f_flag & st != 0)
        .fold(0, |flags, (_, ms)| flags | ms))
}

fn mount(
    source: Option<&Path>,
    target: impl AsRef<Path>,
    kind: Option<&str>,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> Result<(), String> {
    let target = target.as_ref();
    let source = source.map(c_path).transpose()?;
    let kind = kind.map(|kind| CString::new(kind).unwrap_or_default());
    let data = data.map(|data| CString::new(data).unwrap_or_default());
    let c_target = c_path(target)?;
    let done = unsafe {
        libc::mount(
            source.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            c_target.as_ptr(),
            kind.as_ref().map_or(std::ptr::null(), |k| k.as_ptr()),
            flags,
            data.as_ref()
                .map_or(std::ptr::null(), |d| d.as_ptr().cast()),
        )
    };
    check(done, &format!("mount {}", target.display()))
}

fn loopback_up() -> Result<(), String> {
    let socket = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    check(socket, "socket")?;
    let socket = unsafe { OwnedFd::from_raw_fd(socket) };
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    for (slot, byte) in request.ifr_name.iter_mut().zip(b"lo\0") {
        *slot = *byte as libc::c_char;
    }
    check(
        unsafe { libc::ioctl(socket.as_raw_fd(), libc::SIOCGIFFLAGS as _, &mut request) },
        "SIOCGIFFLAGS",
    )?;
    unsafe { request.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short };
    check(
        unsafe { libc::ioctl(socket.as_raw_fd(), libc::SIOCSIFFLAGS as _, &request) },
        "SIOCSIFFLAGS",
    )
}

fn write(path: &str, contents: &str) -> Result<(), String> {
    fs::write(path, contents).map_err(|error| format!("{path}: {error}"))
}

fn c_path(path: &Path) -> Result<CString, String> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| format!("{}: NUL", path.display()))
}

fn check(result: i32, step: &str) -> Result<(), String> {
    if result < 0 {
        Err(format!("{step}: {}", std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(device: u32, root: &str, at: &str) -> Mount {
        Mount {
            device: (8, device),
            root: PathBuf::from(root),
            at: PathBuf::from(at),
        }
    }

    #[test]
    fn a_second_mount_of_the_same_files_is_a_door() {
        // WSL with Docker Desktop: the disk again at /mnt/wslg/distro, and
        // a folder of home bound under /mnt/wsl.
        let wsl = [
            at(48, "/", "/"),
            at(48, "/", "/mnt/wslg/distro"),
            at(48, "/snap", "/snap"),
            at(48, "/home/ana/sites", "/mnt/wsl/docker/1"),
            at(48, "/var/lib/docker", "/mnt/wsl/docker/2"),
            at(9, "/", "/mnt/c"),
        ];
        let home = Path::new("/home/ana");
        assert_eq!(
            doors(&wsl, &[home]),
            [
                PathBuf::from("/mnt/wslg/distro"),
                PathBuf::from("/mnt/wsl/docker/1")
            ]
        );
    }

    #[test]
    fn a_file_system_mounted_once_has_no_door() {
        // /home and /tmp of their own, as on many distributions.
        let own = [
            at(1, "/", "/"),
            at(2, "/", "/home"),
            at(3, "/", "/tmp"),
            at(1, "/srv/data", "/srv/data"),
        ];
        assert!(
            doors(
                &own,
                &[Path::new("/home/ana"), Path::new("/home/ana/project")]
            )
            .is_empty()
        );
    }

    #[test]
    fn the_last_of_the_deepest_mount_is_the_one_a_path_goes_through() {
        let stacked = [at(1, "/", "/"), at(2, "/", "/home"), at(3, "/", "/home")];
        let (mount, inside) = resolve(&stacked, Path::new("/home/ana")).unwrap();
        assert_eq!((mount.device, inside), ((8, 3), PathBuf::from("/ana")));
    }

    #[test]
    fn mountinfo_fields_are_read_unescaped() {
        let line =
            "36 35 98:0 /my\\040dir /mnt/my\\040mount rw,noatime master:1 - ext3 /dev/root rw";
        let mount = mount_line(line).unwrap();
        assert_eq!(mount.device, (98, 0));
        assert_eq!(mount.root, PathBuf::from("/my dir"));
        assert_eq!(mount.at, PathBuf::from("/mnt/my mount"));
    }

    #[test]
    fn paths_in_a_setting_are_the_projects_or_homes() {
        let project = Path::new("/nonexistent/ana/p");
        let home = Path::new("/nonexistent/ana");
        let found = |key, value| paths_in(project, home, key, value);
        assert_eq!(
            found("alias.t", "!sh ./t.sh --flag"),
            [(PathBuf::from("/nonexistent/ana/p/t.sh"), true)]
        );
        assert_eq!(
            found("core.hookspath", "hooks"),
            [(PathBuf::from("/nonexistent/ana/p/hooks"), true)]
        );
        // A ref, or a path whose first folder isn't the project's.
        assert_eq!(
            found("alias.up", "!git fetch && git rebase origin/main"),
            [(PathBuf::from("/nonexistent/ana/p/origin/main"), false)]
        );
        // ~/ is home's, outside the project; a bare program is the PATH's;
        // a quoted or escaped space stays in its word.
        assert!(found("core.hookspath", "~/hooks").is_empty());
        assert!(found("core.pager", "less -R").is_empty());
        assert!(found("credential.helper", "!/usr/bin/gh auth git-credential").is_empty());
        assert!(found("core.editor", "\"/mnt/c/Program Files/Code/code\" --wait").is_empty());
        assert!(found("credential.helper", "/mnt/c/Program\\ Files/Git/gcm.exe").is_empty());
        assert!(found("url.x.insteadof", "https://example.com/").is_empty());
        assert!(found("core.hookspath", "../../..").is_empty());
        // The person's include counts by a full path only.
        assert_eq!(
            found("includeif.gitdir:~/p/.path", "~/p/.gitconfig"),
            [(PathBuf::from("/nonexistent/ana/p/.gitconfig"), true)]
        );
        assert!(found("include.path", "local.gitconfig").is_empty());
        assert_eq!(
            config::words("a \"b c\" d\\ e 'f g'"),
            ["a", "b c", "d e", "f g"]
        );
        // What git runs by itself means a file, even one that reads like a ref.
        assert_eq!(
            found("core.pager", "scripts/x"),
            [(PathBuf::from("/nonexistent/ana/p/scripts/x"), true)]
        );
        assert!(secret_like("GH_TOKEN") && secret_like("AWS_REGION") && secret_like("MY_API_KEY"));
        assert!(!secret_like("PATH") && !secret_like("LANG") && !secret_like("MONKEY"));
        assert_eq!(lexical(Path::new("/p/./a/../b")), PathBuf::from("/p/b"));
    }
}
