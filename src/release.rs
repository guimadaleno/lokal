//! The release manifest every app updates from: what a `latest.json` says,
//! whether to believe it, and whether it is newer.
//!
//! This file is guimadaleno/base's `release/client/release.rs`, copied into
//! each app unchanged; `release/README.md` there is the format and the rules.
//! It is pure: fetching, hashing the downloads and putting them in place are
//! the app's. What it decides is everything that comes before a single byte
//! is fetched on the manifest's word.

use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;

/// A file an installed copy puts in place of its own, as the release names it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct File {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// The part of a release an updater reads. The rest (`downloads`, `notes`,
/// `published`) is the page's.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Release {
    pub app: String,
    pub version: String,
    pub files: Vec<File>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not a signed release at all.
    Malformed,
    /// Not signed by this app's key.
    Signature,
    /// Signed, but for another app, or for none.
    App,
    /// Signed for this app, but naming something it never installs.
    Contents,
}

/// What an app is, to this file: its name, its key, the files it installs
/// (the first one is its executable, which every release must carry) and
/// the largest file it believes.
pub struct App<'a> {
    pub name: &'a str,
    pub key: &'a [u8; 32],
    pub files: &'a [&'a str],
    pub max_size: u64,
}

#[derive(Deserialize)]
struct Envelope {
    release: String,
    signature: String,
}

/// The release in a `latest.json`, when it is this app's and signed by its
/// key. Nothing inside `release` is read before the signature holds.
pub fn open(text: &[u8], app: &App) -> Result<Release, Refused> {
    let envelope: Envelope = serde_json::from_slice(text).map_err(|_| Refused::Malformed)?;
    let signature: [u8; 64] = unhex(&envelope.signature)
        .and_then(|b| b.try_into().ok())
        .ok_or(Refused::Malformed)?;
    // verify_strict refuses the small-order keys that let one signature hold
    // under more than one key. Unreachable with a compiled-in key, and free.
    let key = VerifyingKey::from_bytes(app.key).map_err(|_| Refused::Signature)?;
    key.verify_strict(envelope.release.as_bytes(), &Signature::from_bytes(&signature))
        .map_err(|_| Refused::Signature)?;

    #[derive(Deserialize)]
    struct Named {
        app: Option<String>,
    }
    let named: Named = serde_json::from_str(&envelope.release).map_err(|_| Refused::Malformed)?;
    if named.app.as_deref() != Some(app.name) {
        return Err(Refused::App);
    }
    let release: Release =
        serde_json::from_str(&envelope.release).map_err(|_| Refused::Malformed)?;
    if parse(&release.version).is_none() {
        return Err(Refused::Contents);
    }
    if !release.files.iter().any(|f| Some(&f.name.as_str()) == app.files.first()) {
        return Err(Refused::Contents);
    }
    for (i, f) in release.files.iter().enumerate() {
        let ok = app.files.contains(&f.name.as_str())
            && release.files[..i].iter().all(|g| g.name != f.name)
            && f.url.starts_with("https://")
            && f.size > 0
            && f.size <= app.max_size
            && f.sha256.len() == 64
            && f.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !ok {
            return Err(Refused::Contents);
        }
    }
    Ok(release)
}

/// `1.2.3` and `1.2.3-beta.4` in parts: the three numbers, and the
/// pre-release when there is one.
fn parse(v: &str) -> Option<([u64; 3], Option<&str>)> {
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let mut n = [0u64; 3];
    let mut parts = core.split('.');
    for slot in &mut n {
        let p = parts.next()?;
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = p.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    if let Some(pre) = pre {
        let ok = pre
            .split('.')
            .all(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()));
        if !ok {
            return None;
        }
    }
    Some((n, pre))
}

/// Two pre-releases, part by part: numbers by value (`alpha.10` after
/// `alpha.9`), words by their letters (`alpha`, `beta`, `rc`), a number
/// before a word, and the shorter first when one is the start of the other.
fn pre_order(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut x, mut y) = (a.split('.'), b.split('.'));
    loop {
        match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) => {
                let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => p.cmp(q),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

/// True when `candidate` comes after `current`. Refusing to go backwards is
/// what makes a replayed old manifest useless: it stays validly signed forever.
pub fn newer(candidate: &str, current: &str) -> bool {
    use std::cmp::Ordering;
    let (Some((a, ap)), Some((b, bp))) = (parse(candidate), parse(current)) else {
        return false;
    };
    match a.cmp(&b) {
        Ordering::Equal => match (ap, bp) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => pre_order(x, y) == Ordering::Greater,
            _ => false,
        },
        o => o == Ordering::Greater,
    }
}

