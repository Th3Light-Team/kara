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
    let owned: Vec<(std::ffi::OsString, Vec<std::ffi::OsString>)> = launchers
        .iter()
        .map(|(program, args)| ((*program).into(), args.iter().map(Into::into).collect()))
        .collect();
    spawn_first(&owned, path, Some(path))
}

/// Runs the first launcher that is installed, in `cwd`, with `target` appended
/// as the last argument when there is one. `cwd` must exist.
fn spawn_first(
    launchers: &[(std::ffi::OsString, Vec<std::ffi::OsString>)],
    cwd: &Path,
    target: Option<&Path>,
) -> Result<(), OpenError> {
    let shown = cwd.display().to_string();
    match cwd.try_exists() {
        Ok(true) => {}
        Ok(false) => return Err(OpenError::Missing(shown)),
        Err(source) => return Err(OpenError::Launch { path: shown, source }),
    }

    for (program, args) in launchers {
        let mut command = Command::new(program);
        command.args(args);
        if let Some(target) = target {
            command.arg(target);
        } else {
            command.current_dir(cwd);
        }
        let spawned = command
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

/// Opens a terminal whose working directory is `directory`.
///
/// Reference convenience: `ground/spec/06-contexto-power.md`, «Abrir terminal
/// aquí». The user's own choice wins: `$TERMINAL` is tried first, then the
/// emulators of the common desktops, each told where to start in the way it
/// understands. Every launcher also gets `directory` as its working directory,
/// which is all the plain ones (`xterm`, `x-terminal-emulator`) need.
pub fn open_terminal(directory: &Path) -> Result<(), OpenError> {
    let dir = directory.as_os_str().to_os_string();
    let with = |flag: &str| -> Vec<std::ffi::OsString> {
        let mut args = vec![std::ffi::OsString::from(flag)];
        args.push(dir.clone());
        args
    };

    let mut launchers: Vec<(std::ffi::OsString, Vec<std::ffi::OsString>)> = Vec::new();
    if let Some(chosen) = std::env::var_os("TERMINAL").filter(|t| !t.is_empty()) {
        launchers.push((chosen, Vec::new()));
    }
    launchers.extend([
        ("konsole".into(), with("--workdir")),
        ("gnome-terminal".into(), with("--working-directory")),
        ("xfce4-terminal".into(), with("--working-directory")),
        ("kitty".into(), with("--directory")),
        ("alacritty".into(), with("--working-directory")),
        ("x-terminal-emulator".into(), Vec::new()),
        ("xterm".into(), Vec::new()),
    ]);
    spawn_first(&launchers, directory, None)
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

    #[test]
    fn a_terminal_that_is_not_installed_falls_through_to_the_next() {
        // The first one is not installed, so the next is the one that runs.
        let dir = std::env::temp_dir();
        let launchers = [("kara-no-such-terminal".into(), Vec::new()), ("true".into(), Vec::new())];
        assert!(spawn_first(&launchers, &dir, None).is_ok());
    }

    #[test]
    fn a_terminal_in_a_folder_that_is_gone_says_so() {
        let result = spawn_first(&[("true".into(), Vec::new())], Path::new("/no/such/kara/dir"), None);
        assert!(matches!(result, Err(OpenError::Missing(_))));
    }
}
