//! `mimeapps.list`, per the «MIME Applications Associations» specification
//! 1.0.1: which application is the default for a type, which ones the user
//! added or removed, and in which files, in which order, to look.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// One `mimeapps.list` (or legacy `defaults.list`), read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MimeAppsList {
    pub defaults: BTreeMap<String, Vec<String>>,
    pub added: BTreeMap<String, Vec<String>>,
    pub removed: BTreeMap<String, Vec<String>>,
}

const DEFAULTS: &str = "Default Applications";
const ADDED: &str = "Added Associations";
const REMOVED: &str = "Removed Associations";

/// Parses a `mimeapps.list`. Keys are MIME types, lower-cased; values are
/// desktop file IDs, in order.
#[must_use]
pub fn parse(text: &str) -> MimeAppsList {
    let mut list = MimeAppsList::default();
    let mut group: Option<&mut BTreeMap<String, Vec<String>>> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = match name {
                DEFAULTS => Some(&mut list.defaults),
                ADDED => Some(&mut list.added),
                REMOVED => Some(&mut list.removed),
                _ => None,
            };
            continue;
        }
        let (Some(map), Some((key, value))) = (group.as_deref_mut(), line.split_once('=')) else {
            continue;
        };
        let apps: Vec<String> = value
            .split(';')
            .map(str::trim)
            .filter(|app| !app.is_empty())
            .map(String::from)
            .collect();
        map.insert(key.trim().to_ascii_lowercase(), apps);
    }
    list
}

/// Every `mimeapps.list` to read, most important first:
/// for each config directory and then each data directory's `applications`,
/// the desktop-specific `<desktop>-mimeapps.list` before the plain one. The
/// data directories also carry the legacy `defaults.list`, still shipped by
/// distributions, read last within each directory.
#[must_use]
pub fn search_paths(config_dirs: &[PathBuf], data_dirs: &[PathBuf], desktops: &[String]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for dir in config_dirs {
        for desktop in desktops {
            paths.push(dir.join(format!("{desktop}-mimeapps.list")));
        }
        paths.push(dir.join("mimeapps.list"));
    }
    for dir in data_dirs {
        let apps = dir.join("applications");
        for desktop in desktops {
            paths.push(apps.join(format!("{desktop}-mimeapps.list")));
        }
        paths.push(apps.join("mimeapps.list"));
        paths.push(apps.join("defaults.list"));
    }
    paths
}

/// Makes `app` the default for `mime` in the user's `mimeapps.list`, and puts
/// it first among the added associations — what GIO's
/// `g_app_info_set_as_default_for_type` writes, so GNOME and Kara agree.
pub fn set_default(file: &Path, mime: &str, app: &str) -> io::Result<()> {
    let text = read_or_empty(file)?;
    let text = edit(&text, DEFAULTS, mime, |apps| {
        apps.clear();
        apps.push(app.to_string());
    });
    let text = edit(&text, ADDED, mime, |apps| move_to_front(apps, app));
    write_atomically(file, &text)
}

/// Records that `app` was just used for `mime`: it moves to the front of the
/// added associations, which is how GIO keeps «recently used»
/// (`g_app_info_set_as_last_used_for_type`). The default does not change.
pub fn remember_used(file: &Path, mime: &str, app: &str) -> io::Result<()> {
    let text = read_or_empty(file)?;
    let edited = edit(&text, ADDED, mime, |apps| move_to_front(apps, app));
    if edited == text {
        return Ok(());
    }
    write_atomically(file, &edited)
}

fn move_to_front(apps: &mut Vec<String>, app: &str) {
    apps.retain(|a| a != app);
    apps.insert(0, app.to_string());
}

