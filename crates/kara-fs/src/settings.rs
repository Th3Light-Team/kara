//! On-disk settings store: a generic key-value box, not a place that knows
//! about views, folders or columns.
//!
//! # Format
//!
//! `[section]` headers and `key=value` lines, the same hand-rolled shape the
//! project already reads elsewhere (`user-dirs.dirs`, `index.theme`,
//! `.trashinfo`). No serialization crate: the whole point of keeping this
//! text-based is that it can be trimmed apart by hand and tested one edge
//! case at a time.
//!
//! # Design
//!
//! - **Chopping the text and touching the disk are two different things.**
//!   [`parse`] and [`serialize`] are pure functions over `&str`/[`Sections`];
//!   [`load`] and [`save`] are the only functions that do I/O, and they are
//!   thin wrappers around the pure pair. That split is where the tests live.
//! - **A file that cannot be read or decoded never takes the process down.**
//!   [`load`] has no `Result` at all: an absent file, a permission error, a
//!   directory where the file should be, or bytes that are not valid UTF-8
//!   all degrade to empty [`Settings`] plus a [`LoadOutcome`] that says so.
//!   [`load`] never writes; a corrupt file on disk stays exactly as it was
//!   until a caller calls [`save`] on purpose.
//! - **Resolving the path can fail for real** (no `$HOME`, nowhere to write),
//!   and that is kept separate from the above: [`default_path`] returns a
//!   [`SettingsError`] instead of guessing.
//! - **Parsing is permissive line by line**, the same policy
//!   `places::parse_user_dirs` uses: a line that does not fit the format is
//!   dropped and the rest of the file is still read, rather than failing the
//!   whole file over one bad line.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use thiserror::Error;

/// `section -> key -> value`. Both maps are `BTreeMap`s so that
/// [`serialize`]'s output is deterministic: saving the same settings twice
/// produces byte-identical files.
pub type Sections = BTreeMap<String, BTreeMap<String, String>>;

/// Failures that stop before a settings file is even attempted: there is
/// nowhere to look, or nowhere to write. Distinct from [`LoadOutcome::Corrupt`],
/// which is what happens once a path was found but its contents made no sense.
#[derive(Debug, Error)]
pub enum SettingsError {
    /// Neither `$XDG_CONFIG_HOME` (absolute) nor `$HOME` is set, so there is
    /// no base directory to put `kara/settings.conf` under.
    #[error("no home directory to resolve the settings path in")]
    NoHome,
    #[error("could not create the settings directory {dir}")]
    CreateDir {
        dir: PathBuf,
        source: std::io::Error,
    },
    #[error("could not write the settings file {path}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not replace the settings file {path}")]
    Rename {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// A key-value store, grouped into sections. What sections and keys exist is
/// entirely up to the caller; this type only knows how to hold and persist
/// strings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    sections: Sections,
}

impl Settings {
    #[must_use]
    pub fn new() -> Self {
        Settings::default()
    }

    /// Builds a `Settings` directly from an already-parsed map. Mostly useful
    /// for tests that want to compare [`load`]'s output against a value they
    /// built with [`parse`].
    #[must_use]
    pub fn from_sections(sections: Sections) -> Self {
        Settings { sections }
    }

    /// The underlying map, for callers that want to enumerate everything
    /// (e.g. list every pinned folder) rather than look up one key.
    #[must_use]
    pub fn sections(&self) -> &Sections {
        &self.sections
    }

    #[must_use]
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections.get(section)?.get(key).map(String::as_str)
    }

    pub fn set(&mut self, section: &str, key: &str, value: impl Into<String>) {
        self.sections
            .entry(section.to_string())
            .or_default()
            .insert(key.to_string(), value.into());
    }

    /// Removes one key. Drops the section entirely once it is left empty, so
    /// an emptied-out `Settings` serializes back to nothing rather than a
    /// dangling `[section]` header.
    /// Drops a whole section.
    ///
    /// Needed by any setting stored as a numbered list: rewriting it key by
    /// key would leave the stale tail of a longer previous list behind, and
    /// those entries would come back on the next load.
    pub fn remove_section(&mut self, section: &str) -> bool {
        self.sections.remove(section).is_some()
    }

    pub fn remove(&mut self, section: &str, key: &str) -> Option<String> {
        let values = self.sections.get_mut(section)?;
        let removed = values.remove(key);
        if values.is_empty() {
            self.sections.remove(section);
        }
        removed
    }
}

/// What happened when [`load`] tried to read a settings file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// Nothing on disk yet — a first run, or a settings key nobody has saved
    /// before. Not an error.
    Absent,
    /// Read and understood.
    Loaded,
    /// The file exists but could not be read (permission denied, a directory
    /// in its place...) or its bytes are not valid UTF-8. `settings` is
    /// empty and the file itself has not been touched: only an explicit
    /// [`save`] call will overwrite it.
    Corrupt { reason: String },
}

/// The result of [`load`]: never hidden behind a silent fallback — a caller
/// that ignores `outcome` still gets a usable (possibly empty) `settings`,
/// but the outcome is right there to surface in the UI or a log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSettings {
    pub settings: Settings,
    pub outcome: LoadOutcome,
}

