//! The hard core: what a command's question opens on No for, and what
//! auto's judge never decides. These are the steps that can't be undone,
//! that leave the machine, or that change what runs later; everything else
//! opens on Yes. A line Lokal can't read whole is read roughly
//! (`rough_programs`), and a program in it that Lokal can't name, or whose
//! paths it can't place, counts as the hard core.

use std::env;
use std::path::{Component, Path, PathBuf};

use super::{Output, Program, git_command, glob_may_write, split, uses_any};
use crate::protected::is_protected;
use crate::{config, gitconfig};

/// Root, and other machines.
const PAST_THE_PERSON: &[&str] = &["sudo", "su", "doas", "pkexec", "run0", "sudoedit", "ssh"];
/// They send data off the machine.
const SENDERS: &[&str] = &[
    "scp",
    "sftp",
    "nc",
    "ncat",
    "netcat",
    "socat",
    "telnet",
    "ftp",
    "tftp",
    "lftp",
    "smbclient",
    "rclone",
];
/// Disks, power, and what runs on a schedule.
const MACHINE: &[&str] = &[
    "dd", "wipefs", "shred", "fdisk", "sfdisk", "cfdisk", "parted", "mkswap", "shutdown", "reboot",
    "poweroff", "halt", "crontab",
];
/// They delete or move files: on No unless every path lands in the folders.
const DELETERS: &[&str] = &["rm", "rmdir", "unlink", "truncate", "mv"];
/// What a download piped into would run.
const RUNNERS: &[&str] = &[
    "sh", "bash", "zsh", "dash", "ksh", "fish", "csh", "tcsh", "pwsh", "busybox", "python",
    "python3", "perl", "ruby", "node", "php", "lua", "deno", "bun",
];
/// Programs that run the command or the code in their arguments. Those are
/// read for the names here, as a line Lokal can't read whole is.
const PASSING_ON: &[&str] = &[
    "sh", "bash", "zsh", "dash", "ksh", "fish", "csh", "tcsh", "pwsh", "busybox", "toybox", "eval",
    "source", ".", "exec", "xargs", "env", "nice", "nohup", "timeout", "time", "command",
    "builtin", "watch", "setsid", "stdbuf", "ionice", "taskset", "chroot", "unshare", "nsenter",
    "flock", "parallel", "script", "strace", "awk", "gawk", "mawk", "nawk", "sed", "find",
    "python", "python3", "perl", "ruby", "node", "php", "lua", "deno", "bun", "docker", "podman",
];
/// Programs that run the program in their arguments: git behind one of
/// them is out of reach of the settings check.
const WRAPPERS: &[&str] = &[
    "env", "timeout", "nice", "nohup", "stdbuf", "ionice", "taskset", "time", "command", "builtin",
    "exec", "setsid", "flock", "watch", "xargs", "parallel", "strace",
];
/// Windows programs, in lowercase, that run as another user or run scripts
/// Lokal can't read: WSL's own door (`wsl.exe -u root`), and script hosts.
const WINDOWS_PAST_THE_PERSON: &[&str] = &[
    "wsl.exe",
    "wsl",
    "bash.exe",
    "wscript.exe",
    "cscript.exe",
    "mshta.exe",
    "runas.exe",
];
/// Words, in lowercase, that make a Windows shell's line delete, send, run
/// what it downloads, or change the system.
const WINDOWS_HARD: &[&str] = &[
    "del",
    "erase",
    "rd",
    "rmdir",
    "remove-item",
    "ri",
    "format",
    "format-volume",
    "robocopy",
    "iex",
    "invoke-expression",
    "-infile",
    "set-executionpolicy",
    "runas",
    "reg",
    "start-process",
];
/// curl options that send data, a file or the person's credentials.
const CURL_SENDING: &[&str] = &[
    "-d",
    "--data",
    "-F",
    "--form",
    "-T",
    "--upload-file",
    "-X",
    "--request",
    "-b",
    "--cookie",
    "-u",
    "--user",
    "-n",
    "--netrc",
    "-K",
    "--config",
    "--json",
    "--url-query",
    "--variable",
];
const WGET_SENDING: &[&str] = &[
    "--post-data",
    "--post-file",
    "--body-data",
    "--body-file",
    "--method",
    "-e",
    "--execute",
    "--use-askpass",
];
/// gh's first words that only read.
const GH_READING_FIRST: &[&str] = &["search", "status", "version", "help", "browse"];
/// gh's second words that only read, or fetch to this machine.
const GH_READING: &[&str] = &[
    "list", "view", "status", "diff", "checks", "watch", "clone", "download", "checkout",
];
const GH_API_SENDING: &[&str] = &[
    "-X",
    "--method",
    "-f",
    "-F",
    "--field",
    "--raw-field",
    "--input",
];
/// Package tools, and their words for sending out what can't be taken back.
const PUBLISHERS: &[&str] = &[
    "npm", "pnpm", "yarn", "bun", "cargo", "uv", "poetry", "twine", "gem",
];
const PUBLISHING: &[&str] = &["publish", "unpublish", "upload", "push", "yank"];
/// docker that reaches the machine (the daemon is root) or deletes data.
const DOCKER_HARD: &[&str] = &[
    "-v",
    "--volume",
    "--volumes",
    "--mount",
    "--privileged",
    "--pid",
    "--cap-add",
    "--device",
    "prune",
    "push",
];
/// git's commands that may throw work away or send it elsewhere, for a
/// line Lokal can't read whole.
const GIT_HARD_WORDS: &[&str] = &[
    "push",
    "reset",
    "clean",
    "restore",
    "checkout",
    "switch",
    "stash",
    "branch",
    "tag",
    "rm",
    "remote",
    "config",
    "reflog",
    "gc",
    "prune",
    "filter-branch",
    "filter-repo",
    "update-ref",
    "worktree",
];
/// Places no command line may name: where programs run later (shell
/// start-up, autostart, services, a PATH folder, git's hooks and settings),
/// keys, and Lokal's own files.
const RUN_LATER: &[&str] = &[
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
    ".config/fish",
    ".xprofile",
    ".xinitrc",
    ".pam_environment",
    ".config/environment.d",
    ".envrc",
    ".ssh",
    ".gnupg",
    ".netrc",
    "authorized_keys",
    "/etc/sudoers",
    ".gitconfig",
    ".config/git",
    ".git/hooks",
    ".git/config",
    ".curlrc",
    ".wgetrc",
    ".npmrc",
    ".config/autostart",
    "systemd/",
    "/etc/cron",
    "/var/spool/cron",
    ".local/bin",
    ".config/lokal",
    ".local/state/lokal",
    "/dev/tcp/",
    "/dev/udp/",
];

