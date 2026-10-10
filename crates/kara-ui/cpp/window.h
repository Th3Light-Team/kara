// The few window-system calls cxx-qt-lib does not wrap.
//
// Like `clipboard.h`, this decides nothing: it forwards to Qt.
#pragma once

#include "rust/cxx.h"

namespace kara {

/// Names the `.desktop` file the application belongs to. On Wayland it becomes
/// the window's app_id, which is how GNOME Shell and Plasma find Kara's icon
/// and name for the dash, the task bar and Alt+Tab; on X11 it is the window
/// class. Without it the app_id is the binary's name, `kara-ui`, which no
/// `.desktop` file matches.
void set_desktop_file_name(::rust::Str name);

/// Brings Kara's window to the front for a request that came from another
/// application: "show in folder" from a browser.
///
/// `activation_token` is the XDG activation token (Wayland) or startup id
/// (X11) the caller passed along, possibly empty. On Wayland a window cannot
/// take the focus on its own, only with a token from the application the user
/// was in; Qt reads it from `XDG_ACTIVATION_TOKEN` when asked to activate.
void present_window(::rust::Str activation_token);

} // namespace kara
