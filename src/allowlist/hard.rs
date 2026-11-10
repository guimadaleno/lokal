//! The hard core: what a command's question opens on No for, and what
//! auto's judge never decides. These are the steps that can't be undone,
//! that leave the machine, or that change what runs later; everything else
//! opens on Yes. A line Lokal can't read whole counts as the hard core when
//! any of the names here are in it, since where its words go can't be told.

use std::env;
use std::path::{Path, PathBuf};

use super::{Program, glob_may_write, split, uses_any};
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
/// Windows' own deletes, for a Windows program under WSL, in lowercase.
const WINDOWS_DELETES: &[&str] = &[
    "del",
    "erase",
    "rd",
    "rmdir",
    "remove-item",
    "format",
    "format-volume",
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
];

/// Whether `command`, run from the folder `here`, is the hard core. A
/// delete, a move or a `>` that empties a file is let through only when
/// what it touches lands in one of `folders`.
pub(super) fn is_hard(command: &str, here: &Path, folders: &[PathBuf]) -> bool {
    if names_a_place_that_runs_later(command) {
        return true;
    }
    let (programs, whole) = match split(command) {
        Some(programs) => (programs, true),
        None => (rough_programs(command), false),
    };
    let named = |list: &[&str]| {
        programs
            .iter()
            .any(|program| list.contains(&name_of(program)))
    };
    if named(&["curl", "wget"]) && runs_a_download(command) {
        return true;
    }
    // Where the line may be at each program: every place a `cd` before it
    // could have left it, since one that fails stays where it was. None
    // once that can't be told.
    let mut places = Some(vec![here.to_path_buf()]);
    // In a line read roughly, the order isn't sure (a function, a loop):
    // every program is taken as run from every place any `cd` reaches.
    if !whole {
        places = programs
            .iter()
            .fold(places, |places, program| moved(program, places));
        if !rough_outputs_land(command, places.as_deref(), folders) {
            return true;
        }
    }
    programs.iter().any(|program| {
        let empties_elsewhere = program
            .outputs
            .iter()
            .any(|output| output.empties && !lands_in(&output.file, places.as_deref(), folders));
        let hard = empties_elsewhere || program_is_hard(program, places.as_deref(), folders);
        if whole {
            places = moved(program, places.take());
        }
        hard
    })
}

