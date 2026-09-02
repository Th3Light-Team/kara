//! Utilidades compartidas por las pruebas de la papelera.
//!
//! Todas las pruebas trabajan sobre directorios temporales y redirigen
//! `XDG_DATA_HOME`: ninguna toca la papelera real del usuario.

#![allow(dead_code)]

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use kara_fs::trash::{ErrorDecision, Flow, TrashError, TrashObserver, TrashedItem};

/// Unwraps a setup step, reporting the underlying error instead of hiding it.
pub fn ok<T>(result: io::Result<T>, what: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("test setup step `{what}` failed: {error}"),
    }
}

/// Serializes every test that touches process-wide environment variables.
pub fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    match LOCK.get_or_init(|| Mutex::new(())).lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn unique(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(delta) => delta.as_nanos(),
        Err(_) => 0,
    };
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{nanos}-{seq}", std::process::id())
}

/// Base directory on the same device as the crate source tree.
pub fn home_device_base() -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/kara-fs-tests");
    ok(fs::create_dir_all(&base), "create test base directory");
    ok(fs::canonicalize(&base), "canonicalize test base directory")
}

/// Base directory on a different device (tmpfs) than [`home_device_base`].
pub fn tmpfs_base() -> PathBuf {
    PathBuf::from("/tmp")
}

/// `true` when the two paths live on different devices, which is what makes
/// the volume-trash tests meaningful.
pub fn on_different_devices(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(left), Ok(right)) => left.dev() != right.dev(),
        _ => false,
    }
}

/// The current user id, obtained without extra dependencies.
pub fn current_uid(scratch: &Path) -> u32 {
    let probe = scratch.join("uid-probe");
    ok(fs::write(&probe, b""), "write uid probe");
    let uid = ok(fs::metadata(&probe), "stat uid probe").uid();
    let _ = fs::remove_file(&probe);
    uid
}

/// A temporary directory tree removed on drop.
pub struct TempTree {
    pub root: PathBuf,
}

impl TempTree {
    pub fn new_in(base: &Path, prefix: &str) -> TempTree {
        let root = base.join(unique(prefix));
        ok(fs::create_dir_all(&root), "create temp tree");
        TempTree { root }
    }

    pub fn on_home_device(prefix: &str) -> TempTree {
        TempTree::new_in(&home_device_base(), prefix)
    }

    pub fn on_tmpfs(prefix: &str) -> TempTree {
        TempTree::new_in(&tmpfs_base(), prefix)
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Creates a file (and any missing parent) and returns its absolute path.
    pub fn write(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            ok(fs::create_dir_all(parent), "create parent directory");
        }
        ok(fs::write(&path, contents), "write test file");
        path
    }

    pub fn mkdir(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        ok(fs::create_dir_all(&path), "create test directory");
        path
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        restore_modes(&self.root);
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Makes every directory writable again so that cleanup can succeed after a
/// test that locked one down on purpose.
pub fn restore_modes(root: &Path) {
    let Ok(metadata) = fs::symlink_metadata(root) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    if metadata.is_dir() {
        let _ = fs::set_permissions(root, fs::Permissions::from_mode(0o700));
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                restore_modes(&entry.path());
            }
        }
    }
}

pub fn set_mode(path: &Path, mode: u32) {
    ok(
        fs::set_permissions(path, fs::Permissions::from_mode(mode)),
        "set permissions",
    );
}

pub fn mode_of(path: &Path) -> u32 {
    ok(fs::metadata(path), "stat for mode").permissions().mode() & 0o7777
}

pub fn read(path: &Path) -> Vec<u8> {
    ok(fs::read(path), "read file")
}

/// Sorted file names directly inside `dir`. An absent directory yields an
/// empty list so that "nothing was created" is expressible.
pub fn entry_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Recursive fingerprint of a tree: relative path, kind, size and mtime.
/// Used to prove that a call had no side effects at all.
pub fn snapshot(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    collect_snapshot(root, root, &mut out);
    out.sort();
    out
}

fn collect_snapshot(root: &Path, current: &Path, out: &mut Vec<String>) {
    let Ok(metadata) = fs::symlink_metadata(current) else {
        return;
    };
    let relative = current.strip_prefix(root).unwrap_or(current);
    let kind = if metadata.file_type().is_symlink() {
        "link"
    } else if metadata.is_dir() {
        "dir"
    } else {
        "file"
    };
    out.push(format!(
        "{}|{kind}|{}|{}|{}",
        relative.to_string_lossy(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec()
    ));
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        if let Ok(entries) = fs::read_dir(current) {
            for entry in entries.flatten() {
                collect_snapshot(root, &entry.path(), out);
            }
        }
    }
}

