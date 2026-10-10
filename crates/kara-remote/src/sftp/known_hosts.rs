//! OpenSSH `known_hosts`: read, check, append. Never rewritten.
//!
//! Supported, as `ssh` reads them:
//!
//! - plain host patterns, comma separated, with `*`/`?` wildcards and `!`
//!   negation (`host`, `*.lan,!evil.lan`);
//! - the `[host]:port` form for ports other than 22;
//! - hashed entries `|1|<salt>|<hmac-sha1>` (`HashKnownHosts yes`);
//! - `@revoked` lines (a revoked key is refused, without a prompt);
//! - `@cert-authority` lines are skipped: host certificates are checked as
//!   plain keys, which is what `ssh` does without a matching CA line.
//!
//! A line that cannot be parsed is ignored, as `ssh` does: one bad line never
//! hides the others.
//!
//! The file is only ever **appended to**, and only after the user trusted a
//! new key and asked to remember it. A changed key is never written: the user
//! has to remove the old line themselves (`ssh-keygen -R host`).

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

use data_encoding::BASE64;
use hmac::{Hmac, KeyInit, Mac};
use russh::keys::ssh_key::{HashAlg, PublicKey};
use sha1::Sha1;

/// What `known_hosts` says about a key presented by a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyStatus {
    /// A line for this host has exactly this key.
    Trusted,
    /// No line for this host has a key of this type.
    Unknown,
    /// A line for this host has a different key of the same type: possible
    /// man-in-the-middle. `known` is that key's SHA-256 fingerprint.
    Changed { known: String },
    /// The key is marked `@revoked`.
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    None,
    Revoked,
    CertAuthority,
}

#[derive(Debug, Clone)]
struct Line {
    marker: Marker,
    patterns: String,
    key: PublicKey,
}

/// The parsed lines of one `known_hosts` file.
#[derive(Debug, Clone, Default)]
pub struct KnownHosts {
    lines: Vec<Line>,
}

/// The name a host is recorded under: `host`, or `[host]:port` when the port
/// is not 22. Lower case, as `ssh` records it.
#[must_use]
pub fn host_entry(host: &str, port: u16) -> String {
    let host = host.to_ascii_lowercase();
    if port == 22 {
        host
    } else {
        format!("[{host}]:{port}")
    }
}

/// The SHA-256 fingerprint `ssh` prints: `SHA256:<base64>`.
#[must_use]
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

impl KnownHosts {
    /// Reads `path`. A missing file is an empty list; any other read error is
    /// returned.
    pub fn load(path: &Path) -> io::Result<KnownHosts> {
        match fs::read(path) {
            Ok(bytes) => Ok(KnownHosts::parse(&String::from_utf8_lossy(&bytes))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(KnownHosts::default()),
            Err(error) => Err(error),
        }
    }

    /// Parses the text of a `known_hosts` file, skipping what it cannot read.
    #[must_use]
    pub fn parse(text: &str) -> KnownHosts {
        let lines = text.lines().filter_map(parse_line).collect();
        KnownHosts { lines }
    }

    /// What the file says about `key` presented by the host recorded as
    /// `entry` (see [`host_entry`]).
    #[must_use]
    pub fn check(&self, entry: &str, key: &PublicKey) -> HostKeyStatus {
        let matching: Vec<&Line> = self
            .lines
            .iter()
            .filter(|line| line.marker != Marker::CertAuthority && matches_host(&line.patterns, entry))
            .collect();
        let same = |line: &&&Line| line.key.key_data() == key.key_data();
        if matching
            .iter()
            .filter(|line| line.marker == Marker::Revoked)
            .any(|line| same(&line))
        {
            return HostKeyStatus::Revoked;
        }
        let plain = matching.iter().filter(|line| line.marker == Marker::None);
        if plain.clone().any(|line| same(&line)) {
            return HostKeyStatus::Trusted;
        }
        match plain
            .clone()
            .find(|line| line.key.algorithm() == key.algorithm())
        {
            Some(line) => HostKeyStatus::Changed {
                known: fingerprint(&line.key),
            },
            None => HostKeyStatus::Unknown,
        }
    }
}

fn parse_line(raw: &str) -> Option<Line> {
    let text = raw.trim();
    if text.is_empty() || text.starts_with('#') {
        return None;
    }
    let mut fields = text.split_whitespace();
    let mut first = fields.next()?;
    let marker = match first {
        "@revoked" => Marker::Revoked,
        "@cert-authority" => Marker::CertAuthority,
        _ => Marker::None,
    };
    if marker != Marker::None {
        first = fields.next()?;
    }
    let key_type = fields.next()?;
    let base64 = fields.next()?;
    let key = PublicKey::from_openssh(&format!("{key_type} {base64}")).ok()?;
    Some(Line {
        marker,
        patterns: first.to_owned(),
        key,
    })
}

/// Whether a comma-separated pattern list names `entry`. A negated pattern
/// that matches excludes the host whatever else matches.
fn matches_host(patterns: &str, entry: &str) -> bool {
    let mut matched = false;
    for pattern in patterns.split(',') {
        if let Some(negated) = pattern.strip_prefix('!') {
            if wildcard(&negated.to_ascii_lowercase(), entry) {
                return false;
            }
        } else if pattern.starts_with("|1|") {
            matched |= hashed_matches(pattern, entry);
        } else {
            matched |= wildcard(&pattern.to_ascii_lowercase(), entry);
        }
    }
    matched
}

/// `|1|<base64 salt>|<base64 HMAC-SHA1(salt, entry)>`.
fn hashed_matches(pattern: &str, entry: &str) -> bool {
    let mut parts = pattern.split('|').skip(2);
    let (Some(salt), Some(hash)) = (parts.next(), parts.next()) else {
        return false;
    };
    let (Ok(salt), Ok(hash)) = (BASE64.decode(salt.as_bytes()), BASE64.decode(hash.as_bytes()))
    else {
        return false;
    };
    match Hmac::<Sha1>::new_from_slice(&salt) {
        Ok(mac) => mac.chain_update(entry.as_bytes()).verify_slice(&hash).is_ok(),
        Err(_) => false,
    }
}

/// `*` matches any run, `?` any one character; everything else literally.
fn wildcard(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, ti));
                pi += 1;
            }
            Some(c) if *c == '?' || Some(c) == t.get(ti) => {
                pi += 1;
                ti += 1;
            }
            _ => match star {
                Some((star_p, star_t)) => {
                    pi = star_p + 1;
                    ti = star_t + 1;
                    star = Some((star_p, star_t + 1));
                }
                None => return false,
            },
        }
    }
    p.get(pi..).is_some_and(|rest| rest.iter().all(|c| *c == '*'))
}

/// Appends `entry key` to the file at `path`. Creates the directory (0700) and
/// the file (0600) when missing; never touches existing lines, and never
/// changes the mode of a file that is already there.
pub fn append(path: &Path, entry: &str, key: &PublicKey) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let openssh = key
        .to_openssh()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    // `<type> <base64>` without the comment.
    let mut fields = openssh.split_whitespace();
    let (Some(key_type), Some(base64)) = (fields.next(), fields.next()) else {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    };
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)?;
    let mut line = String::new();
    if file.seek(SeekFrom::End(-1)).is_ok() {
        let mut last = [0u8; 1];
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            line.push('\n');
        }
    }
    line.push_str(&format!("{entry} {key_type} {base64}\n"));
    file.write_all(line.as_bytes())?;
    file.sync_all()
}