fn read_or_empty(file: &Path) -> io::Result<String> {
    match std::fs::read_to_string(file) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

/// Changes one key of one group and leaves every other byte of the file as
/// it was: comments, other desktops' keys, unknown groups. The user's file is
/// shared with every other application on the desktop.
#[must_use]
pub fn edit(text: &str, group: &str, key: &str, change: impl FnOnce(&mut Vec<String>)) -> String {
    let header = format!("[{group}]");
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let key_lower = key.to_ascii_lowercase();

    let start = lines.iter().position(|l| l.trim() == header);
    let Some(start) = start else {
        let mut apps = Vec::new();
        change(&mut apps);
        let mut out = text.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.trim().is_empty() {
            out.push('\n');
        }
        out.push_str(&header);
        out.push('\n');
        out.push_str(&render(key, &apps));
        out.push('\n');
        return out;
    };

    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |offset| start + 1 + offset);

    let existing = (start + 1..end).find(|&i| {
        lines[i]
            .split_once('=')
            .is_some_and(|(k, _)| k.trim().to_ascii_lowercase() == key_lower)
    });

    match existing {
        Some(index) => {
            let mut apps: Vec<String> = lines[index]
                .split_once('=')
                .map(|(_, v)| v.split(';').map(str::trim).filter(|a| !a.is_empty()).map(String::from).collect())
                .unwrap_or_default();
            change(&mut apps);
            lines[index] = render(key, &apps);
        }
        None => {
            let mut apps = Vec::new();
            change(&mut apps);
            // After the group's last non-blank line, so a blank line that
            // separates it from the next group stays where it was.
            let mut at = end;
            while at > start + 1 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            lines.insert(at, render(key, &apps));
        }
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn render(key: &str, apps: &[String]) -> String {
    let mut line = format!("{key}=");
    for app in apps {
        line.push_str(app);
        line.push(';');
    }
    line
}

/// Writes through a temporary file in the same directory and a rename, so a
/// crash halfway leaves the old file and never half of the new one.
fn write_atomically(file: &Path, text: &str) -> io::Result<()> {
    let dir = file
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent directory"))?;
    std::fs::create_dir_all(dir)?;
    let name = file
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(name);
    temp_name.push(".kara-tmp");
    let temp = dir.join(temp_name);
    std::fs::write(&temp, text)?;
    if let Err(error) = std::fs::rename(&temp, file) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_groups_are_read_and_trailing_semicolons_are_optional() {
        let list = parse(
            "[Default Applications]\ntext/html=google-chrome.desktop\n\n[Added Associations]\nText/Plain=a.desktop;b.desktop;\n[Removed Associations]\nimage/png=c.desktop;\n[Other]\nx=y\n",
        );
        assert_eq!(list.defaults["text/html"], ["google-chrome.desktop"]);
        assert_eq!(list.added["text/plain"], ["a.desktop", "b.desktop"]);
        assert_eq!(list.removed["image/png"], ["c.desktop"]);
        assert!(!list.defaults.contains_key("x"));
    }

    #[test]
    fn desktop_specific_files_come_before_the_plain_ones_in_each_directory() {
        let paths = search_paths(
            &[PathBuf::from("/home/u/.config"), PathBuf::from("/etc/xdg")],
            &[PathBuf::from("/usr/share")],
            &["ubuntu".into(), "gnome".into()],
        );
        let shown: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(
            shown,
            [
                "/home/u/.config/ubuntu-mimeapps.list",
                "/home/u/.config/gnome-mimeapps.list",
                "/home/u/.config/mimeapps.list",
                "/etc/xdg/ubuntu-mimeapps.list",
                "/etc/xdg/gnome-mimeapps.list",
                "/etc/xdg/mimeapps.list",
                "/usr/share/applications/ubuntu-mimeapps.list",
                "/usr/share/applications/gnome-mimeapps.list",
                "/usr/share/applications/mimeapps.list",
                "/usr/share/applications/defaults.list",
            ]
        );
    }

    #[test]
    fn editing_one_key_keeps_everything_else_byte_for_byte() {
        let before = "# mine\n[Default Applications]\ntext/html=google-chrome.desktop\n\n[Added Associations]\nx-scheme-handler/opencode=ai.opencode.desktop.desktop;\n";
        let after = edit(before, "Default Applications", "inode/directory", |apps| apps.push("kara.desktop".into()));
        assert_eq!(
            after,
            "# mine\n[Default Applications]\ntext/html=google-chrome.desktop\ninode/directory=kara.desktop;\n\n[Added Associations]\nx-scheme-handler/opencode=ai.opencode.desktop.desktop;\n"
        );
    }

    #[test]
    fn an_existing_key_is_rewritten_in_place() {
        let before = "[Added Associations]\ntext/plain=a.desktop;b.desktop;\n";
        let after = edit(before, "Added Associations", "text/plain", |apps| move_to_front(apps, "b.desktop"));
        assert_eq!(after, "[Added Associations]\ntext/plain=b.desktop;a.desktop;\n");
    }

    #[test]
    fn a_missing_group_is_appended() {
        assert_eq!(
            edit("", "Default Applications", "text/plain", |a| a.push("e.desktop".into())),
            "[Default Applications]\ntext/plain=e.desktop;\n"
        );
        assert_eq!(
            edit("[Other]\nk=v", "Added Associations", "text/plain", |a| a.push("e.desktop".into())),
            "[Other]\nk=v\n\n[Added Associations]\ntext/plain=e.desktop;\n"
        );
    }

    #[test]
    fn setting_a_default_writes_both_groups_like_gio() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("config/mimeapps.list");
        set_default(&file, "inode/directory", "kara.desktop")?;
        let list = parse(&std::fs::read_to_string(&file)?);
        assert_eq!(list.defaults["inode/directory"], ["kara.desktop"]);
        assert_eq!(list.added["inode/directory"], ["kara.desktop"]);

        remember_used(&file, "inode/directory", "org.gnome.Nautilus.desktop")?;
        let list = parse(&std::fs::read_to_string(&file)?);
        assert_eq!(list.defaults["inode/directory"], ["kara.desktop"]);
        assert_eq!(list.added["inode/directory"], ["org.gnome.Nautilus.desktop", "kara.desktop"]);
        Ok(())
    }
}
