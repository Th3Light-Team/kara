//! Opening a file with the application the desktop has associated to it.
//!
//! Reference convenience: `ground/spec/06-contexto-power.md`, «Abrir con».
//!
//! Kara does not decide what opens a file: `xdg-open` consults the same MIME
//! associations Dolphin and Nautilus use, so the answer is whatever the user
//! has configured. `gio open` is the fallback on systems without `xdg-utils`.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// Why a file could not be handed to the desktop.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("{0} no existe")]
    Missing(String),
    #[error("no hay `xdg-open` ni `gio` para abrir {0}")]
    NoLauncher(String),
    #[error("no se pudo abrir {path}: {source}")]
    Launch { path: String, source: io::Error },
}

/// Hands `path` to the desktop's default application and returns without
/// waiting for it: the application outlives the call and the window must not
/// freeze while it starts.
pub fn open(path: &Path) -> Result<(), OpenError> {
    open_with(&[("xdg-open", &[]), ("gio", &["open"])], path)
}

/// Tries each `(program, leading args)` in turn, moving on only when the
/// program itself is not installed. Any other failure is reported as is: a
/// launcher that exists and fails is not a reason to try another one.
fn open_with(launchers: &[(&str, &[&str])], path: &Path) -> Result<(), OpenError> {
    let shown = path.display().to_string();
    match path.try_exists() {
        Ok(true) => {}
        Ok(false) => return Err(OpenError::Missing(shown)),
        Err(source) => return Err(OpenError::Launch { path: shown, source }),
    }

    for (program, args) in launchers {
        let spawned = Command::new(program)
            .args(*args)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                // Reaped on a thread so the child never lingers as a zombie;
                // its exit status says nothing useful about the application.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => return Err(OpenError::Launch { path: shown, source }),
        }
    }
    Err(OpenError::NoLauncher(shown))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_path_is_reported_before_any_launcher_runs() {
        let result = open_with(&[("true", &[])], Path::new("/no/such/kara/file"));
        assert!(matches!(result, Err(OpenError::Missing(_))));
    }

    #[test]
    fn the_first_installed_launcher_wins() {
        let result = open_with(&[("kara-no-such-launcher", &[]), ("true", &[])], Path::new("/"));
        assert!(result.is_ok());
    }

    #[test]
    fn no_installed_launcher_is_an_error_not_a_silent_success() {
        let result = open_with(&[("kara-no-such-launcher", &[])], Path::new("/"));
        assert!(matches!(result, Err(OpenError::NoLauncher(_))));
    }
}
