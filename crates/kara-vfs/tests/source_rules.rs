//! Rules about the crate's own sources and manifest.
//! Edge cases cb_15 (no panics) and cb_48 (layering and features).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const CRATE: &str = env!("CARGO_MANIFEST_DIR");

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

fn src_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    rust_files(&Path::new(CRATE).join("src"), &mut files);
    files.sort();
    files
}

/// Code lines of every source file: (file, line number, text), comments skipped.
fn code_lines() -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    for file in src_files() {
        let text = fs::read_to_string(&file).expect("read source");
        let rel = file
            .strip_prefix(CRATE)
            .unwrap_or(&file)
            .display()
            .to_string();
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            out.push((rel.clone(), n + 1, line.to_owned()));
        }
    }
    out
}

// cb_15 ---------------------------------------------------------------------

#[test]
fn cb_15_every_module_is_scanned() {
    let rels: BTreeSet<String> = src_files()
        .iter()
        .map(|f| {
            f.strip_prefix(CRATE)
                .expect("inside crate")
                .display()
                .to_string()
        })
        .collect();
    for required in [
        "src/lib.rs",
        "src/path.rs",
        "src/location.rs",
        "src/error.rs",
        "src/capabilities.rs",
        "src/backend.rs",
    ] {
        assert!(rels.contains(required), "{required} missing from {rels:?}");
    }
    assert!(
        rels.iter().any(|r| r.starts_with("src/memory/")),
        "memory module: {rels:?}"
    );
    assert!(
        rels.iter().any(|r| r.starts_with("src/conformance/")),
        "conformance module: {rels:?}"
    );
}

#[test]
fn cb_15_no_unwrap_expect_or_panicking_macros_in_src() {
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
    assert!(
        offending.is_empty(),
        "forbidden in kara-vfs/src:\n{}",
        offending.join("\n")
    );
}

#[test]
fn cb_15_scan_allows_the_non_panicking_unwrap_family() {
    // Guard against an over-eager scan: these must stay legal.
    for ok in [
        "x.unwrap_or(0)",
        "x.unwrap_or_else(f)",
        "x.unwrap_or_default()",
    ] {
        assert!(!ok.contains(".unwrap("), "{ok}");
    }
}

// cb_48 ---------------------------------------------------------------------

/// Keys of a TOML section, from a simple line scan (`name = …` / `name.workspace = …`).
fn section_keys(manifest: &str, section: &str) -> Vec<(String, String)> {
    let mut current = String::new();
    let mut out = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            current = line.trim_matches(|c| c == '[' || c == ']').to_owned();
            continue;
        }
        if current == section
            && let Some((key, value)) = line.split_once('=')
        {
            let key = key
                .trim()
                .trim_end_matches(".workspace")
                .trim_matches('"')
                .to_owned();
            out.push((key, value.trim().to_owned()));
        }
    }
    out
}

fn manifest() -> String {
    fs::read_to_string(Path::new(CRATE).join("Cargo.toml")).expect("kara-vfs Cargo.toml")
}

#[test]
fn cb_48_normal_dependencies_are_exactly_core_percent_encoding_thiserror() {
    let deps: BTreeSet<String> = section_keys(&manifest(), "dependencies")
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    let expected: BTreeSet<String> = ["kara-core", "percent-encoding", "thiserror"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(deps, expected);
    assert!(section_keys(&manifest(), "target.'cfg(unix)'.dependencies").is_empty());
    assert!(section_keys(&manifest(), "build-dependencies").is_empty());
}

#[test]
fn cb_48_features_default_empty_memory_and_conformance_opt_in() {
    let features: Vec<(String, String)> = section_keys(&manifest(), "features");
    let get = |name: &str| {
        features
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.replace(' ', ""))
            .unwrap_or_else(|| panic!("feature {name} missing: {features:?}"))
    };
    assert_eq!(get("default"), "[]");
    get("memory");
    get("conformance");
}

#[test]
fn cb_48_self_dev_dependency_enables_the_suite_for_plain_cargo_test() {
    let dev = section_keys(&manifest(), "dev-dependencies");
    let (_, value) = dev
        .iter()
        .find(|(k, _)| k == "kara-vfs")
        .unwrap_or_else(|| panic!("kara-vfs must dev-depend on itself: {dev:?}"));
    let compact = value.replace(' ', "");
    assert!(compact.contains("path=\".\""), "{value}");
    assert!(
        compact.contains("\"memory\"") && compact.contains("\"conformance\""),
        "{value}"
    );
}

#[test]
fn cb_48_only_the_backend_crates_depend_on_kara_vfs() {
    // Step 1 allowed nobody; kara-fs (LocalBackend), kara-ops, kara-remote
    // and kara-ui (drives panel) are the intended consumers since steps 2-6.
    const ALLOWED: [&str; 4] = ["kara-fs", "kara-ops", "kara-remote", "kara-ui"];
    let crates = Path::new(CRATE).parent().expect("crates dir");
    for entry in fs::read_dir(crates).expect("read crates") {
        let dir = entry.expect("entry").path();
        if dir
            .file_name()
            .is_some_and(|n| n == "kara-vfs" || ALLOWED.iter().any(|a| n == *a))
        {
            continue;
        }
        let toml = dir.join("Cargo.toml");
        if let Ok(text) = fs::read_to_string(&toml) {
            assert!(
                !text.contains("kara-vfs"),
                "{} mentions kara-vfs",
                toml.display()
            );
        }
    }
}

#[test]
fn cb_48_workspace_lists_the_crate() {
    let root = Path::new(CRATE)
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let text = fs::read_to_string(root.join("Cargo.toml")).expect("root manifest");
    assert!(text.contains("\"crates/kara-vfs\""), "member missing");
    let deps = section_keys(&text, "workspace.dependencies");
    let (_, value) = deps
        .iter()
        .find(|(k, _)| k == "kara-vfs")
        .expect("workspace dependency");
    assert!(
        value.replace(' ', "").contains("path=\"crates/kara-vfs\""),
        "{value}"
    );
}

#[test]
fn cb_48_no_protocol_code_runtime_threads_or_upper_layers_in_src() {
    const FORBIDDEN: [&str; 16] = [
        "async fn",
        ".await",
        "tokio",
        "russh",
        "object_store::",
        "oo7",
        "async_trait",
        "kara_fs",
        "kara_index",
        "kara_ops",
        "kara_ui",
        "thread::spawn",
        "println!",
        "eprintln!",
        "dbg!",
        "std::process",
    ];
    let offending: Vec<String> = code_lines()
        .into_iter()
        .filter(|(_, _, line)| FORBIDDEN.iter().any(|f| line.contains(f)))
        .map(|(file, n, line)| format!("{file}:{n}: {}", line.trim()))
        .collect();
    assert!(
        offending.is_empty(),
        "forbidden in kara-vfs/src:\n{}",
        offending.join("\n")
    );
}

#[test]
fn cb_48_crate_doc_names_the_contract_and_the_design_doc() {
    let lib = fs::read_to_string(Path::new(CRATE).join("src/lib.rs")).expect("lib.rs");
    assert!(lib.contains("remote-backend-contract-kara-vfs-foundation-location-remotep"));
    assert!(lib.contains("docs/remote-backends.md"));
}
