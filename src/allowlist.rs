//! Which commands run without asking. The rule belongs to Lokal, never to the
//! model: a command runs on its own only if every program in it is on a fixed
//! list of read-only ones and uses none of the options that would make it write
//! or run something else. Anything Lokal can't be sure of is asked about.

/// Read-only programs, each with the options that make it write or run other
/// programs. A one-letter option also counts inside a group, as in -ro.
const READ_ONLY: &[(&str, &[&str])] = &[
    ("basename", &[]),
    ("cat", &[]),
    ("cmp", &[]),
    ("column", &[]),
    ("cut", &[]),
    ("date", &["-s", "--set"]),
    ("df", &[]),
    ("diff", &[]),
    ("dirname", &[]),
    ("du", &[]),
    ("echo", &[]),
    ("file", &["-C", "--compile"]),
    (
        "find",
        &[
            "-delete", "-exec", "-execdir", "-ok", "-okdir", "-fls", "-fprint",
        ],
    ),
    ("free", &[]),
    ("grep", &[]),
    ("head", &[]),
    ("id", &[]),
    ("jq", &[]),
    ("ls", &[]),
    ("lsblk", &[]),
    ("lscpu", &[]),
    ("md5sum", &[]),
    ("nl", &[]),
    ("nproc", &[]),
    ("printenv", &[]),
    ("printf", &[]),
    ("ps", &[]),
    ("pwd", &[]),
    ("readlink", &[]),
    ("realpath", &[]),
    ("rg", &["--pre"]),
    ("sha1sum", &[]),
    ("sha256sum", &[]),
    ("sort", &["-o", "--output", "--compress-program"]),
    ("stat", &[]),
    ("tac", &[]),
    ("tail", &[]),
    ("tr", &[]),
    ("tree", &["-o", "-R"]),
    ("type", &[]),
    ("uname", &[]),
    ("uptime", &[]),
    ("wc", &[]),
    ("which", &[]),
    ("whoami", &[]),
    ("wslpath", &[]),
];

const GIT_READ_ONLY: &[&str] = &[
    "blame",
    "diff",
    "log",
    "ls-files",
    "rev-parse",
    "show",
    "status",
];
const GIT_BRANCH_LISTING: &[&str] = &[
    "-a",
    "--all",
    "-r",
    "--remotes",
    "-v",
    "-vv",
    "--list",
    "--show-current",
];
const DOCKER_READ_ONLY: &[&str] = &["images", "info", "inspect", "logs", "ps", "version"];

pub fn is_read_only(command: &str) -> bool {
    match split(command) {
        Some(programs) => programs.iter().all(|words| program_is_read_only(words)),
        None => false,
    }
}

fn program_is_read_only(words: &[String]) -> bool {
    let Some((program, args)) = words.split_first() else {
        return false;
    };
    match program.as_str() {
        "git" => git_is_read_only(args),
        "docker" => args
            .first()
            .is_some_and(|sub| DOCKER_READ_ONLY.contains(&sub.as_str())),
        // `uniq input output` writes to output.
        "uniq" => args.iter().filter(|arg| !arg.starts_with('-')).count() <= 1,
        _ => match READ_ONLY.iter().find(|(name, _)| name == program) {
            Some((_, writing)) => !args
                .iter()
                .any(|arg| writing.iter().any(|option| matches_option(arg, option))),
            None => false,
        },
    }
}

fn git_is_read_only(args: &[String]) -> bool {
    let Some((sub, rest)) = args.split_first() else {
        return false;
    };
    match sub.as_str() {
        "branch" => rest
            .iter()
            .all(|arg| GIT_BRANCH_LISTING.contains(&arg.as_str())),
        "remote" => rest.iter().all(|arg| arg == "-v"),
        sub => {
            GIT_READ_ONLY.contains(&sub) && !rest.iter().any(|arg| matches_option(arg, "--output"))
        }
    }
}

fn matches_option(arg: &str, option: &str) -> bool {
    if option.starts_with("--") {
        // GNU tools also accept any unambiguous start of a long option: --out for --output.
        let name = arg.split('=').next().unwrap_or(arg);
        name.starts_with(option)
            || (name.starts_with("--") && name.len() > 2 && option.starts_with(name))
    } else if option.len() == 2 {
        let letter = &option[1..];
        arg.starts_with('-') && !arg.starts_with("--") && arg[1..].contains(letter)
    } else {
        arg.starts_with(option)
    }
}

