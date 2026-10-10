//! cb_27..cb_30 and ac_05: error paths across every method, unusual names,
//! thread safety, the public API, and that nothing existing changed.

mod local_backend_support;

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use kara_core::FileEntry;
use kara_fs::{LocalBackend, LocalPathError};
use kara_vfs::{Backend, BackendError, Cancel, Capabilities, RemotePath, RemotePathError};
use local_backend_support::{backend_source, code_lines, get, put, rooted, rp, tmp_tempdir};

// ---------------------------------------------------------------------------
// cb_27

#[test]
fn cb_27_every_error_names_the_remote_argument_never_the_local_path() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("file"), b"f")?;
    fs::create_dir(tmp.path().join("dir"))?;
    fs::write(tmp.path().join("dir/child"), b"c")?;
    let cancelled = Cancel::new();
    cancelled.cancel();
    let fresh = Cancel::new();

    let mut cases: Vec<(&str, Result<(), BackendError>, &str)> = vec![
        (
            "list missing",
            backend.list(&rp("/missing")?, &fresh).map(|_| ()),
            "/missing",
        ),
        (
            "list file",
            backend.list(&rp("/file")?, &fresh).map(|_| ()),
            "/file",
        ),
        (
            "list cancelled",
            backend.list(&rp("/dir")?, &cancelled).map(|_| ()),
            "/dir",
        ),
        (
            "stat missing",
            backend.stat(&rp("/missing")?).map(|_| ()),
            "/missing",
        ),
        (
            "stat under file",
            backend.stat(&rp("/file/x")?).map(|_| ()),
            "/file/x",
        ),
        (
            "read missing",
            backend.open_read(&rp("/missing")?, 0).map(|_| ()),
            "/missing",
        ),
        (
            "read dir",
            backend.open_read(&rp("/dir")?, 0).map(|_| ()),
            "/dir",
        ),
        (
            "read past end",
            backend.open_read(&rp("/file")?, 99).map(|_| ()),
            "/file",
        ),
        (
            "write existing",
            backend.begin_write(&rp("/file")?, None, false).map(|_| ()),
            "/file",
        ),
        (
            "write missing parent",
            backend
                .begin_write(&rp("/missing/t")?, None, false)
                .map(|_| ()),
            "/missing/t",
        ),
        (
            "write under file",
            backend.begin_write(&rp("/file/t")?, None, true).map(|_| ()),
            "/file/t",
        ),
        ("mkdir existing", backend.create_dir(&rp("/dir")?), "/dir"),
        (
            "mkdir missing parent",
            backend.create_dir(&rp("/missing/d")?),
            "/missing/d",
        ),
        (
            "rename missing",
            backend.rename(&rp("/missing")?, &rp("/x")?),
            "/missing",
        ),
        (
            "rename onto existing",
            backend.rename(&rp("/file")?, &rp("/dir")?),
            "/dir",
        ),
        (
            "rename into missing parent",
            backend.rename(&rp("/file")?, &rp("/missing/x")?),
            "/missing/x",
        ),
        (
            "rename under file",
            backend.rename(&rp("/dir")?, &rp("/file/x")?),
            "/file/x",
        ),
        (
            "rename into itself",
            backend.rename(&rp("/dir")?, &rp("/dir/x")?),
            "/dir/x",
        ),
        (
            "remove missing",
            backend.remove(&rp("/missing")?),
            "/missing",
        ),
        ("remove non-empty", backend.remove(&rp("/dir")?), "/dir"),
        ("remove root", backend.remove(&RemotePath::root()), "/"),
        (
            "remove_tree missing",
            backend.remove_tree(&rp("/missing")?, &fresh),
            "/missing",
        ),
        (
            "remove_tree cancelled",
            backend.remove_tree(&rp("/dir")?, &cancelled),
            "/dir",
        ),
        (
            "remove_tree root",
            backend.remove_tree(&RemotePath::root(), &fresh),
            "/",
        ),
        (
            "copy_within",
            backend.copy_within(&rp("/file")?, &rp("/copy")?),
            "/file",
        ),
    ];

    // A lost race at finish names the target, not the temporary.
    let mut session = backend.begin_write(&rp("/raced")?, None, false)?;
    session.write_all(b"late")?;
    fs::write(tmp.path().join("raced"), b"winner")?;
    cases.push(("finish race", session.finish(), "/raced"));

    let local = tmp.path().to_string_lossy().into_owned();
    for (what, result, expected) in cases {
        let error = match result {
            Ok(()) => panic!("{what}: expected an error"),
            Err(error) => error,
        };
        assert_eq!(
            error.path,
            Some(rp(expected)?),
            "{what}: the error must name {expected}, got {error:?}"
        );
        let shown = error.to_string();
        assert!(!shown.contains(".kara-part"), "{what}: {shown}");
        assert!(
            !shown.contains(&local),
            "{what}: names the local path: {shown}"
        );
    }
    assert!(
        local_backend_support::temp_names(tmp.path())?.is_empty(),
        "no temporary may survive"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_28

#[test]
fn cb_28_special_names_survive_byte_for_byte() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let specials = [
        "a b", "#", "100%", "why?", "[x]", "a\\b", "c:d", "*", "l\nb", "-rf",
    ];
    for (index, name) in specials.iter().enumerate() {
        let path = RemotePath::root().join(name).map_err(io::Error::other)?;
        put(
            &backend,
            &path,
            format!("content {index}").as_bytes(),
            false,
        )?;
    }
    let mut on_disk: Vec<OsString> = local_backend_support::names(tmp.path())?;
    on_disk.sort();
    let mut expected: Vec<OsString> = specials.iter().map(OsString::from).collect();
    expected.sort();
    assert_eq!(
        on_disk, expected,
        "local names are exactly the RemotePath bytes"
    );
    for (index, name) in specials.iter().enumerate() {
        let path = RemotePath::root().join(name).map_err(io::Error::other)?;
        assert_eq!(
            get(&backend, &path, 0)?,
            format!("content {index}").as_bytes()
        );
        assert_eq!(
            fs::read(tmp.path().join(name))?,
            format!("content {index}").as_bytes()
        );
    }
    let listing = backend.list(&RemotePath::root(), &Cancel::new())?;
    assert_eq!(listing.entries.len(), specials.len());
    assert!(listing.errors.is_empty());
    Ok(())
}

