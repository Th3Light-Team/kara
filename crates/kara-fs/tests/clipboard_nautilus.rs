//! Clipboard interoperability with Nautilus, pinned to its real wire format.
//!
//! Verified against the Nautilus 50.2 / GTK 4.22 sources
//! (`src/nautilus-clipboard.c`, `gdk/gdkcontentserializer.c`), the versions
//! Ubuntu 26.04 ships:
//!
//! - Nautilus *writes* `x-special/gnome-copied-files` as `copy` or `cut`
//!   followed by `"\n" + uri` per file: no trailing newline, no NUL.
//! - Nautilus *reads* it by splitting on `"\n"`, and any empty line — so a
//!   trailing newline — or a `\r` makes the whole payload invalid. There is no
//!   fallback to `text/uri-list` once this type is offered: the paste is
//!   silently dropped. A malformed payload from Kara is therefore not a
//!   degraded paste but no paste at all.
//! - GTK's `text/uri-list` ends every URI with `"\r\n"`, the last one included.

use std::path::PathBuf;

use kara_fs::clipboard::{
    ClipboardAction, ClipboardFormats, ClipboardState, parse, render_gnome_copied_files,
};

/// What Nautilus' own parser (`nautilus_clipboard_from_string`) accepts:
/// split on `\n`, first line exactly `copy` or `cut`, no empty line anywhere,
/// every other line a URI.
fn nautilus_accepts(payload: &str) -> bool {
    let mut lines = payload.split('\n');
    let Some(first) = lines.next() else {
        return false;
    };
    if first != "copy" && first != "cut" {
        return false;
    }
    let mut any = false;
    for line in lines {
        if line.is_empty() || line.contains('\r') || !line.starts_with("file:///") {
            return false;
        }
        any = true;
    }
    any
}

#[test]
fn what_kara_writes_for_a_cut_passes_nautilus_parser() {
    let state = ClipboardState::cut(vec![
        PathBuf::from("/home/ana/informe final.pdf"),
        PathBuf::from("/home/ana/Imágenes"),
    ]);
    let payload = state.to_formats().gnome_copied_files;
    assert!(nautilus_accepts(&payload), "Nautilus would drop: {payload:?}");
    assert!(payload.starts_with("cut\n"));
}

#[test]
fn what_kara_writes_for_a_copy_passes_nautilus_parser() {
    let payload = render_gnome_copied_files(ClipboardAction::Copy, &[PathBuf::from("/tmp/a")]);
    assert_eq!(payload, "copy\nfile:///tmp/a");
    assert!(nautilus_accepts(&payload));
}

#[test]
fn kara_never_ends_the_gnome_payload_with_a_newline_or_a_nul() {
    for action in [ClipboardAction::Copy, ClipboardAction::Cut] {
        let payload = render_gnome_copied_files(action, &[PathBuf::from("/x"), PathBuf::from("/y")]);
        assert!(!payload.ends_with('\n'));
        assert!(!payload.ends_with('\0'));
        assert!(!payload.contains('\r'));
    }
}

#[test]
fn a_cut_written_by_nautilus_reads_as_a_cut() {
    // Byte for byte what `nautilus_clipboard_to_string` produces.
    let gnome = b"cut\nfile:///home/ana/a%20b.txt\nfile:///home/ana/c";
    let uri_list = b"file:///home/ana/a%20b.txt\r\nfile:///home/ana/c\r\n";
    let state = parse(ClipboardFormats {
        uri_list: Some(uri_list),
        gnome_copied_files: Some(gnome),
        kde_cut_selection: None,
    })
    .expect("a Nautilus cut is readable");
    assert!(state.is_cut());
    assert_eq!(
        state.paths,
        vec![PathBuf::from("/home/ana/a b.txt"), PathBuf::from("/home/ana/c")]
    );
}

#[test]
fn a_copy_written_by_nautilus_reads_as_a_copy() {
    let state = parse(ClipboardFormats {
        uri_list: Some(b"file:///srv/x\r\n"),
        gnome_copied_files: Some(b"copy\nfile:///srv/x"),
        kde_cut_selection: None,
    })
    .expect("a Nautilus copy is readable");
    assert!(!state.is_cut());
    assert_eq!(state.paths, vec![PathBuf::from("/srv/x")]);
}

#[test]
fn gtk_uri_list_alone_with_its_final_crlf_is_a_copy() {
    // What any other GTK 4 application offers for a file list.
    let state = parse(ClipboardFormats {
        uri_list: Some(b"file:///a\r\nfile:///b\r\n"),
        gnome_copied_files: None,
        kde_cut_selection: None,
    })
    .expect("a plain GTK file list is readable");
    assert!(!state.is_cut());
    assert_eq!(state.paths, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
}
