//! File clipboard: producing and parsing the byte payloads real file
//! managers exchange through the system clipboard, and the two paste
//! special-cases the spec calls out (`ground/spec/02-seleccion.md`, "Cortar,
//! copiar y pegar").
//!
//! The `application/x-kde-cutselection` values (`"1"`/`"0"`) and the fact
//! that current KIO does not write `x-special/gnome-copied-files` at all
//! were confirmed by disassembling `libKF6KIOWidgets.so.6.24.0`, the real
//! library installed on this machine — see the module doc on
//! `kara_fs::clipboard` for how. These tests exercise the contract that
//! grounds, not the disassembly itself.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use kara_fs::clipboard::{
    ClipboardAction, ClipboardFormats, ClipboardState, kde_cut_selection_value, parse,
    parse_gnome_copied_files, parse_kde_cut_selection, parse_uri_list, path_from_uri,
    render_gnome_copied_files, render_uri_list,
};

fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(OsStr::from_bytes(bytes))
}

// ---------------------------------------------------------------------
// `text/uri-list`
// ---------------------------------------------------------------------

#[test]
fn uri_list_lines_are_crlf_terminated_per_rfc_2483() {
    let rendered = render_uri_list(&[PathBuf::from("/home/ana/nota.txt")]);
    assert_eq!(rendered, "file:///home/ana/nota.txt\r\n");
}

#[test]
fn uri_list_round_trips_a_path_with_spaces() {
    let original = vec![PathBuf::from("/home/ana/Mis Documentos/informe final.pdf")];
    let rendered = render_uri_list(&original);
    assert_eq!(parse_uri_list(rendered.as_bytes()), original);
}

#[test]
fn uri_list_round_trips_a_path_with_accents() {
    let original = vec![PathBuf::from("/home/ana/Álbum de fotos/Ámbar.jpg")];
    let rendered = render_uri_list(&original);
    assert_eq!(parse_uri_list(rendered.as_bytes()), original);
}

/// `#` starts a comment only at the start of a line (RFC 2483); a `#` that
/// is part of a file name is percent-encoded by `file_uri` and so never
/// lands at the start of a rendered line, but a parser that just checked
/// "contains `#`" would wrongly drop it anyway. This nails the distinction.
#[test]
fn uri_list_round_trips_a_path_containing_a_hash() {
    let original = vec![PathBuf::from("/home/ana/notas #2.txt")];
    let rendered = render_uri_list(&original);
    assert_eq!(parse_uri_list(rendered.as_bytes()), original);
}

/// A literal newline is a legal byte in a Linux file name (only `NUL` and
/// `/` are forbidden). `file_uri` percent-encodes control characters, so it
/// must not turn into a spurious line break that `text/uri-list` would then
/// misread as two entries.
#[test]
fn uri_list_round_trips_a_path_containing_a_literal_newline() {
    let original = vec![path_from_bytes(b"/home/ana/raro\nnombre.txt")];
    let rendered = render_uri_list(&original);
    assert_eq!(rendered.lines().count(), 1, "the embedded newline must be escaped, not raw");
    assert_eq!(parse_uri_list(rendered.as_bytes()), original);
}

#[test]
fn uri_list_comment_lines_are_ignored() {
    let payload = b"# a comment\r\nfile:///home/ana/a.txt\r\n";
    assert_eq!(parse_uri_list(payload), vec![PathBuf::from("/home/ana/a.txt")]);
}

#[test]
fn uri_list_blank_lines_are_ignored() {
    let payload = b"file:///home/ana/a.txt\r\n\r\nfile:///home/ana/b.txt\r\n";
    assert_eq!(
        parse_uri_list(payload),
        vec![PathBuf::from("/home/ana/a.txt"), PathBuf::from("/home/ana/b.txt")]
    );
}

/// Real senders are not always RFC-perfect: a bare `LF` must still parse,
/// not take the whole list down.
#[test]
fn uri_list_accepts_bare_lf_as_well_as_crlf() {
    let payload = b"file:///home/ana/a.txt\nfile:///home/ana/b.txt\n";
    assert_eq!(
        parse_uri_list(payload),
        vec![PathBuf::from("/home/ana/a.txt"), PathBuf::from("/home/ana/b.txt")]
    );
}

/// One malformed line does not take the rest of the batch down with it.
#[test]
fn uri_list_skips_one_bad_line_and_keeps_the_rest() {
    let payload = b"file:///home/ana/a.txt\r\nnot a uri at all\r\nfile:///home/ana/b.txt\r\n";
    assert_eq!(
        parse_uri_list(payload),
        vec![PathBuf::from("/home/ana/a.txt"), PathBuf::from("/home/ana/b.txt")]
    );
}

