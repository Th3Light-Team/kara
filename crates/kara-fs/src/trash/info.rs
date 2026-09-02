//! Lectura y escritura del fichero `.trashinfo` de FreeDesktop.
//!
//! El registro es lo que hace reversible la operación: ruta original y fecha
//! (`ground/spec/05-operaciones.md`, «Enviar a la papelera»).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::error::TrashInfoError;

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
        let _ = (time, utc_offset_seconds);
        todo!("DeletionDate::from_system_time_local")
    }

    /// Renders the date as `%Y-%m-%dT%H:%M:%S`, zero padded.
    pub fn format(&self) -> String {
        let _ = self;
        todo!("DeletionDate::format")
    }

    /// Parses `%Y-%m-%dT%H:%M:%S`. Any suffix (including `Z`) is malformed.
    pub fn parse(raw: &str) -> Result<DeletionDate, TrashInfoError> {
        let _ = raw;
        todo!("DeletionDate::parse")
    }
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
        let _ = (self, relative_to);
        todo!("TrashInfo::serialize")
    }

    /// Parses the raw bytes of a `.trashinfo`. `top_dir` is `Some` when the
    /// entry belongs to a volume trash, so a relative `Path=` can be rebuilt
    /// into an absolute one.
    pub fn parse(bytes: &[u8], top_dir: Option<&Path>) -> Result<TrashInfo, TrashInfoError> {
        let _ = (bytes, top_dir);
        todo!("TrashInfo::parse")
    }
}

/// Reads and parses a `.trashinfo` file from disk.
pub fn read_trash_info(info_path: &Path) -> Result<TrashInfo, TrashInfoError> {
    let _ = info_path;
    todo!("read_trash_info")
}
