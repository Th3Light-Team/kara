//! DriveId and Location: validation, URI form, round trip, drive relations.
//! Edge cases cb_05, cb_06, cb_07, cb_08, cb_09, cb_10.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use kara_vfs::{DriveId, DriveIdError, Location, LocationError, RemotePath, RemotePathError};

fn p(s: &str) -> RemotePath {
    RemotePath::parse(s).unwrap_or_else(|e| panic!("{s:?} should parse: {e}"))
}

fn drive(scheme: &str, name: &str) -> DriveId {
    DriveId::new(scheme, name).unwrap_or_else(|e| panic!("{scheme}/{name}: {e}"))
}

fn remote(scheme: &str, name: &str, path: &str) -> Location {
    Location::Remote {
        drive: drive(scheme, name),
        path: p(path),
    }
}

fn local(path: &str) -> Location {
    Location::Local(PathBuf::from(path))
}

fn local_bytes(bytes: &[u8]) -> Location {
    Location::Local(PathBuf::from(OsStr::from_bytes(bytes)))
}

fn uri(l: &Location) -> String {
    l.to_uri()
        .unwrap_or_else(|e| panic!("to_uri({l:?}) failed: {e}"))
}

// cb_05 ---------------------------------------------------------------------

#[test]
fn cb_05_valid_drive_ids() {
    let d = drive("sftp", "work-nas");
    assert_eq!(d.scheme(), "sftp");
    assert_eq!(d.name(), "work-nas");
    for (scheme, name) in [
        ("s3", "x"),
        ("mem", "0"),
        ("a234567890123456", "a-b"),
        ("sftp", "a--b"),
        ("sftp", "9lives"),
        ("sftp", &"a".repeat(63)),
    ] {
        let d = drive(scheme, name);
        assert_eq!((d.scheme(), d.name()), (scheme, name));
    }
}

#[test]
fn cb_05_invalid_schemes_are_rejected_not_repaired() {
    let long = "a".repeat(17);
    for scheme in [
        "SFTP",
        "",
        "1s",
        "s-3",
        " sftp",
        "sftp ",
        "s+ftp",
        "kara+sftp",
        long.as_str(),
    ] {
        assert_eq!(
            DriveId::new(scheme, "x"),
            Err(DriveIdError::InvalidScheme {
                raw: scheme.to_owned()
            }),
            "scheme {scheme:?}"
        );
    }
}

#[test]
fn cb_05_invalid_names_are_rejected_not_repaired() {
    let long = "a".repeat(64);
    for name in [
        "",
        "-x",
        "x-",
        "Work",
        "a b",
        "a.b",
        "a_b",
        "x/y",
        " x",
        "é",
        long.as_str(),
    ] {
        assert_eq!(
            DriveId::new("sftp", name),
            Err(DriveIdError::InvalidName {
                raw: name.to_owned()
            }),
            "name {name:?}"
        );
    }
}

#[test]
fn cb_05_equality_includes_the_scheme() {
    assert_ne!(drive("sftp", "x"), drive("s3", "x"));
    assert_eq!(drive("sftp", "x"), drive("sftp", "x"));
    assert_ne!(remote("sftp", "x", "/a"), remote("s3", "x", "/a"));
    let set: HashSet<Location> = [
        remote("sftp", "x", "/a"),
        remote("s3", "x", "/a"),
        remote("sftp", "x", "/a"),
    ]
    .into_iter()
    .collect();
    assert_eq!(set.len(), 2);
}

// cb_06 ---------------------------------------------------------------------

