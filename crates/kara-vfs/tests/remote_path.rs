//! RemotePath: canonical form, rejections, segment-aware operations.
//! Edge cases cb_01, cb_02, cb_03, cb_04.

use kara_vfs::{RemotePath, RemotePathError};

fn p(s: &str) -> RemotePath {
    RemotePath::parse(s).unwrap_or_else(|e| panic!("{s:?} should parse: {e}"))
}

// cb_01 ---------------------------------------------------------------------

#[test]
fn cb_01_parse_collapses_repeated_slashes_and_strips_trailing_ones() {
    assert_eq!(p("/a//b/").as_str(), "/a/b");
    assert_eq!(p("/a/b///").as_str(), "/a/b");
    assert_eq!(p("//a").as_str(), "/a");
    assert_eq!(p("/a////b////c").as_str(), "/a/b/c");
    assert_eq!(p("/a/b"), p("/a//b//"));
}

#[test]
fn cb_01_root_is_exactly_one_slash() {
    assert_eq!(RemotePath::root().as_str(), "/");
    assert!(RemotePath::root().is_root());
    assert_eq!(p("/"), RemotePath::root());
    assert_eq!(p("///"), RemotePath::root());
    assert!(p("/").is_root());
    assert!(p("////").is_root());
    assert!(!p("/a").is_root());
    assert!(!p("/a/b").is_root());
}

#[test]
fn cb_01_display_and_from_str_agree_with_parse() {
    let x: RemotePath = "/a//b/".parse().expect("FromStr");
    assert_eq!(x, p("/a/b"));
    assert_eq!(x.to_string(), "/a/b");
    assert_eq!(RemotePath::root().to_string(), "/");
    assert_eq!("".parse::<RemotePath>(), Err(RemotePathError::Empty));
    assert!(matches!(
        "rel".parse::<RemotePath>(),
        Err(RemotePathError::NotAbsolute { .. })
    ));
}

// cb_02 ---------------------------------------------------------------------

#[test]
fn cb_02_empty_is_rejected() {
    assert_eq!(RemotePath::parse(""), Err(RemotePathError::Empty));
    assert_eq!(RemotePath::from_bytes(b""), Err(RemotePathError::Empty));
}

#[test]
fn cb_02_relative_is_rejected_not_repaired() {
    for raw in ["a/b", "a", "./a", "../a", " /a"] {
        match RemotePath::parse(raw) {
            Err(RemotePathError::NotAbsolute { raw: got }) => {
                assert_eq!(got, raw, "NotAbsolute must carry the raw input");
            }
            other => panic!("{raw:?}: expected NotAbsolute, got {other:?}"),
        }
    }
}

#[test]
fn cb_02_dot_segments_are_rejected_not_resolved() {
    for raw in [
        "/a/./b",
        "/a/../b",
        "/..",
        "/.",
        "/a/.",
        "/a/..",
        "/a//..//b/",
        "/./a",
    ] {
        assert!(
            matches!(
                RemotePath::parse(raw),
                Err(RemotePathError::DotSegment { .. })
            ),
            "{raw:?}: expected DotSegment, got {:?}",
            RemotePath::parse(raw)
        );
    }
}

#[test]
fn cb_02_nul_is_rejected() {
    assert_eq!(
        RemotePath::parse("/a\0b"),
        Err(RemotePathError::ContainsNul)
    );
    assert_eq!(RemotePath::parse("/\0"), Err(RemotePathError::ContainsNul));
    assert_eq!(
        RemotePath::from_bytes(b"/a\0"),
        Err(RemotePathError::ContainsNul)
    );
}

#[test]
fn cb_02_non_utf8_bytes_are_rejected() {
    assert_eq!(
        RemotePath::from_bytes(b"/a/\xff"),
        Err(RemotePathError::NotUtf8)
    );
    assert_eq!(
        RemotePath::from_bytes(b"/\xc3"),
        Err(RemotePathError::NotUtf8)
    );
}

#[test]
fn cb_02_from_bytes_normalises_like_parse() {
    assert_eq!(RemotePath::from_bytes(b"/a//b/"), Ok(p("/a/b")));
    assert_eq!(
        RemotePath::from_bytes("/caf\u{e9}".as_bytes()),
        Ok(p("/caf\u{e9}"))
    );
    assert!(matches!(
        RemotePath::from_bytes(b"x/y"),
        Err(RemotePathError::NotAbsolute { .. })
    ));
    assert!(matches!(
        RemotePath::from_bytes(b"/a/../b"),
        Err(RemotePathError::DotSegment { .. })
    ));
}

