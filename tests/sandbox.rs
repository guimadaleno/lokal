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

impl Place {
    /// Whether `lokal --box work` refused to set up, with git's global
    /// settings read from `global` when given.
    fn work_refused(&self, line: &str, global: Option<&Path>) -> bool {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lokal"));
        command
            .args(["--box", "work"])
            .arg(&self.project)
            .arg(&self.scratch)
            .arg(line);
        if let Some(global) = global {
            command.env("GIT_CONFIG_GLOBAL", global);
        }
        command.output().unwrap().status.code() == Some(UNAVAILABLE)
    }

    fn git(&self, args: &[&str]) {
        let done = Command::new("git")
            .args(args)
            .current_dir(&self.project)
            .status()
            .unwrap();
        assert!(done.success());
    }

    /// A file of git's global settings beside the project.
    fn global(&self, contents: &str) -> PathBuf {
        let path = self.base.join("global.gitconfig");
        fs::write(&path, contents).unwrap();
        path
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
    // In the scratch folder, which comes into the box: a short path, and one
    // the command can see, so only the filter stands in the way.
    let path = place.scratch.join("s.sock");
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
fn only_internet_sockets_can_be_made() {
    if !has("python3") {
        return;
    }
    let place = Place::new("families");
    let line = "for family in AF_INET AF_INET6 AF_UNIX AF_VSOCK AF_PACKET; do \
                python3 -c \"import socket; socket.socket(socket.$family)\" 2>/dev/null \
                && echo \"$family MADE\" || echo \"$family REFUSED\"; done";
    let Some((_, text)) = place.run("read", line) else {
        return;
    };
    for made in ["AF_INET MADE", "AF_INET6 MADE"] {
        assert!(text.contains(made), "{text}");
    }
    for refused in ["AF_UNIX REFUSED", "AF_VSOCK REFUSED", "AF_PACKET REFUSED"] {
        assert!(text.contains(refused), "{text}");
    }
}

#[test]
fn a_descriptor_open_outside_doesnt_come_in() {
    let place = Place::new("descriptors");
    let victim = place.base.join("victim.txt");
    fs::write(&victim, "").unwrap();
    let line = format!(
        "exec {} --box read {} {} 'echo PWNED >&7' 7>>{}",
        env!("CARGO_BIN_EXE_lokal"),
        place.project.display(),
        place.scratch.display(),
        victim.display()
    );
    let output = Command::new("sh").arg("-c").arg(&line).output().unwrap();
    if output.status.code() == Some(UNAVAILABLE) {
        return;
    }
    assert_eq!(fs::read_to_string(&victim).unwrap(), "");
}

#[test]
fn a_second_mount_of_the_same_files_is_covered() {
    if !has("unshare") {
        return;
    }
    let place = Place::new("doors");
    fs::write(place.project.join("AGENTS.md"), "notes\n").unwrap();
    let home = place.base.join("home");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/id_test"), "SECRET-KEY\n").unwrap();
    let config = fs::read_to_string(place.project.join(".git/config")).unwrap();
    // As Docker Desktop binds a folder of home elsewhere, and WSL the whole disk.
    let project_door = place.base.join("project-door");
    let home_door = place.base.join("home-door");
    let inside = format!(
        "echo x >> {pd}/.git/config; echo x >> {pd}/AGENTS.md; cat {hd}/.ssh/id_test; true",
        pd = project_door.display(),
        hd = home_door.display()
    );
    let line = format!(
        "mkdir -p {pd} {hd} && mount --bind {p} {pd} && mount --bind {h} {hd} && \
         HOME={h} exec {lokal} --box work {p} {s} \"$0\"",
        pd = project_door.display(),
        hd = home_door.display(),
        p = place.project.display(),
        h = home.display(),
        s = place.scratch.display(),
        lokal = env!("CARGO_BIN_EXE_lokal"),
    );
    let output = Command::new("unshare")
        .args(["-Urm", "sh", "-c", &line, &inside])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    if !output.status.success() && (text.contains("couldn't be set up") || text.contains("unshare"))
    {
        eprintln!("no namespace for the second mount here, skipped: {text}");
        return;
    }
    assert_eq!(
        fs::read_to_string(place.project.join(".git/config")).unwrap(),
        config
    );
    assert_eq!(
        fs::read_to_string(place.project.join("AGENTS.md")).unwrap(),
        "notes\n"
    );
    assert!(!text.contains("SECRET"), "{text}");
}

/// `inside` run by `lokal --box <profile>` from a shell in a namespace of
/// its own, after `setup` there; None where that can't be had.
fn in_namespace(
    setup: &str,
    profile: &str,
    folder: &str,
    scratch: &str,
    inside: &str,
) -> Option<String> {
    if !has("unshare") {
        return None;
    }
    let line = format!(
        "{setup} && exec {} --box {profile} {folder} {scratch} \"$0\"",
        env!("CARGO_BIN_EXE_lokal")
    );
    let output = Command::new("unshare")
        .args(["-Urm", "sh", "-c", &line, inside])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(UNAVAILABLE) && text.contains("unshare")
        || text.contains("unshare:")
    {
        eprintln!("no namespace here, skipped: {text}");
        return None;
    }
    Some(text)
}

#[test]
fn a_scratch_folder_on_its_own_tmpfs_stays_writable() {
    // Fedora, Arch and Debian 13 keep /tmp on a tmpfs of its own.
    let place = Place::new("tmpfs");
    let mount = place.base.join("tmpfs");
    fs::create_dir_all(&mount).unwrap();
    let setup = format!(
        "mount -t tmpfs none {m} && mkdir {m}/scratch",
        m = mount.display()
    );
    let scratch = format!("{}/scratch", mount.display());
    let Some(text) = in_namespace(
        &setup,
        "read",
        &place.project.display().to_string(),
        &scratch,
        &format!("touch {scratch}/x && echo WROTE"),
    ) else {
        return;
    };
    assert!(text.contains("WROTE"), "{text}");
}

#[test]
fn a_home_on_its_own_file_system_works() {
    // A /home partition: home and the project on a file system mounted once.
    let place = Place::new("partition");
    let mount = place.base.join("partition");
    fs::create_dir_all(&mount).unwrap();
    let m = mount.display();
    let setup = format!(
        "mount -t tmpfs none {m} && mkdir -p {m}/ana/project && git init -q {m}/ana/project && export HOME={m}/ana"
    );
    let Some(text) = in_namespace(
        &setup,
        "work",
        &format!("{m}/ana/project"),
        &place.scratch.display().to_string(),
        "touch made && echo WROTE",
    ) else {
        return;
    };
    assert!(text.contains("WROTE"), "{text}");
}

#[test]
fn a_folder_on_a_windows_drive_can_use_the_read_box() {
    let windows = Path::new("/mnt/c/Windows");
    if !windows.is_dir() {
        return;
    }
    let place = Place::new("drive");
    let output = Command::new(env!("CARGO_BIN_EXE_lokal"))
        .args(["--box", "read"])
        .arg(windows)
        .arg(&place.scratch)
        .arg("ls >/dev/null && echo LISTED")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("LISTED"), "{text}");
}

#[test]
fn hook_managers_kept_in_the_tree_stay_read_only() {
    let place = Place::new("husky");
    fs::create_dir_all(place.project.join(".husky")).unwrap();
    let Some(_) = place.run("work", "echo x > .husky/pre-commit; touch other; true") else {
        return;
    };
    assert!(!place.project.join(".husky/pre-commit").exists());
    assert!(place.project.join("other").exists());
}

#[test]
fn the_repositorys_own_settings_beyond_the_plain_keep_the_work_box_shut() {
    // Each names, or could name, a program git runs later, outside.
    for setting in [
        ["core.hooksPath", "tools"],
        ["alias.t", "!sh tools/t.sh"],
        ["filter.x.clean", "sh tools/c.sh"],
        ["difftool.x.cmd", "./tools/d.sh"],
        ["core.pager", "./tools/p.sh"],
        ["include.path", "../shared.gitconfig"],
    ] {
        let place = Place::new("repository");
        if place.run("read", "true").is_none() {
            return;
        }
        place.git(&["config", setting[0], setting[1]]);
        assert!(
            place.work_refused("mkdir -p tools; echo x > tools/t.sh", None),
            "{setting:?}"
        );
        assert!(!place.project.join("tools").exists(), "{setting:?}");
    }
}

#[test]
fn settings_git_cant_read_keep_the_work_box_shut() {
    let place = Place::new("unreadable");
    if place.run("read", "true").is_none() {
        return;
    }
    fs::write(place.project.join("bad.cfg"), "[core\n").unwrap();
    let config = place.project.join(".git/config");
    let mut text = fs::read_to_string(&config).unwrap();
    text.push_str("[include]\n\tpath = ../bad.cfg\n");
    fs::write(&config, text).unwrap();
    assert!(place.work_refused("echo '[core]' > bad.cfg", None));
    assert_eq!(
        fs::read_to_string(place.project.join("bad.cfg")).unwrap(),
        "[core\n"
    );
}

#[test]
fn paths_in_the_persons_own_settings_that_lead_into_the_tree_stay_read_only() {
    let place = Place::new("global");
    fs::create_dir_all(place.project.join("tools/hooks")).unwrap();
    fs::write(place.project.join("tools/t.sh"), "#!/bin/sh\n").unwrap();
    let global = place.global(
        "[core]\n\thooksPath = tools/hooks\n[alias]\n\tt = !sh tools/t.sh\n\
         [include]\n\tpath = ~/.gitconfig.local\n",
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_lokal"));
    command
        .args(["--box", "work"])
        .arg(&place.project)
        .arg(&place.scratch)
        .arg("echo x > tools/hooks/pre-commit; echo evil >> tools/t.sh; touch other; echo RAN")
        .env("GIT_CONFIG_GLOBAL", &global);
    let output = command.output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(UNAVAILABLE) && text.contains("unshare") {
        return;
    }
    // ~/ is the home folder's, so a missing ~/.gitconfig.local refuses nothing.
    assert!(text.contains("RAN"), "{text}");
    assert!(!place.project.join("tools/hooks/pre-commit").exists());
    assert_eq!(
        fs::read_to_string(place.project.join("tools/t.sh")).unwrap(),
        "#!/bin/sh\n"
    );
    assert!(place.project.join("other").exists());
}

#[test]
fn a_path_the_persons_settings_name_that_isnt_there_keeps_the_work_box_shut() {
    let place = Place::new("missing");
    if place.run("read", "true").is_none() {
        return;
    }
    let global = place.global("[core]\n\thooksPath = .hooks\n");
    assert!(place.work_refused("mkdir .hooks && echo x > .hooks/pre-commit", Some(&global)));
    assert!(!place.project.join(".hooks").exists());
    // Outside the project, it's the rest of the box's to keep.
    let upward = place.global("[core]\n\thooksPath = ../../..\n");
    assert!(!place.work_refused("touch made", Some(&upward)));
    assert!(place.project.join("made").exists());
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
fn the_work_box_never_holds_the_home_folder() {
    let place = Place::new("home");
    let home = place.project.join("home");
    fs::create_dir_all(&home).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lokal"))
        .args(["--box", "work"])
        .arg(&place.project)
        .arg(&place.scratch)
        .arg("touch x")
        .env("HOME", &home)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(UNAVAILABLE));
    assert!(!place.project.join("x").exists());
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
