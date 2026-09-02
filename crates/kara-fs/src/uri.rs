//! `file://` URIs for local paths.
//!
//! This is not a formatting detail: the thumbnail cache is keyed by the MD5 of
//! this exact string, and it is shared with every other file manager on the
//! desktop. Escape one byte differently from GLib or Qt and Kara silently stops
//! seeing the thumbnails Dolphin already generated — and writes its own
//! duplicates next to them.

use std::path::Path;

use percent_encoding::{AsciiSet, CONTROLS, percent_encode};

/// Bytes escaped on top of the ASCII controls.
///
/// What is left unescaped is the RFC 3986 unreserved set plus the sub-delims
/// and `:@/`, which is what both `g_filename_to_uri` and `QUrl::fromLocalFile`
/// leave alone. Every byte `>= 0x80` is escaped unconditionally by
/// `percent_encoding`, which only ever lets plain ASCII through.
const URI_PATH_ESCAPE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// The `file://` URI of an absolute local path.
///
/// The path is taken as raw bytes, not as text: a file name on Linux is a byte
/// string and need not be valid UTF-8. Going through a lossy conversion would
/// change the URI, and with it the thumbnail every other application agreed on.
#[must_use]
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let mut uri = String::from("file://");
    uri.extend(percent_encode(path.as_os_str().as_bytes(), URI_PATH_ESCAPE_SET));
    uri
}
