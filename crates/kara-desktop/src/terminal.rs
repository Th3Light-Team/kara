//! «Abrir terminal aquí».
//!
//! Reference convenience: `ground/spec/06-contexto-power.md`, «Abrir terminal
//! aquí»: «lanza la terminal predeterminada … Debe respetar la terminal
//! preferida del usuario».
//!
//! The user's preference is looked for in this order: `$TERMINAL`, which a
//! user sets by hand and means it; then `xdg-terminal-exec`, the Default
//! Terminal specification's launcher, which reads `xdg-terminals.list` and is
//! what Ubuntu and Fedora ship (on Ubuntu 26.04 GNOME it picks Ptyxis); and
//! only then a list of known emulators, each told where to start in the way
//! it understands. Konsole is one entry of that list, not an assumption.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::launch::{LaunchError, spawn_first};

/// Opens a terminal whose working directory is `directory`. Blocks for as
/// long as asking `xdg-terminal-exec` which terminal it would use takes:
/// call it off the UI thread.
pub fn open_terminal(directory: &Path) -> Result<(), LaunchError> {
    spawn_first(&launchers(directory, xdg_terminal_exec_works()), directory, None)
}

/// The command lines to try, in order. Every one also runs with `directory`
/// as its working directory, which is all the plain ones need.
#[must_use]
pub fn launchers(directory: &Path, with_xdg_terminal_exec: bool) -> Vec<(OsString, Vec<OsString>)> {
    let dir = directory.as_os_str().to_os_string();
    let flag_then_dir = |flag: &str| vec![OsString::from(flag), dir.clone()];
    let flag_with_dir = |flag: &str| {
        let mut joined = OsString::from(flag);
        joined.push("=");
        joined.push(&dir);
        vec![joined]
    };

    let mut out: Vec<(OsString, Vec<OsString>)> = Vec::new();
    if let Some(chosen) = std::env::var_os("TERMINAL").filter(|t| !t.is_empty()) {
        out.push((chosen, Vec::new()));
    }
    if with_xdg_terminal_exec {
        out.push(("xdg-terminal-exec".into(), flag_with_dir("--dir")));
    }
    out.extend([
        ("ptyxis".into(), {
            let mut args = vec![OsString::from("--new-window")];
            args.extend(flag_with_dir("--working-directory"));
            args
        }),
        ("kgx".into(), flag_with_dir("--working-directory")),
        ("gnome-terminal".into(), flag_with_dir("--working-directory")),
        ("konsole".into(), flag_then_dir("--workdir")),
        ("xfce4-terminal".into(), flag_with_dir("--working-directory")),
        ("mate-terminal".into(), flag_with_dir("--working-directory")),
        ("tilix".into(), flag_with_dir("--working-directory")),
        ("kitty".into(), flag_then_dir("--directory")),
        ("alacritty".into(), flag_then_dir("--working-directory")),
        ("foot".into(), flag_with_dir("--working-directory")),
        ("x-terminal-emulator".into(), Vec::new()),
        ("xterm".into(), Vec::new()),
    ]);
    out
}

/// A command line that runs `argv` inside a terminal, for applications whose
/// entry says `Terminal=true`.
#[must_use]
pub fn wrap(argv: &[OsString]) -> Vec<OsString> {
    let mut out: Vec<OsString> = if xdg_terminal_exec_works() {
        vec!["xdg-terminal-exec".into(), "--".into()]
    } else if crate::apps::find_program("ptyxis").is_some() {
        vec!["ptyxis".into(), "--new-window".into(), "--".into()]
    } else if crate::apps::find_program("gnome-terminal").is_some() {
        vec!["gnome-terminal".into(), "--".into()]
    } else if crate::apps::find_program("konsole").is_some() {
        vec!["konsole".into(), "-e".into()]
    } else {
        vec!["x-terminal-emulator".into(), "-e".into()]
    };
    out.extend(argv.iter().cloned());
    out
}

/// Whether `xdg-terminal-exec` is installed **and** has a terminal to run.
/// Installed but unconfigured, it would exit with an error after Kara has
/// already reported success; `--print-id` finds that out first. Versions
/// before `--print-id` existed also lack `--dir`, so they fail here too and
/// the known emulators are used instead.
fn xdg_terminal_exec_works() -> bool {
    Command::new("xdg-terminal-exec")
        .arg("--print-id")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|out| out.status.success() && !out.stdout.trim_ascii().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn programs(list: &[(OsString, Vec<OsString>)]) -> Vec<String> {
        list.iter().map(|(p, _)| p.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn xdg_terminal_exec_comes_before_any_named_emulator() {
        let list = launchers(Path::new("/srv/x"), true);
        let names = programs(&list);
        let xdg = names.iter().position(|p| p == "xdg-terminal-exec");
        let konsole = names.iter().position(|p| p == "konsole");
        assert!(xdg.is_some() && xdg < konsole);
        let (_, args) = &list[xdg.unwrap_or(0)];
        assert_eq!(args, &[OsString::from("--dir=/srv/x")]);
    }

    #[test]
    fn without_xdg_terminal_exec_ptyxis_leads_and_konsole_is_still_offered() {
        let names = programs(&launchers(Path::new("/srv/x"), false));
        assert!(!names.contains(&"xdg-terminal-exec".to_string()));
        assert!(names.iter().position(|p| p == "ptyxis") < names.iter().position(|p| p == "konsole"));
    }

    #[test]
    fn each_emulator_is_told_where_to_start_in_its_own_syntax() {
        let list = launchers(Path::new("/a b"), false);
        let args = |name: &str| {
            list.iter()
                .find(|(p, _)| p == name)
                .map(|(_, a)| a.iter().map(|s| s.to_string_lossy().into_owned()).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        assert_eq!(args("ptyxis"), ["--new-window", "--working-directory=/a b"]);
        assert_eq!(args("konsole"), ["--workdir", "/a b"]);
        assert_eq!(args("gnome-terminal"), ["--working-directory=/a b"]);
    }

    #[test]
    fn a_terminal_in_a_folder_that_is_gone_says_so() {
        let result = spawn_first(&launchers(Path::new("/no/such/kara/dir"), false), Path::new("/no/such/kara/dir"), None);
        assert!(matches!(result, Err(LaunchError::Missing(_))));
    }
}