/// Whether `command`, run from the folder `here` (the shell's own, which
/// may be reached through a link), is the hard core. A delete, a move or a
/// `>` that empties a file is let through only when what it touches lands
/// in one of `folders`, and nothing may be written where programs run
/// later.
pub(super) fn is_hard(command: &str, here: &Path, folders: &[PathBuf]) -> bool {
    if names_a_place_that_runs_later(command)
        || sets_git_from_the_environment(command)
        // ANSI-C quoting spells any name: `$'\x72m'` is rm.
        || command.contains("$'")
    {
        return true;
    }
    let (programs, whole) = read(command);
    let named = |list: &[&str]| {
        programs
            .iter()
            .any(|program| list.contains(&name_of(program)))
    };
    if named(&["curl", "wget"]) && runs_a_download(command) {
        return true;
    }
    let mut places = starting_places(command, &programs, here, whole);
    if !whole {
        let outputs = rough_outputs(command);
        if outputs
            .iter()
            .any(|output| output_is_hard(output, places.as_deref(), folders))
        {
            return true;
        }
    }
    programs.iter().any(|program| {
        let hard = program
            .outputs
            .iter()
            .any(|output| output_is_hard(output, places.as_deref(), folders))
            || program_is_hard(program, places.as_deref(), folders, whole);
        if whole {
            places = moved(program, places.take());
        }
        // A `cd` into where programs run later, as `cd ~/.config`.
        hard || places
            .as_deref()
            .is_some_and(|places| places.iter().any(|place| is_protected(place)))
    })
}

/// A line's programs: read whole when `split` can, roughly otherwise, and
/// which of the two.
fn read(command: &str) -> (Vec<Program>, bool) {
    match split(command) {
        Some(programs) => (programs, true),
        None => (rough_programs(command), false),
    }
}

/// Where a line's programs may run from, as bash keeps it: `here`, and in a
/// line read roughly every place any `cd` in it reaches, since its order
/// isn't sure (a function, a loop). None when that can't be told: a link
/// made in the line could take a path anywhere, and `CDPATH` or
/// `cdable_vars` send a `cd` where its words don't say.
fn starting_places(
    command: &str,
    programs: &[Program],
    here: &Path,
    whole: bool,
) -> Option<Vec<PathBuf>> {
    let unseen = ["CDPATH", "cdable_vars", "shopt"]
        .iter()
        .any(|word| command.contains(word))
        || programs.iter().any(|program| name_of(program) == "ln");
    let places = (!unseen).then(|| vec![here.to_path_buf()]);
    if whole {
        places
    } else {
        programs
            .iter()
            .fold(places, |places, program| moved(program, places))
    }
}

/// An output that may not go where it goes: into a protected place, or
/// emptying a file outside the folders.
fn output_is_hard(output: &Output, places: Option<&[PathBuf]>, folders: &[PathBuf]) -> bool {
    protected_at(&output.file, places)
        || (output.empties && !lands_in(&output.file, places, folders))
}

fn program_is_hard(
    whole_program: &Program,
    places: Option<&[PathBuf]>,
    folders: &[PathBuf],
    whole: bool,
) -> bool {
    let Some((program, args)) = command_of(&whole_program.words).split_first() else {
        return false;
    };
    // Read roughly, a program named by a variable or a substitution can be
    // anything: `$X -rf ~`, `$(echo rm) -rf ~` leaves `-rf` in its place.
    if !whole && (program.contains(['$', '`']) || program.starts_with('-')) {
        return true;
    }
    let name = base_name(program);
    let lower = name.to_ascii_lowercase();
    if glob_may_write(name, whole_program)
        || [PAST_THE_PERSON, SENDERS, MACHINE]
            .iter()
            .any(|list| list.contains(&name))
        || name.starts_with("mkfs")
        || WINDOWS_PAST_THE_PERSON.contains(&lower.as_str())
        || writes_where_it_may_not(name, args, places, folders)
    {
        return true;
    }
    let words = words_of(args);
    // Under WSL, a Windows program is out of every Linux rule's reach.
    if (lower.ends_with(".exe") || name == "pwsh")
        && (names_the_hard_core(&words)
            || words
                .iter()
                .any(|word| WINDOWS_HARD.contains(&word.to_lowercase().as_str())))
    {
        return true;
    }
    // `command -v x` only looks a name up.
    let looks_up =
        name == "command" && matches!(args.first().map(String::as_str), Some("-v" | "-V"));
    if PASSING_ON.contains(&name) && !looks_up && names_the_hard_core(&words) {
        return true;
    }
    match name {
        "curl" => sends_with_curl(args),
        "wget" => uses_any(args, WGET_SENDING),
        "rsync" => {
            uses_any(
                args,
                &["--delete", "--remove-source-files", "--remove-sent-files"],
            ) || args
                .iter()
                .any(|arg| !arg.starts_with('-') && is_remote(arg))
        }
        "find" => find_is_hard(args, places, folders),
        "openssl" => uses_any(args, &["s_client", "s_server"]),
        "gh" => !gh_only_reads(args),
        "git" => git_is_hard(args, places),
        "docker" | "podman" => uses_any(args, DOCKER_HARD),
        name if PUBLISHERS.contains(&name) => uses_any(args, PUBLISHING),
        _ => false,
    }
}

