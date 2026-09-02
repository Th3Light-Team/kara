//! The on-disk settings store: chopping/serializing `settings.conf` text, and
//! the disk-touching behaviour around it (atomic writes, missing/corrupt
//! files, path resolution).

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use kara_fs::settings::{LoadOutcome, Sections, Settings, SettingsError, load, parse, resolve_path, save, serialize};
use tempfile::TempDir;

fn section(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn sections(pairs: &[(&str, &[(&str, &str)])]) -> Sections {
    pairs
        .iter()
        .map(|(name, kv)| (name.to_string(), section(kv)))
        .collect()
}

// --- parsing: pure, permissive, one bad line never sinks the file ---------

#[test]
fn comments_and_blank_lines_are_ignored() {
    // Both comment lines below look exactly like `key=value` on their own
    // (`generated=by kara`, `another=comment`); if `#` stopped being treated
    // as a comment marker they would silently turn into real settings.
    let text = "# generated=by kara\n\n[view]\n\n# another=comment\nmode=details\n\n";
    assert_eq!(parse(text), sections(&[("view", &[("mode", "details")])]));
}

#[test]
fn only_the_first_equals_sign_splits_key_from_value() {
    // A saved query or a filter expression is a realistic value that itself
    // contains `=`; if the parser split on every `=` it would truncate it.
    let text = "[filters]\nexpr=size>=1024 && name=foo\n";
    let parsed = parse(text);
    assert_eq!(
        parsed.get("filters").and_then(|s| s.get("expr")),
        Some(&"size>=1024 && name=foo".to_string())
    );
}

#[test]
fn an_empty_value_is_a_real_key_with_an_empty_string_not_a_missing_key() {
    let text = "[panel]\nfilter=\n";
    let settings = Settings::from_sections(parse(text));
    assert_eq!(settings.get("panel", "filter"), Some(""));
}

#[test]
fn a_repeated_key_keeps_the_last_value() {
    let text = "[view]\nmode=list\nmode=icons\n";
    let settings = Settings::from_sections(parse(text));
    assert_eq!(settings.get("view", "mode"), Some("icons"));
}

#[test]
fn a_key_line_before_any_section_header_is_discarded() {
    // There is no implicit top-level section: a key with nowhere to live is
    // dropped instead of inventing a home for it.
    let text = "stray=value\n[view]\nmode=list\n";
    let parsed = parse(text);
    assert_eq!(parsed.len(), 1);
    assert_eq!(
        parsed.get("view").and_then(|s| s.get("mode")),
        Some(&"list".to_string())
    );
}

#[test]
fn an_empty_or_unterminated_section_header_is_discarded() {
    let text = "[]\nkey=value\n[unterminated\nother=value\n";
    assert!(parse(text).is_empty());
}

#[test]
fn a_line_with_no_equals_sign_is_skipped_without_sinking_the_rest_of_the_file() {
    let text = "[view]\ngarbage line with no separator\nmode=list\n";
    let settings = Settings::from_sections(parse(text));
    assert_eq!(settings.get("view", "mode"), Some("list"));
}

// --- escaping: values must survive whatever a real path throws at them ----

#[test]
fn a_value_with_a_newline_survives_the_round_trip() {
    // A filename can legally contain a literal newline byte on Linux; only
    // `/` and NUL are forbidden.
    let mut settings = Settings::new();
    settings.set("pinned", "0", "Weird\nfolder name");
    let recovered = Settings::from_sections(parse(&serialize(settings.sections())));
    assert_eq!(recovered.get("pinned", "0"), Some("Weird\nfolder name"));
}

#[test]
fn a_value_with_a_backslash_survives_the_round_trip() {
    let mut settings = Settings::new();
    settings.set("pinned", "0", r"C:\odd\windows\looking\path");
    let recovered = Settings::from_sections(parse(&serialize(settings.sections())));
    assert_eq!(
        recovered.get("pinned", "0"),
        Some(r"C:\odd\windows\looking\path")
    );
}

#[test]
fn a_value_with_spaces_and_accents_survives_the_round_trip() {
    let mut settings = Settings::new();
    settings.set("pinned", "0", "Área de descargas compartida");
    let recovered = Settings::from_sections(parse(&serialize(settings.sections())));
    assert_eq!(
        recovered.get("pinned", "0"),
        Some("Área de descargas compartida")
    );
}

#[test]
fn an_unrecognised_escape_sequence_decodes_to_the_literal_character() {
    // A hand-edited file with a stray backslash should still load with
    // everything else intact rather than failing the whole file.
    let text = "[view]\nkey=a\\qb\n";
    let settings = Settings::from_sections(parse(text));
    assert_eq!(settings.get("view", "key"), Some("aqb"));
}

#[test]
fn serialize_then_parse_recovers_the_exact_same_data() {
    let original = sections(&[
        ("panel", &[("width", "260"), ("show_hidden", "true")]),
        ("pinned", &[("0", "/home/ana/Proyectos"), ("1", "Área\ncon salto")]),
    ]);
    let recovered = parse(&serialize(&original));
    assert_eq!(recovered, original);
}

#[test]
fn saving_the_same_settings_twice_produces_byte_identical_text() {
    // BTreeMap ordering makes serialization deterministic: this is what lets
    // an on-disk settings.conf sit still under version control / diffing.
    let data = sections(&[("view", &[("mode", "list")])]);
    assert_eq!(serialize(&data), serialize(&data));
}

// --- disk: atomic writes, absent/corrupt files never crash startup --------

#[test]
fn loading_a_missing_file_yields_absent_and_empty_settings() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("kara").join("settings.conf");

    let loaded = load(&path);

    assert_eq!(loaded.outcome, LoadOutcome::Absent);
    assert_eq!(loaded.settings, Settings::new());
}