#[test]
fn cb_02_names_that_merely_contain_dots_are_valid() {
    for raw in [
        "/a/...", "/.hidden", "/a..b", "/..a", "/a.", "/.../x", "/a/.b/c.",
    ] {
        assert_eq!(p(raw).as_str(), raw, "{raw:?} must be kept verbatim");
    }
}

// cb_03 ---------------------------------------------------------------------

#[test]
fn cb_03_unusual_characters_are_kept_verbatim() {
    let x = p("/a b/#%?\\x");
    assert_eq!(x.as_str(), "/a b/#%?\\x");
    assert_eq!(x.segments().collect::<Vec<_>>(), vec!["a b", "#%?\\x"]);

    for raw in [
        "/[x]",
        "/a:b",
        "/a*b",
        "/tab\there",
        "/l\nb",
        "/\u{1}ctl",
        "/ünï cødé",
        "/日本語",
    ] {
        assert_eq!(p(raw).as_str(), raw, "{raw:?} must be kept verbatim");
    }
}

#[test]
fn cb_03_no_unicode_normalisation() {
    let nfc = p("/caf\u{e9}");
    let nfd = p("/cafe\u{301}");
    assert_ne!(nfc, nfd);
    assert_eq!(nfc.as_str(), "/caf\u{e9}");
    assert_eq!(nfd.as_str(), "/cafe\u{301}");
}

// cb_04 ---------------------------------------------------------------------

#[test]
fn cb_04_join_appends_exactly_one_segment() {
    assert_eq!(RemotePath::root().join("a"), Ok(p("/a")));
    assert_eq!(p("/a").join("b c"), Ok(p("/a/b c")));
    assert_eq!(
        p("/a").join("...").map(|x| x.as_str().to_owned()),
        Ok("/a/...".to_owned())
    );
}

#[test]
fn cb_04_join_rejects_anything_that_is_not_one_segment() {
    for seg in ["a/b", "", ".", "..", "/", "a/", "/a"] {
        assert!(
            matches!(
                RemotePath::root().join(seg),
                Err(RemotePathError::InvalidSegment { .. })
            ),
            "join({seg:?}): expected InvalidSegment, got {:?}",
            RemotePath::root().join(seg)
        );
    }
    assert_eq!(p("/a").join("x\0y"), Err(RemotePathError::ContainsNul));
}

#[test]
fn cb_04_starts_with_compares_whole_segments() {
    assert!(!p("/ab").starts_with(&p("/a")));
    assert!(!p("/a b").starts_with(&p("/a")));
    assert!(!p("/a.txt").starts_with(&p("/a")));
    assert!(p("/a/b").starts_with(&p("/a")));
    assert!(p("/a/b/c").starts_with(&p("/a/b")));
    assert!(p("/a").starts_with(&p("/a")));
    assert!(!p("/a").starts_with(&p("/a/b")));
    for raw in ["/", "/x", "/x/y/z"] {
        assert!(
            p(raw).starts_with(&RemotePath::root()),
            "{raw} starts with root"
        );
    }
}

#[test]
fn cb_04_parent_and_file_name() {
    assert_eq!(p("/a").parent(), Some(RemotePath::root()));
    assert_eq!(p("/a/b").parent(), Some(p("/a")));
    assert_eq!(p("/a/b c/d").parent(), Some(p("/a/b c")));
    assert_eq!(RemotePath::root().parent(), None);
    assert_eq!(RemotePath::root().file_name(), None);
    assert_eq!(p("/a/b").file_name(), Some("b"));
    assert_eq!(p("/x y.txt").file_name(), Some("x y.txt"));
}

#[test]
fn cb_04_segments_of_root_is_empty_and_join_inverts_parent() {
    assert_eq!(RemotePath::root().segments().count(), 0);
    assert_eq!(
        p("/a/b/c").segments().collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );

    let x = p("/one/two/three");
    let parent = x.parent().expect("parent");
    let name = x.file_name().expect("file_name");
    assert_eq!(parent.join(name), Ok(x.clone()));
}
