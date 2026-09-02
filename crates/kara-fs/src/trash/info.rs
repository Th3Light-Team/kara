//! Lectura y escritura del fichero `.trashinfo` de FreeDesktop.
//!
//! El registro es lo que hace reversible la operación: ruta original y fecha
//! (`ground/spec/05-operaciones.md`, «Enviar a la papelera»).

use std::io::Read;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use percent_encoding::{AsciiSet, CONTROLS, percent_decode, percent_encode};

use super::error::TrashInfoError;

/// Extra bytes escaped on top of the ASCII control characters: space, `%`,
/// `#`, `?`, `[` and `]`. Every byte `>= 0x80` is escaped unconditionally by
/// `percent_encoding` because it only ever allows plain ASCII through.
const TRASH_INFO_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'%')
    .add(b'#')
    .add(b'?')
    .add(b'[')
    .add(b']');

const HEADER_LINE: &[u8] = b"[Trash Info]";
const PATH_KEY: &[u8] = b"Path=";
const DATE_KEY: &[u8] = b"DeletionDate=";

/// Local wall-clock deletion date, as FreeDesktop records it: no timezone
/// suffix, no offset. The UTC offset is injected by the caller so that the
/// conversion stays deterministic and testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeletionDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl DeletionDate {
    /// Converts a system time into local wall-clock components by applying
    /// `utc_offset_seconds`.
    pub fn from_system_time_local(
        time: SystemTime,
        utc_offset_seconds: i32,
    ) -> Result<DeletionDate, TrashInfoError> {
        let overflow = || TrashInfoError::MalformedDate {
            raw: "system time out of range".to_string(),
        };

        let epoch_seconds: i64 = match time.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(delta) => i64::try_from(delta.as_secs()).map_err(|_| overflow())?,
            Err(before_epoch) => {
                let secs =
                    i64::try_from(before_epoch.duration().as_secs()).map_err(|_| overflow())?;
                secs.checked_neg().ok_or_else(overflow)?
            }
        };

        let local_seconds = epoch_seconds
            .checked_add(i64::from(utc_offset_seconds))
            .ok_or_else(overflow)?;

        let days = local_seconds.div_euclid(86_400);
        let mut seconds_of_day = local_seconds.rem_euclid(86_400);

        let (year, month, day) = civil_from_days(days)?;
        let hour = seconds_of_day / 3_600;
        seconds_of_day -= hour * 3_600;
        let minute = seconds_of_day / 60;
        let second = seconds_of_day - minute * 60;

        let hour = u8::try_from(hour).map_err(|_| overflow())?;
        let minute = u8::try_from(minute).map_err(|_| overflow())?;
        let second = u8::try_from(second).map_err(|_| overflow())?;

        Ok(DeletionDate {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    /// Renders the date as `%Y-%m-%dT%H:%M:%S`, zero padded.
    pub fn format(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// Parses `%Y-%m-%dT%H:%M:%S`. Any suffix (including `Z`) is malformed.
    pub fn parse(raw: &str) -> Result<DeletionDate, TrashInfoError> {
        let malformed = || TrashInfoError::MalformedDate {
            raw: raw.to_string(),
        };

        let bytes = raw.as_bytes();
        if bytes.len() != 19 || !raw.is_ascii() {
            return Err(malformed());
        }
        if bytes[4] != b'-'
            || bytes[7] != b'-'
            || bytes[10] != b'T'
            || bytes[13] != b':'
            || bytes[16] != b':'
        {
            return Err(malformed());
        }

        let digits = |start: usize, len: usize| -> Result<i32, TrashInfoError> {
            let mut value: i32 = 0;
            for offset in 0..len {
                let byte = bytes[start + offset];
                if !byte.is_ascii_digit() {
                    return Err(malformed());
                }
                let digit = i32::from(byte - b'0');
                value = value
                    .checked_mul(10)
                    .and_then(|v| v.checked_add(digit))
                    .ok_or_else(malformed)?;
            }
            Ok(value)
        };

        let year = digits(0, 4)?;
        let month = digits(5, 2)?;
        let day = digits(8, 2)?;
        let hour = digits(11, 2)?;
        let minute = digits(14, 2)?;
        let second = digits(17, 2)?;

        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return Err(malformed());
        }

        Ok(DeletionDate {
            year,
            month: month as u8,
            day: day as u8,
            hour: hour as u8,
            minute: minute as u8,
            second: second as u8,
        })
    }
}

/// Howard Hinnant's `civil_from_days`, adapted to return an error instead of
/// panicking on overflow.
fn civil_from_days(days_since_epoch: i64) -> Result<(i32, u8, u8), TrashInfoError> {
    let overflow = || TrashInfoError::MalformedDate {
        raw: "date out of range".to_string(),
    };

    let z = days_since_epoch.checked_add(719_468).ok_or_else(overflow)?;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };

    let year = i32::try_from(y).map_err(|_| overflow())?;
    let month = u8::try_from(m).map_err(|_| overflow())?;
    let day = u8::try_from(d).map_err(|_| overflow())?;
    Ok((year, month, day))
}

