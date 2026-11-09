//! The tool lists the person has seen, a hash a server: a list that
//! changes isn't offered to the model until they've seen it again. The
//! file is in Lokal's state folder, which the model can't write.

use std::fs;
use std::io;

use sha2::{Digest, Sha256};

use crate::{config, text};

/// What a server tells the model, as a hash: the command that starts it and
/// the tools as they're offered, names, descriptions and parameters.
pub fn hash(command: &[String], offered: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(command.join("\0").as_bytes());
    hasher.update(b"\n");
    hasher.update(offered.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn is_seen(name: &str, hash: &str) -> bool {
    entries()
        .iter()
        .any(|(kept, seen)| kept == name && seen == hash)
}

/// Keeps `hash` as what the person has seen of `name`, in place of the last.
pub fn keep(name: &str, hash: &str) -> io::Result<()> {
    let mut entries = entries();
    entries.retain(|(kept, _)| kept != name);
    entries.push((name.to_string(), hash.to_string()));
    let mut contents = String::from(text::MCP_SEEN_HEADER);
    for (name, hash) in entries {
        contents.push_str(&format!("{name}\t{hash}\n"));
    }
    config::write(&config::mcp_seen_path(), &contents)
}

fn entries() -> Vec<(String, String)> {
    fs::read_to_string(config::mcp_seen_path())
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('\t'))
        .map(|(name, hash)| (name.to_string(), hash.to_string()))
        .collect()
}
