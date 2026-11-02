//! Tries to get out of Lokal's box (`docs/sandbox.md`), one test per thing
//! it promises. Where the box can't be set up (an older kernel, user
//! namespaces refused), each test says so and passes.

use std::fs;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

const UNAVAILABLE: i32 = 125;

struct Place {
    /// Outside the box, beside the project: never under /tmp, which the box replaces.
    base: PathBuf,
    project: PathBuf,
    scratch: PathBuf,
}

impl Place {
    fn new(name: &str) -> Place {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        let id = format!("{name}-{}-{n}", std::process::id());
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("box-{id}"));
        let _ = fs::remove_dir_all(&base);
        let project = base.join("project");
        fs::create_dir_all(&project).unwrap();
        let initialized = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&project)
            .status()
            .unwrap();
        assert!(initialized.success());
        let scratch = std::env::temp_dir().join(format!("lokal-box-test-{id}"));
        fs::create_dir_all(&scratch).unwrap();
        Place {
            base,
            project,
            scratch,
        }
    }

    /// What the command printed and its code, or None without a box here.
    fn run(&self, profile: &str, line: &str) -> Option<(i32, String)> {
        self.run_with(profile, line, &[])
    }

    fn run_with(&self, profile: &str, line: &str, env: &[(&str, &Path)]) -> Option<(i32, String)> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lokal"));
        command
            .arg("--box")
            .arg(profile)
            .arg(&self.project)
            .arg(&self.scratch)
            .arg(line)
            .stdin(Stdio::null());
        for (name, value) in env {
            command.env(name, value);
        }
        let output = command.output().unwrap();
        let code = output.status.code().unwrap_or(-1);
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        if code == UNAVAILABLE && text.contains("couldn't be set up") {
            eprintln!("no box here, skipped: {text}");
            return None;
        }
        Some((code, text))
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

fn has(program: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program}"))
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn writes_land_only_where_the_box_allows() {
    let place = Place::new("writes");
    let outside = place.base.join("outside.txt");
    let host_tmp = format!("/tmp/lokal-box-escaped-{}", std::process::id());
    let host_shm = format!("/dev/shm/lokal-box-escaped-{}", std::process::id());
    let line = format!(
        "touch project-file; touch {}; touch {host_tmp}; touch {host_shm}; touch {}/scratch-file; true",
        outside.display(),
        place.scratch.display()
    );
    let Some(_) = place.run("work", &line) else {
        return;
    };
    assert!(place.project.join("project-file").exists());
    assert!(place.scratch.join("scratch-file").exists());
    assert!(!outside.exists(), "wrote beside the project");
    assert!(!Path::new(&host_tmp).exists(), "wrote to the real /tmp");
    assert!(!Path::new(&host_shm).exists(), "wrote to the real /dev/shm");

    place.run("read", "touch read-file; true").unwrap();
    assert!(
        !place.project.join("read-file").exists(),
        "the read box wrote to the project"
    );
}

#[test]
fn what_runs_later_stays_read_only() {
    let place = Place::new("protected");
    fs::write(place.project.join("AGENTS.md"), "notes\n").unwrap();
    fs::create_dir_all(place.project.join(".vscode")).unwrap();
    fs::write(place.project.join(".vscode/tasks.json"), "{}\n").unwrap();
    let config = fs::read_to_string(place.project.join(".git/config")).unwrap();
    let line = "echo x >> .git/config; echo '#!/bin/sh' > .git/hooks/pre-commit; \
                echo steer >> AGENTS.md; echo x > .vscode/tasks.json; true";
    let Some(_) = place.run("work", line) else {
        return;
    };
    assert_eq!(
        fs::read_to_string(place.project.join(".git/config")).unwrap(),
        config
    );
    assert!(!place.project.join(".git/hooks/pre-commit").exists());
    assert_eq!(
        fs::read_to_string(place.project.join("AGENTS.md")).unwrap(),
        "notes\n"
    );
    assert_eq!(
        fs::read_to_string(place.project.join(".vscode/tasks.json")).unwrap(),
        "{}\n"
    );
}