/// Contents of a `.trashinfo` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashInfo {
    pub original_path: PathBuf,
    pub deletion_date: DeletionDate,
}

impl TrashInfo {
    /// Serializes the record. `relative_to` is `Some(top_dir)` for volume
    /// trashes, where `Path=` must be relative to the mount point, and `None`
    /// for the home trash, where it is absolute.
    pub fn serialize(&self, relative_to: Option<&Path>) -> Result<String, TrashInfoError> {
        let raw_bytes: Vec<u8> = match relative_to {
            Some(top) => match self.original_path.strip_prefix(top) {
                Ok(relative) => relative.as_os_str().as_bytes().to_vec(),
                // The caller is expected to pass a `top` that really is an
                // ancestor of `original_path`; silently falling back to the
                // absolute path here would write a `Path=` that looks
                // relative-trash-shaped but resolves somewhere else the next
                // time it is parsed with a `top_dir`. That is the caller's
                // bug to fix, not something to paper over.
                Err(_) => {
                    return Err(TrashInfoError::Io {
                        path: self.original_path.clone(),
                        source: std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "relative_to is not an ancestor of original_path",
                        ),
                    });
                }
            },
            None => self.original_path.as_os_str().as_bytes().to_vec(),
        };

        let encoded = percent_encode(&raw_bytes, TRASH_INFO_ENCODE_SET).to_string();
        let date = self.deletion_date.format();
        Ok(format!(
            "[Trash Info]\nPath={encoded}\nDeletionDate={date}\n"
        ))
    }

    /// Parses the raw bytes of a `.trashinfo`. `top_dir` is `Some` when the
    /// entry belongs to a volume trash, so a relative `Path=` can be rebuilt
    /// into an absolute one.
    pub fn parse(bytes: &[u8], top_dir: Option<&Path>) -> Result<TrashInfo, TrashInfoError> {
        let mut lines = bytes.split(|&b| b == b'\n');

        let header = match lines.next() {
            Some(line) => line,
            None => return Err(TrashInfoError::MissingHeader),
        };
        if header != HEADER_LINE {
            return Err(TrashInfoError::MissingHeader);
        }

        let mut path_value: Option<&[u8]> = None;
        let mut date_value: Option<&[u8]> = None;
        for line in lines {
            if path_value.is_none() && line.starts_with(PATH_KEY) {
                path_value = Some(&line[PATH_KEY.len()..]);
            } else if date_value.is_none() && line.starts_with(DATE_KEY) {
                date_value = Some(&line[DATE_KEY.len()..]);
            }
        }

        let path_value = match path_value {
            Some(value) if !value.is_empty() => value,
            _ => return Err(TrashInfoError::MissingPath),
        };
        let date_value = match date_value {
            Some(value) => value,
            None => return Err(TrashInfoError::MissingDeletionDate),
        };

        let path_str = match std::str::from_utf8(path_value) {
            Ok(text) => text,
            Err(_) => {
                return Err(TrashInfoError::InvalidPercentEncoding {
                    raw: String::from_utf8_lossy(path_value).into_owned(),
                });
            }
        };
        validate_percent_encoding(path_str)?;
        let decoded: Vec<u8> = percent_decode(path_value).collect();
        let decoded_os = std::ffi::OsString::from_vec(decoded);

        let original_path = match top_dir {
            Some(top) => {
                let relative = PathBuf::from(decoded_os);
                // A shared volume trash is writable by other processes: a
                // hostile or corrupted `.trashinfo` could carry an absolute
                // `Path=` (which `Path::join` would substitute wholesale,
                // discarding `top` entirely) or one with `..` components
                // that walks back out of `top`. Either would point a
                // `restore_item` call anywhere on the filesystem.
                if relative.is_absolute()
                    || relative.components().any(|component| {
                        matches!(
                            component,
                            std::path::Component::ParentDir | std::path::Component::RootDir
                        )
                    })
                {
                    return Err(TrashInfoError::NotAbsolute {
                        raw: path_str.to_string(),
                    });
                }
                top.join(relative)
            }
            None => {
                let candidate = PathBuf::from(decoded_os);
                // The home trash is 0700, so a hostile `.trashinfo` here
                // would have to come from the user's own account — but a
                // stale or hand-edited one is still plausible, and this is
                // also what `restore_item` trusts to build the destination
                // path. A `..` component is rejected the same way the
                // volume-trash branch above rejects one, rather than only
                // requiring the path to start with `/`.
                if !candidate.is_absolute()
                    || candidate
                        .components()
                        .any(|component| component == std::path::Component::ParentDir)
                {
                    return Err(TrashInfoError::NotAbsolute {
                        raw: path_str.to_string(),
                    });
                }
                candidate
            }
        };

        let date_str = match std::str::from_utf8(date_value) {
            Ok(text) => text,
            Err(_) => {
                return Err(TrashInfoError::MalformedDate {
                    raw: String::from_utf8_lossy(date_value).into_owned(),
                });
            }
        };
        let deletion_date = DeletionDate::parse(date_str)?;

        Ok(TrashInfo {
            original_path,
            deletion_date,
        })
    }
}