/// Redirects `XDG_DATA_HOME` (and optionally `HOME`) to temporary directories
/// for the lifetime of the guard, holding the environment lock meanwhile.
pub struct TrashEnv {
    _guard: MutexGuard<'static, ()>,
    previous_xdg: Option<OsString>,
    previous_home: Option<OsString>,
    pub data_home: TempTree,
}

impl TrashEnv {
    /// `XDG_DATA_HOME` on the same device as the source files.
    pub fn on_home_device() -> TrashEnv {
        TrashEnv::with_data_home(TempTree::on_home_device("xdg"))
    }

    /// `XDG_DATA_HOME` on tmpfs, i.e. on a different device than files created
    /// under [`home_device_base`].
    pub fn on_tmpfs() -> TrashEnv {
        TrashEnv::with_data_home(TempTree::on_tmpfs("xdg"))
    }

    fn with_data_home(data_home: TempTree) -> TrashEnv {
        let guard = env_lock();
        let previous_xdg = std::env::var_os("XDG_DATA_HOME");
        let previous_home = std::env::var_os("HOME");
        // SAFETY: every test that reads these variables holds `env_lock`.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", &data_home.root);
        }
        TrashEnv {
            _guard: guard,
            previous_xdg,
            previous_home,
            data_home,
        }
    }

    /// Overrides `XDG_DATA_HOME` with an arbitrary value (empty or relative)
    /// to exercise the fallback, and points `HOME` at a temporary directory so
    /// the real one is never touched.
    pub fn override_xdg_and_home(&self, xdg: &str, home: &Path) {
        // SAFETY: the environment lock is held by this guard.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", xdg);
            std::env::set_var("HOME", home);
        }
    }

    pub fn trash_root(&self) -> PathBuf {
        self.data_home.root.join("Trash")
    }

    pub fn files(&self) -> PathBuf {
        self.trash_root().join("files")
    }

    pub fn info(&self) -> PathBuf {
        self.trash_root().join("info")
    }
}

impl Drop for TrashEnv {
    fn drop(&mut self) {
        // SAFETY: the environment lock is still held until this guard drops.
        unsafe {
            match &self.previous_xdg {
                Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
            match &self.previous_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }
}

/// Observer that records what it was told and answers as configured.
pub struct Recorder {
    pub starts: Vec<PathBuf>,
    pub totals: Vec<usize>,
    pub byte_calls: u32,
    pub errors: Vec<PathBuf>,
    pub done: Vec<PathBuf>,
    /// Answer returned by `on_error`.
    pub decision: ErrorDecision,
    /// Index (0-based) at which `on_item_start` answers `Cancel`.
    pub cancel_at: Option<usize>,
    /// Number of `on_bytes` calls after which it answers `Cancel`.
    pub cancel_after_bytes: Option<u32>,
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder {
            starts: Vec::new(),
            totals: Vec::new(),
            byte_calls: 0,
            errors: Vec::new(),
            done: Vec::new(),
            decision: ErrorDecision::Skip,
            cancel_at: None,
            cancel_after_bytes: None,
        }
    }

    pub fn deciding(decision: ErrorDecision) -> Recorder {
        let mut recorder = Recorder::new();
        recorder.decision = decision;
        recorder
    }

    pub fn cancelling_at(index: usize) -> Recorder {
        let mut recorder = Recorder::new();
        recorder.cancel_at = Some(index);
        recorder
    }
}

impl Default for Recorder {
    fn default() -> Recorder {
        Recorder::new()
    }
}

impl TrashObserver for Recorder {
    fn on_item_start(&mut self, path: &Path, index: usize, total: usize) -> Flow {
        self.starts.push(path.to_path_buf());
        self.totals.push(total);
        if self.cancel_at == Some(index) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn on_bytes(&mut self, _copied: u64, _total: Option<u64>) -> Flow {
        self.byte_calls = self.byte_calls.saturating_add(1);
        match self.cancel_after_bytes {
            Some(limit) if self.byte_calls > limit => Flow::Cancel,
            _ => Flow::Continue,
        }
    }

    fn on_error(&mut self, path: &Path, _error: &TrashError) -> ErrorDecision {
        self.errors.push(path.to_path_buf());
        self.decision
    }

    fn on_item_done(&mut self, item: &TrashedItem) {
        self.done.push(item.original_path.clone());
    }
}

/// Observer that retries an item, unlocking a directory on the second failure
/// so that the third attempt succeeds.
pub struct RetryThenFix {
    pub calls: u32,
    pub unlock_after: u32,
    pub unlock_dir: PathBuf,
}

impl TrashObserver for RetryThenFix {
    fn on_error(&mut self, _path: &Path, _error: &TrashError) -> ErrorDecision {
        self.calls = self.calls.saturating_add(1);
        if self.calls == self.unlock_after {
            set_mode(&self.unlock_dir, 0o700);
        }
        ErrorDecision::Retry
    }
}

/// Observer that ignores everything: the null observer.
pub struct Silent;

impl TrashObserver for Silent {}