/// `update.pub` as the key, when the executable is built:
/// `const KEY: [u8; 32] = release::hex32(include_str!("…/update.pub"));`
/// A malformed file is a compile error, not an updater that never updates.
pub const fn hex32(s: &str) -> [u8; 32] {
    let b = s.as_bytes();
    assert!(b.len() >= 64, "update.pub is shorter than 64 hex characters");
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = nibble(b[2 * i]) << 4 | nibble(b[2 * i + 1]);
        i += 1;
    }
    out
}

const fn nibble(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        _ => panic!("update.pub is not lowercase hex"),
    }
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    let text = text.as_bytes();
    if text.len() % 2 != 0 {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    text.chunks(2).map(|p| Some(digit(p[0])? << 4 | digit(p[1])?)).collect()
}

/// The vectors in guimadaleno/base's release/testdata/, which an app copies to
/// `tests/release/`. Every copy of this file reaches the same verdicts.
#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! vector {
        ($name:literal) => {
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/release/", $name))
        };
    }

    const KEY: [u8; 32] = hex32(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/release/test.pub"
    )));

    fn example() -> App<'static> {
        App { name: "example", key: &KEY, files: &["example.exe"], max_size: 8 * 1024 * 1024 }
    }

    #[test]
    fn the_vectors_are_accepted_and_refused_as_testdata_says() {
        let r = open(vector!("valid.json"), &example()).unwrap();
        assert_eq!(r.version, "1.2.0");
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files[0].url, "https://download.guimadaleno.com/example/1.2.0/example.exe");
        assert_eq!(open(vector!("prerelease.json"), &example()).unwrap().version, "1.2.0-beta.2");

        assert_eq!(open(vector!("tampered.json"), &example()), Err(Refused::Signature));
        assert_eq!(open(vector!("other-app.json"), &example()), Err(Refused::App));
        assert_eq!(open(vector!("no-app.json"), &example()), Err(Refused::App));
        assert_eq!(open(vector!("plain-http.json"), &example()), Err(Refused::Contents));
        assert_eq!(open(vector!("unknown-file.json"), &example()), Err(Refused::Contents));
        assert_eq!(open(vector!("short-signature.json"), &example()), Err(Refused::Malformed));
    }

    #[test]
    fn another_key_or_another_name_refuses_a_valid_release() {
        let other = [7u8; 32];
        let app = App { key: &other, ..example() };
        assert!(open(vector!("valid.json"), &app).is_err());
        let app = App { name: "other", ..example() };
        assert_eq!(open(vector!("valid.json"), &app), Err(Refused::App));
    }

    #[test]
    fn the_executable_must_be_there_and_within_bounds() {
        let app = App { files: &["other.exe", "example.exe"], ..example() };
        assert_eq!(open(vector!("valid.json"), &app), Err(Refused::Contents));
        let app = App { max_size: 1_000_000, ..example() };
        assert_eq!(open(vector!("valid.json"), &app), Err(Refused::Contents));
        assert_eq!(open(b"{}", &example()), Err(Refused::Malformed));
        assert_eq!(open(b"not json", &example()), Err(Refused::Malformed));
    }

    // The same table as release.py's tests.
    #[test]
    fn versions_compare() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(newer("0.2.0", "0.2.0-beta.3"));
        assert!(newer("0.2.0-beta.3", "0.2.0-beta.2"));
        assert!(newer("1.0.0-alpha.10", "1.0.0-alpha.9"));
        assert!(!newer("1.0.0-alpha.9", "1.0.0-alpha.10"));
        assert!(newer("1.0.0-beta.1", "1.0.0-alpha.12"));
        assert!(newer("1.0.0-rc.1", "1.0.0-beta.7"));
        assert!(newer("1.0.0", "1.0.0-rc.2"));
        assert!(newer("1.0.0-alpha.1", "0.0.1"));
        assert!(!newer("1.0.0-alpha.1", "1.0.0"));
        assert!(!newer("0.2.0", "0.2.0"));
        assert!(!newer("0.1.0", "0.2.0"));
        assert!(!newer("garbage", "0.1.0"));
        assert!(parse("1.0.0-alpha.1").is_some() && parse("2.3.4").is_some());
        for bad in ["1.0", "1.0.0-", "1.0.0-alpha..1", "1.0.0-al pha", "v1.0.0", "1.+2.0"] {
            assert!(parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_key_reads_from_update_pub() {
        assert_eq!(KEY[0], 0x03);
        assert_eq!(unhex("00ff10"), Some(vec![0, 255, 16]));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("AB"), None);
    }
}
