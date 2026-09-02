// Access to the desktop clipboard.
//
// Qt owns the clipboard and cxx-qt-lib does not wrap `QClipboard` or
// `QMimeData`, so this is the smallest possible bridge: it moves opaque strings
// in and out and decides nothing. Which MIME types exist, how the URIs are
// escaped and what a cut means all live in `kara-fs::clipboard`, where they can
// be tested without a running desktop.
#pragma once

#include <cstdint>

#include "rust/cxx.h"

namespace kara {

/// Puts a file selection on the clipboard, in every format the desktop's file
/// managers read.
void clipboard_write(::rust::Str uri_list, ::rust::Str gnome, bool cut);

/// The `text/uri-list` on the clipboard, or empty if there is none.
::rust::String clipboard_uri_list();

/// The `x-special/gnome-copied-files` on the clipboard, or empty.
::rust::String clipboard_gnome();

/// Empties the clipboard, after a cut has been pasted.
void clipboard_clear();

/// Whether KDE marked the current clipboard contents as a cut.
bool clipboard_kde_cut();

} // namespace kara
