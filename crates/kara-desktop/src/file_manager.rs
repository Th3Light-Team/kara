//! Being the desktop's file manager: the `org.freedesktop.FileManager1`
//! D-Bus interface.
//!
//! It is what a browser's «Show in folder», an editor's «Reveal» and the
//! OpenURI portal's `OpenDirectory` call (snap and Flatpak applications go
//! through the portal, which calls `ShowItems`). Whoever owns the name gets the
//! request; without it, GNOME keeps opening Nautilus.
//!
//! The name is requested allowing replacement and queueing: a newer Kara
//! takes over from an older one, and if another file manager holds it —
//! Nautilus keeps running as a GApplication service on GNOME and does not let
//! go — Kara waits in line and gets it the moment that one exits.

use std::path::PathBuf;
use std::sync::Arc;

use zbus::blocking::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};

const NAME: &str = "org.freedesktop.FileManager1";
const PATH: &str = "/org/freedesktop/FileManager1";

/// What another application asked Kara to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileManagerRequest {
    /// Open each folder.
    ShowFolders { folders: Vec<PathBuf>, activation_token: String },
    /// Open the folders that contain these items, with the items selected.
    ShowItems { items: Vec<PathBuf>, activation_token: String },
    /// Show the items' properties.
    ShowItemProperties { items: Vec<PathBuf>, activation_token: String },
}

/// Whether Kara got the name or is waiting for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    Owner,
    Queued,
}

/// The running service. Dropping it gives the name back.
pub struct FileManagerService {
    _connection: Connection,
    pub ownership: Ownership,
}

struct Interface {
    handler: Arc<dyn Fn(FileManagerRequest) + Send + Sync>,
}

/// Local paths from the URIs the caller sent. Anything but `file://` is
/// dropped: Kara browses the local tree (gvfs locations included, through
/// their FUSE paths) and has nowhere to show a bare `smb://` URI.
fn paths(uris: &[String]) -> Vec<PathBuf> {
    uris.iter()
        .filter_map(|uri| kara_fs::clipboard::path_from_uri(uri.as_bytes()))
        .collect()
}

#[zbus::interface(name = "org.freedesktop.FileManager1")]
impl Interface {
    fn show_folders(&self, uris: Vec<String>, startup_id: String) {
        (self.handler)(FileManagerRequest::ShowFolders {
            folders: paths(&uris),
            activation_token: startup_id,
        });
    }

    fn show_items(&self, uris: Vec<String>, startup_id: String) {
        (self.handler)(FileManagerRequest::ShowItems {
            items: paths(&uris),
            activation_token: startup_id,
        });
    }

    fn show_item_properties(&self, uris: Vec<String>, startup_id: String) {
        (self.handler)(FileManagerRequest::ShowItemProperties {
            items: paths(&uris),
            activation_token: startup_id,
        });
    }
}

/// Exports the interface on the session bus and asks for the name. The
/// handler runs on one of zbus's threads.
pub fn serve(handler: Arc<dyn Fn(FileManagerRequest) + Send + Sync>) -> zbus::Result<FileManagerService> {
    let connection = zbus::blocking::connection::Builder::session()?
        .serve_at(PATH, Interface { handler })?
        .build()?;
    let reply = connection.request_name_with_flags(
        NAME,
        RequestNameFlags::AllowReplacement | RequestNameFlags::ReplaceExisting,
    )?;
    let ownership = match reply {
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => Ownership::Owner,
        RequestNameReply::InQueue | RequestNameReply::Exists => Ownership::Queued,
    };
    Ok(FileManagerService {
        _connection: connection,
        ownership,
    })
}

#[cfg(test)]
mod tests {
    use super::paths;
    use std::path::PathBuf;

    #[test]
    fn only_local_uris_become_paths() {
        let uris = vec![
            "file:///home/ana/informe%20final.pdf".to_string(),
            "smb://nas/fotos".to_string(),
            "file://otra-maquina/x".to_string(),
        ];
        assert_eq!(paths(&uris), [PathBuf::from("/home/ana/informe final.pdf")]);
    }
}