#[test]
fn cb_28_nfc_and_nfd_names_are_two_files() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let nfc = rp("/\u{e9}")?;
    let nfd = rp("/e\u{301}")?;
    put(&backend, &nfc, b"composed", false)?;
    put(&backend, &nfd, b"decomposed", false)?;
    let listing = backend.list(&RemotePath::root(), &Cancel::new())?;
    assert_eq!(listing.entries.len(), 2);
    assert_eq!(get(&backend, &nfc, 0)?, b"composed");
    assert_eq!(get(&backend, &nfd, 0)?, b"decomposed");
    assert_eq!(fs::read(tmp.path().join("\u{e9}"))?, b"composed");
    assert_eq!(fs::read(tmp.path().join("e\u{301}"))?, b"decomposed");
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_29

fn send_sync_clone<T: Send + Sync + Clone + 'static>() {}

#[test]
fn cb_29_local_backend_is_send_sync_clone_and_object_safe() -> io::Result<()> {
    send_sync_clone::<LocalBackend>();
    let tmp = tmp_tempdir()?;
    let shared: Arc<dyn Backend> = Arc::new(rooted(tmp.path())?);
    assert!(
        shared
            .list(&RemotePath::root(), &Cancel::new())?
            .entries
            .is_empty()
    );
    Ok(())
}

#[test]
fn cb_29_the_backend_module_holds_no_locks() -> io::Result<()> {
    let source = backend_source()?;
    let offenders: Vec<&str> = code_lines(&source)
        .into_iter()
        .filter(|line| line.contains("Mutex") || line.contains("RwLock"))
        .collect();
    assert!(
        offenders.is_empty(),
        "no Mutex/RwLock in the backend: {offenders:?}"
    );
    Ok(())
}

#[test]
fn cb_29_concurrent_lists_and_writes_in_one_dir_finish() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let (tx, rx) = mpsc::channel();
    for thread in 0..4 {
        let backend = backend.clone();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let outcome = (|| -> Result<(), String> {
                for iteration in 0..200 {
                    backend
                        .list(&RemotePath::root(), &Cancel::new())
                        .map_err(|e| format!("list: {e}"))?;
                    let path = RemotePath::root()
                        .join(&format!("t{thread}-{iteration}"))
                        .map_err(|e| e.to_string())?;
                    let mut session = backend
                        .begin_write(&path, Some(4), false)
                        .map_err(|e| format!("begin {path}: {e}"))?;
                    session
                        .write_all(b"data")
                        .map_err(|e| format!("write: {e}"))?;
                    session
                        .finish()
                        .map_err(|e| format!("finish {path}: {e}"))?;
                }
                Ok(())
            })();
            let _ = tx.send((thread, outcome));
        });
    }
    drop(tx);
    let deadline = Instant::now() + Duration::from_secs(30);
    for _ in 0..4 {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((_, Ok(()))) => {}
            Ok((thread, Err(error))) => panic!("thread {thread}: {error}"),
            Err(_) => panic!("the four workers did not finish within 30 s"),
        }
    }
    let listing = backend.list(&RemotePath::root(), &Cancel::new())?;
    assert_eq!(listing.entries.len(), 800);
    assert!(local_backend_support::temp_names(tmp.path())?.is_empty());
    Ok(())
}