fn program_is_hard(whole: &Program, places: Option<&[PathBuf]>, folders: &[PathBuf]) -> bool {
    let Some((program, args)) = without_assignments(&whole.words).split_first() else {
        return false;
    };
    let name = base_name(program);
    if glob_may_write(name, whole)
        || [PAST_THE_PERSON, SENDERS, MACHINE]
            .iter()
            .any(|list| list.contains(&name))
        || name.starts_with("mkfs")
    {
        return true;
    }
    if DELETERS.contains(&name) {
        return places.is_none() || !paths(args).all(|path| lands_in(path, places, folders));
    }
    // Under WSL, a Windows program is out of every Linux rule's reach.
    let windows = name.to_ascii_lowercase().ends_with(".exe") || name == "pwsh";
    // `command -v x` only looks a name up.
    let looks_up =
        name == "command" && matches!(args.first().map(String::as_str), Some("-v" | "-V"));
    if (windows || (PASSING_ON.contains(&name) && !looks_up))
        && names_the_hard_core(&args.iter().flat_map(|arg| words(arg)).collect::<Vec<_>>())
    {
        return true;
    }
    match name {
        "curl" => sends_with_curl(args),
        "wget" => uses_any(args, WGET_SENDING),
        "rsync" => {
            uses_any(args, &["--delete", "--remove-source-files"])
                || args
                    .iter()
                    .any(|arg| !arg.starts_with('-') && is_remote(arg))
        }
        "find" => uses_any(args, &["-delete"]),
        "gh" => !gh_only_reads(args),
        "git" => git_is_hard(args, places),
        "docker" | "podman" => uses_any(args, DOCKER_HARD),
        name if PUBLISHERS.contains(&name) => uses_any(args, PUBLISHING),
        _ => false,
    }
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
/// somewhere new, or names a program for later git commands to run.
fn git_is_hard(args: &[String], places: Option<&[PathBuf]>) -> bool {
    let Some((sub, rest)) = args.split_first() else {
        return false;
    };
    let said = |word: &str| rest.iter().any(|arg| arg == word);
    // A variable could hold a forced refspec, a path or a setting.
    if GIT_HARD_WORDS.contains(&sub.as_str()) && rest.iter().any(|arg| arg.contains(['$', '`'])) {
        return true;
    }
    match sub.as_str() {
        // Options before the command (-C, -c) change where it runs, or what.
        sub if sub.starts_with('-') => true,
        "push" => {
            uses_any(
                rest,
                &["-f", "--force", "-d", "--delete", "--mirror", "--prune"],
            ) || rest.iter().any(|arg| {
                // A forced or deleting refspec, or a remote that isn't one yet.
                arg.starts_with('+')
                    || arg.starts_with(':')
                    || arg.contains("://")
                    || (!arg.starts_with('-') && arg.contains('@'))
            })
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
        "filter-branch" | "filter-repo" | "update-ref" | "prune" | "replace" => true,
        "config" => config_names_a_program(rest),
        _ => false,
    }
}

/// `git checkout <file>` throws away that file's changes, as `--` does: a
/// word that names something in the folder may be one.
fn names_a_path(rest: &[String], places: Option<&[PathBuf]>) -> bool {
    let Some(place) = places.and_then(|places| places.first()) else {
        return true;
    };
    rest.iter()
        .filter(|arg| !arg.starts_with('-'))
        .any(|arg| place.join(arg).exists())
}

/// `git config` that sets anything but a plain setting (`gitconfig::is_plain`),
/// edits the file whole, or writes another file.
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
        [name, value @ ..] => !gitconfig::is_plain(&listed(name), value.first().unwrap_or(&"")),
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

/// Where the line is after `program`: a `cd` or `pushd` adds where it goes,
/// and one Lokal can't follow (`cd -`, `popd`, a glob, a variable) leaves
/// it unknown.
fn moved(program: &Program, places: Option<Vec<PathBuf>>) -> Option<Vec<PathBuf>> {
    let Some((name, args)) = without_assignments(&program.words).split_first() else {
        return places;
    };
    match base_name(name) {
        "cd" | "pushd" => {
            let mut places = places?;
            let targets: Vec<&String> = args
                .iter()
                .filter(|arg| !arg.starts_with('-') || *arg == "-")
                .collect();
            let to = match targets.as_slice() {
                [] => config::home(),
                [target] if *target != "-" && !target.contains(['*', '?', '[', '$', '`']) => {
                    expanded(target)?
                }
                _ => return None,
            };
            let reached: Vec<PathBuf> = places
                .iter()
                .map(|place| config::resolved(&place.join(&to)))
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

/// Whether `path`, written in a command run from each of `places`, lands in
/// one of `folders`: below it, or among its own entries through a glob
/// (`*`, `build/*.o`). Never the folder itself, and never with no place known.
fn lands_in(path: &str, places: Option<&[PathBuf]>, folders: &[PathBuf]) -> bool {
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
    let literal: PathBuf = parts[..glob.unwrap_or(parts.len())].iter().collect();
    // `.*` can be `..`.
    if glob.is_some_and(|at| parts[at].as_os_str().to_string_lossy().starts_with('.')) {
        return false;
    }
    places.iter().all(|place| {
        let full = config::resolved(&place.join(&literal));
        folders
            .iter()
            .any(|folder| full.starts_with(folder) && (glob.is_some() || full != *folder))
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
/// every word after `--`, and an option's value after `=`.
fn paths(args: &[String]) -> impl Iterator<Item = &str> {
    let mut options_over = false;
    args.iter().filter_map(move |arg| {
        if options_over || !arg.starts_with('-') || arg == "-" {
            Some(arg.as_str())
        } else if arg == "--" {
            options_over = true;
            None
        } else {
            arg.split_once('=').map(|(_, value)| value)
        }
    })
}

/// Whether a line names a place where programs run later, a PATH folder in
/// the home folder included, wherever in it and however it's used.
fn names_a_place_that_runs_later(command: &str) -> bool {
    RUN_LATER.iter().any(|place| command.contains(place))
        || path_folders_at_home()
            .iter()
            .any(|folder| command.contains(folder.as_str()))
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
                format!("$HOME/{inside}"),
            ]
        })
        .collect()
}

/// Whether any of the hard core's names are among `words`: a line Lokal
/// can't read whole, or what a program passes on to another.
fn names_the_hard_core(words: &[&str]) -> bool {
    let names: Vec<&str> = words.iter().map(|word| base_name(word)).collect();
    let named = |list: &[&str]| names.iter().any(|name| list.contains(name));
    let said = |list: &[&str]| words.iter().any(|word| list.contains(word));
    let options: Vec<String> = words
        .iter()
        .filter(|word| word.starts_with('-') && !word.starts_with("--user-agent"))
        .map(|word| word.to_string())
        .collect();
    // What follows a program's name, as its arguments.
    let after = |name: &str| {
        names.iter().position(|named| *named == name).map(|at| {
            words[at + 1..]
                .iter()
                .map(|word| word.to_string())
                .collect::<Vec<_>>()
        })
    };
    [PAST_THE_PERSON, SENDERS, MACHINE, DELETERS]
        .iter()
        .any(|list| named(list))
        || after("gh").is_some_and(|args| !gh_only_reads(&args))
        || after("rsync").is_some_and(|args| {
            uses_any(&args, &["--delete", "--remove-source-files"])
                || args
                    .iter()
                    .any(|arg| !arg.starts_with('-') && is_remote(arg))
        })
        || names.iter().any(|name| {
            name.starts_with("mkfs") || WINDOWS_DELETES.contains(&name.to_lowercase().as_str())
        })
        || (named(&["git"]) && said(GIT_HARD_WORDS))
        || (named(&["curl", "wget"])
            && (named(RUNNERS)
                || uses_any(&options, CURL_SENDING)
                || uses_any(&options, WGET_SENDING)
                || words.iter().any(|word| word.starts_with('@'))))
        || (named(&["docker", "podman"]) && uses_any(&options, DOCKER_HARD))
        || (named(&["docker", "podman"]) && said(&["prune", "push"]))
        || said(&["publish", "unpublish"])
        || (named(&["find"]) && said(&["-delete"]))
}

/// A line Lokal can't read whole, cut into the programs it may hold: at
/// every mark that could start one (`;`, `|`, `&`, a parenthesis, a
/// backtick, a line break). The words that only lead into a program
/// (`do`, `then`, `!`) go, and a loop's list or a case's word is left out.
/// Words keep their `$`, so a path made of a variable is never placed.
fn rough_programs(text: &str) -> Vec<Program> {
    text.split(|c: char| ";|&()`\n\r".contains(c))
        .filter_map(|part| {
            // A brace's words belong to the program it's in: `find . {-delete,}`.
            let mut words: Vec<String> = part
                .split(|c: char| c.is_whitespace() || "'\",{}".contains(c))
                .filter(|word| !word.is_empty())
                .map(str::to_string)
                .collect();
            loop {
                match words.first().map(String::as_str) {
                    Some(
                        "do" | "then" | "else" | "elif" | "if" | "while" | "until" | "!" | "time",
                    ) => {
                        words.remove(0);
                    }
                    Some(
                        "for" | "select" | "case" | "in" | "done" | "fi" | "esac" | "function",
                    ) => {
                        return None;
                    }
                    Some(_) => break,
                    None => return None,
                }
            }
            let globbed = unquoted_glob(part);
            Some(Program {
                words,
                globbed,
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

/// Whether a line runs what it downloads: piped into a shell or an
/// interpreter that reads its code from there (`| sh`, `| python3 -`, but
/// not `| python3 -c '…'`, which reads it as data), or handed over through a
/// substitution (`sh -c "$(curl …)"`, `bash <(curl …)`).
fn runs_a_download(text: &str) -> bool {
    let piped = text.split('|').skip(1).any(|after| {
        let program = after.split([';', '&', '\n']).next().unwrap_or("");
        let mut words = program.split_whitespace();
        words
            .next()
            .is_some_and(|first| RUNNERS.contains(&base_name(first)))
            && words.next().is_none_or(|next| next == "-" || next == "-s")
    });
    let handed = ["$(curl", "$(wget", "`curl", "`wget", "<(curl", "<(wget"]
        .iter()
        .any(|start| text.contains(start));
    piped || handed
}

/// Whether every `>` in a line read roughly empties a file in `folders`, or
/// discards its output. `>>` adds, and `>&` merges.
fn rough_outputs_land(text: &str, places: Option<&[PathBuf]>, folders: &[PathBuf]) -> bool {
    let chars: Vec<char> = text.chars().collect();
    chars.iter().enumerate().all(|(at, c)| {
        let before = at.checked_sub(1).map(|at| chars[at]);
        let after = chars.get(at + 1);
        if *c != '>' || before == Some('>') || matches!(after, Some('>' | '&')) {
            return true;
        }
        let file: String = chars[at + 1..]
            .iter()
            .skip_while(|c| **c == ' ' || **c == '\t')
            .take_while(|c| !c.is_whitespace() && !";|&()".contains(**c))
            .collect();
        file == "/dev/null" || (!file.is_empty() && lands_in(&file, places, folders))
    })
}

/// The words of a line, split wherever bash could end one: its spaces, its
/// quotes, the marks that join programs or start a substitution, braces
/// and their commas.
pub(super) fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || ";|&(){}<>$`'\"\\,".contains(c))
        .filter(|word| !word.is_empty())
}

/// A program's name without the folder it was named in: `/bin/rm` is rm.
pub(super) fn base_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// The program's name, after any `NAME=value` that sets its environment.
fn name_of(program: &Program) -> &str {
    without_assignments(&program.words)
        .first()
        .map_or("", |word| base_name(word))
}

/// The words after the `NAME=value` ones that start a program.
fn without_assignments(words: &[String]) -> &[String] {
    let assignments = words
        .iter()
        .take_while(|word| {
            word.split_once('=').is_some_and(|(name, _)| {
                name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        })
        .count();
    &words[assignments..]
}