/// A program that deletes, moves or writes files, where it may not: a
/// delete or a move of anything not placed in the folders, a write into a
/// protected place, or one over a file that exists outside the folders. A
/// path made of a variable is never placed for a delete or a move, and
/// isn't checked as an overwrite: the judge reads it.
fn writes_where_it_may_not(
    name: &str,
    args: &[String],
    places: Option<&[PathBuf]>,
    folders: &[PathBuf],
) -> bool {
    let removes = DELETERS.contains(&name);
    if removes && (places.is_none() || !paths(args).all(|path| lands_in(path, places, folders))) {
        return true;
    }
    let (written, overwrites): (Vec<String>, bool) = match name {
        "mv" | "cp" | "install" => (destinations(args), name != "mv"),
        "ln" => (destinations(args), uses_any(args, &["-f", "--force"])),
        "tee" => (
            paths(args).map(str::to_string).collect(),
            !uses_any(args, &["-a", "--append"]),
        ),
        "sed" if uses_any(args, &["-i", "--in-place"]) => {
            (paths(args).skip(1).map(str::to_string).collect(), true)
        }
        "curl" => (values_of(args, &["-o", "--output"]), true),
        "wget" => (values_of(args, &["-O", "--output-document"]), true),
        "rsync" => (
            paths(args).last().map(str::to_string).into_iter().collect(),
            true,
        ),
        "touch" | "mkdir" | "chmod" | "chown" | "truncate" => {
            (paths(args).map(str::to_string).collect(), false)
        }
        _ => return false,
    };
    written.iter().any(|path| {
        protected_at(path, places) || (overwrites && overwrites_outside(path, places, folders))
    })
}

/// Where cp, mv, ln or install write: the folder `-t` names, or the last
/// path, and each source's name inside it, in case it's a folder.
fn destinations(args: &[String]) -> Vec<String> {
    let mut paths: Vec<&str> = paths(args).collect();
    let target = target_folder(args);
    let to = match target {
        Some(folder) => {
            paths.retain(|path| *path != folder);
            folder
        }
        None => match paths.pop() {
            Some(last) => last,
            None => return Vec::new(),
        },
    };
    let mut written = vec![to.to_string()];
    written.extend(paths.iter().filter_map(|source| {
        Path::new(source)
            .file_name()
            .map(|name| Path::new(to).join(name).display().to_string())
    }));
    written
}

/// The folder given with `-t`, `-tDIR`, `--target-directory=DIR` or
/// `--target-directory DIR`.
fn target_folder(args: &[String]) -> Option<&str> {
    let mut words = args.iter();
    while let Some(arg) = words.next() {
        if let Some(folder) = arg.strip_prefix("--target-directory=") {
            return Some(folder);
        }
        if arg == "--target-directory" || arg == "-t" {
            return words.next().map(String::as_str);
        }
        if let Some(letters) = arg.strip_prefix('-')
            && !letters.starts_with('-')
            && let Some(at) = letters.find('t')
            && at + 1 < letters.len()
        {
            return Some(&letters[at + 1..]);
        }
    }
    None
}

/// The values given after any of `options`, as `-o file` or `--output=file`.
fn values_of(args: &[String], options: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    let mut words = args.iter();
    while let Some(arg) = words.next() {
        for option in options {
            if arg == option {
                values.extend(words.next().cloned());
            } else if let Some(value) = arg.strip_prefix(&format!("{option}=")) {
                values.push(value.to_string());
            }
        }
    }
    values
}

/// Whether `path`, written out in full, is a file that exists outside the
/// folders, from any of `places`.
fn overwrites_outside(path: &str, places: Option<&[PathBuf]>, folders: &[PathBuf]) -> bool {
    if path.contains(['$', '`', '*', '?', '[']) {
        return false;
    }
    let Some(path) = expanded(path) else {
        return true;
    };
    let candidates: Vec<PathBuf> = if path.is_absolute() {
        vec![config::resolved(&path)]
    } else {
        places
            .unwrap_or_default()
            .iter()
            .map(|place| config::resolved(&place.join(&path)))
            .collect()
    };
    candidates.iter().any(|full| {
        full.symlink_metadata().is_ok_and(|meta| !meta.is_dir())
            && !folders
                .iter()
                .any(|folder| full.starts_with(folder) && full != folder)
    })
}