/// The real settings path: `$XDG_CONFIG_HOME/kara/settings.conf` when
/// `XDG_CONFIG_HOME` is set and absolute, otherwise
/// `$HOME/.config/kara/settings.conf`.
pub fn default_path() -> Result<PathBuf, SettingsError> {
    resolve_path(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// The pure half of [`default_path`]: given the two environment variables it
/// cares about, works out the path with no I/O. Kept separate — and public —
/// so tests can drive every combination (unset, empty, relative, absolute)
/// without mutating the process's real environment, which `#[test]`s run in
/// parallel and share.
pub fn resolve_path(
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Result<PathBuf, SettingsError> {
    let base = xdg_config_home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| PathBuf::from(h).join(".config")))
        .ok_or(SettingsError::NoHome)?;
    Ok(base.join("kara").join("settings.conf"))
}

/// Reads and chops `path`. Cannot fail: every way a file can be unusable
/// (missing, unreadable, not UTF-8) turns into a [`LoadOutcome`] instead of
/// an `Err`, per the "a corrupt file can't take down startup" rule.
#[must_use]
pub fn load(path: &Path) -> LoadedSettings {
    match std::fs::read(path) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => LoadedSettings {
            settings: Settings::new(),
            outcome: LoadOutcome::Absent,
        },
        Err(source) => LoadedSettings {
            settings: Settings::new(),
            outcome: LoadOutcome::Corrupt {
                reason: source.to_string(),
            },
        },
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => LoadedSettings {
                settings: Settings::from_sections(parse(&text)),
                outcome: LoadOutcome::Loaded,
            },
            Err(_) => LoadedSettings {
                settings: Settings::new(),
                outcome: LoadOutcome::Corrupt {
                    reason: format!("{path:?} is not valid UTF-8"),
                },
            },
        },
    }
}

/// Serializes `settings` and writes it to `path` atomically: a temporary
/// file next to `path` (same directory, so the final `rename` stays on one
/// filesystem and is atomic), written and flushed in full, then renamed over
/// the real file. A crash or power loss can only ever leave the old file or
/// the fully-written new one — never a half-written `settings.conf`.
///
/// Creates `path`'s parent directories if they are missing.
pub fn save(path: &Path, settings: &Settings) -> Result<(), SettingsError> {
    let text = serialize(&settings.sections);
    write_atomically(path, text.as_bytes())
}

fn write_atomically(path: &Path, contents: &[u8]) -> Result<(), SettingsError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(|source| SettingsError::CreateDir {
        dir: dir.to_path_buf(),
        source,
    })?;

    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(path.file_name().unwrap_or_default());
    tmp_name.push(format!(".tmp.{}", std::process::id()));
    let tmp_path = dir.join(tmp_name);

    // Settings may include filesystem paths the user would rather not have
    // world-readable (pinned folders, a mounted network share). Restricting
    // to the owner costs nothing here — nothing reads this file but Kara
    // itself — so it is the safer default rather than trusting the umask.
    // `OpenOptions::mode` only *requests* 0o600 (the umask can still narrow
    // it further); `set_permissions` below forces the exact bits.
    let write_result = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp_path)
        .and_then(|mut file| {
            file.write_all(contents)?;
            file.sync_all()
        });

    if let Err(source) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(SettingsError::Write {
            path: tmp_path,
            source,
        });
    }

    if let Err(source) =
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))
    {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(SettingsError::Write {
            path: tmp_path,
            source,
        });
    }

    std::fs::rename(&tmp_path, path).map_err(|source| SettingsError::Rename {
        path: path.to_path_buf(),
        source,
    })
}

/// Chops the contents of a `settings.conf`.
///
/// Deliberately permissive, the same policy `places::parse_user_dirs` uses:
/// a line that does not fit the format — a stray `key=value` before any
/// `[section]`, a line with no `=` at all, an empty key, an empty or
/// unterminated `[section]` header — is dropped, and the rest of the file is
/// still read. Whether the *file as a whole* is unreadable or not valid
/// UTF-8 is [`load`]'s problem, not this function's: given a `&str`, parsing
/// always succeeds, if only into an empty map.
#[must_use]
pub fn parse(text: &str) -> Sections {
    let mut sections: Sections = BTreeMap::new();
    let mut current: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(name) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            let name = name.trim();
            current = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some(section) = current.as_deref() else {
            // A key=value line with no [section] above it has nowhere to
            // live; there is no implicit top-level section.
            continue;
        };
        // Split on the raw (untrimmed) line so that only the key gets
        // trimmed of surrounding whitespace; the value keeps everything
        // after the first `=` exactly as written, so a value that itself
        // contains `=` never needs escaping — only the first `=` is ever
        // the separator.
        let Some(eq_pos) = line.find('=') else {
            continue;
        };
        let key = line[..eq_pos].trim();
        if key.is_empty() {
            continue;
        }
        let value = unescape(&line[eq_pos + 1..]);

        sections
            .entry(section.to_string())
            .or_default()
            .insert(key.to_string(), value);
    }

    sections
}

/// Serializes a [`Sections`] map back to text. Sections and keys both come
/// out of a `BTreeMap`, so the output is deterministic, and [`parse`] undoes
/// exactly what this does: `parse(&serialize(sections)) == sections` for any
/// map this function can produce.
#[must_use]
pub fn serialize(sections: &Sections) -> String {
    let mut out = String::new();
    for (section, values) in sections {
        if values.is_empty() {
            // Nothing to say about a section with no keys; see `parse`,
            // which never records one in the first place.
            continue;
        }
        out.push('[');
        out.push_str(section);
        out.push_str("]\n");
        for (key, value) in values {
            out.push_str(key);
            out.push('=');
            out.push_str(&escape(value));
            out.push('\n');
        }
    }
    out
}

/// Escapes a value for storage on a single line: backslash itself and the
/// two control characters that would otherwise split or truncate the line.
/// Everything else — `=`, leading/trailing spaces, accented and other
/// non-ASCII text — passes through untouched.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// Undoes [`escape`]. An unrecognised `\X` sequence decodes to a literal `X`
/// with the backslash dropped, rather than failing: a hand-edited file with
/// a stray backslash should still load with everything else intact, the same
/// "permissive, never fail the whole file" policy as the rest of `parse`. A
/// trailing backslash with nothing after it is dropped the same way.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}