#[test]
fn uri_list_of_only_comments_and_blanks_is_empty_not_a_crash() {
    assert!(parse_uri_list(b"# nothing here\r\n\r\n").is_empty());
}

// ---------------------------------------------------------------------
// `file://` <-> `PathBuf`
// ---------------------------------------------------------------------

#[test]
fn path_from_uri_rejects_a_non_file_scheme() {
    assert_eq!(path_from_uri(b"http://example.com/a.txt"), None);
    assert_eq!(path_from_uri(b"trash:/a.txt"), None);
}

/// `file://host/path` names a file on a *different* machine; Kara has no
/// business resolving that to a local path.
#[test]
fn path_from_uri_rejects_a_non_empty_authority() {
    assert_eq!(path_from_uri(b"file://otherhost/etc/passwd"), None);
}

#[test]
fn path_from_uri_rejects_a_relative_reference() {
    assert_eq!(path_from_uri(b"relative/path.txt"), None);
    assert_eq!(path_from_uri(b""), None);
}

/// A `NUL` byte cannot be part of a real path; a percent-encoded `%00`
/// safely marks the input as hostile rather than being passed through.
#[test]
fn path_from_uri_rejects_an_embedded_nul() {
    assert_eq!(path_from_uri(b"file:///home/ana/a%00b.txt"), None);
}

// ---------------------------------------------------------------------
// `x-special/gnome-copied-files`
// ---------------------------------------------------------------------

#[test]
fn gnome_copied_files_round_trips_copy() {
    let paths = vec![PathBuf::from("/home/ana/a.txt"), PathBuf::from("/home/ana/b.txt")];
    let rendered = render_gnome_copied_files(ClipboardAction::Copy, &paths);
    let parsed = parse_gnome_copied_files(rendered.as_bytes()).expect("should parse");
    assert_eq!(parsed.action, ClipboardAction::Copy);
    assert_eq!(parsed.paths, paths);
}

#[test]
fn gnome_copied_files_round_trips_cut() {
    let paths = vec![PathBuf::from("/home/ana/a.txt")];
    let rendered = render_gnome_copied_files(ClipboardAction::Cut, &paths);
    let parsed = parse_gnome_copied_files(rendered.as_bytes()).expect("should parse");
    assert_eq!(parsed.action, ClipboardAction::Cut);
    assert_eq!(parsed.paths, paths);
}

#[test]
fn gnome_copied_files_first_line_is_lf_joined_not_crlf() {
    // This is not `text/uri-list`; no real implementation of this
    // convention uses `CRLF` between the marker and the URIs.
    let rendered = render_gnome_copied_files(ClipboardAction::Copy, &[PathBuf::from("/a.txt")]);
    assert_eq!(rendered, "copy\nfile:///a.txt");
}

#[test]
fn gnome_copied_files_rejects_an_unrecognised_marker() {
    assert_eq!(parse_gnome_copied_files(b"move\nfile:///a.txt"), None);
}

#[test]
fn gnome_copied_files_rejects_an_empty_payload() {
    assert_eq!(parse_gnome_copied_files(b""), None);
}

/// A valid marker with no usable URI behind it is not usable either — this
/// is the case `parse` relies on to fall back to `text/uri-list` instead of
/// silently losing files.
#[test]
fn gnome_copied_files_rejects_a_marker_with_no_usable_uri() {
    assert_eq!(parse_gnome_copied_files(b"copy\nnot a uri"), None);
}

// ---------------------------------------------------------------------
// `application/x-kde-cutselection`
// ---------------------------------------------------------------------

/// Values confirmed against the real `KIO::setClipboardDataCut` shipped on
/// this machine: exactly `"1"` for cut, `"0"` for copy — see the module doc.
#[test]
fn kde_cut_selection_value_matches_real_kio() {
    assert_eq!(kde_cut_selection_value(ClipboardAction::Cut), "1");
    assert_eq!(kde_cut_selection_value(ClipboardAction::Copy), "0");
}

#[test]
fn kde_cut_selection_reads_exactly_one_as_cut() {
    assert!(parse_kde_cut_selection(b"1"));
}

#[test]
fn kde_cut_selection_tolerates_surrounding_whitespace() {
    assert!(parse_kde_cut_selection(b"1\n"));
    assert!(parse_kde_cut_selection(b" 1 "));
}

#[test]
fn kde_cut_selection_treats_anything_else_as_not_cut() {
    assert!(!parse_kde_cut_selection(b"0"));
    assert!(!parse_kde_cut_selection(b""));
    assert!(!parse_kde_cut_selection(b"true"));
}

// ---------------------------------------------------------------------
// `parse`: combining whatever formats are actually present
// ---------------------------------------------------------------------