/// Whether `path`, from any of `places`, is a protected place
/// (`protected::is_protected`). One made of a variable is checked by its
/// written name only (`names_a_place_that_runs_later`).
fn protected_at(path: &str, places: Option<&[PathBuf]>) -> bool {
    if path.contains(['$', '`']) {
        return false;
    }
    let Some(path) = expanded(path) else {
        return false;
    };
    if path.is_absolute() {
        return is_protected(&path);
    }
    places
        .unwrap_or_default()
        .iter()
        .any(|place| is_protected(&place.join(&path)))
}

/// find that deletes, runs a program that isn't read-only on what it finds
/// outside the folders, or writes a list where it may not.
fn find_is_hard(args: &[String], places: Option<&[PathBuf]>, folders: &[PathBuf]) -> bool {
    if uses_any(args, &["-delete"]) {
        return true;
    }
    let starts: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .take_while(|arg| !arg.starts_with('-') && !matches!(*arg, "(" | "!"))
        .collect();
    let starts = if starts.is_empty() { vec!["."] } else { starts };
    let lists = values_of(args, &["-fprint", "-fprint0", "-fprintf", "-fls"]);
    (!runs_only_reads(args)
        && !starts
            .iter()
            .all(|start| lands_within(start, places, folders)))
        || lists
            .iter()
            .any(|list| !lands_in(list, places, folders) || protected_at(list, places))
}

/// Whether every program find runs (`-exec … ;`, `-ok … +`) is read-only:
/// `find ~ -exec ls -l {} ;` lists, `-exec cp /dev/null {} ;` doesn't.
fn runs_only_reads(args: &[String]) -> bool {
    let mut words = args.iter();
    while let Some(arg) = words.next() {
        if matches!(arg.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
            let run: Vec<&str> = words
                .by_ref()
                .map(String::as_str)
                .take_while(|word| !matches!(*word, ";" | "+"))
                .collect();
            if !super::is_read_only(&run.join(" ")) {
                return false;
            }
        }
    }
    true
}

/// curl that sends: data, a form, a file, a method, cookies or credentials,
/// or an argument read from a file (`@secrets`). `--user-agent` only names
/// the client.
fn sends_with_curl(args: &[String]) -> bool {
    let options: Vec<String> = args
        .iter()
        .filter(|arg| !matches!(arg.split('=').next(), Some("--user-agent")))
        .cloned()
        .collect();
    uses_any(&options, CURL_SENDING) || args.iter().any(|arg| arg.starts_with('@'))
}

/// rsync's `host:path`, `host::module` or `rsync://`.
fn is_remote(arg: &str) -> bool {
    arg.contains("::")
        || arg.contains("://")
        || arg
            .split('/')
            .next()
            .is_some_and(|first| first.contains(':'))
}

/// gh that only reads, or fetches to this machine. `gh auth` hands out the
/// person's token, and anything else acts as them online.
fn gh_only_reads(args: &[String]) -> bool {
    let words: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|word| !word.starts_with('-'))
        .collect();
    match words.as_slice() {
        [] => true,
        ["api", ..] => !uses_any(args, GH_API_SENDING),
        ["auth", "status", ..] => !uses_any(args, &["-t", "--show-token"]),
        ["auth", ..] => false,
        [first, ..] if GH_READING_FIRST.contains(first) => true,
        [_, second, ..] => GH_READING.contains(second),
        [_] => false,
    }
}

/// git that throws work away, rewrites what can't be had back, sends it
/// somewhere new, runs what a repository it fetches names, or names a
/// program for later git commands to run.
fn git_is_hard(args: &[String], places: Option<&[PathBuf]>) -> bool {
    // An option that brings settings of its own (`-c core.pager=…`).
    let Some((_, args)) = git_command(args) else {
        return true;
    };
    let Some((sub, rest)) = args.split_first() else {
        return false;
    };
    let said = |word: &str| rest.iter().any(|arg| arg == word);
    // A variable could hold a forced refspec, a path or a setting.
    if GIT_HARD_WORDS.contains(&sub.as_str()) && rest.iter().any(|arg| arg.contains(['$', '`'])) {
        return true;
    }
    match sub.as_str() {
        "push" => {
            let mut words = rest.iter().filter(|arg| !arg.starts_with('-'));
            // A remote that is a path or an address, not one set up already.
            let somewhere_new = words
                .next()
                .is_some_and(|remote| remote.contains([':', '/', '@']) || remote.starts_with('.'));
            somewhere_new
                || uses_any(
                    rest,
                    &["-f", "--force", "-d", "--delete", "--mirror", "--prune"],
                )
                || words.any(|refspec| refspec.starts_with('+') || refspec.starts_with(':'))
        }
        "remote" => uses_any(rest, &["add", "set-url", "rename"]),
        "reset" => uses_any(rest, &["--hard", "--merge", "--keep"]),
        "clean" => !uses_any(rest, &["-n", "--dry-run"]),
        "restore" => {
            !(uses_any(rest, &["-S", "--staged"]) && !uses_any(rest, &["-W", "--worktree"]))
        }
        "checkout" => {
            uses_any(
                rest,
                &["-f", "--force", "-p", "--patch", "--ours", "--theirs"],
            ) || said("--")
                || said(".")
                || names_a_path(rest, places)
        }
        "switch" => uses_any(
            rest,
            &["-f", "--force", "--discard-changes", "-C", "--force-create"],
        ),
        "stash" => uses_any(rest, &["drop", "clear"]),
        "branch" => uses_any(rest, &["-D", "-f", "--force", "-M", "-C"]),
        "tag" => uses_any(rest, &["-d", "--delete", "-f", "--force"]),
        "rm" => uses_any(rest, &["-f", "--force"]),
        "worktree" => uses_any(rest, &["remove"]) && uses_any(rest, &["-f", "--force"]),
        "reflog" => uses_any(rest, &["expire", "delete"]),
        "gc" => uses_any(rest, &["--prune"]),
        "checkout-index" => uses_any(rest, &["-f", "--force"]),
        "read-tree" => uses_any(rest, &["-u", "--reset"]),
        // A fetched repository's hooks run with these.
        "clone" => uses_any(rest, &["-c", "--config", "--template"]),
        "filter-branch" | "filter-repo" | "update-ref" | "prune" | "replace" | "send-email" => true,
        "config" => config_names_a_program(rest),
        _ => false,
    }
}

