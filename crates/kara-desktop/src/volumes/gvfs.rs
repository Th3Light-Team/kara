//! Network locations GNOME mounted, through gvfs's FUSE directory.
//!
//! Every location gvfs mounts — SMB shares, SFTP, WebDAV, FTP, phones over
//! MTP — also appears as a plain directory under `$XDG_RUNTIME_DIR/gvfs`,
//! named by its mount spec (`smb-share:server=nas,share=fotos`). Kara browses
//! those directories like any other: they are where the files really are for
//! a program that does not speak GIO.
//!
//! The names follow `g_mount_spec_to_string` (gvfs `common/gmountspec.c`):
//! `type:key=value,…`, keys sorted, values percent-encoded except for
//! unreserved characters and `$&'()*+`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use percent_encoding::percent_decode_str;

use super::{NetworkLocation, Volume, VolumeError, VolumeKind, default_icons};

/// Where gvfs puts its FUSE mounts for this user, if it exists.
#[must_use]
pub fn fuse_root() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", rustix::process::getuid().as_raw())));
    let root = runtime.join("gvfs");
    root.is_dir().then_some(root)
}

/// The network locations mounted now. Reading the FUSE root is one
/// directory listing; it never touches the remote side.
#[must_use]
pub fn network_volumes() -> Vec<Volume> {
    let Some(root) = fuse_root() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut volumes: Vec<Volume> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            volume_for(&name, &root.join(&name))
        })
        .collect();
    volumes.sort_by(|a, b| a.id.cmp(&b.id));
    volumes
}

/// Reads one FUSE directory name.
#[must_use]
pub fn volume_for(name: &str, path: &Path) -> Option<Volume> {
    let location = parse_name(name)?;
    let kind = match location.scheme.as_str() {
        "mtp" | "afc" => VolumeKind::Phone,
        "gphoto2" => VolumeKind::Camera,
        _ => VolumeKind::Network,
    };
    Some(Volume {
        id: format!("gvfs:{name}"),
        label: None,
        size: 0,
        kind,
        icons: default_icons(kind).iter().map(|s| (*s).to_string()).collect(),
        mount_point: Some(path.to_path_buf()),
        filesystem: None,
        locked: false,
        drive: None,
        drive_name: None,
        network: Some(location),
    })
}

/// `smb-share:domain=X,server=nas,share=fotos` → its parts. `None` for
/// anything that is not a mount spec (a stray file, a dot-directory).
#[must_use]
pub fn parse_name(name: &str) -> Option<NetworkLocation> {
    if name.starts_with('.') {
        return None;
    }
    let (scheme, rest) = name.split_once(':')?;
    if scheme.is_empty() {
        return None;
    }
    let mut location = NetworkLocation {
        scheme: scheme.to_string(),
        ..NetworkLocation::default()
    };
    for pair in rest.split(',') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = percent_decode_str(value).decode_utf8_lossy().into_owned();
        if value.is_empty() {
            continue;
        }
        match key {
            "host" | "server" => location.host = Some(value),
            "share" | "volume" => location.share = Some(value),
            "user" => location.user = Some(value),
            _ => {}
        }
    }
    Some(location)
}

/// Disconnects a network location. `gio mount -u` accepts the FUSE path and
/// maps it back to the gvfs mount, so no URI needs rebuilding here.
pub fn unmount(path: &Path) -> Result<(), VolumeError> {
    let output = Command::new("gio")
        .arg("mount")
        .arg("-u")
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| VolumeError::Failed("no está `gio` para desconectar la ubicación de red".into()))?;
    if output.status.success() {
        return Ok(());
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if message.contains("busy") || message.contains("ocupado") {
        let shown = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        return Err(VolumeError::Busy(shown));
    }
    Err(VolumeError::Failed(message))
}

/// Calls `changed` when gvfs mounts or unmounts something. gvfsd broadcasts
/// `Mounted`/`Unmounted` on `org.gtk.vfs.MountTracker`; a FUSE directory gets
/// no inotify events, so these signals are the only way to hear of it.
pub fn watch(changed: Arc<dyn Fn() + Send + Sync>) {
    let (tx, rx) = mpsc::channel::<()>();
    let listener = std::thread::Builder::new().name("kara-gvfs".into()).spawn(move || {
        let Ok(connection) = zbus::blocking::Connection::session() else {
            return;
        };
        let Ok(rule) = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.gtk.vfs.MountTracker")
            .map(|b| b.build())
        else {
            return;
        };
        let Ok(messages) = zbus::blocking::MessageIterator::for_match_rule(rule, &connection, Some(64)) else {
            return;
        };
        for message in messages {
            if message.is_err() || tx.send(()).is_err() {
                break;
            }
        }
    });
    if listener.is_err() {
        return;
    }
    let _ = std::thread::Builder::new().name("kara-gvfs-debounce".into()).spawn(move || {
        while rx.recv().is_ok() {
            while rx.recv_timeout(Duration::from_millis(200)).is_ok() {}
            changed();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_smb_share_names_its_server_and_share() {
        let location = parse_name("smb-share:domain=WORKGROUP,server=nas.local,share=fotos%202024,user=ana");
        assert_eq!(
            location,
            Some(NetworkLocation {
                scheme: "smb-share".into(),
                host: Some("nas.local".into()),
                share: Some("fotos 2024".into()),
                user: Some("ana".into()),
                display_name: None,
            })
        );
    }

    #[test]
    fn sftp_and_dav_name_their_host_and_user() {
        let sftp = parse_name("sftp:host=example.org,port=2222,user=bob").unwrap_or_default();
        assert_eq!(sftp.scheme, "sftp");
        assert_eq!(sftp.host.as_deref(), Some("example.org"));
        assert_eq!(sftp.user.as_deref(), Some("bob"));
        let dav = parse_name("dav:host=cloud.example.org,ssl=true,user=ana,prefix=%2Fremote.php%2Fdav").unwrap_or_default();
        assert_eq!(dav.host.as_deref(), Some("cloud.example.org"));
    }

    #[test]
    fn phones_and_cameras_get_their_own_kind() {
        let phone = volume_for("mtp:host=SAMSUNG_Android_R58M", Path::new("/run/user/1000/gvfs/x"));
        assert_eq!(phone.map(|v| v.kind), Some(VolumeKind::Phone));
        let camera = volume_for("gphoto2:host=Canon_EOS", Path::new("/run/user/1000/gvfs/y"));
        assert_eq!(camera.map(|v| v.kind), Some(VolumeKind::Camera));
    }

    #[test]
    fn a_network_volume_is_mounted_where_its_directory_is() {
        let path = Path::new("/run/user/1000/gvfs/smb-share:server=nas,share=x");
        let volume = volume_for("smb-share:server=nas,share=x", path);
        assert_eq!(volume.as_ref().and_then(|v| v.mount_point.as_deref()), Some(path));
        assert!(volume.is_some_and(|v| v.can_eject() && v.id.starts_with("gvfs:")));
    }

    #[test]
    fn what_is_not_a_mount_spec_is_skipped() {
        assert!(parse_name(".hidden").is_none());
        assert!(parse_name("no-colon-here").is_none());
        assert!(parse_name(":x=y").is_none());
    }
}
