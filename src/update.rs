//! Keeping an installed Lokal up to date from download.guimadaleno.com, the
//! way every app here updates: a signed latest.json, checked at most once a
//! day in the background, taking effect at the next start. Every failure is
//! silent and leaves everything as it was. guimadaleno/base/release's README is
//! the authority on the format and the rules; release.rs is its client.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::{config, release, text};

const APP: &str = "lokal";
const MANIFEST_URL: &str = "https://download.guimadaleno.com/lokal/latest.json";
const PUBLIC_KEY: [u8; 32] = release::hex32(include_str!("../update.pub"));
/// What a release may put in place of this copy: the executable, and nothing else.
const FILES: [&str; 1] = ["lokal"];
/// Bounds on what is believed to be a Lokal executable.
const MIN_SIZE: u64 = 256 * 1024;
const MAX_SIZE: u64 = 64 * 1024 * 1024;
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Only the executable the release workflow builds updates itself: one built
/// from a working tree never replaces itself, or a rebuild would be undone.
const RELEASE_BUILD: bool = option_env!("LOKAL_RELEASE_BUILD").is_some();

/// A check running in the background.
pub struct Check {
    result: Receiver<String>,
}

impl Check {
    /// The version put in place during this session, if the check is done and found one.
    pub fn installed(&self) -> Option<String> {
        self.result.try_recv().ok()
    }
}

/// Whether this copy keeps itself up to date, and if not, why not.
pub enum Standing {
    /// It checks once a day.
    Daily,
    /// `updates = off` in the settings.
    SwitchedOff,
    /// A build made outside the release workflow.
    NotARelease,
    /// It runs from somewhere it can't write, like a package manager's folder.
    CantReplace,
}

/// The copy that's running and whether it may update itself.
fn standing(switched_off: bool) -> (Standing, Option<PathBuf>) {
    if !RELEASE_BUILD {
        return (Standing::NotARelease, None);
    }
    if switched_off {
        return (Standing::SwitchedOff, None);
    }
    match std::env::current_exe() {
        Ok(executable) if can_replace(&executable) => (Standing::Daily, Some(executable)),
        _ => (Standing::CantReplace, None),
    }
}

pub fn standing_of(switched_off: bool) -> Standing {
    standing(switched_off).0
}

/// Starts the day's check, when this copy may update itself and one is due.
pub fn start(switched_off: bool) -> Option<Check> {
    let (_, Some(executable)) = standing(switched_off) else {
        return None;
    };
    if !is_due(read_last_check(), now()) {
        return None;
    }
    let (sender, result) = mpsc::channel();
    thread::spawn(move || {
        if let Ok(Some(version)) = checked(&executable) {
            let _ = sender.send(version);
        }
    });
    Some(Check { result })
}

pub enum Asked {
    /// A newer version is on disk and runs from the next start.
    Installed(String),
    Newest,
    Failed,
    Unavailable(Standing),
}

/// `/update`: the check, now, whenever the last one was.
pub fn check_now(switched_off: bool) -> Asked {
    let (standing, executable) = standing(switched_off);
    let Some(executable) = executable else {
        return Asked::Unavailable(standing);
    };
    match checked(&executable) {
        Ok(Some(version)) => Asked::Installed(version),
        Ok(None) => Asked::Newest,
        Err(()) => Asked::Failed,
    }
}

/// A check, recorded first: one that fails waits a day like one that succeeds.
fn checked(executable: &Path) -> Result<Option<String>, ()> {
    let _ = config::write(&config::last_update_check_path(), &now().to_string());
    check(executable)
}

/// `Ok(Some(version))`: a newer Lokal is on disk and runs from the next start.
/// `Ok(None)`: this one is the newest.
fn check(executable: &Path) -> Result<Option<String>, ()> {
    let text = download(MANIFEST_URL)?;
    // Nothing in the release is read before its signature holds, and a release
    // that isn't Lokal's or names anything but `lokal` is refused whole.
    let app = release::App {
        name: APP,
        key: &PUBLIC_KEY,
        files: &FILES,
        max_size: MAX_SIZE,
    };
    let release = release::open(&text, &app).map_err(|_| ())?;
    if !release::newer(&release.version, text::VERSION) {
        return Ok(None);
    }
    let file = release
        .files
        .iter()
        .find(|file| file.name == FILES[0])
        .ok_or(())?;
    let bytes = download(&file.url)?;
    if !is_expected(&bytes, file) {
        return Err(());
    }
    put_in_place(executable, &bytes).map_err(|_| ())?;
    Ok(Some(release.version))
}

/// Linux has no HTTPS of its own to offer, and curl is on nearly every
/// system. The connection isn't what makes an update trusted: the signature
/// and the hashes are.
fn download(url: &str) -> Result<Vec<u8>, ()> {
    let output = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--proto",
            "=https",
            "--max-time",
            "300",
        ])
        .arg("--max-filesize")
        .arg(MAX_SIZE.to_string())
        .arg(url)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| ())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(())
    }
}