/// `git checkout <file>` throws away that file's changes, as `--` does: a
/// word that names something in the folder may be one.
fn names_a_path(rest: &[String], places: Option<&[PathBuf]>) -> bool {
    let Some(places) = places else {
        return true;
    };
    rest.iter()
        .filter(|arg| !arg.starts_with('-'))
        .any(|arg| places.iter().any(|place| place.join(arg).exists()))
}

/// `git config` that sets anything but a plain setting (`gitconfig::is_plain`)
/// or one that sends pushes and fetches elsewhere, edits the file whole, or
/// writes another file.
fn config_names_a_program(rest: &[String]) -> bool {
    if uses_any(rest, &["-e", "--edit", "-f", "--file", "--blob"]) {
        return true;
    }
    let reading = [
        "--get",
        "--get-all",
        "--get-regexp",
        "--get-urlmatch",
        "-l",
        "--list",
        "--unset",
        "--unset-all",
        "--remove-section",
    ];
    if uses_any(rest, &reading) {
        return false;
    }
    let words: Vec<&str> = rest
        .iter()
        .map(String::as_str)
        .filter(|word| !word.starts_with('-'))
        .collect();
    // Since git 2.46 a word says what to do first.
    let words = match words.first() {
        Some(&"get" | &"list" | &"unset" | &"remove-section") => return false,
        Some(&"edit" | &"rename-section") => return true,
        Some(&"set") => &words[1..],
        _ => &words[..],
    };
    match words {
        [] => false,
        [name, value @ ..] => {
            let name = listed(name);
            let redirects = (name.starts_with("remote.")
                && [
                    ".url",
                    ".pushurl",
                    ".push",
                    ".mirror",
                    ".receivepack",
                    ".uploadpack",
                    ".proxy",
                ]
                .iter()
                .any(|key| name.ends_with(key)))
                || (name.starts_with("url.")
                    && (name.ends_with(".insteadof") || name.ends_with(".pushinsteadof")));
            redirects || !gitconfig::is_plain(&name, value.first().unwrap_or(&""))
        }
    }
}

/// A setting's name as git lists it: section and key in lowercase, a
/// subsection between them as written.
fn listed(name: &str) -> String {
    match (name.split_once('.'), name.rsplit_once('.')) {
        (Some((section, _)), Some((head, key))) if head.len() > section.len() => format!(
            "{}.{}.{}",
            section.to_lowercase(),
            &head[section.len() + 1..],
            key.to_lowercase()
        ),
        _ => name.to_lowercase(),
    }
}

/// Whether git is among the programs in a line, behind a wrapper (`env`,
/// `timeout`) included (`allowlist::uses_git`).
pub(super) fn names_git(command: &str) -> bool {
    let (programs, _) = read(command);
    programs.iter().any(|program| {
        let words = command_of(&program.words);
        words.first().is_some_and(|name| base_name(name) == "git") || wraps_git(words)
    })
}

/// A wrapper with git among its words: git's own options and folder are
/// then out of reach.
fn wraps_git(words: &[String]) -> bool {
    words.split_first().is_some_and(|(name, args)| {
        WRAPPERS.contains(&base_name(name)) && args.iter().any(|arg| base_name(arg) == "git")
    })
}

/// The folders whose settings apply to the git commands in `command`, run
/// from `here` (`allowlist::git_folders`).
pub(super) fn git_folders(command: &str, here: &Path) -> Option<Vec<PathBuf>> {
    let (programs, whole) = read(command);
    let mut places = starting_places(command, &programs, here, whole);
    let mut folders = Vec::new();
    for program in &programs {
        let words = command_of(&program.words);
        if wraps_git(words) {
            return None;
        }
        if let Some((name, args)) = words.split_first()
            && base_name(name) == "git"
        {
            let (given, _) = git_command(args)?;
            for place in places.as_deref()? {
                let folder = given.iter().try_fold(place.clone(), |folder, given| {
                    Some(logical(&folder.join(expanded(given)?)))
                })?;
                let folder = config::resolved(&folder);
                if !folders.contains(&folder) {
                    folders.push(folder);
                }
            }
        }
        if whole {
            places = moved(program, places.take());
        }
    }
    Some(folders)
}

