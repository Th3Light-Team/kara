//! cb_02..cb_05: capabilities, the backend root, and the lexical mapping
//! between local paths and RemotePath.

mod local_backend_support;

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use kara_fs::{LocalBackend, LocalPathError};
use kara_vfs::{Backend, BackendErrorKind, Capabilities, RemotePath, RemotePathError};
use local_backend_support::{err_of, io_kind, rooted, rp, tmp_tempdir};

fn expected_caps() -> Capabilities {
    Capabilities {
        trash: true,
        atomic_rename: true,
        server_side_copy: false,
        real_directories: true,
        posix_permissions: true,
        symlinks: true,
        watch: true,
        undo_rename: true,
        undo_move: true,
    }
}

// ---------------------------------------------------------------------------
// cb_02

#[test]
fn cb_02_capabilities_constant_matches_a_local_posix_volume() {
    assert_eq!(LocalBackend::CAPABILITIES, expected_caps());
}

#[test]
fn cb_02_capabilities_are_the_constant_for_every_root_and_call() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let system = LocalBackend::system();
    let local = rooted(tmp.path())?;
    assert_eq!(system.capabilities(), expected_caps());
    assert_eq!(local.capabilities(), expected_caps());
    assert_eq!(system.capabilities(), system.capabilities());
    assert_eq!(local.capabilities(), LocalBackend::CAPABILITIES);
    assert!(
        local.capabilities().trash,
        "trash = false would make Supr delete local files permanently"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_03

#[test]
fn cb_03_system_is_rooted_at_slash() {
    let system = LocalBackend::system();
    assert_eq!(system.root(), Path::new("/"));
}

#[test]
fn cb_03_with_root_refuses_a_relative_path_without_naming_a_path() {
    let error = err_of(LocalBackend::with_root(Path::new("rel")), "with_root(rel)");
    assert_eq!(error.kind, BackendErrorKind::Other, "{error:?}");
    assert_eq!(error.path, None, "{error:?}");
    assert_eq!(
        io_kind(&error),
        Some(io::ErrorKind::InvalidInput),
        "{error:?}"
    );
}

#[test]
fn cb_03_with_root_of_a_missing_dir_is_not_found_at_the_root() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let error = err_of(
        LocalBackend::with_root(&tmp.path().join("missing")),
        "with_root(missing)",
    );
    assert_eq!(error.kind, BackendErrorKind::NotFound, "{error:?}");
    assert_eq!(error.path, Some(RemotePath::root()), "{error:?}");
    Ok(())
}

#[test]
fn cb_03_with_root_of_a_file_is_other_not_a_directory() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let file = tmp.path().join("file");
    fs::write(&file, b"x")?;
    let error = err_of(LocalBackend::with_root(&file), "with_root(file)");
    assert_eq!(error.kind, BackendErrorKind::Other, "{error:?}");
    assert_eq!(error.path, Some(RemotePath::root()), "{error:?}");
    assert_eq!(
        io_kind(&error),
        Some(io::ErrorKind::NotADirectory),
        "{error:?}"
    );
    Ok(())
}

#[test]
fn cb_03_with_root_accepts_a_symlink_to_a_dir_and_keeps_it_as_given() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let real = tmp.path().join("real");
    fs::create_dir(&real)?;
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&real, &link)?;
    let backend = rooted(&link)?;
    assert_eq!(
        backend.root(),
        link.as_path(),
        "the root is not canonicalised"
    );

    let plain = rooted(&real)?;
    assert_eq!(plain.root(), real.as_path());
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_04

#[test]
fn cb_04_to_local_joins_the_segments_byte_for_byte() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let root = tmp.path().join("x").join("y");
    fs::create_dir_all(&root)?;
    let backend = rooted(&root)?;
    assert_eq!(
        backend.to_local(&rp("/a b/c#%?.txt")?),
        root.join("a b").join("c#%?.txt")
    );
    assert_eq!(backend.to_local(&RemotePath::root()), root);
    assert_eq!(
        backend.to_local(&rp("/\u{e9}/e\u{301}")?),
        root.join("\u{e9}").join("e\u{301}"),
        "no Unicode normalisation"
    );
    Ok(())
}

#[test]
fn cb_04_system_to_local_is_the_same_absolute_path() -> io::Result<()> {
    let system = LocalBackend::system();
    assert_eq!(
        system.to_local(&rp("/etc/hosts")?),
        PathBuf::from("/etc/hosts")
    );
    assert_eq!(system.to_local(&RemotePath::root()), PathBuf::from("/"));
    Ok(())
}

#[test]
fn cb_04_to_local_never_leaves_the_root() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let root = tmp.path().join("r");
    fs::create_dir(&root)?;
    let backend = rooted(&root)?;
    let samples = [
        "/", "/a", "/a/b", "/a/b/c", "/..a", "/a..", "/...", "/.hidden", "/a/.b", "/-rf", "/ ",
        "/a b", "/x:y", "/*", "/[x]", "/a\\b", "/l\nb", "/%2e%2e", "/\u{e9}", "/z/y/x/w",
    ];
    let root_depth = root.components().count();
    for sample in samples {
        let path = rp(sample)?;
        let local = backend.to_local(&path);
        assert!(
            local.starts_with(&root),
            "{sample:?} mapped outside the root: {}",
            local.display()
        );
        assert_eq!(
            local.components().count(),
            root_depth + path.segments().count(),
            "{sample:?} must add exactly its segments: {}",
            local.display()
        );
    }
    Ok(())
}

#[test]
fn cb_04_to_local_does_no_io() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let root = tmp.path().join("gone");
    fs::create_dir(&root)?;
    let backend = rooted(&root)?;
    fs::remove_dir(&root)?;
    assert_eq!(backend.to_local(&rp("/a/b")?), root.join("a").join("b"));
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_05