#[test]
fn links_lead_nowhere_outside() {
    let place = Place::new("links");
    let target = place.base.join("target.txt");
    fs::write(&target, "kept\n").unwrap();
    std::os::unix::fs::symlink(&target, place.project.join("link")).unwrap();
    let line = format!(
        "echo x > link; ln {} hard; mv link {}/moved; true",
        target.display(),
        place.base.display()
    );
    let Some(_) = place.run("work", &line) else {
        return;
    };
    assert_eq!(fs::read_to_string(&target).unwrap(), "kept\n");
    assert!(
        !place.project.join("hard").exists(),
        "hard-linked a file from outside"
    );
    assert!(!place.base.join("moved").exists(), "moved a file out");
}

#[test]
fn there_is_no_network_but_the_loopback() {
    let place = Place::new("network");
    let Some((_, text)) = place.run(
        "read",
        "exec 3<>/dev/tcp/192.0.2.1/80 || echo NO-TCP; getent hosts example.com || echo NO-DNS",
    ) else {
        return;
    };
    assert!(text.contains("NO-TCP") && text.contains("NO-DNS"), "{text}");
    if has("python3") {
        let (code, text) = place
            .run(
                "read",
                "python3 -c 'import socket; s = socket.socket(); s.bind((\"127.0.0.1\", 0)); s.listen()'",
            )
            .unwrap();
        assert_eq!(code, 0, "a project's tests can't listen on lo: {text}");
    }
}

#[test]
fn no_socket_outside_can_be_reached() {
    if !has("python3") {
        return;
    }
    let place = Place::new("sockets");
    let path = place.base.join("service.sock");
    let _listener = UnixListener::bind(&path).unwrap();
    let line = format!(
        "python3 -c 'import socket; s = socket.socket(socket.AF_UNIX); s.connect(\"{}\")' \
         && echo REACHED; python3 -c 'import socket; socket.socketpair(); print(\"PAIR\")'; \
         echo RUN=$(ls -A /run | wc -l)",
        path.display()
    );
    let Some((_, text)) = place.run("read", &line) else {
        return;
    };
    assert!(!text.contains("REACHED"), "{text}");
    assert!(
        text.contains("PAIR"),
        "socketpair should still work: {text}"
    );
    assert!(text.contains("RUN=0"), "/run isn't hidden: {text}");
}

#[test]
fn nothing_outside_can_be_signalled() {
    let place = Place::new("signals");
    let line = format!("kill -0 {} && echo SIGNALLED", std::process::id());
    let Some((_, text)) = place.run("read", &line) else {
        return;
    };
    assert!(!text.contains("SIGNALLED"), "{text}");
}

#[test]
fn a_layer_cant_be_undone_from_inside() {
    let place = Place::new("layers");
    let line = "umount /run; umount -l /tmp; mount -t tmpfs none /run; \
                unshare -Urm sh -c 'umount -l /run; ls -A /run | wc -l' || true; \
                ls -A /run | wc -l";
    let Some((_, text)) = place.run("read", line) else {
        return;
    };
    assert!(
        text.lines()
            .filter(|line| line.trim().parse::<u32>().is_ok())
            .all(|n| n.trim() == "0"),
        "{text}"
    );
}

#[test]
fn keys_and_lokals_own_files_are_hidden() {
    let place = Place::new("keys");
    let home = place.base.join("home");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/id_test"), "SECRET-KEY\n").unwrap();
    fs::write(home.join(".netrc"), "SECRET-NETRC\n").unwrap();
    let config = place.base.join("config");
    fs::create_dir_all(config.join("lokal")).unwrap();
    fs::write(config.join("lokal/approved"), "SECRET-APPROVAL\n").unwrap();
    let agent = place.base.join("agent.sock");
    let line = "cat ~/.ssh/id_test ~/.netrc \"$XDG_CONFIG_HOME/lokal/approved\"; echo \"agent[$SSH_AUTH_SOCK]\"";
    let env = [
        ("HOME", home.as_path()),
        ("XDG_CONFIG_HOME", config.as_path()),
        ("SSH_AUTH_SOCK", agent.as_path()),
    ];
    let Some((_, text)) = place.run_with("read", line, &env) else {
        return;
    };
    assert!(!text.contains("SECRET"), "{text}");
    assert!(text.contains("agent[]"), "{text}");
}