/// The size and SHA-256 the signed release gives, and a Linux executable's header.
fn is_expected(bytes: &[u8], file: &release::File) -> bool {
    let size = bytes.len() as u64;
    size == file.size
        && size >= MIN_SIZE
        && bytes.starts_with(b"\x7fELF")
        && hex(&Sha256::digest(bytes)) == file.sha256
}

/// Linux lets a running executable be replaced: the new one is written beside
/// it, then renamed over it in one step. This Lokal keeps running the old one
/// until it ends.
fn put_in_place(executable: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let staged = staged_path(executable);
    let permissions = fs::metadata(executable)?.permissions();
    // Only a file made here: one already there is another check's.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    file.write_all(bytes)
        .and_then(|()| fs::set_permissions(&staged, permissions))
        .and_then(|()| fs::rename(&staged, executable))
        .inspect_err(|_| {
            let _ = fs::remove_file(&staged);
        })
}

/// Beside the executable, and this call's own. Two checks at once, in one
/// Lokal (the day's and `/update`) or in two, would otherwise write into the
/// same file, and one could rename it in while the other was still writing.
fn staged_path(executable: &Path) -> PathBuf {
    static CALLS: AtomicU32 = AtomicU32::new(0);
    let call = CALLS.fetch_add(1, Ordering::Relaxed);
    let mut name = executable.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".new-{}-{call}", std::process::id()));
    executable.with_file_name(name)
}

/// A copy updates itself only where it can write: one a package manager
/// installed, under root, is that package manager's to update.
fn can_replace(executable: &Path) -> bool {
    let Some(folder) = executable.parent() else {
        return false;
    };
    let Ok(metadata) = fs::metadata(executable) else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o200 != 0 && is_writable(folder)
}

fn is_writable(folder: &Path) -> bool {
    let Ok(path) = std::ffi::CString::new(folder.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    // SAFETY: a plain syscall on a valid, NUL-terminated path.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

fn read_last_check() -> Option<u64> {
    fs::read_to_string(config::last_update_check_path())
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn is_due(last_check: Option<u64>, now: u64) -> bool {
    match last_check {
        Some(last) => now.saturating_sub(last) >= CHECK_EVERY.as_secs() || last > now,
        None => true,
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_update_pub() {
        let text = include_str!("../update.pub").trim().to_string();
        assert_eq!(hex(&PUBLIC_KEY), text);
    }

    #[test]
    fn the_manifest_is_lokals_own() {
        assert_eq!(
            MANIFEST_URL,
            format!("https://download.guimadaleno.com/{APP}/latest.json")
        );
    }

    #[test]
    fn a_check_is_due_once_a_day() {
        let day = CHECK_EVERY.as_secs();
        assert!(is_due(None, 1000));
        assert!(!is_due(Some(1000), 1000 + day - 1));
        assert!(is_due(Some(1000), 1000 + day));
        // A clock set back must not stop checks for good.
        assert!(is_due(Some(1000 + day), 1000));
    }

    #[test]
    fn only_the_signed_bytes_of_a_linux_executable_pass() {
        let mut bytes = b"\x7fELF".to_vec();
        bytes.resize(MIN_SIZE as usize, 0);
        let file = release::File {
            name: "lokal".into(),
            url: "https://download.guimadaleno.com/lokal/9.9.9/lokal".into(),
            size: bytes.len() as u64,
            sha256: hex(&Sha256::digest(&bytes)),
        };
        assert!(is_expected(&bytes, &file));

        let mut tampered = bytes.clone();
        tampered[100] = 1;
        assert!(!is_expected(&tampered, &file));

        let mut windows = bytes.clone();
        windows[..4].copy_from_slice(b"MZ\0\0");
        let windows_file = release::File {
            sha256: hex(&Sha256::digest(&windows)),
            ..file.clone()
        };
        assert!(!is_expected(&windows, &windows_file));
    }

    #[test]
    fn the_new_executable_takes_the_place_of_the_old_one() {
        let folder = std::env::temp_dir().join(format!("lokal-update-test-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let executable = folder.join("lokal");
        fs::write(&executable, b"old").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();

        put_in_place(&executable, b"new").unwrap();

        assert_eq!(fs::read(&executable).unwrap(), b"new");
        assert_eq!(
            fs::metadata(&executable).unwrap().permissions().mode() & 0o777,
            0o755
        );
        // Nothing staged is left beside it.
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
        assert!(can_replace(&executable));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn two_checks_at_once_each_put_a_whole_file_in_place() {
        let folder = std::env::temp_dir().join(format!("lokal-update-race-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let executable = folder.join("lokal");
        fs::write(&executable, b"old").unwrap();
        // Big enough that the two writes overlap.
        let new: Vec<u8> = (0..8 * 1024 * 1024).map(|n| (n % 251) as u8).collect();

        let checks: Vec<_> = (0..2)
            .map(|_| {
                let (executable, new) = (executable.clone(), new.clone());
                thread::spawn(move || put_in_place(&executable, &new))
            })
            .collect();
        for check in checks {
            assert!(check.join().unwrap().is_ok());
        }

        assert!(fs::read(&executable).unwrap() == new);
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
        fs::remove_dir_all(&folder).unwrap();
    }
}
