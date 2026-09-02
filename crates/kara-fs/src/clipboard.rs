//! The file clipboard: what goes on the system clipboard on cut/copy, and
//! how to read what another program put there.
//!
//! Reference convenience: `ground/spec/02-seleccion.md`, "Cortar, copiar y
//! pegar" — "Pegar tras copiar en la misma carpeta genera un duplicado con
//! sufijo ('archivo - copia')" and "Cortar+pegar en la misma carpeta no debe
//! hacer nada".
//!
//! **No Qt here.** This module only produces and parses the byte payloads
//! for the mimetypes involved; talking to the actual system clipboard
//! (`QClipboard`/`QMimeData`) is the UI layer's job. Everything below takes
//! and returns plain bytes/strings so it can be tested without a display.
//!
//! # Interop: what the format actually is
//!
//! `text/uri-list` (RFC 2483) is the one universally understood format —
//! one `file://` URI per line, CRLF-terminated — but it cannot tell a copy
//! from a cut. Two more mimetypes carry that:
//!
//! - `x-special/gnome-copied-files`: a first line of `copy` or `cut`,
//!   followed by the same `file://` URIs, LF-joined. This is the
//!   long-standing GTK/GLib convention (Nautilus, Nemo) for the same
//!   purpose.
//! - `application/x-kde-cutselection`: KDE's own marker, independent of the
//!   above.
//!
//! This was **not** taken on faith. This machine has real KIO 6.24
//! installed (`ground/spec` frozen references aside, `CLAUDE.md` records
//! the environment as KDE Plasma with the KF packages present), so the
//! claim was checked against it rather than against memory:
//! `libKF6KIOWidgets.so.6.24.0` exports `KIO::setClipboardDataCut` /
//! `KIO::isClipboardDataCut`, and disassembling `setClipboardDataCut`
//! shows it calls `QMimeData::setData("application/x-kde-cutselection",
//! QByteArray(...))` with the literal one-byte string `"1"` when cutting
//! and `"0"` otherwise — no size-based encoding, no trailing newline.
//! Critically, **the string `x-special/gnome-copied-files` does not appear
//! anywhere** in that library, in `libKF6KIOCore`/`libKF6KIOGui`, in
//! `libdolphinprivate`, or in the `dolphin` binary itself: current KIO
//! does not write it at all, and relies purely on `text/uri-list` (handled
//! by Qt itself — `QMimeData::setUrls`, whose own `text/uri-list` literal
//! lives in `libQt6Gui`) plus its own cut marker.
//!
//! So Kara writes all three anyway — `text/uri-list` and
//! `application/x-kde-cutselection` because that is what really makes
//! pasting into (and reading a copy/cut from) *this* Dolphin work, and
//! `x-special/gnome-copied-files` on top as cheap insurance for GTK file
//! managers, which were not available on this machine to check the same
//! way. Reading is equally defensive: `x-special/gnome-copied-files` is
//! honoured when present (it is self-contained: one format names both the
//! paths and the action), and `text/uri-list` plus the KDE marker are the
//! fallback — which is the path real Dolphin content actually takes.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use percent_encoding::percent_decode;

use kara_core::unique_name;

use crate::uri::file_uri;

/// `text/uri-list`, RFC 2483: the one format every file manager reads.
pub const MIME_URI_LIST: &str = "text/uri-list";
/// GTK/GLib's cut-vs-copy convention (Nautilus, Nemo). Not written by
/// current KIO (see the module doc), but read defensively and still
/// written for GTK interop.
pub const MIME_GNOME_COPIED_FILES: &str = "x-special/gnome-copied-files";
/// KDE's own cut marker, alongside plain `text/uri-list`. Confirmed against
/// the `KIO::setClipboardDataCut`/`KIO::isClipboardDataCut` implementation
/// installed on this machine — see the module doc.
pub const MIME_KDE_CUT_SELECTION: &str = "application/x-kde-cutselection";

/// Whether the clipboard holds a move-pending cut or a plain copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardAction {
    Copy,
    Cut,
}

/// A snapshot of the file clipboard: what is on it, and whether pasting it
/// should move or duplicate.
///
/// This is deliberately not tied to the system clipboard's lifetime: it is
/// either produced locally (Ctrl+X/Ctrl+C on the current selection) or
/// parsed from what another process put there ([`parse`]), and it is up to
/// the caller to hold on to it and to hand the result of [`after_paste`]
/// back to whatever tracks "what is on the clipboard right now".
///
/// [`after_paste`]: ClipboardState::after_paste
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardState {
    pub action: ClipboardAction,
    pub paths: Vec<PathBuf>,
}

impl ClipboardState {
    #[must_use]
    pub fn copy(paths: Vec<PathBuf>) -> Self {
        Self {
            action: ClipboardAction::Copy,
            paths,
        }
    }

    #[must_use]
    pub fn cut(paths: Vec<PathBuf>) -> Self {
        Self {
            action: ClipboardAction::Cut,
            paths,
        }
    }

    #[must_use]
    pub fn is_cut(&self) -> bool {
        self.action == ClipboardAction::Cut
    }

