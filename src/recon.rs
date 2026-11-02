//! `/recon`: a short account of the machine and the project, so the model
//! knows what it has to work with without finding it out on every task.
//! Lokal gathers it itself, without the model and without a shell: the
//! probes are fixed, nothing in them changes anything, and what could be a
//! secret is left out by this code rather than by a model's care.

use std::env;
use std::fs;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::layout::clean;
use crate::{approvals, config, system, text};

/// Programs worth knowing about, in the order they're listed. Only the ones
/// on the PATH are named.
const TOOLS: &[&str] = &[
    "git",
    "gh",
    "ssh",
    "curl",
    "wget",
    "make",
    "cmake",
    "gcc",
    "clang",
    "cargo",
    "rustc",
    "go",
    "node",
    "npm",
    "pnpm",
    "yarn",
    "bun",
    "deno",
    "python3",
    "pip3",
    "uv",
    "java",
    "dotnet",
    "php",
    "composer",
    "ruby",
    "docker",
    "podman",
    "kubectl",
    "helm",
    "terraform",
    "aws",
    "gcloud",
    "az",
    "psql",
    "mysql",
    "sqlite3",
    "redis-cli",
    "rg",
    "fd",
    "jq",
    "tmux",
    "vim",
    "nvim",
];

/// Files at the project's root that say how it's built, with the programs
/// whose versions then matter.
const MANIFESTS: &[(&str, &[&str])] = &[
    ("Cargo.toml", &["cargo", "rustc"]),
    ("package.json", &["node"]),
    ("deno.json", &["deno"]),
    ("pyproject.toml", &["python3"]),
    ("requirements.txt", &["python3"]),
    ("go.mod", &["go"]),
    ("composer.json", &["php"]),
    ("Gemfile", &["ruby"]),
    ("pom.xml", &["java"]),
    ("build.gradle", &["java"]),
    ("build.gradle.kts", &["java"]),
    ("CMakeLists.txt", &["cmake"]),
    ("Makefile", &[]),
    ("Dockerfile", &[]),
    ("compose.yaml", &[]),
    ("docker-compose.yml", &[]),
];

/// The same for a kind of file whose name varies.
const PROJECT_FILES: &[(&str, &[&str])] = &[("csproj", &["dotnet"]), ("sln", &["dotnet"])];

/// How long a probe may take; one that hangs is left out.
const PROBE_TIME: Duration = Duration::from_secs(3);
/// Text that comes from the disk is cut to this, so a branch name can't
/// carry a page of instructions; a path or an address gets more, since a
/// long one is still only one.
const FIELD_LIMIT: usize = 80;
const PATH_LIMIT: usize = 240;

/// The account, as labelled rows: what the person sees, and what the
/// model is given.
pub fn gather(folder: &Path) -> Vec<(&'static str, String)> {
    let wsl = system::is_wsl();
    let found = on_path(TOOLS, wsl);
    let root = approvals::project_root(folder);
    let (manifests, runtimes) = manifests(&root);
    let mut rows = vec![
        (text::RECON_HOST, host(wsl)),
        (text::RECON_TOOLS, tools(&found, &runtimes)),
    ];
    if let Some(docker) = found.iter().find(|(name, _)| *name == "docker") {
        rows.push((text::RECON_DOCKER, docker_state(&docker.1)));
    }
    let git = found.iter().find(|(name, _)| *name == "git");
    rows.push((
        text::RECON_PROJECT,
        project(&root, &manifests, git.map(|(_, path)| path.as_path())),
    ));
    rows
}