#[test]
fn save_then_load_round_trips_through_disk() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("kara").join("settings.conf");

    let mut settings = Settings::new();
    settings.set("view", "mode", "tiles");
    settings.set("pinned", "0", "/home/ana/Música");
    save(&path, &settings).expect("save must succeed");

    let loaded = load(&path);
    assert_eq!(loaded.outcome, LoadOutcome::Loaded);
    assert_eq!(loaded.settings, settings);
}

#[test]
fn save_creates_missing_parent_directories() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("kara").join("nested").join("settings.conf");

    save(&path, &Settings::new()).expect("save must create the parent directories");

    assert!(path.parent().expect("has a parent").is_dir());
}

#[test]
fn the_saved_file_is_owner_only_readable() {
    // Settings can hold paths the user would rather not have world-readable
    // (pinned folders, a mounted network share); restricting to the owner
    // costs nothing since nothing but Kara itself reads this file.
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("settings.conf");

    save(&path, &Settings::new()).expect("save must succeed");

    let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn save_leaves_no_temporary_file_behind() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("settings.conf");

    save(&path, &Settings::new()).expect("save must succeed");

    let names: Vec<String> = std::fs::read_dir(dir.path())
        .expect("read dir")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["settings.conf".to_string()]);
}

#[test]
fn save_survives_an_existing_file_with_different_content_replacing_it_atomically() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("settings.conf");
    std::fs::write(&path, b"[view]\nmode=old\n").expect("seed file");

    let mut settings = Settings::new();
    settings.set("view", "mode", "new");
    save(&path, &settings).expect("save must succeed");

    let loaded = load(&path);
    assert_eq!(loaded.settings.get("view", "mode"), Some("new"));
}

#[test]
fn a_non_utf8_file_is_reported_corrupt_and_left_untouched_on_disk() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("settings.conf");
    let garbage: &[u8] = &[0x5b, 0x76, 0xff, 0xfe, 0x5d, 0x0a]; // "[v\xFF\xFE]\n" — not valid UTF-8
    std::fs::write(&path, garbage).expect("seed garbage file");

    let loaded = load(&path);

    assert!(matches!(loaded.outcome, LoadOutcome::Corrupt { .. }));
    assert_eq!(loaded.settings, Settings::new());
    // `load` must never have rewritten the file: the corrupt bytes are still
    // exactly what was there, unmodified.
    assert_eq!(std::fs::read(&path).expect("read back"), garbage);
}

#[test]
fn a_directory_where_the_file_should_be_is_reported_corrupt_not_a_panic() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("settings.conf");
    std::fs::create_dir_all(&path).expect("seed a directory in place of the file");

    let loaded = load(&path);

    assert!(matches!(loaded.outcome, LoadOutcome::Corrupt { .. }));
    assert_eq!(loaded.settings, Settings::new());
}

// --- path resolution: pure over the two environment variables it needs ----

fn os(s: &str) -> OsString {
    OsString::from(s)
}

#[test]
fn an_absolute_xdg_config_home_wins_over_home() {
    let path = resolve_path(Some(&os("/xdg/config")), Some(&os("/home/ana")))
        .expect("both are set, must resolve");
    assert_eq!(path, Path::new("/xdg/config/kara/settings.conf"));
}

#[test]
fn a_relative_xdg_config_home_falls_back_to_home_dot_config() {
    let path = resolve_path(Some(&os("relative/config")), Some(&os("/home/ana")))
        .expect("must fall back instead of using a relative base");
    assert_eq!(path, Path::new("/home/ana/.config/kara/settings.conf"));
}

#[test]
fn an_empty_xdg_config_home_falls_back_to_home_dot_config() {
    let path =
        resolve_path(Some(&os("")), Some(&os("/home/ana"))).expect("empty must not be absolute");
    assert_eq!(path, Path::new("/home/ana/.config/kara/settings.conf"));
}

#[test]
fn no_xdg_config_home_falls_back_to_home_dot_config() {
    let path = resolve_path(None, Some(&os("/home/ana"))).expect("HOME alone is enough");
    assert_eq!(path, Path::new("/home/ana/.config/kara/settings.conf"));
}

#[test]
fn neither_variable_set_is_reported_as_an_error_not_a_panic() {
    let error = resolve_path(None, None).expect_err("nowhere to resolve the path");
    assert!(matches!(error, SettingsError::NoHome));
}

#[test]
fn resolve_path_never_touches_the_disk() {
    // Purely a contract check: an unresolvable, nonexistent XDG_CONFIG_HOME
    // still resolves fine as long as it is absolute — this function only
    // looks at the strings, it never stats anything.
    let path = resolve_path(Some(OsStr::new("/does/not/exist")), None)
        .expect("absolute is enough, existence is irrelevant here");
    assert_eq!(path, Path::new("/does/not/exist/kara/settings.conf"));
}

#[test]
fn removing_a_section_takes_every_key_with_it() {
    // A numbered list rewritten key by key would keep the tail of a longer
    // previous list, and those entries would come back on the next load.
    let mut settings = Settings::new();
    settings.set("pinned", "0", "/home/ana/a");
    settings.set("pinned", "1", "/home/ana/b");
    settings.set("window", "sidebar_width", "240");

    assert!(settings.remove_section("pinned"));

    assert_eq!(settings.get("pinned", "0"), None);
    assert_eq!(settings.get("pinned", "1"), None);
    assert_eq!(
        settings.get("window", "sidebar_width"),
        Some("240"),
        "other sections are untouched"
    );
}

#[test]
fn removing_a_section_that_is_not_there_says_so() {
    let mut settings = Settings::new();
    assert!(!settings.remove_section("pinned"));
}