#[test]
fn parse_prefers_gnome_copied_files_when_it_is_usable() {
    let gnome = b"cut\nfile:///home/ana/a.txt".to_vec();
    // Deliberately disagrees with the gnome payload, to prove which one won.
    let uri_list = b"file:///home/ana/b.txt\r\n".to_vec();
    let state = parse(ClipboardFormats {
        uri_list: Some(&uri_list),
        gnome_copied_files: Some(&gnome),
        kde_cut_selection: None,
    })
    .expect("should parse");
    assert_eq!(state.action, ClipboardAction::Cut);
    assert_eq!(state.paths, vec![PathBuf::from("/home/ana/a.txt")]);
}

#[test]
fn parse_falls_back_to_uri_list_when_gnome_copied_files_is_unusable() {
    let gnome = b"garbage".to_vec();
    let uri_list = b"file:///home/ana/a.txt\r\n".to_vec();
    let kde_cut = b"1".to_vec();
    let state = parse(ClipboardFormats {
        uri_list: Some(&uri_list),
        gnome_copied_files: Some(&gnome),
        kde_cut_selection: Some(&kde_cut),
    })
    .expect("should parse");
    assert_eq!(state.action, ClipboardAction::Cut);
    assert_eq!(state.paths, vec![PathBuf::from("/home/ana/a.txt")]);
}

/// This is the realistic case: current KIO/Dolphin writes only
/// `text/uri-list` plus its own cut marker, never the gnome format.
#[test]
fn parse_reads_real_dolphin_style_clipboard_content() {
    let uri_list = b"file:///home/ana/a.txt\r\n".to_vec();
    let kde_cut = b"0".to_vec();
    let state = parse(ClipboardFormats {
        uri_list: Some(&uri_list),
        gnome_copied_files: None,
        kde_cut_selection: Some(&kde_cut),
    })
    .expect("should parse");
    assert_eq!(state.action, ClipboardAction::Copy);
    assert_eq!(state.paths, vec![PathBuf::from("/home/ana/a.txt")]);
}

/// No cut marker at all defaults to copy: the mistake that cannot lose
/// data, for a source (some other, unknown application) that never sets
/// KDE's marker.
#[test]
fn parse_defaults_to_copy_when_no_cut_marker_is_present() {
    let uri_list = b"file:///home/ana/a.txt\r\n".to_vec();
    let state = parse(ClipboardFormats {
        uri_list: Some(&uri_list),
        gnome_copied_files: None,
        kde_cut_selection: None,
    })
    .expect("should parse");
    assert_eq!(state.action, ClipboardAction::Copy);
}

#[test]
fn parse_returns_none_when_nothing_usable_is_on_the_clipboard() {
    assert!(parse(ClipboardFormats::default()).is_none());

    let empty_uri_list = b"# just a comment\r\n".to_vec();
    assert!(
        parse(ClipboardFormats {
            uri_list: Some(&empty_uri_list),
            gnome_copied_files: None,
            kde_cut_selection: None,
        })
        .is_none()
    );
}

// ---------------------------------------------------------------------
// `ClipboardState`: same-folder paste and the post-paste transition
// ---------------------------------------------------------------------

/// "Cortar+pegar en la misma carpeta no debe hacer nada."
#[test]
fn cut_pasted_into_its_own_source_folder_is_a_no_op() {
    let state = ClipboardState::cut(vec![PathBuf::from("/home/ana/a.txt")]);
    let target = state.paste_target(Path::new("/home/ana/a.txt"), Path::new("/home/ana"), |_| {
        panic!("must not need to check for a collision on a no-op")
    });
    assert_eq!(target, None);
}

/// "Pegar tras copiar en la misma carpeta genera un duplicado con sufijo."
#[test]
fn copy_pasted_into_its_own_source_folder_gets_a_numbered_duplicate() {
    let state = ClipboardState::copy(vec![PathBuf::from("/home/ana/informe.pdf")]);
    let target = state.paste_target(
        Path::new("/home/ana/informe.pdf"),
        Path::new("/home/ana"),
        |name| name == "informe.pdf", // the original is always taken: it's the source itself
    );
    assert_eq!(target, Some(PathBuf::from("/home/ana/informe (2).pdf")));
}

#[test]
fn copy_pasted_into_its_own_source_folder_keeps_incrementing_past_a_taken_suffix() {
    let state = ClipboardState::copy(vec![PathBuf::from("/home/ana/informe.pdf")]);
    let target = state.paste_target(
        Path::new("/home/ana/informe.pdf"),
        Path::new("/home/ana"),
        |name| name == "informe.pdf" || name == "informe (2).pdf",
    );
    assert_eq!(target, Some(PathBuf::from("/home/ana/informe (3).pdf")));
}

