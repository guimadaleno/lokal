//! Facts about the machine, for finding the server and for the model.

use std::env;
use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::config;

pub struct Facts {
    pub os: String,
    pub wsl: bool,
    pub user: String,
    pub home: String,
    pub cwd: String,
    /// Under WSL: where the Windows Desktop and Documents folders are, as Linux paths.
    pub windows_folders: Vec<(&'static str, String)>,
    /// The session's folder for temporary files, when there is one (`scratch.rs`).
    pub scratch: Option<String>,
}

pub fn gather() -> Facts {
    let wsl = is_wsl();
    Facts {
        os: os_name(),
        wsl,
        user: env::var("USER").unwrap_or_else(|_| "unknown".into()),
        home: config::home().display().to_string(),
        cwd: env::current_dir()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default(),
        windows_folders: if wsl { windows_folders() } else { Vec::new() },
        scratch: None,
    }
}

pub fn is_wsl() -> bool {
    env::var_os("WSL_DISTRO_NAME").is_some()
        || fs::read_to_string("/proc/sys/kernel/osrelease")
            .is_ok_and(|release| release.contains("microsoft"))
}

/// Under WSL's default NAT networking, Windows is the default gateway. Its
/// address changes when WSL restarts, so it's looked up on every launch.
pub fn default_gateway() -> Option<Ipv4Addr> {
    gateway_in(&fs::read_to_string("/proc/net/route").ok()?)
}

/// The default route's gateway in /proc/net/route, written in hex, low byte first.
fn gateway_in(routes: &str) -> Option<Ipv4Addr> {
    for line in routes.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() > 2 && fields[1] == "00000000" {
            let gateway = u32::from_str_radix(fields[2], 16).ok()?;
            return Some(Ipv4Addr::from(gateway.to_le_bytes()));
        }
    }
    None
}

fn os_name() -> String {
    let release = fs::read_to_string("/etc/os-release").unwrap_or_default();
    release
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map(|name| name.trim_matches('"').to_string())
        .unwrap_or_else(|| "Linux".into())
}

/// Asks Windows where Desktop and Documents really are: OneDrive can move them.
fn windows_folders() -> Vec<(&'static str, String)> {
    let output = Command::new("reg.exe")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders",
        ])
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let Ok(listing) = String::from_utf8(output.stdout) else {
        return Vec::new();
    };
    let mut folders = Vec::new();
    for (value, label) in [
        ("Desktop", "Windows Desktop"),
        ("Personal", "Windows Documents"),
    ] {
        let path = listing.lines().find_map(|line| {
            let (name, path) = line.split_once("REG_SZ")?;
            (name.trim() == value).then(|| wsl_path(path.trim()))?
        });
        if let Some(path) = path {
            folders.push((label, path));
        }
    }
    folders
}

/// C:\Users\Ana\Desktop becomes /mnt/c/Users/Ana/Desktop, if that folder exists.
fn wsl_path(windows_path: &str) -> Option<String> {
    let path = mounted(windows_path)?;
    Path::new(&path).is_dir().then_some(path)
}

/// Where WSL mounts a Windows path, whether or not it exists.
fn mounted(windows_path: &str) -> Option<String> {
    let (drive, rest) = windows_path.split_once(":\\")?;
    if drive.len() != 1 {
        return None;
    }
    Some(format!(
        "/mnt/{}/{}",
        drive.to_ascii_lowercase(),
        rest.replace('\\', "/")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gateway_is_the_default_routes() {
        let routes = "Iface\tDestination\tGateway \tFlags\n\
                      eth0\t0030A8C0\t00000000\t0001\n\
                      eth0\t00000000\t0130A8C0\t0003\n";
        assert_eq!(gateway_in(routes), Some(Ipv4Addr::new(192, 168, 48, 1)));
        assert_eq!(gateway_in("Iface\tDestination\tGateway\n"), None);
    }

    #[test]
    fn windows_paths_map_to_their_mount() {
        assert_eq!(
            mounted(r"C:\Users\Ana\Desktop").as_deref(),
            Some("/mnt/c/Users/Ana/Desktop")
        );
        assert_eq!(mounted(r"\\server\share\Desktop"), None);
        assert_eq!(mounted("relative"), None);
    }
}