fn validate_percent_encoding(raw: &str) -> Result<(), TrashInfoError> {
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let has_two_more = index + 3 <= bytes.len();
            let valid =
                has_two_more && is_hex_digit(bytes[index + 1]) && is_hex_digit(bytes[index + 2]);
            if !valid {
                return Err(TrashInfoError::InvalidPercentEncoding {
                    raw: raw.to_string(),
                });
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn is_hex_digit(byte: u8) -> bool {
    byte.is_ascii_hexdigit()
}

/// Upper bound on the bytes read from a single `.trashinfo`: a well-formed
/// record is `[Trash Info]` plus two short lines, a few hundred bytes at
/// most. The trash directory (especially a volume one) is shared with other
/// processes, so this must never grow proportionally to whatever some other
/// writer left there (cb_30).
const MAX_TRASH_INFO_BYTES: u64 = 64 * 1024;

/// Reads and parses a `.trashinfo` file from disk. Refuses to read anything
/// that is not a regular file (a FIFO or device dropped into `info/` by
/// another process must not be opened for reading), and reads at most
/// [`MAX_TRASH_INFO_BYTES`]: exceeding that is reported as a typed error
/// rather than allocating memory proportional to an arbitrary file.
pub fn read_trash_info(info_path: &Path) -> Result<TrashInfo, TrashInfoError> {
    let metadata = std::fs::symlink_metadata(info_path).map_err(|source| TrashInfoError::Io {
        path: info_path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(TrashInfoError::Io {
            path: info_path.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a regular file",
            ),
        });
    }

    let file = std::fs::File::open(info_path).map_err(|source| TrashInfoError::Io {
        path: info_path.to_path_buf(),
        source,
    })?;
    let mut limited = file.take(MAX_TRASH_INFO_BYTES.saturating_add(1));
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(|source| TrashInfoError::Io {
            path: info_path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > MAX_TRASH_INFO_BYTES {
        return Err(TrashInfoError::Io {
            path: info_path.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "trash info file exceeds the documented size limit",
            ),
        });
    }

    let top_dir = infer_top_dir(info_path);
    TrashInfo::parse(&bytes, top_dir.as_deref())
}

/// Recovers the volume `top_dir` (if any) purely from the shape of
/// `info_path`, without touching the filesystem: `.../<topdir>/.Trash-<uid>/info/…`
/// or `.../<topdir>/.Trash/<uid>/info/…` are volume trashes; anything else
/// (in particular the home trash's `.../Trash/info/…`) is not.
fn infer_top_dir(info_path: &Path) -> Option<PathBuf> {
    let info_dir = info_path.parent()?;
    if info_dir.file_name()?.to_str()? != "info" {
        return None;
    }
    let root = info_dir.parent()?;
    let root_name = root.file_name()?.to_str()?;

    if root_name == "Trash" {
        return None;
    }

    if let Some(rest) = root_name.strip_prefix(".Trash-")
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return root.parent().map(Path::to_path_buf);
    }

    if !root_name.is_empty() && root_name.bytes().all(|b| b.is_ascii_digit()) {
        let dot_trash = root.parent()?;
        if dot_trash.file_name()?.to_str()? == ".Trash" {
            return dot_trash.parent().map(Path::to_path_buf);
        }
    }

    None
}
