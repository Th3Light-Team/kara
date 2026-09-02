//! Looking inside the trash: pairing `info/` with `files/` in the personal
//! trash and in every volume's `.Trash-$uid`.
//!
//! Conveniencias cubiertas (fuente: `ground/spec/05-operaciones.md`):
//!
//! - **Restaurar desde la papelera** (§ «Restaurar desde la papelera»): this
//!   is the missing half — without a listing there is nothing to point
//!   `restore_item` at. Every [`TrashEntry::Item`] this module returns is a
//!   plain [`TrashedItem`], so it goes straight into `restore_item` (or into
//!   [`delete_trash_entry`]) with no conversion step.
//! - **Vaciar la papelera** (§ «Vaciar la papelera»): [`empty_trash`] and
//!   [`empty_trash_in`], shaped like [`super::trash_batch`] — a
//!   [`TrashObserver`] drives progress/retry/skip/cancel, and the outcome is
//!   never a `Result` because one item failing to delete must not stop the
//!   rest. [`delete_trash_entry`] is the "elimina este elemento sin vaciarla
//!   entera" the spec also asks for, and `empty_trash_in` is built directly
//!   on top of it rather than duplicating the removal logic.
//!
//! # Unpaired entries are not errors
//!
//! A `.trashinfo` whose `files/` counterpart is gone, or a `files/` entry
//! with no `.trashinfo`, is the interesting case the spec calls out
//! explicitly: it "se enseña como no disponible en vez de fallar en
//! silencio". [`TrashEntry::MissingFile`] and [`TrashEntry::MissingInfo`]
//! keep the two apart so a caller never has to guess which side survived.
//! A `.trashinfo` that exists but fails to *parse* is a third, separate
//! situation — the record was found, just unreadable — and is reported in
//! [`TrashListing::unreadable_records`] instead of manufacturing a
//! `MissingInfo` for its `files/` counterpart (which does have a paired
//! record, just a broken one).
//!
//! # Never created here, never a real `/proc/mounts` read from `list_trash_in`
//!
//! Listing must not have the side effect of creating a trash directory that
//! does not exist yet — that would turn "look inside" into "make one appear".
//! Volume discovery is injected (`volume_top_dirs`) rather than read from
//! `/proc/mounts` directly inside [`list_trash_in`]/[`empty_trash_in`], which
//! keeps both testable without a real removable drive; [`list_trash`] and
//! [`empty_trash`] are the convenience wrappers that discover the real
//! mounted volumes via [`crate::places::this_computer`].

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::dir;
use super::error::classify_io_error;
use super::{DeletionDate, ErrorDecision, Flow, TrashDir, TrashError, TrashInfoError, TrashKind};
use super::{TrashObserver, TrashedItem, read_trash_info};

/// One thing found while listing a trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashEntry {
    /// A `.trashinfo` that parsed and whose `files/` counterpart exists:
    /// everything `restore_item`/`delete_trash_entry` need.
    Item(TrashedItem),
    /// A `.trashinfo` that parsed fine, but nothing sits at the matching
    /// `files/` path any more (e.g. deleted by hand, or by another trash
    /// implementation). Restoring it is not possible; deleting the stray
    /// record still is.
    MissingFile {
        info_path: PathBuf,
        original_path: PathBuf,
        deletion_date: DeletionDate,
        kind: TrashKind,
        top_dir: Option<PathBuf>,
    },
    /// A `files/` entry with no matching `.trashinfo`: the original location
    /// and deletion date are unknown, but the entry still occupies space and
    /// still belongs to this trash.
    MissingInfo {
        file_path: PathBuf,
        kind: TrashKind,
        top_dir: Option<PathBuf>,
    },
}

impl TrashEntry {
    /// The path that identifies this entry to a [`TrashObserver`]: the
    /// trashed file for a paired entry, or whichever half of an unpaired one
    /// is actually present.
    #[must_use]
    pub fn display_path(&self) -> &Path {
        match self {
            TrashEntry::Item(item) => &item.trashed_path,
            TrashEntry::MissingFile { info_path, .. } => info_path,
            TrashEntry::MissingInfo { file_path, .. } => file_path,
        }
    }
}

