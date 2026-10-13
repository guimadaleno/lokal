//! Commands the person said not to ask about again in a project. They live in
//! the person's own settings folder, never in the project: a repository must
//! not be able to grant itself commands that run unasked. Exact command text
//! only, and the model can't write the file (`config::is_lokal_own`).

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::{config, text};

/// The file keeps the newest this many; nothing grows without bound.
const KEPT: usize = 500;

pub struct ProjectApprovals {
    project: PathBuf,
    commands: HashSet<String>,
}

impl ProjectApprovals {
    pub fn load(project: PathBuf) -> Self {
        let commands = read_entries()
            .into_iter()
            .filter(|(folder, _)| Path::new(folder) == project)
            .map(|(_, command)| command)
            .collect();
        ProjectApprovals { project, commands }
    }

    pub fn contains(&self, command: &str) -> bool {
        self.commands.contains(command)
    }

    pub fn count(&self) -> usize {
        self.commands.len()
    }

    pub fn project(&self) -> &Path {
        &self.project
    }

    /// A command can be kept if it fits on one line of the file.
    pub fn can_keep(&self, command: &str) -> bool {
        !command.contains(['\n', '\r', '\t']) && !self.project.to_string_lossy().contains('\t')
    }

    pub fn add(&mut self, command: &str) -> io::Result<()> {
        self.commands.insert(command.to_string());
        let mut entries = read_entries();
        entries.retain(|(folder, kept)| !(Path::new(folder) == self.project && kept == command));
        entries.push((
            self.project.to_string_lossy().into_owned(),
            command.to_string(),
        ));
        let skip = entries.len().saturating_sub(KEPT);
        let mut contents = String::from(text::APPROVALS_HEADER);
        for (folder, command) in entries.iter().skip(skip) {
            contents.push_str(&format!("{folder}\t{command}\n"));
        }
        config::write(&config::approvals_path(), &contents)
    }
}

fn read_entries() -> Vec<(String, String)> {
    fs::read_to_string(config::approvals_path())
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('\t'))
        .map(|(folder, command)| (folder.to_string(), command.to_string()))
        .collect()
}

/// The project a folder belongs to: the nearest one up with a `.git`, or the
/// folder itself.
pub fn project_root(folder: &Path) -> PathBuf {
    folder
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .unwrap_or(folder)
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_is_the_git_root_or_the_folder() {
        let base = std::env::temp_dir().join(format!("lokal-project-{}", std::process::id()));
        let inner = base.join("repo/src/deep");
        fs::create_dir_all(&inner).unwrap();
        fs::create_dir_all(base.join("repo/.git")).unwrap();
        fs::create_dir_all(base.join("loose")).unwrap();
        assert_eq!(project_root(&inner), base.join("repo"));
        assert_eq!(project_root(&base.join("loose")), base.join("loose"));
        fs::remove_dir_all(&base).unwrap();
    }
}
