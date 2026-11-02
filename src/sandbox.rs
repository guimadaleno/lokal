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
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
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
];
const SECRET_FILES: &[&str] = &[".netrc", ".git-credentials"];
/// In the work box, read-only: git's hooks and settings, and the hook
/// managers that keep them in the tree, editor tasks, what a shell runs on
/// its own, and the project's notes for the model. A name that isn't there
/// yet can't be covered: the caller checks for new ones after the command.
pub const PROTECTED: &[&str] = &[
    ".git",
    ".githooks",
    ".husky",
    ".pre-commit-config.yaml",
    "lefthook.yml",
    "lefthook.yaml",
    ".lefthook.yml",
    ".vscode",
    ".idea",
    ".envrc",
    ".bashrc",
    ".zshrc",
    ".profile",
    ".bash_profile",
    ".gitconfig",
];
pub const NOTES: &[&str] = &["LOKAL.md", "AGENTS.md"];
/// Programs run only from here, the home folder and what the box may write:
/// never `/init`, WSL's door to Windows, nor a Windows drive.
const PROGRAM_FOLDERS: &[&str] = &[
    "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/opt", "/snap", "/nix",
];
/// Variables that point at what the box hides.
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
    // Read while git still runs as it would outside: where the repository
    // keeps hooks of its own, if inside it.
    let hooks = (profile == Profile::Work)
        .then(|| hooks_path(&project))
        .flatten();
    if profile == Profile::Work
        && (!project.join(".git").exists()
            || home.starts_with(&project)
            || project == Path::new("/"))
    {
        return Err("the work box needs a git repository".into());
    }
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    check(
        unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNS | libc::CLONE_NEWNET) },
        "unshare",
    )?;
    write("/proc/self/setgroups", "deny")?;
    write("/proc/self/uid_map", &format!("{uid} {uid} 1"))?;
    write("/proc/self/gid_map", &format!("{gid} {gid} 1"))?;
    mount(None, "/", None, libc::MS_REC | libc::MS_PRIVATE, None)?;
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
        for path in protected(&project, folder).into_iter().chain(hooks) {
            read_only(&path)?;
        }
    }
    let devices = [Path::new("/"), &home, &project, folder, scratch]
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|meta| device_numbers(meta.dev()))
        .collect::<Vec<_>>();
    cover_other_doors(&devices)?;
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

    landlock(profile, &project, &home)?;
    seccomp()
}

/// core.hooksPath as the repository sets it, when it's inside the project.
fn hooks_path(project: &Path) -> Option<PathBuf> {
    let output = Command::new(system::program("git")?)
        .arg("-C")
        .arg(project)
        .args(["config", "--local", "--get", "core.hooksPath"])
        .output()
        .ok()?;
    let value = String::from_utf8(output.stdout).ok()?;
    let path = project.join(value.trim());
    (!value.trim().is_empty() && path.starts_with(project) && path.exists()).then_some(path)
}

/// The same files can be reached through a second mount of their file
/// system: WSL mounts the whole distribution again at /mnt/wslg/distro,
/// and Docker Desktop binds folders of home under /mnt/wsl. Landlock's
/// rules follow the files, not the path, and the hidden keys are hidden
/// at one path only. So every mount of these file systems that shows a
/// folder somewhere else than where it lives is covered, the shortest
/// first, as covering one covers what's beneath it.
fn cover_other_doors(devices: &[(u32, u32)]) -> Result<(), String> {
    let mountinfo = fs::read_to_string("/proc/self/mountinfo")
        .map_err(|error| format!("mountinfo: {error}"))?;
    let mut doors: Vec<PathBuf> = mountinfo
        .lines()
        .filter_map(other_door)
        .filter(|(device, _)| devices.contains(device))
        .map(|(_, at)| at)
        .collect();
    doors.sort_by_key(|at| at.as_os_str().len());
    let mut covered: Vec<PathBuf> = Vec::new();
    for at in doors {
        if covered.iter().any(|done| at.starts_with(done)) {
            continue;
        }
        tmpfs(&at, libc::MS_RDONLY)?;
        covered.push(at);
    }
    Ok(())
}

/// From a mountinfo line, the file system's device and where it's mounted,
/// when what it shows lives somewhere else ("/home/ana/sites" mounted at
/// "/mnt/wsl/…"). A mount of a folder at its own place, like /snap, isn't
/// one.
fn other_door(line: &str) -> Option<((u32, u32), PathBuf)> {
    let mut fields = line.split(' ');
    let (_, _, device, root, at) = (
        fields.next()?,
        fields.next()?,
        fields.next()?,
        fields.next()?,
        fields.next()?,
    );
    let (major, minor) = device.split_once(':')?;
    let device = (major.parse().ok()?, minor.parse().ok()?);
    let (root, at) = (unescape(root), unescape(at));
    (root != at && at != "/").then(|| (device, PathBuf::from(at)))
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

fn device_numbers(device: u64) -> (u32, u32) {
    (libc::major(device), libc::minor(device))
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
fn landlock(profile: Profile, project: &Path, home: &Path) -> Result<(), String> {
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
        handled_access_fs: WRITES | EXECUTE,
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

    let mut writable = vec![PathBuf::from("/tmp"), PathBuf::from("/dev/shm")];
    if profile == Profile::Work {
        writable.push(project.to_path_buf());
    }
    for path in &writable {
        allow(&ruleset, path, WRITES | EXECUTE)?;
    }
    allow(&ruleset, Path::new("/dev/null"), WRITE_FILE | TRUNCATE)?;
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
        access & (EXECUTE | WRITE_FILE | TRUNCATE)
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