/// Result of listing one or more trash roots. Never a `Result`: a trash root
/// this process cannot even open, or a `.trashinfo` it cannot parse, does not
/// stop the rest from being reported (cb-style batch rule, same as
/// [`super::BatchOutcome`]).
#[derive(Debug)]
pub struct TrashListing {
    pub entries: Vec<TrashEntry>,
    /// A `.trashinfo` that exists but could not be parsed, with why.
    pub unreadable_records: Vec<(PathBuf, TrashInfoError)>,
    /// A trash root's `files/` or `info/` that exists but could not be
    /// opened for reading (e.g. permission denied on a volume trash).
    /// Distinct from a root that simply does not exist yet, which is an
    /// empty trash, not a failure.
    pub unreadable_roots: Vec<(PathBuf, TrashError)>,
}

/// Lists the personal trash and `$top/.Trash-$uid` for every path in
/// `volume_top_dirs`, pairing `info/` with `files/` in each.
///
/// A volume top dir whose `.Trash-$uid` does not exist yet contributes
/// nothing (that is a volume that has never been used as a trash, not a
/// failure). One that exists but is a symlink, not a directory, or not owned
/// by this user is skipped the same way [`super::dir`]'s own trust check
/// treats it when *writing* — never followed, never reported as a partial
/// success, because a world-writable `$top` lets anybody plant it pointing
/// wherever they like (see [`is_trustworthy_volume_trash_root`]).
#[must_use]
pub fn list_trash_in(volume_top_dirs: &[PathBuf]) -> TrashListing {
    let mut entries = Vec::new();
    let mut unreadable_records = Vec::new();
    let mut unreadable_roots = Vec::new();

    match home_trash_dir_location() {
        Ok(home_dir) => scan_trash_dir(
            &home_dir,
            &mut entries,
            &mut unreadable_records,
            &mut unreadable_roots,
        ),
        Err(error) => unreadable_roots.push((PathBuf::from("$HOME"), error)),
    }

    let uid = dir::current_uid();
    for top_dir in volume_top_dirs {
        if let Some(volume_dir) = volume_trash_dir_for(top_dir, uid) {
            scan_trash_dir(
                &volume_dir,
                &mut entries,
                &mut unreadable_records,
                &mut unreadable_roots,
            );
        }
    }

    entries.sort_by(|a, b| a.display_path().cmp(b.display_path()));
    unreadable_records.sort_by(|a, b| a.0.cmp(&b.0));
    unreadable_roots.sort_by(|a, b| a.0.cmp(&b.0));

    TrashListing {
        entries,
        unreadable_records,
        unreadable_roots,
    }
}

/// [`list_trash_in`] over every volume [`crate::places::this_computer`]
/// reports as mounted.
#[must_use]
pub fn list_trash() -> TrashListing {
    list_trash_in(&mounted_volume_top_dirs())
}

fn mounted_volume_top_dirs() -> Vec<PathBuf> {
    crate::places::this_computer()
        .into_iter()
        .filter(|place| place.kind == crate::places::PlaceKind::Volume)
        .map(|place| place.path)
        .collect()
}

fn home_trash_dir_location() -> Result<TrashDir, TrashError> {
    let root = dir::home_trash_root()?;
    Ok(TrashDir {
        files: root.join("files"),
        info: root.join("info"),
        root,
        kind: TrashKind::Home,
        top_dir: None,
    })
}

/// The same distrust `trash::dir`'s own (private, and so unreachable from
/// here) `volume_root_state` applies before *writing* into
/// `$topdir/.Trash-$uid`: a world-writable `$topdir` lets anybody pre-create
/// that path as a symlink pointing wherever they like. Reading or deleting
/// through it would hand this user's trashed files — and whatever else lives
/// at the other end — to whoever planted it. Kept as a small, deliberate
/// duplicate of that check rather than widening `dir`'s own visibility.
fn is_trustworthy_volume_trash_root(root: &Path, uid: u32) -> bool {
    match std::fs::symlink_metadata(root) {
        Ok(metadata) => {
            !metadata.file_type().is_symlink() && metadata.is_dir() && metadata.uid() == uid
        }
        Err(_) => false,
    }
}

fn volume_trash_dir_for(top_dir: &Path, uid: u32) -> Option<TrashDir> {
    let root = top_dir.join(format!(".Trash-{uid}"));
    if !is_trustworthy_volume_trash_root(&root, uid) {
        return None;
    }
    Some(TrashDir {
        files: root.join("files"),
        info: root.join("info"),
        root,
        kind: TrashKind::Volume,
        top_dir: Some(top_dir.to_path_buf()),
    })
}

