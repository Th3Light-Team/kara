//! Property tests of `RemotePath` (parse, join, parent, file_name,
//! starts_with) and `Location` (lossless URI round trips for arbitrary local
//! bytes and for remote paths with unicode and reserved characters, and
//! injectivity: two different URI strings never parse to the same place,
//! except the documented `kara+x://name` / `kara+x://name/` root alias).

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use kara_vfs::{DriveId, Location, RemotePath, RemotePathError};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies.

/// One valid segment: no `/`, no NUL, not `.` or `..`, non-empty. Biased
/// towards the characters URIs and paths treat specially.
fn segment() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z]{1,6}",
        "[ #%?\\[\\]{}|^`<>\"\\\\:@!$&'()*+,;=~.-]{1,6}",
        "[\u{80}-\u{10FFFF}]{1,4}",
        "[^/\u{0}]{1,8}",
    ]
    .prop_filter("not a dot segment", |s| s != "." && s != "..")
}

fn segments() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(segment(), 0..6)
}

fn canonical(segments: &[String]) -> String {
    if segments.is_empty() {
        String::from("/")
    } else {
        segments.iter().map(|s| format!("/{s}")).collect()
    }
}

fn remote_path() -> impl Strategy<Value = RemotePath> {
    segments().prop_map(|segs| match RemotePath::parse(&canonical(&segs)) {
        Ok(path) => path,
        Err(e) => panic!("generated canonical path refused: {e}"),
    })
}

fn drive() -> impl Strategy<Value = DriveId> {
    ("[a-z][a-z0-9]{0,15}", "[a-z0-9]([a-z0-9-]{0,20}[a-z0-9])?").prop_map(|(scheme, name)| {
        match DriveId::new(&scheme, &name) {
            Ok(drive) => drive,
            Err(e) => panic!("generated drive refused: {e}"),
        }
    })
}

/// An absolute local path in the form `components()` yields: any bytes but
/// `/` and NUL in each component, no `.` component (`..` is a real name here).
fn local_path() -> impl Strategy<Value = PathBuf> {
    prop::collection::vec(
        prop::collection::vec(1u8..=255, 1..8)
            .prop_filter("no slash, not '.'", |b| !b.contains(&b'/') && b != b"."),
        0..5,
    )
    .prop_map(|components| {
        let mut path = PathBuf::from("/");
        for component in components {
            path.push(OsStr::from_bytes(&component));
        }
        path
    })
}

fn location() -> impl Strategy<Value = Location> {
    prop_oneof![
        local_path().prop_map(Location::Local),
        (drive(), remote_path()).prop_map(|(drive, path)| Location::Remote { drive, path }),
    ]
}

/// Segment-wise prefix: the model `starts_with` must agree with.
fn model_starts_with(a: &RemotePath, b: &RemotePath) -> bool {
    let a: Vec<&str> = a.segments().collect();
    let b: Vec<&str> = b.segments().collect();
    a.len() >= b.len() && a[..b.len()] == b[..]
}

/// `%C3%A9` -> `%c3%a9`: the same bytes in a non-canonical spelling.
fn lowercase_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let escape = rest.get(i..i + 3).unwrap_or(&rest[i..]);
        out.push_str(&escape.to_lowercase());
        rest = &rest[i + escape.len()..];
    }
    out.push_str(rest);
    out
}

