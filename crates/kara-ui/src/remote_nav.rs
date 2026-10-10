//! Remote locations inside the tab model.
//!
//! Tabs, history and the view all hold a `PathBuf`. A remote folder rides in
//! that same type as its canonical URI (`kara+sftp://work-nas/dir`), so the
//! rest of the bridge is untouched; every place that must tell the two apart
//! asks here. Pure logic, no Qt.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use kara_core::breadcrumb::{Segment, SegmentKind};
use kara_vfs::{DriveId, Location, RemotePath};

const PREFIX: &str = "kara+";

/// Whether `path` is a remote location rather than a local folder.
pub fn is_remote(path: &Path) -> bool {
    path.to_str().is_some_and(|text| text.starts_with(PREFIX))
}

/// The drive and the path inside it.
pub fn parse(path: &Path) -> Option<(DriveId, RemotePath)> {
    match Location::from_uri(path.to_str()?).ok()? {
        Location::Remote { drive, path } => Some((drive, path)),
        Location::Local(_) => None,
    }
}

/// The location as the `PathBuf` the tabs hold.
pub fn to_path(drive: &DriveId, path: &RemotePath) -> Option<PathBuf> {
    let location = Location::Remote {
        drive: drive.clone(),
        path: path.clone(),
    };
    location.to_uri().ok().map(PathBuf::from)
}

/// `name` inside the remote folder `folder`.
pub fn child(folder: &Path, name: &str) -> Option<PathBuf> {
    let (drive, path) = parse(folder)?;
    to_path(&drive, &path.join(name).ok()?)
}

/// The containing folder; `None` at the drive's root.
pub fn parent(folder: &Path) -> Option<PathBuf> {
    let (drive, path) = parse(folder)?;
    to_path(&drive, &path.parent()?)
}

/// Breadcrumb for a remote folder: the drive (named by `label`), then each
/// segment. The drive is the `Root` kind, which the view draws as the start.
pub fn crumbs(folder: &Path, label: &str) -> Vec<Segment> {
    let Some((drive, path)) = parse(folder) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut acc = RemotePath::root();
    if let Some(root) = to_path(&drive, &acc) {
        out.push(Segment {
            path: root,
            name: OsString::from(label),
            kind: SegmentKind::Root,
        });
    }
    for segment in path.segments() {
        let Ok(next) = acc.join(segment) else { break };
        acc = next;
        if let Some(target) = to_path(&drive, &acc) {
            out.push(Segment {
                path: target,
                name: OsString::from(segment),
                kind: SegmentKind::Directory,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    #[test]
    fn a_remote_uri_is_told_from_a_local_path() {
        assert!(is_remote(&uri("kara+sftp://nas/a")));
        assert!(!is_remote(Path::new("/home/x")));
    }

    #[test]
    fn child_and_parent_round_trip_and_escape_names() {
        let root = uri("kara+sftp://nas/");
        let Some(inner) = child(&root, "my dir") else {
            panic!("child of root")
        };
        assert_eq!(inner, uri("kara+sftp://nas/my%20dir"));
        assert_eq!(parent(&inner), Some(uri("kara+sftp://nas/")));
        assert_eq!(parent(&uri("kara+sftp://nas/")), None);
    }

    #[test]
    fn crumbs_start_at_the_drive_and_end_at_the_folder() {
        let segments = crumbs(&uri("kara+sftp://nas/a/b"), "Mi NAS");
        let names: Vec<_> = segments
            .iter()
            .map(|s| s.name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Mi NAS", "a", "b"]);
        assert_eq!(segments[0].kind, SegmentKind::Root);
        assert_eq!(segments[2].path, uri("kara+sftp://nas/a/b"));
    }

    #[test]
    fn a_local_path_has_no_remote_parts() {
        assert!(parse(Path::new("/tmp")).is_none());
        assert!(crumbs(Path::new("/tmp"), "x").is_empty());
    }
}