#[test]
fn windows_stays_out() {
    let whoami = Path::new("/mnt/c/Windows/System32/whoami.exe");
    if !whoami.exists() {
        return;
    }
    let place = Place::new("windows");
    let line = format!(
        "{} && echo RAN-EXE; cp {} w.exe && ./w.exe && echo RAN-COPY; wslpath -w / && echo RAN-WSLPATH; true",
        whoami.display(),
        whoami.display()
    );
    let Some((_, text)) = place.run("work", &line) else {
        return;
    };
    assert!(!text.contains("RAN-"), "{text}");
}

#[test]
fn the_work_box_needs_a_git_repository() {
    // Under /tmp, where no repository is around it, unlike the target folder.
    let plain = std::env::temp_dir().join(format!("lokal-box-plain-{}", std::process::id()));
    fs::create_dir_all(&plain).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lokal"))
        .args(["--box", "work"])
        .arg(&plain)
        .arg(&plain)
        .arg("touch x")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(UNAVAILABLE));
    assert!(!plain.join("x").exists());
    fs::remove_dir_all(&plain).unwrap();
}

#[test]
fn a_folder_under_tmp_comes_along() {
    let folder = std::env::temp_dir().join(format!("lokal-box-tmp-{}", std::process::id()));
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("seen.txt"), "here\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lokal"))
        .args(["--box", "read"])
        .arg(&folder)
        .arg(&folder)
        .arg("cat seen.txt; touch made")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    if output.status.code() != Some(UNAVAILABLE) {
        assert!(text.contains("here"), "{text}");
        // There it's the scratch folder as well, so it may be written.
        assert!(folder.join("made").exists(), "{text}");
    }
    fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn a_folder_under_tmp_stays_read_only_in_the_read_box() {
    let tmp = std::env::temp_dir();
    let folder = tmp.join(format!("lokal-box-tmp-ro-{}", std::process::id()));
    let scratch = tmp.join(format!("lokal-box-tmp-sc-{}", std::process::id()));
    fs::create_dir_all(&folder).unwrap();
    fs::create_dir_all(&scratch).unwrap();
    // In the folder itself, then in /tmp, where the person's other files are.
    let mut boxed = true;
    for working in [&folder, &tmp] {
        let line = format!(
            "touch {}/made; touch {}/scratch-made-{}",
            folder.display(),
            scratch.display(),
            working.components().count()
        );
        let output = Command::new(env!("CARGO_BIN_EXE_lokal"))
            .args(["--box", "read"])
            .arg(working)
            .arg(&scratch)
            .arg(&line)
            .output()
            .unwrap();
        if output.status.code() == Some(UNAVAILABLE) {
            boxed = false;
            break;
        }
    }
    let wrote_folder = folder.join("made").exists();
    let in_scratch = fs::read_dir(&scratch).unwrap().count();
    fs::remove_dir_all(&folder).unwrap();
    fs::remove_dir_all(&scratch).unwrap();
    if boxed {
        assert!(!wrote_folder, "the read box wrote in its folder");
        assert_eq!(in_scratch, 2, "the scratch folder should take both writes");
    }
}

#[test]
fn everyday_work_still_runs() {
    let place = Place::new("everyday");
    let line = format!(
        "git status --short >/dev/null && echo GIT; echo ERR > /dev/stderr; \
         printf '#!/bin/sh\\necho SCRIPT\\n' > {s}/s.sh && chmod +x {s}/s.sh && {s}/s.sh; \
         mkdir -p build && echo x > build/out && echo BUILD",
        s = place.scratch.display()
    );
    let Some((code, text)) = place.run("work", &line) else {
        return;
    };
    assert_eq!(code, 0, "{text}");
    for word in ["GIT", "ERR", "SCRIPT", "BUILD"] {
        assert!(text.contains(word), "{word} missing: {text}");
    }
}