/// "x86_64 · 28 CPUs · 31 GB of memory · shell bash · time zone America/Sao_Paulo"
fn host(wsl: bool) -> String {
    let mut parts = vec![env::consts::ARCH.to_string()];
    if let Ok(cpus) = thread::available_parallelism() {
        parts.push(format!("{cpus} CPUs"));
    }
    if let Some(gigabytes) = memory_gigabytes() {
        parts.push(format!("{gigabytes} GB of memory"));
    }
    if let Some(shell) = env::var_os("SHELL") {
        let name = Path::new(&shell).file_name().unwrap_or_default();
        parts.push(format!(
            "shell {}",
            clean(&name.to_string_lossy(), FIELD_LIMIT)
        ));
    }
    if let Some(zone) = time_zone() {
        parts.push(format!("time zone {zone}"));
    }
    // Found like the other probes, never through the PATH's relative folders.
    if wsl
        && let Some((_, wslinfo)) = on_path(&["wslinfo"], wsl).first()
        && let Some(mode) =
            output_of(wslinfo, &["--networking-mode"]).filter(|mode| !mode.is_empty())
    {
        parts.push(format!("WSL networking {mode}"));
    }
    parts.join(" · ")
}

fn memory_gigabytes() -> Option<u64> {
    let info = fs::read_to_string("/proc/meminfo").ok()?;
    let kilobytes: u64 = info
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some((kilobytes + 512 * 1024) / (1024 * 1024))
}

fn time_zone() -> Option<String> {
    let zone = env::var("TZ")
        .ok()
        .or_else(|| fs::read_to_string("/etc/timezone").ok())
        .or_else(|| {
            let link = fs::read_link("/etc/localtime").ok()?;
            let link = link.to_string_lossy();
            Some(link.split_once("zoneinfo/")?.1.to_string())
        })?;
    let zone = clean(&zone, FIELD_LIMIT);
    (!zone.is_empty()).then_some(zone)
}

/// The tools found, with the versions that matter for the project and a
/// word on the ones set up to reach an account.
fn tools(found: &[(&str, PathBuf)], runtimes: &[&str]) -> String {
    let names: Vec<String> = found
        .iter()
        .map(|(name, path)| {
            let mut shown = name.to_string();
            if *name == "git" || runtimes.contains(name) {
                let args: &[&str] = if *name == "go" {
                    &["version"]
                } else {
                    &["--version"]
                };
                if let Some(version) = output_of(path, args).as_deref().and_then(version_in) {
                    shown.push_str(&format!(" {version}"));
                }
            }
            if is_set_up(name) {
                shown.push_str(&format!(" ({})", text::RECON_SET_UP));
            }
            shown
        })
        .collect();
    if names.is_empty() {
        text::RECON_NONE.to_string()
    } else {
        names.join(", ")
    }
}

/// Whether a cloud or account tool has its settings in place: only whether
/// the file or variable is there, never what it holds.
fn is_set_up(name: &str) -> bool {
    let home = config::home();
    match name {
        "aws" => {
            ["AWS_PROFILE", "AWS_ACCESS_KEY_ID"]
                .iter()
                .any(|variable| env::var_os(variable).is_some())
                || home.join(".aws/config").exists()
                || home.join(".aws/credentials").exists()
        }
        "gh" => home.join(".config/gh/hosts.yml").exists(),
        "kubectl" => home.join(".kube/config").exists(),
        _ => false,
    }
}

/// Whether Docker's daemon answers, and how many containers it runs.
fn docker_state(docker: &Path) -> String {
    let Some(version) = output_of(docker, &["info", "--format", "{{.ServerVersion}}"])
        .map(|version| clean(&version, FIELD_LIMIT))
        .filter(|version| !version.is_empty())
    else {
        return text::RECON_DOCKER_DOWN.to_string();
    };
    let running = output_of(docker, &["ps", "--quiet"]).map_or(0, |listing| {
        listing
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
    });
    text::recon_docker(&version, running)
}