/// Where the line is after `program`, as bash's `cd` takes it: logically,
/// `..` dropping the last name written rather than going up from where a
/// link led. A `cd` or `pushd` adds where it goes, and one Lokal can't
/// follow (`cd -`, `cd -P`, `popd`, a glob, a variable, a name `CDPATH`
/// may find, or a `cd` behind `builtin`, `command` or `eval`) leaves it
/// unknown.
fn moved(program: &Program, places: Option<Vec<PathBuf>>) -> Option<Vec<PathBuf>> {
    let Some((name, args)) = command_of(&program.words).split_first() else {
        return places;
    };
    let name = base_name(name);
    let hidden_cd = args
        .iter()
        .any(|arg| matches!(base_name(arg), "cd" | "pushd" | "popd"));
    if matches!(name, "eval" | "source" | ".") || (WRAPPERS.contains(&name) && hidden_cd) {
        return None;
    }
    match name {
        "cd" | "pushd" => {
            let mut places = places?;
            if args
                .iter()
                .any(|arg| arg.starts_with('-') && arg != "-" && arg != "-L")
            {
                return None;
            }
            let targets: Vec<&String> = args.iter().filter(|arg| *arg != "-L").collect();
            let to = match targets.as_slice() {
                [] => config::home(),
                [target]
                    if *target != "-"
                        && !target.starts_with('+')
                        && !target.contains(['*', '?', '[', '$', '`']) =>
                {
                    let relative = !target.starts_with(['/', '.', '~']);
                    if relative && env::var_os("CDPATH").is_some_and(|path| !path.is_empty()) {
                        return None;
                    }
                    expanded(target)?
                }
                _ => return None,
            };
            let reached: Vec<PathBuf> = places
                .iter()
                .map(|place| logical(&place.join(&to)))
                .collect();
            places.extend(reached);
            places.sort();
            places.dedup();
            Some(places)
        }
        "popd" => None,
        _ => places,
    }
}

