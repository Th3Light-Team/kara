//! Source rules for `src/sftp`: nothing that panics, nothing that prints, and
//! the secret handed to the protocol in exactly one place.

use std::fs;
use std::path::{Path, PathBuf};

const CRATE: &str = env!("CARGO_MANIFEST_DIR");

fn sources() -> Vec<PathBuf> {
    let dir = Path::new(CRATE).join("src/sftp");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e == "rs"));
    files.sort();
    files
}

/// (file, line number, text) of every code line; comment lines skipped.
fn code_lines() -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    for file in sources() {
        let text = fs::read_to_string(&file).unwrap_or_default();
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            out.push((name.clone(), n + 1, line.to_owned()));
        }
    }
    out
}

#[test]
fn every_module_is_scanned() {
    let names: Vec<String> = sources()
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    for expected in [
        "backend.rs",
        "connect.rs",
        "error.rs",
        "io.rs",
        "known_hosts.rs",
        "mod.rs",
        "params.rs",
        "session.rs",
    ] {
        assert!(names.iter().any(|n| n == expected), "{expected} not scanned: {names:?}");
    }
}

#[test]
fn no_unwrap_expect_or_panicking_macros() {
    const FORBIDDEN: [&str; 10] = [
        ".unwrap(",
        ".expect(",
        "panic!",
        "unreachable!",
        "todo!",
        "unimplemented!",
        "assert!",
        "assert_eq!",
        "assert_ne!",
        "debug_assert",
    ];
    let offending: Vec<String> = code_lines()
        .into_iter()
        .filter(|(_, _, line)| FORBIDDEN.iter().any(|f| line.contains(f)))
        .map(|(file, n, line)| format!("{file}:{n}: {}", line.trim()))
        .collect();
    assert!(offending.is_empty(), "forbidden in src/sftp:\n{}", offending.join("\n"));
}

#[test]
fn nothing_is_printed_or_logged() {
    const FORBIDDEN: [&str; 6] = ["println!", "eprintln!", "dbg!", "log::", "tracing::", "print!("];
    let offending: Vec<String> = code_lines()
        .into_iter()
        .filter(|(_, _, line)| FORBIDDEN.iter().any(|f| line.contains(f)))
        .map(|(file, n, line)| format!("{file}:{n}: {}", line.trim()))
        .collect();
    assert!(offending.is_empty(), "printing in src/sftp:\n{}", offending.join("\n"));
}

#[test]
fn the_secret_is_exposed_only_where_it_goes_to_the_protocol() {
    let uses: Vec<(String, usize)> = code_lines()
        .into_iter()
        .filter(|(_, _, line)| line.contains(".expose()"))
        .map(|(file, n, _)| (file, n))
        .collect();
    assert!(
        uses.iter().all(|(file, _)| file == "connect.rs"),
        "expose() outside connect.rs: {uses:?}"
    );
    // The password, and the passphrase of the key file.
    assert_eq!(uses.len(), 2, "{uses:?}");
}