const TRASHINFO_SUFFIX_BYTES: &[u8] = b".trashinfo";

/// Reads one trash root's `info/` and `files/`, pairing them by basename, and
/// appends whatever it finds to the caller's accumulators. A missing `info/`
/// or `files/` is silently zero entries (an unused trash); anything else that
/// prevents opening either directory goes to `unreadable_roots` instead of
/// being swallowed.
fn scan_trash_dir(
    trash_dir: &TrashDir,
    entries: &mut Vec<TrashEntry>,
    unreadable_records: &mut Vec<(PathBuf, TrashInfoError)>,
    unreadable_roots: &mut Vec<(PathBuf, TrashError)>,
) {
    // Basenames seen in `info/`, whether or not their record actually parsed:
    // a `files/` entry paired with a *broken* record still has a record, so
    // it must not also show up as `MissingInfo` (cb-listing-03).
    let mut info_basenames: HashSet<OsString> = HashSet::new();

    match std::fs::read_dir(&trash_dir.info) {
        Ok(read_dir) => {
            for dir_entry in read_dir.flatten() {
                let file_name = dir_entry.file_name();
                let Some(basename_bytes) = file_name.as_bytes().strip_suffix(TRASHINFO_SUFFIX_BYTES)
                else {
                    continue;
                };
                if basename_bytes.is_empty() {
                    continue;
                }
                let basename = OsStr::from_bytes(basename_bytes).to_os_string();
                info_basenames.insert(basename.clone());

                let info_path = dir_entry.path();
                match read_trash_info(&info_path) {
                    Ok(info) => {
                        let file_path = trash_dir.files.join(&basename);
                        if std::fs::symlink_metadata(&file_path).is_ok() {
                            entries.push(TrashEntry::Item(TrashedItem {
                                original_path: info.original_path,
                                trashed_path: file_path,
                                info_path,
                                deletion_date: info.deletion_date,
                                kind: trash_dir.kind,
                                top_dir: trash_dir.top_dir.clone(),
                                bytes_copied: None,
                            }));
                        } else {
                            entries.push(TrashEntry::MissingFile {
                                info_path,
                                original_path: info.original_path,
                                deletion_date: info.deletion_date,
                                kind: trash_dir.kind,
                                top_dir: trash_dir.top_dir.clone(),
                            });
                        }
                    }
                    Err(error) => unreadable_records.push((info_path, error)),
                }
            }
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            unreadable_roots.push((trash_dir.info.clone(), classify_io_error(&trash_dir.info, source)));
        }
    }

    match std::fs::read_dir(&trash_dir.files) {
        Ok(read_dir) => {
            for dir_entry in read_dir.flatten() {
                let file_name = dir_entry.file_name();
                if info_basenames.contains(&file_name) {
                    continue;
                }
                entries.push(TrashEntry::MissingInfo {
                    file_path: dir_entry.path(),
                    kind: trash_dir.kind,
                    top_dir: trash_dir.top_dir.clone(),
                });
            }
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            unreadable_roots.push((trash_dir.files.clone(), classify_io_error(&trash_dir.files, source)));
        }
    }
}

/// Permanently removes one trash entry — a paired item (the file/tree and its
/// `.trashinfo`) or either half of an unpaired one — without touching
/// anything else in the trash. This is the primitive both "eliminar de la
/// papelera un elemento concreto sin vaciarla entera" (§ «Vaciar la
/// papelera») and [`empty_trash_in`] are built from; the latter is just this
/// called once per [`TrashEntry`] a listing produced.
///
/// Never follows a symlink to delete outside the trash: a directory is
/// removed entry by entry (via [`super::delete_recursive`], the same walk
/// [`super::delete_permanently`] uses), and the top-level `files/`/`info/`
/// entry itself is only ever `lstat`-ed, never `stat`-ed.
pub fn delete_trash_entry(
    entry: &TrashEntry,
    observer: &mut dyn TrashObserver,
) -> Result<(), TrashError> {
    match entry {
        TrashEntry::Item(item) => {
            let metadata = dir::lstat(&item.trashed_path)?;
            let mut removed_count: u64 = 0;
            super::delete_recursive(&item.trashed_path, &metadata, observer, &mut removed_count)?;
            // Best-effort, exactly like `restore_item`'s own cleanup: once
            // the file is gone there is nothing left to protect by failing
            // here, and a `.trashinfo` that outlives it is precisely the
            // `MissingFile` case listing already surfaces instead of hiding.
            let _ = std::fs::remove_file(&item.info_path);
            Ok(())
        }
        TrashEntry::MissingFile { info_path, .. } => {
            std::fs::remove_file(info_path).map_err(|source| classify_io_error(info_path, source))
        }
        TrashEntry::MissingInfo { file_path, .. } => {
            let metadata = dir::lstat(file_path)?;
            let mut removed_count: u64 = 0;
            super::delete_recursive(file_path, &metadata, observer, &mut removed_count)
        }
    }
}