/// "~/sites/lokal · Cargo.toml · branch main · remote origin https://example.com/lokal.git"
fn project(root: &Path, manifests: &[String], git: Option<&Path>) -> String {
    let mut parts = vec![clean(&config::shown(root), PATH_LIMIT)];
    parts.extend(manifests.iter().cloned());
    let is_repository = root.join(".git").exists();
    match git {
        Some(git) if is_repository => {
            let root = root.to_string_lossy();
            let branch = output_of(git, &["-C", &root, "branch", "--show-current"])
                .map(|branch| clean(&branch, FIELD_LIMIT))
                .unwrap_or_default();
            parts.push(if branch.is_empty() {
                text::RECON_NO_BRANCH.to_string()
            } else {
                format!("branch {branch}")
            });
            let listing = output_of(git, &["-C", &root, "remote", "-v"]).unwrap_or_default();
            parts.extend(remotes(&listing));
        }
        _ if is_repository => {}
        _ => parts.push(text::RECON_NOT_A_REPOSITORY.to_string()),
    }
    parts.join(" · ")
}

/// The manifests at the root, and the programs they make worth a version.
fn manifests(root: &Path) -> (Vec<String>, Vec<&'static str>) {
    let mut found = Vec::new();
    let mut runtimes = Vec::new();
    for (name, programs) in MANIFESTS {
        if root.join(name).is_file() {
            found.push(name.to_string());
            runtimes.extend_from_slice(programs);
        }
    }
    let extensions: Vec<String> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| Some(entry.path().extension()?.to_string_lossy().into_owned()))
        .collect();
    for (extension, programs) in PROJECT_FILES {
        if extensions.iter().any(|found| found == extension) {
            found.push(format!("a .{extension}"));
            runtimes.extend_from_slice(programs);
        }
    }
    (found, runtimes)
}

/// `git remote -v` as "remote origin <address>", each once and without
/// what could be a secret in its address. At most three.
fn remotes(listing: &str) -> Vec<String> {
    let mut remotes: Vec<String> = Vec::new();
    for line in listing.lines() {
        let mut fields = line.split_whitespace();
        let (Some(name), Some(address)) = (fields.next(), fields.next()) else {
            continue;
        };
        let remote = format!(
            "remote {} {}",
            clean(name, FIELD_LIMIT),
            clean(&without_secrets(address), PATH_LIMIT)
        );
        if !remotes.contains(&remote) {
            remotes.push(remote);
        }
    }
    remotes.truncate(3);
    remotes
}

/// An address without the user and password before an @, and without what
/// follows ? or #: a token in a remote's address would otherwise reach the
/// model, and through it a command.
fn without_secrets(address: &str) -> String {
    let address = address.split(['?', '#']).next().unwrap_or_default();
    let (scheme, rest) = match address.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), address),
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let rest = match rest[..authority_end].rfind('@') {
        Some(at) => &rest[at + 1..],
        None => rest,
    };
    format!("{scheme}{rest}")
}

/// Where each of `names` is on the PATH, skipping folders given relative to
/// wherever Lokal runs (a project could put a program there) and, under
/// WSL, Windows' drives, whose programs are Windows' own and slow to look
/// through.
fn on_path<'a>(names: &[&'a str], wsl: bool) -> Vec<(&'a str, PathBuf)> {
    let path = env::var_os("PATH").unwrap_or_default();
    let folders: Vec<PathBuf> = env::split_paths(&path)
        .filter(|folder| folder.is_absolute())
        .filter(|folder| !(wsl && folder.starts_with("/mnt")))
        .collect();
    names
        .iter()
        .filter_map(|name| {
            let program = folders
                .iter()
                .map(|folder| folder.join(name))
                .find(|path| system::is_program(path))?;
            Some((*name, program))
        })
        .collect()
}