#[test]
fn cb_06_local_uris_match_kara_fs_file_uri_byte_for_byte() {
    let cases: [(Location, &str); 7] = [
        (
            local("/tmp/a b#c%d?.txt"),
            "file:///tmp/a%20b%23c%25d%3F.txt",
        ),
        (local("/tmp/caf\u{e9}"), "file:///tmp/caf%C3%A9"),
        (local_bytes(b"/tmp/\xff"), "file:///tmp/%FF"),
        (local("/tmp/[x]{y}"), "file:///tmp/%5Bx%5D%7By%7D"),
        (local("/tmp/l\nb"), "file:///tmp/l%0Ab"),
        (
            local("/tmp/q\"<>\\^`|"),
            "file:///tmp/q%22%3C%3E%5C%5E%60%7C",
        ),
        (
            local("/a&b=c;d,e+f!g'h(i)*j~k_l-m.n:o@p$q"),
            "file:///a&b=c;d,e+f!g'h(i)*j~k_l-m.n:o@p$q",
        ),
    ];
    for (loc, expected) in cases {
        assert_eq!(uri(&loc), expected, "{loc:?}");
    }
    assert_eq!(uri(&local("/")), "file:///");
}

#[test]
fn cb_06_relative_or_empty_local_path_has_no_uri() {
    assert_eq!(
        local("rel/x").to_uri(),
        Err(LocationError::RelativeLocalPath)
    );
    assert_eq!(local("").to_uri(), Err(LocationError::RelativeLocalPath));
}

// cb_07 ---------------------------------------------------------------------

#[test]
fn cb_07_remote_uri_form() {
    assert_eq!(
        uri(&remote("sftp", "work", "/home/ana/a b.txt")),
        "kara+sftp://work/home/ana/a%20b.txt"
    );
    assert_eq!(uri(&remote("sftp", "work", "/")), "kara+sftp://work/");
    assert_eq!(
        uri(&remote("s3", "bucket-1", "/caf\u{e9}")),
        "kara+s3://bucket-1/caf%C3%A9"
    );
    assert_eq!(
        uri(&remote("mem", "t", "/a#b?c%d")),
        "kara+mem://t/a%23b%3Fc%25d"
    );
}

#[test]
fn cb_07_round_trip_is_lossless() {
    let locations = vec![
        local("/tmp/a b"),
        local("/tmp/#hash"),
        local("/tmp/100%"),
        local("/tmp/why?"),
        local("/tmp/l\nb"),
        local_bytes(b"/tmp/\xff\xfe"),
        local("/"),
        local("/a&b=c;d,e+f!g'h(i)*j~k_l-m.n:o@p$q"),
        remote("sftp", "work", "/"),
        remote("sftp", "work", "/a b/#%?[]"),
        remote("sftp", "work", "/ünï/cødé"),
        remote("sftp", "work", "/a\\b"),
        remote("s3", "work", "/a b/c"),
        remote("mem", "x-1", "/l\nb"),
    ];
    for loc in locations {
        let text = uri(&loc);
        assert_eq!(
            Location::from_uri(&text),
            Ok(loc.clone()),
            "round trip of {loc:?} via {text:?}"
        );
    }
}

// cb_08 ---------------------------------------------------------------------

#[test]
fn cb_08_remote_root_with_or_without_trailing_slash() {
    let expected = Location::Remote {
        drive: drive("sftp", "work"),
        path: RemotePath::root(),
    };
    assert_eq!(Location::from_uri("kara+sftp://work"), Ok(expected.clone()));
    assert_eq!(
        Location::from_uri("kara+sftp://work/"),
        Ok(expected.clone())
    );
    assert_eq!(uri(&expected), "kara+sftp://work/");
}

// cb_09 ---------------------------------------------------------------------

#[test]
fn cb_09_foreign_schemes_are_unsupported() {
    for raw in [
        "smb://h/x",
        "http://x",
        "sftp://h/x",
        "kara+://x/",
        "FILE:///x",
        "KARA+SFTP://w/",
    ] {
        assert!(
            matches!(
                Location::from_uri(raw),
                Err(LocationError::UnsupportedScheme { .. })
            ),
            "{raw:?}: expected UnsupportedScheme, got {:?}",
            Location::from_uri(raw)
        );
    }
}

#[test]
fn cb_09_file_uri_with_a_host_is_refused() {
    assert_eq!(
        Location::from_uri("file://host/x"),
        Err(LocationError::NonLocalFileHost {
            host: "host".to_owned()
        })
    );
}