fn remote_of(backend: &LocalBackend, local: &Path) -> Result<RemotePath, LocalPathError> {
    backend.to_remote(local)
}

#[test]
fn cb_05_to_remote_of_the_root_is_the_remote_root() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    assert_eq!(remote_of(&backend, tmp.path()), Ok(RemotePath::root()));
    let mut trailing = tmp.path().as_os_str().to_os_string();
    trailing.push("/");
    assert_eq!(
        remote_of(&backend, Path::new(&trailing)),
        Ok(RemotePath::root()),
        "a trailing '/' is accepted"
    );
    let system = LocalBackend::system();
    assert_eq!(remote_of(&system, Path::new("/")), Ok(RemotePath::root()));
    Ok(())
}

#[test]
fn cb_05_to_remote_maps_paths_under_the_root() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    assert_eq!(
        remote_of(&backend, &tmp.path().join("a/b")),
        Ok(rp("/a/b")?)
    );
    assert_eq!(
        remote_of(&backend, &tmp.path().join("a/b/")),
        Ok(rp("/a/b")?),
        "a trailing '/' is accepted"
    );
    let system = LocalBackend::system();
    assert_eq!(
        remote_of(&system, Path::new("/etc/hosts")),
        Ok(rp("/etc/hosts")?)
    );
    Ok(())
}

#[test]
fn cb_05_to_remote_drops_interior_dot_components() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    assert_eq!(
        remote_of(&backend, &tmp.path().join("a/./b")),
        Ok(rp("/a/b")?)
    );
    Ok(())
}

#[test]
fn cb_05_to_remote_refuses_dotdot_as_a_dot_segment() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let result = remote_of(&backend, &tmp.path().join("a/../b"));
    assert!(
        matches!(
            result,
            Err(LocalPathError::Path(RemotePathError::DotSegment { .. }))
        ),
        "'..' is never resolved, got {result:?}"
    );
    let system = LocalBackend::system();
    let result = remote_of(&system, Path::new("/etc/../root"));
    assert!(
        matches!(
            result,
            Err(LocalPathError::Path(RemotePathError::DotSegment { .. }))
        ),
        "got {result:?}"
    );
    Ok(())
}

#[test]
fn cb_05_to_remote_compares_whole_components_against_the_root() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let a = tmp.path().join("a");
    let ab = tmp.path().join("ab");
    fs::create_dir(&a)?;
    fs::create_dir(&ab)?;
    let backend = rooted(&a)?;
    assert_eq!(remote_of(&backend, &ab), Err(LocalPathError::OutsideRoot));
    assert_eq!(
        remote_of(&backend, &ab.join("x")),
        Err(LocalPathError::OutsideRoot)
    );
    assert_eq!(
        remote_of(&backend, tmp.path()),
        Err(LocalPathError::OutsideRoot)
    );
    assert_eq!(
        remote_of(&backend, Path::new("/")),
        Err(LocalPathError::OutsideRoot)
    );
    Ok(())
}

#[test]
fn cb_05_to_remote_refuses_relative_paths() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    assert_eq!(
        remote_of(&backend, Path::new("a/b")),
        Err(LocalPathError::NotAbsolute)
    );
    let system = LocalBackend::system();
    assert_eq!(
        remote_of(&system, Path::new("a/b")),
        Err(LocalPathError::NotAbsolute)
    );
    assert_eq!(
        remote_of(&system, Path::new("")),
        Err(LocalPathError::NotAbsolute)
    );
    Ok(())
}

#[test]
fn cb_05_to_remote_refuses_non_utf8_and_nul() {
    let system = LocalBackend::system();
    assert_eq!(
        remote_of(&system, Path::new(OsStr::from_bytes(b"/\xff"))),
        Err(LocalPathError::Path(RemotePathError::NotUtf8))
    );
    assert_eq!(
        remote_of(&system, Path::new(OsStr::from_bytes(b"/a\0b"))),
        Err(LocalPathError::Path(RemotePathError::ContainsNul))
    );
}

#[test]
fn cb_05_to_remote_is_lexical_and_does_not_resolve_symlinks() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    std::os::unix::fs::symlink("/etc", tmp.path().join("link"))?;
    assert_eq!(
        remote_of(&backend, &tmp.path().join("link/hosts")),
        Ok(rp("/link/hosts")?)
    );
    assert_eq!(
        remote_of(&backend, &tmp.path().join("does/not/exist")),
        Ok(rp("/does/not/exist")?)
    );
    Ok(())
}

#[test]
fn cb_05_to_local_and_to_remote_round_trip() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let samples = [
        "a",
        "a/b",
        "a b/c#%?.txt",
        ".hidden/x",
        "\u{e9}/e\u{301}",
        "-rf",
        "x:y/[z]",
        "100%",
        "l\nb",
        "deep/er/than/that",
    ];
    for sample in samples {
        let local = tmp.path().join(sample);
        let remote = match backend.to_remote(&local) {
            Ok(path) => path,
            Err(error) => panic!("to_remote({}) failed: {error}", local.display()),
        };
        assert_eq!(remote.as_str(), format!("/{sample}"));
        assert_eq!(backend.to_local(&remote), local, "round trip of {sample:?}");
    }
    Ok(())
}

#[test]
fn cb_05_local_path_error_messages() {
    assert_eq!(
        LocalPathError::NotAbsolute.to_string(),
        "local path is not absolute"
    );
    assert_eq!(
        LocalPathError::OutsideRoot.to_string(),
        "local path is outside the backend root"
    );
    assert_eq!(
        LocalPathError::from(RemotePathError::NotUtf8).to_string(),
        RemotePathError::NotUtf8.to_string()
    );
}