/// What a program prints, stdout or else stderr, when it ends within the
/// probe's time. In a session of its own, like the model's commands, so it
/// can't wait on the terminal.
fn output_of(program: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: setsid is safe to call between fork and exec.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command.spawn().ok()?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if started.elapsed() > PROBE_TIME {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    };
    if !status.success() {
        return None;
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    if out.trim().is_empty() {
        child.stderr.take()?.read_to_string(&mut out).ok()?;
    }
    Some(out.trim().to_string())
}

/// The version in a program's `--version`: the first word of its first line
/// that reads like one ("git version 2.43.0", "v22.1.0", "go1.22.2").
fn version_in(output: &str) -> Option<String> {
    let line = output.lines().next()?;
    line.split_whitespace().find_map(|word| {
        let word = word.trim_matches(|c| c == '"' || c == ',');
        let word = word
            .strip_prefix("go")
            .or_else(|| word.strip_prefix('v'))
            .unwrap_or(word);
        let starts_with_digit = word.chars().next().is_some_and(|c| c.is_ascii_digit());
        (starts_with_digit && word.contains('.')).then(|| clean(word, FIELD_LIMIT))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn addresses_lose_what_could_be_a_secret() {
        assert_eq!(
            without_secrets("https://ghp_token123@github.com/owner/repo.git"),
            "https://github.com/owner/repo.git"
        );
        assert_eq!(
            without_secrets("https://user:pass@example.com:8443/repo.git?private_token=x#top"),
            "https://example.com:8443/repo.git"
        );
        assert_eq!(
            without_secrets("git@github.com:owner/repo.git"),
            "github.com:owner/repo.git"
        );
        assert_eq!(
            without_secrets("ssh://git@example.com/srv/repo"),
            "ssh://example.com/srv/repo"
        );
        assert_eq!(without_secrets("/srv/git/repo.git"), "/srv/git/repo.git");
        assert_eq!(
            without_secrets("https://example.com/a@b/repo"),
            "https://example.com/a@b/repo"
        );
    }

    #[test]
    fn remotes_show_once_each_and_at_most_three() {
        let listing = "origin\thttps://tok@github.com/o/r.git (fetch)\n\
                       origin\thttps://tok@github.com/o/r.git (push)\n\
                       a\t/x (fetch)\nb\t/y (fetch)\nc\t/z (fetch)\n";
        assert_eq!(
            remotes(listing),
            [
                "remote origin https://github.com/o/r.git",
                "remote a /x",
                "remote b /y"
            ]
        );
    }

    #[test]
    fn versions_are_found_in_what_programs_print() {
        assert_eq!(version_in("git version 2.43.0").as_deref(), Some("2.43.0"));
        assert_eq!(
            version_in("cargo 1.90.0 (840b83a10 2025-07-30)").as_deref(),
            Some("1.90.0")
        );
        assert_eq!(version_in("v22.1.0").as_deref(), Some("22.1.0"));
        assert_eq!(
            version_in("go version go1.22.2 linux/amd64").as_deref(),
            Some("1.22.2")
        );
        assert_eq!(
            version_in("openjdk version \"21.0.2\" 2024-01-16").as_deref(),
            Some("21.0.2")
        );
        assert_eq!(version_in("no version here"), None);
    }

    #[test]
    fn only_programs_on_absolute_folders_are_found() {
        let base = env::temp_dir().join(format!("lokal-recon-{}", std::process::id()));
        fs::create_dir_all(base.join("bin")).unwrap();
        let program = base.join("bin/tool");
        fs::write(&program, "#!/bin/sh\n").unwrap();
        fs::write(base.join("bin/plain"), "not a program").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(system::is_program(&program));
        assert!(!system::is_program(&base.join("bin/plain")));
        assert!(!system::is_program(&base.join("bin")));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn manifests_name_the_runtimes_whose_versions_matter() {
        let base = env::temp_dir().join(format!("lokal-recon-manifests-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("Cargo.toml"), "").unwrap();
        fs::write(base.join("Makefile"), "").unwrap();
        fs::write(base.join("App.csproj"), "").unwrap();
        let (found, runtimes) = manifests(&base);
        assert_eq!(found, ["Cargo.toml", "Makefile", "a .csproj"]);
        assert_eq!(runtimes, ["cargo", "rustc", "dotnet"]);
        fs::remove_dir_all(&base).unwrap();
    }
}
