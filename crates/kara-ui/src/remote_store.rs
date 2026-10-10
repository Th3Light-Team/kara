//! Remote drives in `settings.conf`, next to what `Prefs` keeps there.
//!
//! Two parts of the window write that file: `Prefs` (view, pins, panel) and the
//! drives panel. Each used to hold its own copy and write all of it back, which
//! would make the last writer erase the other's sections. So the drives panel
//! does a read-modify-write of **only** its `[drive:*]` sections, and `Prefs`
//! carries the on-disk `[drive:*]` sections over when it saves
//! ([`carry_drives`]).
//!
//! An unreadable file is never overwritten: saving a drive then fails with the
//! reason, as `Prefs` promises for its own settings.

use std::path::Path;

use kara_fs::settings::{LoadOutcome, Settings, load, save};
use kara_remote::{DriveConfig, config};
use kara_vfs::DriveId;

const PREFIX: &str = "drive:";

/// Why a drive could not be written down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError(pub String);

impl std::error::Error for StoreError {}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Every stored drive, and a note per section that could not be used.
#[must_use]
pub fn load_configs(path: &Path) -> (Vec<DriveConfig>, Vec<String>) {
    let loaded = load(path);
    let mut problems = Vec::new();
    if let LoadOutcome::Corrupt { reason } = &loaded.outcome {
        problems.push(format!("No se pudieron leer las unidades remotas: {reason}"));
    }
    let (configs, notes) = config::load_all(&loaded.settings);
    problems.extend(notes);
    (configs, problems)
}

fn modify(path: &Path, change: impl FnOnce(&mut Settings)) -> Result<(), StoreError> {
    let loaded = load(path);
    if let LoadOutcome::Corrupt { reason } = loaded.outcome {
        return Err(StoreError(format!(
            "los ajustes no se pueden leer y no se sobrescriben ({reason})"
        )));
    }
    let mut settings = loaded.settings;
    change(&mut settings);
    save(path, &settings).map_err(|error| StoreError(error.to_string()))
}

/// Writes (or replaces) one drive.
pub fn save_config(path: &Path, drive: &DriveConfig) -> Result<(), StoreError> {
    modify(path, |settings| config::store(settings, drive))
}

/// Forgets one drive.
pub fn forget_config(path: &Path, id: &DriveId) -> Result<(), StoreError> {
    modify(path, |settings| {
        config::forget(settings, id);
    })
}

/// Copies the `[drive:*]` sections of `on_disk` into `target`, replacing
/// whatever `target` had under that prefix.
pub fn carry_drives(on_disk: &Settings, target: &mut Settings) {
    let stale: Vec<String> = target
        .sections()
        .keys()
        .filter(|name| name.starts_with(PREFIX))
        .cloned()
        .collect();
    for name in stale {
        target.remove_section(&name);
    }
    for (name, values) in on_disk.sections() {
        if name.starts_with(PREFIX) {
            for (key, value) in values {
                target.set(name, key, value.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(name: &str) -> DriveConfig {
        DriveConfig::new(
            "sftp",
            name,
            name,
            [(String::from("host"), String::from("h"))],
        )
        .expect("valid")
    }

    #[test]
    fn a_drive_survives_a_save_and_a_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.conf");
        save_config(&path, &drive("nas")).expect("save");
        let (drives, problems) = load_configs(&path);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(drives, [drive("nas")]);
    }

    #[test]
    fn saving_a_drive_keeps_every_other_section() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.conf");
        let mut other = Settings::new();
        other.set("window", "sidebar_width", "300");
        save(&path, &other).expect("seed");

        save_config(&path, &drive("nas")).expect("save");
        let reread = load(&path).settings;
        assert_eq!(reread.get("window", "sidebar_width"), Some("300"));
        assert_eq!(reread.get("drive:sftp:nas", "param.host"), Some("h"));
    }

    #[test]
    fn forgetting_removes_only_that_drive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.conf");
        save_config(&path, &drive("one")).expect("save");
        save_config(&path, &drive("two")).expect("save");
        forget_config(&path, &drive("one").id).expect("forget");
        let (drives, _) = load_configs(&path);
        assert_eq!(drives, [drive("two")]);
    }

    #[test]
    fn an_unreadable_file_is_not_overwritten() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.conf");
        std::fs::write(&path, [0xff, 0xfe, 0x00]).expect("write");
        assert!(save_config(&path, &drive("nas")).is_err());
        assert_eq!(std::fs::read(&path).expect("read"), [0xff, 0xfe, 0x00]);
        let (drives, problems) = load_configs(&path);
        assert!(drives.is_empty());
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn prefs_saving_does_not_erase_drives_added_since_it_loaded() {
        // Prefs loaded before the drive existed; the disk has it now.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.conf");
        let mut prefs_copy = Settings::new();
        prefs_copy.set("window", "sidebar_width", "250");
        save_config(&path, &drive("nas")).expect("save");

        carry_drives(&load(&path).settings, &mut prefs_copy);
        save(&path, &prefs_copy).expect("prefs save");

        let (drives, _) = load_configs(&path);
        assert_eq!(drives, [drive("nas")]);
        assert_eq!(load(&path).settings.get("window", "sidebar_width"), Some("250"));
    }

    #[test]
    fn carry_drives_drops_drives_forgotten_on_disk() {
        let mut stale = Settings::new();
        config::store(&mut stale, &drive("gone"));
        let on_disk = Settings::new();
        carry_drives(&on_disk, &mut stale);
        assert!(stale.sections().is_empty());
    }
}