/// Splits a command line into the words of each program, the way bash would,
/// for programs joined by |, ||, && or ;. Returns None when the line uses
/// anything else the shell interprets (variables, substitution, redirection
/// other than to /dev/null, background jobs, subshells, several lines), since
/// those can hide writes or other programs.
fn split(command: &str) -> Option<Vec<Vec<String>>> {
    if command.contains(['\n', '\r']) {
        return None;
    }
    let mut programs = vec![Vec::new()];
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        other => word.push(other),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '$' | '`' => return None,
                        '\\' => match chars.next()? {
                            escaped @ ('"' | '\\' | '$' | '`') => word.push(escaped),
                            other => {
                                word.push('\\');
                                word.push(other);
                            }
                        },
                        other => word.push(other),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.push(chars.next()?);
            }
            ' ' | '\t' => end_word(&mut programs, &mut word, &mut in_word),
            '|' | ';' | '&' => {
                if c == '&' && chars.next() != Some('&') {
                    return None;
                }
                if c == '|' && chars.peek() == Some(&'|') {
                    chars.next();
                }
                end_word(&mut programs, &mut word, &mut in_word);
                programs.push(Vec::new());
            }
            '>' => {
                // Only discarding output (>/dev/null, 2>/dev/null) or merging it (2>&1) is harmless.
                if !(word.is_empty() || word == "1" || word == "2") {
                    return None;
                }
                let mut target = String::new();
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || next == '|' || next == ';' {
                        break;
                    }
                    target.push(next);
                    chars.next();
                }
                if !matches!(target.as_str(), "/dev/null" | "&1" | "&2") {
                    return None;
                }
                word.clear();
                in_word = false;
            }
            '$' | '`' | '<' | '(' | ')' => return None,
            other => {
                in_word = true;
                word.push(other);
            }
        }
    }
    end_word(&mut programs, &mut word, &mut in_word);
    if programs.iter().any(|words| words.is_empty()) {
        return None;
    }
    Some(programs)
}

fn end_word(programs: &mut [Vec<String>], word: &mut String, in_word: &mut bool) {
    if *in_word {
        if let Some(words) = programs.last_mut() {
            words.push(std::mem::take(word));
        }
        *in_word = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_plain_reads() {
        for command in [
            "ls -la",
            "cat /etc/hosts",
            "grep -rn \"fn main\" src",
            "find . -name '*.rs'",
            "ps aux | grep php | head -5",
            "git status && git log --oneline -5",
            "git branch",
            "git branch -a",
            "docker ps -a --filter name=web --format \"{{.Names}}\\t{{.Status}}\"",
            "sort file | uniq -c",
            "ls /missing 2>/dev/null",
            "du -sh * 2>&1 | sort -h",
            "echo \"price: \\$5\"",
            "wc -l a b; pwd",
        ] {
            assert!(is_read_only(command), "should run: {command}");
        }
    }

    #[test]
    fn asks_about_writes_and_other_programs() {
        for command in [
            "rm -rf build",
            "docker start web",
            "git checkout main",
            "git branch -D main",
            "git diff --output=patch",
            "git diff --out=patch",
            "git -C other status",
            "find . -delete",
            "find . \"-exec\" rm {} ;",
            "find . -name x -execdir rm {} +",
            "sort -o out in",
            "sort -ro out in",
            "sort --outp=out in",
            "uniq in out",
            "rg --pre cat x",
            "tree -o out",
            "date -s 2020-01-01",
            "echo hi > file",
            "echo hi >> file",
            "cat < file",
            "ls 1>out",
            "echo $(rm x)",
            "echo `rm x`",
            "echo \"$(rm x)\"",
            "echo $HOME",
            "ls & rm x",
            "ls\nrm x",
            "(rm x)",
            "X=1 ls",
            "/bin/rm x",
            "sudo ls",
            "ls |",
            "ls | | cat",
            "cat 'unterminated",
            "",
        ] {
            assert!(!is_read_only(command), "should ask: {command}");
        }
    }

    #[test]
    fn splits_like_the_shell() {
        let programs = split("grep 'a|b' \"x y\" z\\ w | wc -l").unwrap();
        assert_eq!(
            programs,
            vec![vec!["grep", "a|b", "x y", "z w"], vec!["wc", "-l"]]
        );
    }
}