#[test]
fn pasting_into_a_different_folder_keeps_the_original_name() {
    // A real collision in a different, unrelated folder is a job for the
    // Replace/Skip/Keep-both dialog upstream, not for silent deduplication.
    let state = ClipboardState::copy(vec![PathBuf::from("/home/ana/informe.pdf")]);
    let target = state.paste_target(
        Path::new("/home/ana/informe.pdf"),
        Path::new("/home/ana/Escritorio"),
        |_| true, // even if it collides, this function is not the one that resolves it
    );
    assert_eq!(target, Some(PathBuf::from("/home/ana/Escritorio/informe.pdf")));
}

#[test]
fn cut_pasted_into_a_different_folder_is_a_plain_move_target() {
    let state = ClipboardState::cut(vec![PathBuf::from("/home/ana/informe.pdf")]);
    let target = state.paste_target(
        Path::new("/home/ana/informe.pdf"),
        Path::new("/home/ana/Escritorio"),
        |_| false,
    );
    assert_eq!(target, Some(PathBuf::from("/home/ana/Escritorio/informe.pdf")));
}

/// A non-UTF-8 name cannot go through `unique_name`; the entry is handed
/// back unchanged rather than the function inventing an encoding or
/// panicking.
#[test]
fn same_folder_copy_of_a_non_utf8_name_is_not_deduplicated_but_does_not_panic() {
    let entry = path_from_bytes(b"/home/ana/\xFF\xFEcosa.txt");
    let state = ClipboardState::copy(vec![entry.clone()]);
    let target = state.paste_target(&entry, Path::new("/home/ana"), |_| true);
    assert_eq!(target, Some(entry));
}

#[test]
fn an_entry_with_no_file_name_is_not_a_pasteable_target() {
    let state = ClipboardState::copy(vec![PathBuf::from("/")]);
    let target = state.paste_target(Path::new("/"), Path::new("/mnt"), |_| false);
    assert_eq!(target, None);
}

/// "Un corte que ya se pegó deja de estar pendiente": once moved, a cut has
/// nothing left to paste again, so it clears.
#[test]
fn after_paste_a_cut_clears_the_clipboard() {
    let state = ClipboardState::cut(vec![PathBuf::from("/home/ana/a.txt")]);
    assert_eq!(state.after_paste(), None);
}

/// A copy is not consumed: the source is untouched, so pasting it again is
/// exactly what a second Ctrl+V does in every file manager.
#[test]
fn after_paste_a_copy_stays_on_the_clipboard() {
    let state = ClipboardState::copy(vec![PathBuf::from("/home/ana/a.txt")]);
    let expected = state.clone();
    assert_eq!(state.after_paste(), Some(expected));
}

#[test]
fn is_cut_reflects_the_constructor_used() {
    assert!(ClipboardState::cut(vec![]).is_cut());
    assert!(!ClipboardState::copy(vec![]).is_cut());
}

// ---------------------------------------------------------------------
// Full round trip: what Kara produces, Kara can read back
// ---------------------------------------------------------------------

#[test]
fn to_formats_and_parse_round_trip_a_copy() {
    let original = ClipboardState::copy(vec![
        PathBuf::from("/home/ana/a.txt"),
        PathBuf::from("/home/ana/b con espacios.txt"),
    ]);
    let payload = original.to_formats();
    let parsed = parse(ClipboardFormats {
        uri_list: Some(payload.uri_list.as_bytes()),
        gnome_copied_files: Some(payload.gnome_copied_files.as_bytes()),
        kde_cut_selection: Some(payload.kde_cut_selection.as_bytes()),
    })
    .expect("should parse");
    assert_eq!(parsed, original);
}

#[test]
fn to_formats_and_parse_round_trip_a_cut() {
    let original = ClipboardState::cut(vec![PathBuf::from("/home/ana/Álbum/Ámbar.jpg")]);
    let payload = original.to_formats();
    let parsed = parse(ClipboardFormats {
        uri_list: Some(payload.uri_list.as_bytes()),
        gnome_copied_files: Some(payload.gnome_copied_files.as_bytes()),
        kde_cut_selection: Some(payload.kde_cut_selection.as_bytes()),
    })
    .expect("should parse");
    assert_eq!(parsed, original);
}

/// The realistic case again: only what real KIO actually writes, run
/// through what Kara produces, must still agree with itself.
#[test]
fn to_formats_round_trips_through_only_the_kio_formats() {
    let original = ClipboardState::cut(vec![PathBuf::from("/home/ana/a.txt")]);
    let payload = original.to_formats();
    let parsed = parse(ClipboardFormats {
        uri_list: Some(payload.uri_list.as_bytes()),
        gnome_copied_files: None,
        kde_cut_selection: Some(payload.kde_cut_selection.as_bytes()),
    })
    .expect("should parse");
    assert_eq!(parsed, original);
}