// ---------------------------------------------------------------------------
// ac_05: the public API, as fn pointers of the exact contract types.

#[test]
fn ac_05_public_api_has_the_contract_signatures() {
    let _: fn() -> LocalBackend = LocalBackend::system;
    let _: fn(&Path) -> Result<LocalBackend, BackendError> = LocalBackend::with_root;
    let _: for<'a> fn(&'a LocalBackend) -> &'a Path = LocalBackend::root;
    let _: fn(&LocalBackend, &RemotePath) -> PathBuf = LocalBackend::to_local;
    let _: fn(&LocalBackend, &Path) -> Result<RemotePath, LocalPathError> = LocalBackend::to_remote;
    const CAPS: Capabilities = LocalBackend::CAPABILITIES;
    let _ = CAPS;

    let _: fn(&LocalBackend) -> Capabilities = <LocalBackend as Backend>::capabilities;
    let _: fn(&LocalBackend, &RemotePath) -> Result<FileEntry, BackendError> =
        <LocalBackend as Backend>::stat;

    fn debug_clone<T: std::fmt::Debug + Clone>() {}
    debug_clone::<LocalBackend>();
    fn error_like<T: std::error::Error + Clone + PartialEq + Eq + Send + Sync + 'static>() {}
    error_like::<LocalPathError>();
    let from: LocalPathError = RemotePathError::NotUtf8.into();
    let _ = |e: LocalPathError| match e {
        LocalPathError::NotAbsolute => 0,
        LocalPathError::OutsideRoot => 1,
        LocalPathError::Path(inner) => inner.to_string().len(),
    };
    assert_eq!(from, LocalPathError::Path(RemotePathError::NotUtf8));

    // Re-exported at the crate root and reachable as kara_fs::backend.
    let _: fn() -> kara_fs::backend::LocalBackend = kara_fs::LocalBackend::system;
    // kara_fs::Listing is still kara-fs' own listing, not kara_vfs::Listing.
    let own = kara_fs::Listing::default();
    let _: &Vec<kara_fs::listing::EntryError> = &own.errors;
}

// ---------------------------------------------------------------------------
// cb_30

#[test]
fn cb_30_existing_public_functions_keep_their_signatures() {
    use kara_fs::trash::{TrashError, TrashObserver};
    let _: fn(&Path) -> Result<kara_fs::Listing, kara_fs::EntryError> = kara_fs::list_directory;
    let _: fn(&Path) -> Result<FileEntry, kara_fs::EntryError> = kara_fs::describe;
    let _: fn(&Path, &mut dyn TrashObserver) -> Result<u64, TrashError> =
        kara_fs::trash::delete_permanently;
    let _: fn(
        &Path,
        &kara_fs::trash::TrashPolicy,
    ) -> Result<kara_fs::trash::TrashedItem, TrashError> = kara_fs::trash::trash_one;
    let _: fn(
        &Path,
        &std::ffi::OsStr,
        kara_fs::trash::ConflictPolicy,
    ) -> Result<PathBuf, kara_fs::TransferError> = kara_fs::rename;
}

