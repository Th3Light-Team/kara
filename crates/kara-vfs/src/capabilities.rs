//! `Capabilities`: what a backend has, so the UI never branches on protocol.

/// Flags a backend declares. `Default` is all `false`: a backend declares what
/// it HAS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct Capabilities {
    pub trash: bool,
    pub atomic_rename: bool,
    pub server_side_copy: bool,
    pub real_directories: bool,
    pub posix_permissions: bool,
    pub symlinks: bool,
    pub watch: bool,
    pub undo_rename: bool,
    pub undo_move: bool,
}