#[test]
fn cb_09_query_or_fragment_is_refused() {
    for raw in [
        "file:///a?b",
        "file:///a#b",
        "kara+sftp://w/a#b",
        "kara+sftp://w/a?b",
    ] {
        assert_eq!(
            Location::from_uri(raw),
            Err(LocationError::QueryOrFragment),
            "{raw:?}"
        );
    }
}

#[test]
fn cb_09_bad_percent_encoding_is_refused() {
    for raw in [
        "file:///a%zz",
        "file:///a%4",
        "file:///a%",
        "kara+sftp://w/a%g1",
    ] {
        assert_eq!(
            Location::from_uri(raw),
            Err(LocationError::InvalidPercentEncoding),
            "{raw:?}"
        );
    }
}

#[test]
fn cb_09_encoded_nul_is_refused() {
    assert_eq!(
        Location::from_uri("file:///a%00b"),
        Err(LocationError::ContainsNul)
    );
    assert_eq!(
        Location::from_uri("kara+sftp://w/a%00"),
        Err(LocationError::ContainsNul)
    );
}

#[test]
fn cb_09_remote_path_and_drive_errors_are_typed() {
    assert_eq!(
        Location::from_uri("kara+sftp://w/%FF"),
        Err(LocationError::Path(RemotePathError::NotUtf8))
    );
    assert!(matches!(
        Location::from_uri("kara+sftp://w/a/../b"),
        Err(LocationError::Path(RemotePathError::DotSegment { .. }))
    ));
    assert!(matches!(
        Location::from_uri("kara+sftp://Bad_Name/"),
        Err(LocationError::Drive(DriveIdError::InvalidName { .. }))
    ));
}

#[test]
fn cb_09_relative_or_empty_file_uri_is_never_ok() {
    for raw in ["file:relative", "file://"] {
        let r = Location::from_uri(raw);
        assert!(
            matches!(
                r,
                Err(LocationError::RelativeLocalPath)
                    | Err(LocationError::UnsupportedScheme { .. })
            ),
            "{raw:?}: got {r:?}"
        );
    }
}

#[test]
fn cb_09_parsed_local_path_is_the_decoded_raw_bytes() {
    assert_eq!(
        Location::from_uri("file:///tmp/%FF"),
        Ok(local_bytes(b"/tmp/\xff"))
    );
    assert_eq!(
        Location::from_uri("file:///tmp/a%20b"),
        Ok(local("/tmp/a b"))
    );
    assert_eq!(Location::from_uri("file:///"), Ok(local("/")));
}

// cb_10 ---------------------------------------------------------------------

#[test]
fn cb_10_same_remote_drive_requires_equal_scheme_and_name() {
    let a = remote("sftp", "work", "/a");
    assert!(a.same_remote_drive(&remote("sftp", "work", "/b")));
    assert!(a.same_remote_drive(&a));
    assert!(!a.same_remote_drive(&remote("s3", "work", "/a")));
    assert!(!a.same_remote_drive(&remote("sftp", "home", "/a")));
    assert!(!local("/a").same_remote_drive(&local("/b")));
    assert!(!local("/a").same_remote_drive(&a));
    assert!(!a.same_remote_drive(&local("/a")));
}

#[test]
fn cb_10_parent_stays_on_the_same_drive() {
    assert_eq!(
        remote("sftp", "work", "/a").parent(),
        Some(remote("sftp", "work", "/"))
    );
    assert_eq!(
        remote("sftp", "work", "/a/b c").parent(),
        Some(remote("sftp", "work", "/a"))
    );
    assert_eq!(remote("sftp", "work", "/").parent(), None);
    assert_eq!(local("/").parent(), None);
    assert_eq!(local("/a/b").parent(), Some(local("/a")));
}

#[test]
fn cb_10_is_local_and_drive() {
    assert!(local("/a").is_local());
    assert!(!remote("sftp", "work", "/a").is_local());
    assert_eq!(local("/a").drive(), None);
    assert_eq!(remote("s3", "b", "/k").drive(), Some(&drive("s3", "b")));
}
