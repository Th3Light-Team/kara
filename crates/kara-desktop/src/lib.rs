//! Kara's boundary with the desktop it runs on.
//!
//! The other crates are desktop-agnostic by construction: they follow the
//! FreeDesktop specifications — trash, MIME, thumbnails, icon themes — that
//! GNOME and Plasma implement alike. What is left lives here, behind
//! [`DesktopIntegration`]: how the desktop looks, handing it a file to open,
//! its terminal, its drives, and answering it when another application wants
//! a folder shown.
//!
//! Each piece goes through the most standard route there is, so one Kara
//! behaves natively on both desktops:
//!
//! | Need | Route | Fallback |
//! |---|---|---|
//! | Dark mode, accent, icon theme | Settings portal | what Qt can tell |
//! | Open a file | `gio open` / `xdg-open` | OpenURI portal |
//! | «Abrir con» | `.desktop` + `mimeapps.list` | — |
//! | Terminal | `$TERMINAL`, `xdg-terminal-exec` | known emulators |
//! | Drives | UDisks2 (D-Bus) | the mount table |
//! | Network locations | gvfs FUSE mounts | — |
//! | «Show in folder» | `org.freedesktop.FileManager1` | — |
//!
//! Desktop-specific code is confined to the module that needs it and never
//! leaks into the trait.

#![forbid(unsafe_code)]

pub mod appearance;
pub mod apps;
pub mod file_manager;
pub mod launch;
pub mod session;
pub mod terminal;
pub mod volumes;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use appearance::{Accent, Appearance, ColorScheme};
pub use apps::{AppInfo, Associations, BaseDirs};
pub use file_manager::{FileManagerRequest, FileManagerService, Ownership};
pub use launch::LaunchError;
pub use volumes::{NetworkLocation, Volume, VolumeAction, VolumeError, VolumeKind};

/// A callback the desktop calls from one of its own threads.
pub type Sink<T> = Arc<dyn Fn(T) + Send + Sync>;
/// A notification without data.
pub type Notify = Arc<dyn Fn() + Send + Sync>;
/// What a volume action reports when it finishes: where the volume is mounted
/// now, if it is.
pub type VolumeDone = Box<dyn FnOnce(Result<Option<PathBuf>, VolumeError>) + Send>;

/// Everything Kara asks of the desktop. Calls that may wait on another
/// process take a callback and return at once; none of them blocks the UI.
pub trait DesktopIntegration: Send + Sync {
    /// The current appearance, or `None` when the desktop does not say.
    /// One D-Bus round trip.
    fn appearance(&self) -> Option<Appearance>;
    /// Calls `sink` on every later change.
    fn watch_appearance(&self, sink: Sink<Appearance>);

    /// Opens `path` with its default application; `failed` hears of it if
    /// nothing could.
    fn open(&self, path: &Path, failed: Sink<LaunchError>);
    /// Starts `app` on `files`.
    fn launch(&self, app: &AppInfo, files: &[PathBuf]) -> Result<(), LaunchError>;
    /// Opens the user's terminal in `directory`.
    fn open_terminal(&self, directory: &Path, failed: Sink<LaunchError>);

    /// The volumes to list now: local ones and network locations. May wait
    /// for UDisks2; call it off the UI thread.
    fn volumes(&self) -> Vec<Volume>;
    /// Calls `changed` whenever the answer of [`Self::volumes`] may differ.
    fn watch_volumes(&self, changed: Notify);
    /// Mounts, unlocks or ejects the volume `id`, then calls `done`.
    fn volume_action(&self, id: &str, action: VolumeAction, done: VolumeDone);
    /// Attaches a disk image and mounts it, then calls `done`.
    fn mount_image(&self, image: &Path, done: VolumeDone);

    /// Starts answering `org.freedesktop.FileManager1`.
    fn serve_file_manager(&self, handler: Sink<FileManagerRequest>) -> zbus::Result<FileManagerService>;
}

/// Which way a double click opens a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRoute {
    /// The desktop's launchers (`gio open`, `xdg-open`), with the portal
    /// as the fallback.
    LaunchersFirst,
    /// The OpenURI portal, with the launchers as the fallback.
    PortalFirst,
}

/// The implementation for every FreeDesktop desktop: GNOME, Plasma and the
/// rest.
pub struct Freedesktop {
    open_route: OpenRoute,
}

impl Freedesktop {
    #[must_use]
    pub fn new(open_route: OpenRoute) -> Self {
        Self { open_route }
    }
}

/// The desktop Kara runs on.
#[must_use]
pub fn detect() -> Arc<dyn DesktopIntegration> {
    Arc::new(Freedesktop::new(OpenRoute::LaunchersFirst))
}

fn in_background(name: &str, work: impl FnOnce() + Send + 'static) {
    // Without a thread the work cannot happen without blocking the window;
    // the callers' callbacks then never fire, which is what a desktop that
    // cannot do it looks like anyway.
    let _ = std::thread::Builder::new().name(name.into()).spawn(work);
}

impl DesktopIntegration for Freedesktop {
    fn appearance(&self) -> Option<Appearance> {
        appearance::read()
    }

    fn watch_appearance(&self, sink: Sink<Appearance>) {
        appearance::watch(sink);
    }

    fn open(&self, path: &Path, failed: Sink<LaunchError>) {
        let path = path.to_path_buf();
        let route = self.open_route;
        in_background("kara-open", move || {
            let result = match route {
                OpenRoute::LaunchersFirst => launch::open_with_launchers(&path)
                    .or_else(|error| launch::open_with_portal(&path).map_err(|_| error)),
                OpenRoute::PortalFirst => {
                    launch::open_with_portal(&path).or_else(|_| launch::open_with_launchers(&path))
                }
            };
            if let Err(error) = result {
                failed(error);
            }
        });
    }

    fn launch(&self, app: &AppInfo, files: &[PathBuf]) -> Result<(), LaunchError> {
        launch::launch(app, files)
    }

    fn open_terminal(&self, directory: &Path, failed: Sink<LaunchError>) {
        let directory = directory.to_path_buf();
        in_background("kara-terminal", move || {
            if let Err(error) = terminal::open_terminal(&directory) {
                failed(error);
            }
        });
    }

    fn volumes(&self) -> Vec<Volume> {
        let mut out = volumes::local_volumes();
        out.extend(volumes::gvfs::network_volumes());
        out
    }

    fn watch_volumes(&self, changed: Notify) {
        volumes::udisks::watch(changed.clone());
        volumes::gvfs::watch(changed);
    }

    fn volume_action(&self, id: &str, action: VolumeAction, done: VolumeDone) {
        let id = id.to_string();
        in_background("kara-volume", move || done(volumes::act(&id, &action)));
    }

    fn mount_image(&self, image: &Path, done: VolumeDone) {
        let image = image.to_path_buf();
        in_background("kara-volume", move || done(volumes::mount_image(&image).map(Some)));
    }

    fn serve_file_manager(&self, handler: Sink<FileManagerRequest>) -> zbus::Result<FileManagerService> {
        file_manager::serve(handler)
    }
}