/// A path with `.` and `..` taken by their names, as bash's `cd` does,
/// links left as they are.
fn logical(path: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for part in path.components() {
        match part {
            Component::RootDir | Component::Prefix(_) => out = PathBuf::from("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => out.push(name),
        }
    }
    out
}

/// Whether `path`, written in a command run from each of `places`, lands in
/// one of `folders`: below it, or among its own entries through a glob
/// (`*`, `build/*.o`). Never the folder itself, never past a glob with `..`
/// after it, and never with no place known.
fn lands_in(path: &str, places: Option<&[PathBuf]>, folders: &[PathBuf]) -> bool {
    lands(path, places, folders, false)
}

/// As `lands_in`, the folders themselves included: where find may start.
fn lands_within(path: &str, places: Option<&[PathBuf]>, folders: &[PathBuf]) -> bool {
    lands(path, places, folders, true)
}

fn lands(path: &str, places: Option<&[PathBuf]>, folders: &[PathBuf], itself: bool) -> bool {
    // A variable, or another program's output, could be anywhere.
    if path.contains(['$', '`']) {
        return false;
    }
    let (Some(places), Some(path)) = (places, expanded(path)) else {
        return false;
    };
    let parts: Vec<_> = path.components().collect();
    let glob = parts
        .iter()
        .position(|part| part.as_os_str().to_string_lossy().contains(['*', '?', '[']));
    if let Some(at) = glob {
        // `.*` can be `..`, and `*/../..` climbs out.
        if parts[at].as_os_str().to_string_lossy().starts_with('.')
            || parts[at..].contains(&Component::ParentDir)
        {
            return false;
        }
    }
    let literal: PathBuf = parts[..glob.unwrap_or(parts.len())].iter().collect();
    places.iter().all(|place| {
        let full = config::resolved(&place.join(&literal));
        folders
            .iter()
            .any(|folder| full.starts_with(folder) && (itself || glob.is_some() || full != *folder))
    })
}

/// A path as bash would have it: `~` and `~/` are the home folder; another
/// person's `~name` isn't followed.
fn expanded(word: &str) -> Option<PathBuf> {
    if word == "~" {
        Some(config::home())
    } else if let Some(rest) = word.strip_prefix("~/") {
        Some(config::home().join(rest))
    } else if word.starts_with('~') {
        None
    } else {
        Some(PathBuf::from(word))
    }
}

/// The paths a delete or a move is given: its words that aren't options,
/// every word after `--`, an option's value after `=`, and a folder
/// written right after `-t` (`-t/home/me`).
fn paths(args: &[String]) -> impl Iterator<Item = &str> {
    let mut options_over = false;
    args.iter().filter_map(move |arg| {
        if options_over || !arg.starts_with('-') || arg == "-" {
            Some(arg.as_str())
        } else if arg == "--" {
            options_over = true;
            None
        } else if let Some((_, value)) = arg.split_once('=') {
            Some(value)
        } else if !arg.starts_with("--") {
            arg.find('t')
                .filter(|at| at + 1 < arg.len())
                .map(|at| &arg[at + 1..])
        } else {
            None
        }
    })
}

/// Whether a line names a place where programs run later, a PATH folder in
/// the home folder included, however it's spelled: in any case, with quotes
/// or backslashes inside, or as a glob in a hidden name (`~/.b?shrc`).
fn names_a_place_that_runs_later(command: &str) -> bool {
    let plain: String = command
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
        .collect::<String>()
        .to_lowercase();
    // The hidden names of the list, each a glob in a hidden name could be.
    let hidden: Vec<&str> = RUN_LATER
        .iter()
        .flat_map(|place| place.split('/'))
        .filter(|part| part.starts_with('.') && part.len() > 1)
        .collect();
    let hidden_glob = plain
        .split(|c: char| c.is_whitespace() || ";|&()<>=".contains(c))
        .flat_map(|word| word.split('/'))
        .filter(|part| part.starts_with('.') && part.contains(['*', '?', '[']))
        .any(|glob| hidden.iter().any(|name| glob_matches(glob, name)));
    RUN_LATER.iter().any(|place| plain.contains(place))
        || path_folders_at_home()
            .iter()
            .any(|folder| plain.contains(&folder.to_lowercase()))
        || hidden_glob
}

/// Whether `glob` could be `name`: `*` as any run of characters, `?` and
/// a bracket's set as any one. Near enough to never miss a match.
fn glob_matches(glob: &str, name: &str) -> bool {
    let glob: Vec<char> = glob.chars().collect();
    let name: Vec<char> = name.chars().collect();
    fn from(glob: &[char], name: &[char]) -> bool {
        match glob.split_first() {
            None => name.is_empty(),
            Some(('*', rest)) => (0..=name.len()).any(|at| from(rest, &name[at..])),
            Some(('[', rest)) => {
                let after = rest
                    .iter()
                    .position(|c| *c == ']')
                    .map_or(rest, |end| &rest[end + 1..]);
                !name.is_empty() && from(after, &name[1..])
            }
            Some(('?', rest)) => !name.is_empty() && from(rest, &name[1..]),
            Some((c, rest)) => name.first() == Some(c) && from(rest, &name[1..]),
        }
    }
    from(&glob, &name)
}

/// Whether a line sets one of git's own variables, which bring settings,
/// a repository or a program of their own (`GIT_DIR=`, `GIT_CONFIG_COUNT=`,
/// `GIT_EXTERNAL_DIFF=`).
fn sets_git_from_the_environment(command: &str) -> bool {
    command.match_indices("GIT_").any(|(at, _)| {
        let rest = &command[at + 4..];
        let name_end = rest
            .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .unwrap_or(rest.len());
        name_end > 0 && rest[name_end..].starts_with('=')
    })
}

/// The PATH's folders under the home folder, as a line may write them.
fn path_folders_at_home() -> Vec<String> {
    let home = config::home();
    let Some(path) = env::var_os("PATH") else {
        return Vec::new();
    };
    env::split_paths(&path)
        .filter_map(|folder| {
            let inside = folder.strip_prefix(&home).ok()?;
            (!inside.as_os_str().is_empty()).then(|| inside.display().to_string())
        })
        .flat_map(|inside| {
            [
                format!("{}/{inside}", home.display()),
                format!("~/{inside}"),
                format!("$home/{inside}"),
            ]
        })
        .collect()
}

/// Whether any of the hard core's names are among `words`: what a program
/// passes on to another (a shell's `-c`, xargs, find's `-exec`).
fn names_the_hard_core(words: &[String]) -> bool {
    let names: Vec<&str> = words.iter().map(|word| base_name(word)).collect();
    let named = |list: &[&str]| names.iter().any(|name| list.contains(name));
    let said = |list: &[&str]| words.iter().any(|word| list.contains(&word.as_str()));
    let options: Vec<String> = words
        .iter()
        .filter(|word| word.starts_with('-') && !word.starts_with("--user-agent"))
        .cloned()
        .collect();
    // What follows a program's name, as its arguments.
    let after = |name: &str| {
        names
            .iter()
            .position(|named| *named == name)
            .map(|at| words[at + 1..].to_vec())
    };
    [PAST_THE_PERSON, SENDERS, MACHINE, DELETERS]
        .iter()
        .any(|list| named(list))
        || after("gh").is_some_and(|args| !gh_only_reads(&args))
        || after("rsync").is_some_and(|args| {
            uses_any(
                &args,
                &["--delete", "--remove-source-files", "--remove-sent-files"],
            ) || args
                .iter()
                .any(|arg| !arg.starts_with('-') && is_remote(arg))
        })
        || names.iter().any(|name| {
            let lower = name.to_lowercase();
            name.starts_with("mkfs") || WINDOWS_PAST_THE_PERSON.contains(&lower.as_str())
        })
        || (named(&["git"]) && said(GIT_HARD_WORDS))
        || (named(&["curl", "wget"])
            && (uses_any(&options, CURL_SENDING)
                || uses_any(&options, WGET_SENDING)
                || words.iter().any(|word| word.starts_with('@'))))
        || (named(&["docker", "podman"]) && uses_any(&options, DOCKER_HARD))
        || (named(&["docker", "podman"]) && said(&["prune", "push"]))
        || said(&["publish", "unpublish", "send-email"])
        || (named(&["find"]) && said(&["-delete"]))
}

/// A line Lokal can't read whole, cut into the programs it may hold: at
/// every mark that could start one (`;`, `|`, `&`, a parenthesis, a
/// backtick, a line break). A brace's words stay with the program they're
/// in (`find . {-delete,}`), quotes and backslashes go as bash would take
/// them (`r''m` is rm), and `$` stays, so a path made of a variable is never
/// placed and a program named by one is never trusted.
fn rough_programs(text: &str) -> Vec<Program> {
    text.split(|c: char| ";|&()`\n\r".contains(c))
        .filter_map(|part| {
            let words: Vec<String> = part
                .split(|c: char| c.is_whitespace() || ",{}".contains(c))
                .map(|word| {
                    word.chars()
                        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
                        .collect::<String>()
                })
                .filter(|word| !word.is_empty())
                .collect();
            (!words.is_empty()).then(|| Program {
                globbed: unquoted_glob(part),
                words,
                outputs: Vec::new(),
            })
        })
        .collect()
}

/// Whether `text` has a `*`, `?` or `[` outside quotes, which bash would
/// turn into file names.
fn unquoted_glob(text: &str) -> bool {
    let mut quote = None;
    text.chars().any(|c| match (quote, c) {
        (None, '\'' | '"') => {
            quote = Some(c);
            false
        }
        (Some(open), c) if c == open => {
            quote = None;
            false
        }
        (None, '*' | '?' | '[') => true,
        _ => false,
    })
}

/// The outputs in a line read roughly: each `>` and `>>`, with its file as
/// bash would take it, quotes and backslashes gone. `>&` merges, and `>&3`
/// or `&>` count as files.
fn rough_outputs(text: &str) -> Vec<Output> {
    let chars: Vec<char> = text.chars().collect();
    let mut outputs = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] != '>' {
            at += 1;
            continue;
        }
        let adds = chars.get(at + 1) == Some(&'>');
        let start = at + if adds { 2 } else { 1 };
        let file: String = chars[start..]
            .iter()
            .skip_while(|c| **c == ' ' || **c == '\t' || **c == '|')
            .take_while(|c| !c.is_whitespace() && !";|&()".contains(**c))
            .filter(|c| !matches!(c, '\'' | '"' | '\\'))
            .collect();
        let merges = chars.get(start) == Some(&'&');
        if !merges && file != "/dev/null" {
            outputs.push(Output {
                file: if file.is_empty() { "$".into() } else { file },
                empties: !adds,
            });
        }
        at = start;
    }
    outputs
}

