//! Starting things: a file with the application the desktop associates with
//! it, or with an application the user picked.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ashpd::desktop::open_uri::OpenFileRequest;
use futures_lite::future::block_on;
use rustix::fs::{Mode, OFlags};

use crate::apps::AppInfo;
use crate::apps::exec::{self, Context};

/// Why something could not be started. The messages are the ones the window
/// shows.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("{0} no existe")]
    Missing(String),
    #[error("no hay ninguna aplicación para abrir {0}")]
    NoLauncher(String),
    #[error("no se pudo abrir {path}: {source}")]
    Launch { path: String, source: io::Error },
    #[error("«{0}» no dice cómo arrancarse")]
    BadEntry(String),
}

/// Asks the OpenURI portal to open `path` with its default application.
///
/// Mind that the portal decides on its own whether to ask first: unless the
/// type has a single application or the user has picked the same one several
/// times, it shows its application chooser.
pub fn open_with_portal(path: &Path) -> Result<(), ashpd::Error> {
    // `O_PATH`, as GTK does: the portal needs to know the file, not to read it,
    // and this works on files Kara itself may not read.
    let fd = rustix::fs::open(path, OFlags::PATH | OFlags::CLOEXEC, Mode::empty())
        .map_err(|errno| ashpd::Error::IO(io::Error::from(errno)))?;
    block_on(async {
        let request = OpenFileRequest::default().ask(false).send_file(&fd).await?;
        request.response()
    })
}

/// Opens `path` with the desktop's default application through the desktop's
/// own command-line launchers: `gio open`, then `xdg-open`. Both consult the
/// same `mimeapps.list` Kara reads, and neither asks.
pub fn open_with_launchers(path: &Path) -> Result<(), LaunchError> {
    let launchers: [(OsString, Vec<OsString>); 2] = [
        ("gio".into(), vec!["open".into()]),
        ("xdg-open".into(), Vec::new()),
    ];
    spawn_first(&launchers, path, Some(path))
}

/// Starts `app` with `files`, as its `Exec` line says.
pub fn launch(app: &AppInfo, files: &[PathBuf]) -> Result<(), LaunchError> {
    let shown = || app.name.clone();
    let Some(exec_line) = app.exec.as_deref() else {
        if app.dbus_activatable {
            return activate_over_dbus(app, files);
        }
        return Err(LaunchError::BadEntry(shown()));
    };

    let context = Context {
        name: &app.name,
        icon: app.icon.as_deref(),
        desktop_file: Some(&app.source),
    };
    let commands = exec::expand(exec_line, files, &context).map_err(|_| LaunchError::BadEntry(shown()))?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let cwd = app.path.clone().filter(|p| p.is_dir()).or(home);

    for argv in commands {
        let argv = if app.terminal { crate::terminal::wrap(&argv) } else { argv };
        spawn(&argv, cwd.as_deref()).map_err(|source| match source.kind() {
            io::ErrorKind::NotFound => LaunchError::BadEntry(shown()),
            _ => LaunchError::Launch { path: shown(), source },
        })?;
    }
    Ok(())
}

/// `DBusActivatable=true` without an `Exec` line: the application is a D-Bus
/// service named after its desktop file ID (`org.freedesktop.Application`).
fn activate_over_dbus(app: &AppInfo, files: &[PathBuf]) -> Result<(), LaunchError> {
    let name = app.id.trim_end_matches(".desktop").to_string();
    let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
    let failed = |error: zbus::Error| LaunchError::Launch {
        path: app.name.clone(),
        source: io::Error::other(error),
    };
    let connection = zbus::blocking::Connection::session().map_err(failed)?;
    let platform_data: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
    let result = if files.is_empty() {
        connection.call_method(Some(name.as_str()), path.as_str(), Some("org.freedesktop.Application"), "Activate", &(platform_data,))
    } else {
        let uris: Vec<String> = files.iter().map(|f| kara_fs::file_uri(f)).collect();
        connection.call_method(Some(name.as_str()), path.as_str(), Some("org.freedesktop.Application"), "Open", &(uris, platform_data))
    };
    result.map(|_| ()).map_err(failed)
}