fn uri(location: &Location) -> String {
    match location.to_uri() {
        Ok(uri) => uri,
        Err(e) => panic!("to_uri({location:?}) failed: {e}"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    // -- RemotePath ---------------------------------------------------------

    #[test]
    fn a_canonical_string_parses_to_itself(segs in segments()) {
        let text = canonical(&segs);
        let path = RemotePath::parse(&text).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(path.as_str(), text.as_str());
        prop_assert_eq!(path.to_string(), text.clone());
        let back: Vec<&str> = path.segments().collect();
        prop_assert_eq!(back, segs.iter().map(String::as_str).collect::<Vec<_>>());
        prop_assert_eq!(RemotePath::from_bytes(text.as_bytes()), Ok(path));
    }

    #[test]
    fn extra_slashes_normalise_away(segs in segments(), extra in prop::collection::vec(1usize..4, 0..8), trailing in any::<bool>()) {
        let mut text = String::new();
        for (i, seg) in segs.iter().enumerate() {
            let slashes = extra.get(i).copied().unwrap_or(1);
            text.push_str(&"/".repeat(slashes));
            text.push_str(seg);
        }
        if trailing || text.is_empty() {
            text.push('/');
        }
        let parsed = RemotePath::parse(&text).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(parsed.as_str(), canonical(&segs));
    }

    #[test]
    fn parse_is_idempotent_on_anything_it_accepts(text in "[/a.\u{e9} ]{0,12}") {
        if let Ok(once) = RemotePath::parse(&text) {
            let twice = RemotePath::parse(once.as_str());
            prop_assert_eq!(twice, Ok(once));
        }
    }

    #[test]
    fn dot_segments_nul_and_relative_paths_are_refused(segs in segments(), at in 0usize..6, dots in prop_oneof![Just("."), Just("..")]) {
        let mut with_dot = segs.clone();
        with_dot.insert(at.min(segs.len()), dots.to_owned());
        let refused = RemotePath::parse(&canonical(&with_dot));
        prop_assert!(
            matches!(refused, Err(RemotePathError::DotSegment { .. })),
            "dot segment accepted: {:?}",
            refused
        );
        let relative = canonical(&segs).trim_start_matches('/').to_owned();
        if !relative.is_empty() {
            prop_assert!(
                matches!(RemotePath::parse(&relative), Err(RemotePathError::NotAbsolute { .. })),
                "relative path accepted"
            );
        }
        let with_nul = format!("{}\u{0}", canonical(&segs));
        prop_assert_eq!(RemotePath::parse(&with_nul), Err(RemotePathError::ContainsNul));
    }

    #[test]
    fn invalid_utf8_is_refused_by_from_bytes(segs in segments(), byte in 0x80u8..=0xff) {
        let mut bytes = canonical(&segs).into_bytes();
        bytes.push(byte);
        // A lone byte >= 0x80 at the end is never valid UTF-8.
        prop_assert_eq!(RemotePath::from_bytes(&bytes), Err(RemotePathError::NotUtf8));
    }

    #[test]
    fn join_then_parent_and_file_name_give_the_parts_back(base in remote_path(), name in segment()) {
        let child = base.join(&name).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(child.parent(), Some(base.clone()));
        prop_assert_eq!(child.file_name(), Some(name.as_str()));
        prop_assert!(child.starts_with(&base));
        prop_assert!(!base.starts_with(&child));
        prop_assert_eq!(child.segments().count(), base.segments().count() + 1);
    }

    #[test]
    fn join_refuses_anything_but_one_segment(base in remote_path(), name in segment(), bad in prop_oneof![Just(""), Just("."), Just(".."), Just("a/b"), Just("/"), Just("x\u{0}")]) {
        prop_assert!(base.join(bad).is_err());
        let with_slash = format!("{name}/{name}");
        prop_assert!(base.join(&with_slash).is_err());
    }

    #[test]
    fn the_parent_chain_ends_at_the_root(path in remote_path()) {
        let mut steps = 0;
        let mut current = path.clone();
        while let Some(parent) = current.parent() {
            prop_assert!(current.starts_with(&parent));
            current = parent;
            steps += 1;
        }
        prop_assert!(current.is_root());
        prop_assert_eq!(steps, path.segments().count());
        prop_assert_eq!(current.file_name(), None);
    }

    #[test]
    fn starts_with_is_segment_wise(a in remote_path(), b in remote_path()) {
        prop_assert_eq!(a.starts_with(&b), model_starts_with(&a, &b));
        prop_assert!(a.starts_with(&RemotePath::root()));
        prop_assert!(a.starts_with(&a));
    }

    #[test]
    fn a_sibling_sharing_a_prefix_is_not_inside(base in remote_path(), name in segment(), tail in segment()) {
        let short = base.join(&name).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let long_name = format!("{name}{tail}");
        let long = base.join(&long_name).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(!long.starts_with(&short), "{long} is not inside {short}");
        prop_assert!(!short.starts_with(&long));
    }

    #[test]
    fn equality_and_order_follow_the_canonical_string(a in remote_path(), b in remote_path()) {
        prop_assert_eq!(a == b, a.as_str() == b.as_str());
        prop_assert_eq!(a.cmp(&b), a.as_str().cmp(b.as_str()));
    }

    // -- Location -----------------------------------------------------------

    #[test]
    fn every_location_round_trips_through_its_uri(location in location()) {
        let text = uri(&location);
        prop_assert_eq!(Location::from_uri(&text), Ok(location.clone()));
        prop_assert!(text.is_ascii(), "the URI is ASCII: {}", text);
        prop_assert!(!text.contains([' ', '#', '?', '\u{0}']), "{}", text);
        for (i, _) in text.match_indices('%') {
            let hex = text.get(i + 1..i + 3).unwrap_or("");
            prop_assert!(
                hex.len() == 2 && hex.chars().all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c)),
                "escape at {} is not %XX uppercase: {}", i, text
            );
        }
    }

    #[test]
    fn the_uri_is_a_function_of_the_location_and_tells_them_apart(a in location(), b in location()) {
        prop_assert_eq!(a == b, uri(&a) == uri(&b));
    }

    #[test]
    fn a_parsed_uri_is_the_canonical_one(location in location(), edit in 0usize..10, at in any::<prop::sample::Index>()) {
        // Spellings near the canonical one: whatever of them parses must be
        // the canonical string itself (or the remote root alias).
        let text = uri(&location);
        let i = at.index(text.len().max(1));
        let variant = match edit {
            0 => text.to_lowercase(),
            1 => text.to_uppercase(),
            2 => format!("{text}/"),
            3 => text.replacen("://", ":///", 1),
            4 => lowercase_escapes(&text),
            5 => {
                let mut v = text.clone();
                if text.is_char_boundary(i) { v.insert(i, '/'); }
                v
            }
            6 => text.replace('/', "/./"),
            7 => text.replace("%20", " ").replace("%23", "#").replace("%25", "%"),
            8 => text.replacen('/', "%2F", 3),
            _ => {
                let mut v = text.clone();
                if text.is_char_boundary(i) { v.insert_str(i, "%41"); }
                v
            }
        };
        if let Ok(parsed) = Location::from_uri(&variant) {
            let canonical_text = uri(&parsed);
            let root_alias = matches!(&parsed, Location::Remote { path, .. } if path.is_root())
                && canonical_text.strip_suffix('/') == Some(variant.as_str());
            prop_assert!(
                variant == canonical_text || root_alias,
                "{:?} parsed although its canonical form is {:?}", variant, canonical_text
            );
        }
    }

    #[test]
    fn two_different_uri_strings_never_name_one_place(a in "(file|kara\\+[a-z]{1,3}):/{0,4}[a-z0-9/%. -]{0,10}", b in "(file|kara\\+[a-z]{1,3}):/{0,4}[a-z0-9/%. -]{0,10}") {
        if let (Ok(x), Ok(y)) = (Location::from_uri(&a), Location::from_uri(&b))
            && a != b
            && x == y
        {
            let remote_root = matches!(&x, Location::Remote { path, .. } if path.is_root());
            prop_assert!(
                remote_root && (a.strip_suffix('/') == Some(b.as_str()) || b.strip_suffix('/') == Some(a.as_str())),
                "{:?} and {:?} both name {:?}", a, b, x
            );
        }
    }

    #[test]
    fn the_parent_of_a_location_is_the_parent_of_its_path(drive in drive(), path in remote_path()) {
        let here = Location::Remote { drive: drive.clone(), path: path.clone() };
        let expected = path.parent().map(|parent| Location::Remote { drive, path: parent });
        prop_assert_eq!(here.parent(), expected);
    }
}

#[test]
fn the_remote_root_alias_is_the_only_alias() {
    let with = "kara+sftp://nas/";
    let without = "kara+sftp://nas";
    assert_eq!(Location::from_uri(with), Location::from_uri(without));
    assert!(Location::from_uri("kara+sftp://nas//").is_err());
    assert!(Location::from_uri("file:///tmp/").is_err(), "a local trailing slash is not an alias");
}
