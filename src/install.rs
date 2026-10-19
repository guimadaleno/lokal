//! `lokal --install`: the executable puts itself in ~/.local/bin. That's for
//! one person and needs no root, which is also what lets it update itself
//! there. There's no installer program and no second file to download.
//! `lokal --uninstall` takes it away again.

use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::{config, menu, style, text};

pub fn install() -> Result<String, String> {
    let running = env::current_exe().map_err(|error| text::cant_install(&error.to_string()))?;
    let folder = bin_folder();
    let target = folder.join("lokal");
    if is_same_file(&running, &target) {
        return Ok(text::already_installed(&config::shown(&target)));
    }
    copy_over(&running, &target).map_err(|error| text::cant_install(&error.to_string()))?;
    let mut message = text::installed(&config::shown(&target));
    if !is_on_path(&folder) {
        message.push('\n');
        message.push_str(text::NOT_ON_PATH);
    }
    Ok(message)
}

/// Removes the installed executable, then asks before the person's own
/// files, starting on keeping them. False when the executable couldn't go.
pub fn uninstall() -> bool {
    let target = bin_folder().join("lokal");
    let shown = config::shown(&target);
    match fs::remove_file(&target) {
        Ok(()) => eprintln!("{}", text::uninstalled(&shown)),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("{}", style::gray(&text::not_installed(&shown)));
        }
        Err(error) => {
            eprintln!("{}", style::red(&text::cant_uninstall(&error.to_string())));
            return false;
        }
    }
    let files: Vec<PathBuf> = config::own_files()
        .into_iter()
        .filter(|file| file.exists())
        .collect();
    if files.is_empty() {
        return true;
    }
    eprintln!();
    eprintln!("{}", text::YOUR_FILES);
    for file in &files {
        eprintln!("  {}", style::gray(&config::shown(file)));
    }
    eprintln!();
    let items = [
        menu::Item {
            label: text::KEEP_THEM,
            key: 'k',
        },
        menu::Item {
            label: text::REMOVE_THEM,
            key: 'r',
        },
    ];
    match menu::choose(text::REMOVE_FILES_QUESTION, &items, 0) {
        Some(menu::Choice::Picked(1)) => remove(&files),
        Some(_) => eprintln!("{}", style::gray(&text::kept_files(&folders_of(&files)))),
        None => {
            eprintln!("{}", style::gray(text::NOT_ASKED));
            eprintln!("{}", style::gray(&text::kept_files(&folders_of(&files))));
        }
    }
    true
}

/// The files, then their folders where nothing else is left in them.
fn remove(files: &[PathBuf]) {
    let failed: Vec<String> = files
        .iter()
        .filter(|file| fs::remove_file(file).is_err())
        .map(|file| config::shown(file))
        .collect();
    if failed.is_empty() {
        eprintln!("{}", style::gray(text::REMOVED_FILES));
    } else {
        eprintln!("{}", style::red(&text::cant_remove(&failed)));
    }
    for folder in config::own_folders() {
        // Only an empty folder goes: anything else in it isn't Lokal's.
        if fs::remove_dir(&folder).is_err() && folder.exists() && failed.is_empty() {
            eprintln!(
                "{}",
                style::gray(&text::folder_stays(&config::shown(&folder)))
            );
        }
    }
}

/// The folders the files are in, each once, as the person would write them.
fn folders_of(files: &[PathBuf]) -> Vec<String> {
    let mut folders: Vec<String> = Vec::new();
    for folder in files.iter().filter_map(|file| file.parent()) {
        let folder = config::shown(folder);
        if !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    folders
}

fn bin_folder() -> PathBuf {
    config::home().join(".local/bin")
}

/// Written beside the target and renamed over it, so a Lokal that's running
/// from there keeps running, and a link that was there is replaced, not followed.
fn copy_over(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(folder) = to.parent() {
        fs::create_dir_all(folder)?;
    }
    let staged = to.with_file_name("lokal.new");
    fs::copy(from, &staged)?;
    fs::rename(&staged, to).inspect_err(|_| {
        let _ = fs::remove_file(&staged);
    })
}

/// Also true when the target is a link to the running file, as in a
/// development setup that points ~/.local/bin/lokal at a build.
fn is_same_file(running: &Path, target: &Path) -> bool {
    match (fs::canonicalize(running), fs::canonicalize(target)) {
        (Ok(running), Ok(target)) => running == target,
        _ => false,
    }
}

fn is_on_path(folder: &Path) -> bool {
    env::var_os("PATH").is_some_and(|path| env::split_paths(&path).any(|entry| entry == folder))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let folder = env::temp_dir().join(format!("lokal-install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn copies_over_a_link_without_touching_what_it_pointed_to() {
        let folder = scratch("link");
        let build = folder.join("build");
        fs::write(&build, b"development build").unwrap();
        let target = folder.join("bin/lokal");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&build, &target).unwrap();
        let release = folder.join("release");
        fs::write(&release, b"release").unwrap();

        assert!(is_same_file(&build, &target));
        copy_over(&release, &target).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"release");
        assert_eq!(fs::read(&build).unwrap(), b"development build");
        assert!(!fs::symlink_metadata(&target).unwrap().is_symlink());
        fs::remove_dir_all(&folder).unwrap();
    }
}