/// Outcome of emptying a trash. Never a `Result`, for the same reason
/// [`super::BatchOutcome`] is not one: one entry failing to delete must not
/// stop the rest (§ «Vaciar la papelera», casos borde: "informar cuáles no se
/// pudieron borrar y continuar con el resto").
#[derive(Debug)]
pub struct EmptyOutcome {
    /// Paired entries removed: both the trashed file/tree and its
    /// `.trashinfo`.
    pub removed: Vec<TrashedItem>,
    /// Unpaired entries removed: a stray `.trashinfo` or a stray `files/`
    /// entry with nothing on the other side.
    pub removed_orphans: Vec<PathBuf>,
    /// What could not be removed, and why.
    pub failed: Vec<(PathBuf, TrashError)>,
    pub cancelled: bool,
}

/// Permanently deletes every entry [`list_trash_in`] would report for the
/// personal trash and `volume_top_dirs`, recursively and cancellably, driven
/// by `observer` exactly like [`super::trash_batch`] — the same
/// retry/skip/skip-all/cancel vocabulary, per entry.
///
/// Deliberately takes **no path list**: unlike `trash_batch` (which moves
/// paths *into* the trash) or [`delete_trash_entry`] (which the caller points
/// at one specific entry), this always means "everything this call can see
/// in these trashes". There is nothing here that a caller could pass a
/// caller-chosen file selection into by mistake — confirming that is what is
/// actually wanted is the layer above this one's job, not this function's.
#[must_use]
pub fn empty_trash_in(volume_top_dirs: &[PathBuf], observer: &mut dyn TrashObserver) -> EmptyOutcome {
    let listing = list_trash_in(volume_top_dirs);
    let total = listing.entries.len();
    let mut removed = Vec::new();
    let mut removed_orphans = Vec::new();
    let mut failed = Vec::new();
    let mut cancelled = false;
    let mut skip_all = false;

    'entries: for (index, entry) in listing.entries.into_iter().enumerate() {
        let display_path = entry.display_path().to_path_buf();
        if observer.on_item_start(&display_path, index, total) == Flow::Cancel {
            cancelled = true;
            break 'entries;
        }

        loop {
            match delete_trash_entry(&entry, observer) {
                Ok(()) => {
                    if let TrashEntry::Item(item) = entry {
                        observer.on_item_done(&item);
                        removed.push(item);
                    } else {
                        removed_orphans.push(display_path);
                    }
                    continue 'entries;
                }
                Err(TrashError::Cancelled) => {
                    // Same treatment `trash_batch` gives an internal
                    // cancellation: terminal, not a reportable failure, and
                    // not "skipped" either.
                    cancelled = true;
                    break 'entries;
                }
                Err(error) => {
                    if skip_all {
                        failed.push((display_path.clone(), error));
                        continue 'entries;
                    }
                    match observer.on_error(&display_path, &error) {
                        ErrorDecision::Retry => continue,
                        ErrorDecision::Skip => {
                            failed.push((display_path.clone(), error));
                            continue 'entries;
                        }
                        ErrorDecision::SkipAll => {
                            failed.push((display_path.clone(), error));
                            skip_all = true;
                            continue 'entries;
                        }
                        ErrorDecision::Cancel => {
                            failed.push((display_path.clone(), error));
                            cancelled = true;
                            break 'entries;
                        }
                    }
                }
            }
        }
    }

    EmptyOutcome {
        removed,
        removed_orphans,
        failed,
        cancelled,
    }
}

/// [`empty_trash_in`] over every volume [`crate::places::this_computer`]
/// reports as mounted.
#[must_use]
pub fn empty_trash(observer: &mut dyn TrashObserver) -> EmptyOutcome {
    empty_trash_in(&mounted_volume_top_dirs(), observer)
}
