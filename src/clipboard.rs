//! The clipboard's image, read when the person pastes at the `>`: by
//! Lokal's own fixed programs, found in the PATH's absolute folders and run
//! without a shell, never by the model, which has no tool for it.

use std::env;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::image::{self, Image};
use crate::{system, text};

/// How long reading the clipboard may take; Windows PowerShell's start is
/// the slow part.
const READ_TIME: Duration = Duration::from_secs(10);

/// Under WSL, Windows' clipboard as base64 PNG, from Windows PowerShell,
/// whose `Get-Clipboard -Format Image` PowerShell 7 lacks.
const WINDOWS_READ: &str = "$i = Get-Clipboard -Format Image; \
    if ($i) { $m = New-Object IO.MemoryStream; \
    $i.Save($m, [Drawing.Imaging.ImageFormat]::Png); [Convert]::ToBase64String($m.ToArray()) }";

/// The image on the clipboard, with its data URL; Err says why there's none.
pub fn image() -> Result<(Image, String), String> {
    if system::is_wsl() {
        let program = find("powershell.exe").ok_or(text::CLIPBOARD_NO_READER)?;
        let output = run(
            program,
            &["-NoProfile", "-NonInteractive", "-Command", WINDOWS_READ],
        )?;
        let data = String::from_utf8_lossy(&output);
        if data.trim().is_empty() {
            return Err(text::CLIPBOARD_NO_IMAGE.to_string());
        }
        if data.trim().len() / 4 * 3 > image::MOST_BYTES as usize {
            return Err(text::CLIPBOARD_TOO_BIG.to_string());
        }
        return image::from_base64(&data).ok_or_else(|| text::CLIPBOARD_NO_IMAGE.to_string());
    }
    let (program, args): (&str, &[&str]) = if env::var_os("WAYLAND_DISPLAY").is_some() {
        ("wl-paste", &["--no-newline", "--type", "image/png"])
    } else if env::var_os("DISPLAY").is_some() {
        (
            "xclip",
            &["-selection", "clipboard", "-target", "image/png", "-out"],
        )
    } else {
        return Err(text::CLIPBOARD_NO_READER.to_string());
    };
    let program = find(program).ok_or(text::CLIPBOARD_NO_READER)?;
    // Both end with an error when the clipboard holds no PNG, which `run`
    // says as no image.
    let bytes = run(program, args)?;
    if bytes.len() as u64 > image::MOST_BYTES {
        return Err(text::CLIPBOARD_TOO_BIG.to_string());
    }
    let image = image::read(&bytes).ok_or(text::CLIPBOARD_NO_IMAGE)?;
    let url = image::data_url(image.kind, &bytes);
    Ok((image, url))
}

/// `name` in the PATH's absolute folders: with `.` in the PATH, a project
/// could plant its own `powershell.exe`.
fn find(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path)
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join(name))
        .find(|program| system::is_program(program))
}

/// What a program prints, read as it goes, up to a little over the largest
/// image's base64, within `READ_TIME`, in a session of its own. Err when it
/// fails, takes too long, or prints too much.
fn run(program: PathBuf, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: setsid is safe to call between fork and exec.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| text::clipboard_failed(&error.to_string()))?;
    let Some(stdout) = child.stdout.take() else {
        return Err(text::clipboard_failed("no output"));
    };
    let most = image::MOST_BYTES / 3 * 4 + 1024;
    let (sender, read) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(most + 1).read_to_end(&mut bytes);
        let _ = sender.send(result.map(|_| bytes));
    });
    let started = Instant::now();
    let bytes = match read.recv_timeout(READ_TIME) {
        Ok(Ok(bytes)) if bytes.len() as u64 <= most => bytes,
        Ok(Ok(_)) => return end(child, text::CLIPBOARD_TOO_BIG),
        Ok(Err(_)) => return end(child, text::CLIPBOARD_NO_IMAGE),
        Err(_) => return end(child, text::CLIPBOARD_SLOW),
    };
    // Its output has ended, and so, nearly always, has it.
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(bytes),
            Ok(Some(_)) | Err(_) => return Err(text::CLIPBOARD_NO_IMAGE.to_string()),
            Ok(None) if started.elapsed() > READ_TIME => {
                return end(child, text::CLIPBOARD_SLOW);
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Ends a reader that went wrong, before anything waits on it: one stuck
/// writing to a full pipe would never end on its own.
fn end(mut child: Child, why: &str) -> Result<Vec<u8>, String> {
    let _ = child.kill();
    let _ = child.wait();
    Err(why.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_s_output_comes_back_and_a_failure_says_so() {
        assert_eq!(run(PathBuf::from("/bin/echo"), &["hi"]).unwrap(), b"hi\n");
        assert_eq!(
            run(PathBuf::from("/bin/false"), &[]).unwrap_err(),
            text::CLIPBOARD_NO_IMAGE
        );
        assert!(find("no-such-program-of-lokal").is_none());
    }
}
