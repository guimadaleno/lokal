//! A folder of the session's own for temporary files: a download to look
//! into, output to sift, a throwaway script. The model is told where it is,
//! so what's temporary stays out of the person's folders. Writing there asks
//! first, like anywhere else. It goes when Lokal quits, and one left by a
//! Lokal that was killed goes at the next start, so nothing piles up.

use std::env;
use std::fs::{self, DirBuilder};
use std::io::ErrorKind;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub struct Scratch {
    path: PathBuf,
}

impl Scratch {
    /// None when no private place could be made; then the model isn't told
    /// of one.
    pub fn create() -> Option<Scratch> {
        let uid = fs::metadata("/proc/self").ok()?.uid();
        let base = env::temp_dir().join(format!("lokal-{uid}"));
        match DirBuilder::new().mode(0o700).create(&base) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(_) => return None,
        }
        // The temporary folder is everyone's: a folder someone else made, or a
        // link planted in its place, would let them read or swap what's
        // written there. One of the person's own is closed to others, however
        // it was made; the temporary folder's sticky bit keeps it from being
        // swapped after this check.
        if !is_own_folder(&base, uid)
            || fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).is_err()
        {
            return None;
        }
        clear_left_behind(&base);
        let path = base.join(std::process::id().to_string());
        DirBuilder::new().mode(0o700).create(&path).ok()?;
        Some(Scratch { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A real folder, not a link to one, owned by `uid`.
fn is_own_folder(dir: &Path, uid: u32) -> bool {
    fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir() && meta.uid() == uid)
}

/// Removes the folders of sessions whose process is gone.
fn clear_left_behind(base: &Path) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().filter(|name| name.parse::<u32>().is_ok()) else {
            continue;
        };
        if !Path::new("/proc").join(pid).exists() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_real_folder_of_the_persons_own_will_do() {
        let uid = fs::metadata("/proc/self").unwrap().uid();
        let dir = env::temp_dir().join(format!("lokal-test-{}", std::process::id()));
        let link = env::temp_dir().join(format!("lokal-test-link-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(is_own_folder(&dir, uid));
        assert!(!is_own_folder(&dir, uid + 1));
        assert!(!is_own_folder(&link, uid));
        fs::remove_file(&link).unwrap();
        fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn only_folders_of_gone_sessions_are_cleared() {
        let base = env::temp_dir().join(format!("lokal-test-base-{}", std::process::id()));
        let alive = base.join(std::process::id().to_string());
        // Above the kernel's highest pid, so no process has it.
        let gone = base.join("99999999");
        let other = base.join("notes");
        for dir in [&alive, &gone, &other] {
            fs::create_dir_all(dir).unwrap();
        }
        clear_left_behind(&base);
        assert!(alive.exists() && other.exists() && !gone.exists());
        fs::remove_dir_all(&base).unwrap();
    }
}
