//! What Kara was asked to show when it started.
//!
//! Kara is started three ways: from the menu (no argument), with a folder or
//! file from a terminal or another application (`kara-ui ~/Descargas`,
//! `kara-ui file:///home/ana/informe.pdf` — the `.desktop` file passes `%U`),
//! and by the session bus when another application calls
//! `org.freedesktop.FileManager1` and Kara is the file manager.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The flag the D-Bus service file starts Kara with.
pub const SERVICE_FLAG: &str = "--dbus-service";

/// The parsed command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Startup {
    /// The folder to open first.
    pub folder: Option<PathBuf>,
    /// Entries of that folder to select: a file given on the command line is
    /// shown in its folder, selected, the way «Show in folder» does.
    pub select: Vec<OsString>,
    /// Started by D-Bus activation: the window waits for the request that
    /// woke Kara up instead of showing the home folder first.
    pub service: bool,
}

/// Reads the arguments after the program name. Flags (`--e2e`, the service
/// flag) are not locations; the first argument that names an existing folder
/// or file wins, and one that names nothing is ignored rather than starting
/// on an empty view.
#[must_use]
pub fn parse(args: &[OsString], cwd: Option<&Path>) -> Startup {
    let mut startup = Startup::default();
    for arg in args {
        if arg == SERVICE_FLAG {
            startup.service = true;
            continue;
        }
        if arg.as_encoded_bytes().starts_with(b"-") || startup.folder.is_some() {
            continue;
        }
        let Some(path) = location(arg, cwd) else {
            continue;
        };
        if path.is_dir() {
            startup.folder = Some(path);
        } else if path.exists()
            && let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        {
            startup.folder = Some(parent.to_path_buf());
            startup.select = vec![name.to_os_string()];
        }
    }
    startup
}

/// A path or a `file://` URI, made absolute.
fn location(arg: &OsStr, cwd: Option<&Path>) -> Option<PathBuf> {
    let bytes = arg.as_encoded_bytes();
    let path = if bytes.starts_with(b"file://") {
        kara_fs::clipboard::path_from_uri(bytes)?
    } else if bytes.contains(&b':') && !Path::new(arg).exists() {
        // `smb://…` and other schemes Kara cannot browse directly.
        return None;
    } else {
        PathBuf::from(arg)
    };
    if path.is_absolute() {
        Some(path)
    } else {
        Some(cwd?.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn nothing_means_the_default_folder() {
        assert_eq!(parse(&[], None), Startup::default());
    }

    #[test]
    fn a_folder_opens_and_a_file_is_selected_in_its_folder() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("informe final.pdf");
        std::fs::write(&file, b"x")?;

        let folder = parse(&args(&[&dir.path().to_string_lossy()]), None);
        assert_eq!(folder.folder.as_deref(), Some(dir.path()));
        assert!(folder.select.is_empty());

        let uri = kara_fs::file_uri(&file);
        let shown = parse(&args(&[&uri]), None);
        assert_eq!(shown.folder.as_deref(), Some(dir.path()));
        assert_eq!(shown.select, [OsString::from("informe final.pdf")]);
        Ok(())
    }

    #[test]
    fn flags_are_not_locations_and_the_service_flag_is_noticed() {
        let startup = parse(&args(&["--e2e", SERVICE_FLAG]), None);
        assert!(startup.service);
        assert!(startup.folder.is_none());
    }

    #[test]
    fn a_relative_path_resolves_against_the_working_directory() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::create_dir(dir.path().join("sub"))?;
        let startup = parse(&args(&["sub"]), Some(dir.path()));
        assert_eq!(startup.folder, Some(dir.path().join("sub")));
        Ok(())
    }

    #[test]
    fn what_does_not_exist_or_is_remote_is_ignored() {
        assert!(parse(&args(&["/no/such/kara/place"]), None).folder.is_none());
        assert!(parse(&args(&["smb://nas/fotos"]), None).folder.is_none());
    }
}