    /// What pasting `entry` (one of `self.paths`) into `destination_dir`
    /// should produce: `None` means the paste is a no-op for this entry.
    ///
    /// `exists` answers whether a candidate name is already taken in
    /// `destination_dir`; looking it up is I/O, which this layer does not
    /// do, so it comes in as a closure — the same contract
    /// [`kara_core::naming::unique_name`] already uses.
    ///
    /// Two spec-mandated special cases, both about pasting into the very
    /// folder an entry came from:
    ///
    /// - **Cut, same folder:** the source *is* the destination, so moving
    ///   it would be a no-op at best and a self-clobber at worst. The spec
    ///   is explicit that this does nothing.
    /// - **Copy, same folder:** a plain name collision — but not a real
    ///   conflict, since it is this operation's expected outcome, not a
    ///   coincidence. It is resolved silently with a numbered duplicate via
    ///   [`unique_name`], instead of surfacing the
    ///   Replace/Skip/Keep-both dialog that a genuine collision would.
    ///
    /// Anywhere else — a different destination folder — the entry keeps its
    /// name; a real collision there is a job for that dialog, not this
    /// function.
    #[must_use]
    pub fn paste_target(
        &self,
        entry: &Path,
        destination_dir: &Path,
        exists: impl Fn(&str) -> bool,
    ) -> Option<PathBuf> {
        let Some(name) = entry.file_name() else {
            // No file name at all (e.g. `/`): not a pasteable entry.
            return None;
        };
        let same_folder = entry.parent() == Some(destination_dir);

        let Some(name) = name.to_str() else {
            // A non-UTF-8 name cannot go through `unique_name`, which is
            // `&str`-based. Hand it back unchanged rather than invent an
            // encoding here; a real same-folder collision then falls
            // through to the ordinary conflict dialog instead of being
            // silently deduplicated, which is a defensible degradation for
            // what is already a rare case.
            return Some(destination_dir.join(entry.file_name()?));
        };

        if same_folder {
            if self.action == ClipboardAction::Cut {
                return None;
            }
            return Some(destination_dir.join(unique_name(name, exists)));
        }
        Some(destination_dir.join(name))
    }

    /// The clipboard state to keep after a paste has actually happened.
    ///
    /// A cut is single-use: it names files to move, and once the move has
    /// happened there is nothing left at the source to move again — a
    /// second Ctrl+V is a no-op in Windows Explorer and Dolphin alike, and
    /// the "cut, faded" marking on the originals goes away with it. This is
    /// the explicit decision the spec leaves open ("un corte que ya se
    /// pegó deja de estar pendiente"): a pasted cut clears the clipboard.
    ///
    /// A copy is not consumed: the source files are untouched, so pasting
    /// the same copy again is exactly what repeating Ctrl+V does in every
    /// file manager — each paste grows another numbered duplicate via
    /// [`paste_target`](Self::paste_target).
    #[must_use]
    pub fn after_paste(self) -> Option<Self> {
        match self.action {
            ClipboardAction::Cut => None,
            ClipboardAction::Copy => Some(self),
        }
    }

    /// Renders this state as the byte payloads to hand to the system
    /// clipboard, one per mimetype it should be registered under.
    #[must_use]
    pub fn to_formats(&self) -> ClipboardPayload {
        ClipboardPayload {
            uri_list: render_uri_list(&self.paths),
            gnome_copied_files: render_gnome_copied_files(self.action, &self.paths),
            kde_cut_selection: kde_cut_selection_value(self.action),
        }
    }
}

/// The rendered clipboard payloads, one field per mimetype in
/// [`MIME_URI_LIST`], [`MIME_GNOME_COPIED_FILES`] and
/// [`MIME_KDE_CUT_SELECTION`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPayload {
    pub uri_list: String,
    pub gnome_copied_files: String,
    pub kde_cut_selection: &'static str,
}

/// Whatever of the three clipboard mimetypes this module understands is
/// available to read, straight from the system clipboard's own formats. Any
/// of them can be missing — most real senders only provide some.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClipboardFormats<'a> {
    pub uri_list: Option<&'a [u8]>,
    pub gnome_copied_files: Option<&'a [u8]>,
    pub kde_cut_selection: Option<&'a [u8]>,
}

/// Makes sense of whatever another program left on the clipboard.
///
/// `x-special/gnome-copied-files` is tried first: it is self-contained (one
/// payload names both the action and the paths), so when it parses there is
/// nothing to combine. Otherwise `text/uri-list` supplies the paths and
/// `application/x-kde-cutselection` the action — defaulting to
/// [`ClipboardAction::Copy`] when that marker is absent or unrecognised,
/// since assuming a copy is the side of the mistake that cannot lose data.
/// `None` comes back only when nothing usable was found at all.
#[must_use]
pub fn parse(formats: ClipboardFormats<'_>) -> Option<ClipboardState> {

    if let Some(bytes) = formats.gnome_copied_files
        && let Some(state) = parse_gnome_copied_files(bytes)
    {
        return Some(state);
    }

    let paths = parse_uri_list(formats.uri_list?);
    if paths.is_empty() {
        return None;
    }
    let action = match formats.kde_cut_selection {
        Some(bytes) if parse_kde_cut_selection(bytes) => ClipboardAction::Cut,
        _ => ClipboardAction::Copy,
    };
    Some(ClipboardState { action, paths })
}