const EXISTING_TESTS: &[(&str, &str)] = &[
    (
        "crates/kara-fs/tests/clipboard.rs",
        "abe463cb46db7d11f7f6f991efff126f38661e64ef1940bbbdefc698af4451fd",
    ),
    (
        "crates/kara-fs/tests/icon_theme_choice.rs",
        "d35b9b55a4af74cd8da3b3968340b68ccd3ab6ab393e06dc0dc46c2b489639f1",
    ),
    (
        "crates/kara-fs/tests/icons.rs",
        "9c26e0925f296d453ca00a3de5531803cc8c0e55878d08a4208296930e2a2237",
    ),
    (
        "crates/kara-fs/tests/listing.rs",
        "92854f7b080d59e45ad38a3e72e4fd442dae6fea008f0a5857780c8e2fbcdfc3",
    ),
    (
        "crates/kara-fs/tests/mime.rs",
        "dd2b1b40f73fe73862bc1ba173bcc9b61babad9e44e2b3e337a92db443f95f3d",
    ),
    (
        "crates/kara-fs/tests/mime_generic_icons.rs",
        "56c8570a9e5a0b643a53220cc74873bcb20a7349aaa338224d249e77ff69a280",
    ),
    (
        "crates/kara-fs/tests/places.rs",
        "27052857628329aa5c333dabd9c28f0e5e915f446f174f596a6ff84386cc9a0b",
    ),
    (
        "crates/kara-fs/tests/settings.rs",
        "f53c0d55961a93e8dca8e57e6503bb9f9252cfe806175ae21a6aada1c13ca21b",
    ),
    (
        "crates/kara-fs/tests/thumbnail_failure_path.rs",
        "90b8240165405447a5c3c2b1ba243657d3736a737a3924d8c02eb5a5db1601c6",
    ),
    (
        "crates/kara-fs/tests/thumbnails.rs",
        "8313b6fea219dfd0619c457f8c3de3dc48a4d89fc288b87eefdc69bd69334e4c",
    ),
    (
        "crates/kara-fs/tests/transfer.rs",
        "45247e768e2965474b6e48e5cd004baf3cb79abfc7c15ec6dd762e436a56d58f",
    ),
    (
        "crates/kara-fs/tests/trash_home.rs",
        "84c74b72611764256b6405ba1e4e79d056f95c4e28216d41711a9b7f19520757",
    ),
    (
        "crates/kara-fs/tests/trash_info.rs",
        "7bb18ea3152cb070b4beaa21c20788accf75c8efc63cb150eab590af8634a9b2",
    ),
    (
        "crates/kara-fs/tests/trash_listing.rs",
        "466c7b265226b612c946f88dbae7f7b2f5150b8f40c0a07faebde4f450e2bbd1",
    ),
    (
        "crates/kara-fs/tests/trash_supervisor.rs",
        "2820c8334e43e0909f98e851575f557faba72c80c9c38d58e102646b2860daad",
    ),
    (
        "crates/kara-fs/tests/trash_volume.rs",
        "675c87e3e1ecae04116914dda4aceb040ce01cddf8d374f8f37becc7a757a84c",
    ),
    (
        "crates/kara-fs/tests/common/mod.rs",
        "abf3c34115f7b496213db00ba4af8fbd583c3e55ddfb23d8ef6b0159d70d16d7",
    ),
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn cb_30_existing_test_files_are_byte_identical() -> io::Result<()> {
    let root = workspace_root();
    for (relative, expected) in EXISTING_TESTS {
        let output = match Command::new("sha256sum").arg(root.join(relative)).output() {
            Ok(output) => output,
            Err(error) => {
                println!("cb_30: sha256sum is not available ({error}); skipped");
                return Ok(());
            }
        };
        assert!(output.status.success(), "sha256sum {relative} failed");
        let digest = output
            .stdout
            .split(|b| *b == b' ')
            .next()
            .unwrap_or_default();
        assert_eq!(
            digest,
            expected.as_bytes(),
            "{relative} changed: existing tests are hash-checked"
        );
    }
    Ok(())
}

#[test]
fn cb_30_kara_vfs_and_kara_core_are_unchanged() -> io::Result<()> {
    let root = workspace_root();
    let known = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["cat-file", "-e", "029f06d^{commit}"])
        .status();
    match known {
        Ok(status) if status.success() => {}
        _ => {
            println!("cb_30: git or commit 029f06d unavailable; skipped");
            return Ok(());
        }
    }
    let status = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["diff", "--quiet", "--exit-code", "029f06d", "--"])
        .args(["crates/kara-vfs", "crates/kara-core"])
        .status()?;
    assert!(status.success(), "kara-vfs and kara-core must not change");
    Ok(())
}

#[test]
fn cb_30_describe_still_names_the_entry_and_its_parent() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let file = tmp.path().join("f");
    fs::write(&file, b"12345")?;
    let entry = kara_fs::describe(&file).map_err(io::Error::other)?;
    assert_eq!(entry.name, "f");
    assert_eq!(entry.size, Some(5));
    assert_eq!(entry.location.as_deref(), Some(tmp.path()));
    Ok(())
}