/// Starts a program and returns without waiting for it: the application
/// outlives the call and the window must not freeze while it starts.
pub(crate) fn spawn(argv: &[OsString], cwd: Option<&Path>) -> io::Result<()> {
    let Some((program, args)) = argv.split_first() else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty command"));
    };
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // The AppImage picks a Qt platform for Kara; an application Kara starts
    // must make its own choice, or a Qt one would land on XWayland too.
    if std::env::var_os("KARA_APPRUN_QPA").is_some() {
        command.env_remove("QT_QPA_PLATFORM").env_remove("KARA_APPRUN_QPA");
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    // Its own process group: a signal meant for Kara is not one for the
    // applications it started.
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn()?;
    // Reaped on a thread so the child never lingers as a zombie; its exit
    // status says nothing useful about the application.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Tries each `(program, leading args)` in turn, moving on only when the
/// program itself is not installed. Any other failure is reported as is: a
/// launcher that exists and fails is not a reason to try another one.
///
/// The child runs in `cwd`, which must exist; `target`, when there is one, is
/// appended as the last argument.
pub(crate) fn spawn_first(
    launchers: &[(OsString, Vec<OsString>)],
    cwd: &Path,
    target: Option<&Path>,
) -> Result<(), LaunchError> {
    let shown = cwd.display().to_string();
    match cwd.try_exists() {
        Ok(true) => {}
        Ok(false) => return Err(LaunchError::Missing(shown)),
        Err(source) => return Err(LaunchError::Launch { path: shown, source }),
    }
    let directory = if target.is_some() { cwd.parent().unwrap_or(cwd) } else { cwd };

    for (program, args) in launchers {
        let mut argv = Vec::with_capacity(args.len() + 2);
        argv.push(program.clone());
        argv.extend(args.iter().cloned());
        if let Some(target) = target {
            argv.push(target.as_os_str().to_os_string());
        }
        match spawn(&argv, Some(directory)) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => return Err(LaunchError::Launch { path: shown, source }),
        }
    }
    Err(LaunchError::NoLauncher(shown))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_path_is_reported_before_any_launcher_runs() {
        let result = spawn_first(&[("true".into(), Vec::new())], Path::new("/no/such/kara/file"), Some(Path::new("/no/such/kara/file")));
        assert!(matches!(result, Err(LaunchError::Missing(_))));
    }

    #[test]
    fn the_first_installed_launcher_wins() {
        let launchers = [("kara-no-such-launcher".into(), Vec::new()), ("true".into(), Vec::new())];
        assert!(spawn_first(&launchers, Path::new("/"), Some(Path::new("/"))).is_ok());
    }

    #[test]
    fn no_installed_launcher_is_an_error_not_a_silent_success() {
        let result = spawn_first(&[("kara-no-such-launcher".into(), Vec::new())], Path::new("/"), None);
        assert!(matches!(result, Err(LaunchError::NoLauncher(_))));
    }

    #[test]
    fn an_application_without_a_way_to_start_is_refused() {
        let app = AppInfo {
            id: "x.desktop".into(),
            name: "X".into(),
            generic_name: None,
            icon: None,
            exec: None,
            try_exec: None,
            path: None,
            terminal: false,
            no_display: false,
            dbus_activatable: false,
            only_show_in: Vec::new(),
            not_show_in: Vec::new(),
            mime_types: Vec::new(),
            source: PathBuf::new(),
        };
        assert!(matches!(launch(&app, &[]), Err(LaunchError::BadEntry(_))));
    }

    #[test]
    fn an_exec_that_is_not_installed_says_the_entry_is_broken() {
        let app = AppInfo {
            id: "y.desktop".into(),
            name: "Y".into(),
            generic_name: None,
            icon: None,
            exec: Some("kara-no-such-program %f".into()),
            try_exec: None,
            path: None,
            terminal: false,
            no_display: false,
            dbus_activatable: false,
            only_show_in: Vec::new(),
            not_show_in: Vec::new(),
            mime_types: Vec::new(),
            source: PathBuf::new(),
        };
        assert!(matches!(launch(&app, &[PathBuf::from("/tmp")]), Err(LaunchError::BadEntry(_))));
    }
}