/// Renders `text/uri-list`: one `file://` URI per line, CRLF-terminated, as
/// RFC 2483 asks.
#[must_use]
pub fn render_uri_list(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for path in paths {
        out.push_str(&file_uri(path));
        out.push_str("\r\n");
    }
    out
}

/// Parses a `text/uri-list` payload into the local paths it names.
///
/// This is untrusted input from any process that can write to the system
/// clipboard: a comment line (`#…`), a blank line, a URI of some other
/// scheme, a relative reference, or a bare `LF` instead of the mandated
/// `CRLF` must not take the rest of the list down with it — they are
/// skipped and everything that does parse is still returned.
#[must_use]
pub fn parse_uri_list(bytes: &[u8]) -> Vec<PathBuf> {
    split_lines(bytes)
        .filter(|line| !line.is_empty() && !line.starts_with(b"#"))
        .filter_map(path_from_uri)
        .collect()
}

/// Renders `x-special/gnome-copied-files`: `copy` or `cut` on the first
/// line, then one `file://` URI per line, `LF`-joined (not `CRLF` — this is
/// not `text/uri-list`, and no real implementation of this convention uses
/// `CRLF` here).
#[must_use]
pub fn render_gnome_copied_files(action: ClipboardAction, paths: &[PathBuf]) -> String {
    let mut out = String::from(match action {
        ClipboardAction::Copy => "copy",
        ClipboardAction::Cut => "cut",
    });
    for path in paths {
        out.push('\n');
        out.push_str(&file_uri(path));
    }
    out
}

/// Parses an `x-special/gnome-copied-files` payload.
///
/// `None` covers every way this can be unusable: no first line at all, a
/// first line that is not exactly `copy` or `cut`, or a first line that
/// parses fine but is followed by no URI that survives [`path_from_uri`].
/// That last case matters for [`parse`]: it is what lets a malformed
/// gnome-copied-files payload fall back to `text/uri-list` instead of
/// silently discarding files it could have recovered.
#[must_use]
pub fn parse_gnome_copied_files(bytes: &[u8]) -> Option<ClipboardState> {
    let mut lines = split_lines(bytes);
    let action = match lines.next()? {
        b"copy" => ClipboardAction::Copy,
        b"cut" => ClipboardAction::Cut,
        _ => return None,
    };
    let paths: Vec<PathBuf> = lines
        .filter(|line| !line.is_empty())
        .filter_map(path_from_uri)
        .collect();
    if paths.is_empty() {
        return None;
    }
    Some(ClipboardState { action, paths })
}

/// The `application/x-kde-cutselection` payload for a given action: the
/// literal one-byte strings `KIO::setClipboardDataCut` writes — see the
/// module doc for how that was confirmed rather than assumed.
#[must_use]
pub fn kde_cut_selection_value(action: ClipboardAction) -> &'static str {
    match action {
        ClipboardAction::Cut => "1",
        ClipboardAction::Copy => "0",
    }
}

/// Reads an `application/x-kde-cutselection` payload: `true` only for
/// exactly `"1"` (surrounding ASCII whitespace tolerated, in case some
/// sender adds a trailing newline). Anything else — `"0"`, empty, garbage —
/// reads as "not cut", which is what [`parse`] treats as the safe default
/// even when this mimetype is entirely absent.
#[must_use]
pub fn parse_kde_cut_selection(bytes: &[u8]) -> bool {
    bytes.trim_ascii() == b"1"
}

/// The inverse of [`file_uri`]: a `file://` URI back to the [`PathBuf`] it
/// names, percent-decoding undone.
///
/// A path on Linux is a byte string, not text, so this works on raw bytes
/// throughout and only ever produces a [`PathBuf`] via [`OsString::from_vec`]
/// — never a lossy UTF-8 conversion, which would silently change the path.
///
/// `None` for anything that is not a plain local `file://` URI: another
/// scheme, a `file://host/path` naming a *different* machine (Kara cannot
/// resolve that locally), or a percent-decoded result containing a `NUL`
/// byte, which cannot be part of a real path and safely marks the input as
/// hostile rather than merely unusual.
#[must_use]
pub fn path_from_uri(uri: &[u8]) -> Option<PathBuf> {
    const PREFIX: &[u8] = b"file://";
    let rest = uri.strip_prefix(PREFIX)?;
    if !rest.starts_with(b"/") {
        // Empty authority is the only form `file_uri` (or any sane
        // encoder) ever produces for a local path — `file:///abs/path`.
        // Anything else here is a non-empty host or garbage, not a path on
        // this machine.
        return None;
    }
    let decoded: Vec<u8> = percent_decode(rest).collect();
    if decoded.is_empty() || decoded.contains(&0) {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(decoded)))
}

/// Splits a byte payload into lines, accepting both `CRLF` and a bare `LF`
/// — real senders are not always RFC-perfect, and rejecting the whole
/// payload over one wrong line ending would lose files a stricter parser
/// could have recovered.
fn split_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
}