/// Whether a line runs what it downloads: piped into a shell or an
/// interpreter that reads its code from there (`| sh -x`, `| env bash`,
/// `| python3 -`, but not `| python3 -c '…'`, which reads it as data), or
/// handed over through a substitution (`sh -c "$( curl …)"`, `bash <(curl …)`).
fn runs_a_download(text: &str) -> bool {
    let plain: String = text
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
        .collect();
    let piped = plain.split('|').skip(1).any(|after| {
        // `|&` pipes stderr along.
        let after = after.strip_prefix('&').unwrap_or(after);
        let program = after.split([';', '&', '\n', ')']).next().unwrap_or("");
        let mut words = program
            .split(|c: char| c.is_whitespace() || c == '(')
            .filter(|word| !word.is_empty())
            .skip_while(|word| WRAPPERS.contains(&base_name(word)) || word.contains('='))
            .peekable();
        let Some(first) = words.next() else {
            return false;
        };
        let runner = base_name(first);
        if !RUNNERS.contains(&runner) {
            return false;
        }
        let args: Vec<&str> = words.collect();
        let inline = args.iter().any(|arg| inline_flags(runner).contains(arg));
        let script = args.iter().find(|arg| !arg.starts_with('-'));
        !inline
            && script.is_none_or(|script| {
                matches!(*script, "-" | "/dev/stdin") || script.starts_with("/dev/fd/")
            })
    });
    let handed = ["$(", "<(", "`"].iter().any(|start| {
        plain.match_indices(start).any(|(at, _)| {
            plain[at + start.len()..]
                .split(|c: char| c.is_whitespace() || ")`|;".contains(c))
                .find(|word| !word.is_empty())
                .is_some_and(|word| matches!(base_name(word), "curl" | "wget"))
        })
    });
    piped || handed
}

/// The options with which `runner` takes its code from its arguments, so
/// what's piped in is data: bash's `-e` only stops on an error.
fn inline_flags(runner: &str) -> &'static [&'static str] {
    match runner {
        "python" | "python3" => &["-c", "-m"],
        "perl" | "ruby" => &["-e", "-E"],
        "node" | "bun" => &["-e", "--eval", "-p", "--print"],
        "php" => &["-r"],
        "lua" => &["-e"],
        "deno" => &["eval"],
        "pwsh" => &["-c", "-Command", "-command"],
        _ => &["-c"],
    }
}

/// The words of a program's arguments as bash would hand them over, split
/// wherever it could end one: spaces, the marks that join programs or
/// start a substitution, braces and their commas. Quotes and backslashes go
/// (`r''m` is rm).
fn words_of(args: &[String]) -> Vec<String> {
    args.iter()
        .flat_map(|arg| {
            arg.split(|c: char| c.is_whitespace() || ";|&(){}<>$`,".contains(c))
                .map(|word| {
                    word.chars()
                        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
                        .collect::<String>()
                })
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A program's name without the folder it was named in: `/bin/rm` is rm.
pub(super) fn base_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// The program's name, after what only leads into it.
fn name_of(program: &Program) -> &str {
    command_of(&program.words)
        .first()
        .map_or("", |word| base_name(word))
}

/// A program's own words: without the `NAME=value` ones that set its
/// environment, and the shell's words that only lead into it (`if`,
/// `then`, `do`, `!`, `time`). A loop's list, a case's word and the words
/// that close a block hold no program, so they give none.
fn command_of(words: &[String]) -> &[String] {
    let mut at = 0;
    while let Some(word) = words.get(at) {
        match word.as_str() {
            "do" | "then" | "else" | "elif" | "if" | "while" | "until" | "!" | "time" | "{" => {
                at += 1;
            }
            "for" | "select" | "case" | "in" | "done" | "fi" | "esac" | "function" | "}" => {
                return &[];
            }
            word if is_assignment(word) => at += 1,
            _ => break,
        }
    }
    &words[at..]
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}
