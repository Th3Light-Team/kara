//! The worker's path for jobs that involve a remote drive.
//!
//! Same events, same questions, same batch policy and the same undo records as
//! the local path in the parent module; only the I/O goes through
//! [`kara_vfs::Backend`]. A local item in a mixed job uses the local backend,
//! except local → local, which is handed to the old code.
//!
//! # Invariants
//!
//! - **A half-written destination is never left under its final name.** Bytes
//!   go through a [`kara_vfs::WriteSession`]: `finish` on success, `abort` on
//!   any failure or cancel.
//! - **A move removes its source only after the copy is verified**: the size
//!   of the destination, read back with `stat`, equals what was sent. A failed,
//!   cancelled, skipped or mismatched copy keeps the source.
//! - **A folder moved across backends is removed only after its whole subtree
//!   copied without a failure or a skip**; until then nothing under it is
//!   removed either. Then exactly what was copied is removed, children first:
//!   a file that appeared in the source meanwhile keeps its folder alive.
//! - **Nothing is overwritten silently.** A remote Replace is only file over
//!   file, through `begin_write(replace = true)`; replacing a folder on a drive
//!   without trash would destroy it, so it is reported instead.

use std::io::{self, Read, Write};
use std::path::Path;

use kara_core::FileEntry;
use kara_fs::trash::trash_one;
use kara_vfs::Location;

use super::{
    Answer, CHUNK, Cancelled, ConflictKind, ConflictPrompt, ErrorDecision, Event, Failure,
    FailureKind, Flow, Op, Resolution, Step, Worker, measure, trash_error_to_io,
};
use crate::clock::trash_policy;
use crate::location::{
    self, BackendResolver, Endpoint, Fail, NO_UNDO_CROSS_MOVE, NO_UNDO_MOVE,
    NO_UNDO_REMOTE_REPLACE, display_path, is_walkable_dir,
};
use crate::undo::Action;

/// What [`super::spawn_with`] hands the worker.
pub(super) struct LocationJob {
    pub sources: Vec<Location>,
    pub dest_dir: Location,
    pub resolver: BackendResolver,
}

const NO_NAME: &str = "no tiene un nombre usable";
const INTO_ITSELF: &str = "una carpeta no se puede copiar ni mover dentro de sí misma";
const LINK: &str = "los enlaces simbólicos no se copian a otra unidad";
const REPLACE_WITHOUT_TRASH: &str =
    "reemplazar una carpeta en una unidad sin papelera la borraría para siempre; no se hace";
const SIZE_MISMATCH: &str = "la copia no tiene el tamaño del original; el original se conserva";
const INCOMPLETE_LISTING: &str = "no se pudo leer todo el contenido de la carpeta";
const BAD_NAME: &str = "el nombre no es válido en el destino";

impl Worker {
    pub(super) fn run_locations(&mut self, job: LocationJob) {
        self.emit(Event::Calculating);

        let mut total_bytes = 0u64;
        let mut total_items = 0u64;
        for source in &job.sources {
            if self.cancelled() {
                break;
            }
            let measured = match source {
                Location::Local(path) => measure(path).ok(),
                Location::Remote { .. } => Endpoint::resolve(source, &job.resolver)
                    .ok()
                    .and_then(|end| location::measure(&end, &self.token).ok()),
            };
            match measured {
                Some((items, bytes)) => {
                    total_items = total_items.saturating_add(items);
                    total_bytes = total_bytes.saturating_add(bytes);
                }
                // Reported when its turn comes; the totals come out short.
                None => total_items = total_items.saturating_add(1),
            }
        }
        self.emit(Event::Started {
            total_bytes,
            total_items,
        });

        for source in &job.sources {
            if self.cancelled() {
                break;
            }
            let step = if self.op == Op::Delete {
                self.delete_location(source, &job)
            } else {
                self.transfer_location(source, &job)
            };
            if step.is_err() {
                // Whatever stopped it, the summary has to say it stopped.
                self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                self.token.cancel();
                break;
            }
        }

        self.finish();
    }

    /// [`Worker::guarded`] for backend work: same questions, same policy, but
    /// the failure is already classified and `path` is a display path (a URI
    /// for a remote item).
    fn guarded_loc<T>(
        &mut self,
        path: &Path,
        mut attempt: impl FnMut(&mut Self) -> Result<T, Fail>,
    ) -> Result<Option<T>, Cancelled> {
        loop {
            if self.cancelled() {
                return Err(Cancelled);
            }
            let fail = match attempt(self) {
                Ok(value) => {
                    self.policy.record_success();
                    return Ok(Some(value));
                }
                Err(fail) => fail,
            };
            // An error that is only the cancel being noticed is not a failure
            // to ask about.
            if self.cancelled() {
                return Err(Cancelled);
            }
            let failure = Failure {
                path: path.to_path_buf(),
                kind: fail.kind,
                reason: fail.reason,
            };
            if !self.settle(failure)? {
                return Ok(None);
            }
        }
    }

    /// A failure nobody is asked about: the item cannot be done at all.
    fn refuse(&mut self, path: &Path, reason: &str) {
        let failure = Failure {
            path: path.to_path_buf(),
            kind: FailureKind::Other,
            reason: reason.to_string(),
        };
        self.policy.record(failure, ErrorDecision::Skip);
    }

    /// Counts a subtree as done without touching it, so the bar still ends.
    fn count_skipped_end(&mut self, end: &Endpoint) {
        match location::measure(end, &self.token) {
            Ok((items, bytes)) => {
                self.items_done = self.items_done.saturating_add(items);
                self.bytes_done = self.bytes_done.saturating_add(bytes);
            }
            Err(_) => self.items_done = self.items_done.saturating_add(1),
        }
    }

    fn resolve(&mut self, location: &Location, resolver: &BackendResolver) -> Result<Option<Endpoint>, Cancelled> {
        let display = display_path(location);
        self.guarded_loc(&display, |_| {
            Endpoint::resolve(location, resolver).map_err(|error| error.fail())
        })
    }

    /// Permanently deletes one top-level item. Local items go through the old
    /// code; a remote one is `remove_tree` with the job's cancel token.
    fn delete_location(&mut self, source: &Location, job: &LocationJob) -> Step {
        if let Location::Local(path) = source {
            return self.delete_node(path);
        }
        let display = display_path(source);
        let Some(end) = self.resolve(source, &job.resolver)? else {
            self.items_done = self.items_done.saturating_add(1);
            return Ok(Flow::Partial);
        };
        let before = self.items_done;
        let count = location::measure(&end, &self.token).map_or(1, |(items, _)| items);
        self.tick(end.name(), false);
        let token = self.token.clone();
        let removed = self.guarded_loc(&display, |_| {
            end.backend
                .remove_tree(&end.path, &token)
                .map_err(Fail::from)
        })?;
        self.items_done = before.saturating_add(count);
        Ok(if removed.is_some() {
            Flow::Done
        } else {
            Flow::Partial
        })
    }

    /// Copies or moves one top-level item into the destination folder.
    fn transfer_location(&mut self, source: &Location, job: &LocationJob) -> Step {
        if let (Location::Local(path), Location::Local(dir)) = (source, &job.dest_dir) {
            // Local to local inside a mixed job: the old code, rename(2) and all.
            let Some(name) = path.file_name() else {
                self.refuse(path, NO_NAME);
                return Ok(Flow::Partial);
            };
            return self.node(path, dir.join(name), true);
        }
        let Some(src) = self.resolve(source, &job.resolver)? else {
            self.items_done = self.items_done.saturating_add(1);
            return Ok(Flow::Partial);
        };
        if src.path.is_root() {
            self.refuse(&src.display(), NO_NAME);
            return Ok(Flow::Partial);
        }
        let Some(dir) = self.resolve(&job.dest_dir, &job.resolver)? else {
            self.count_skipped_end(&src);
            return Ok(Flow::Partial);
        };
        let Some(dst) = dir.join(&src.name()) else {
            self.refuse(&src.display(), BAD_NAME);
            self.count_skipped_end(&src);
            return Ok(Flow::Partial);
        };
        let move_now = self.op == Op::Move;
        self.loc_node(&src, dst, true, move_now)
    }

    /// One node and everything under it onto `dst`.
    ///
    /// `record` as in [`Worker::node`]. `move_now` says whether this node may
    /// give up its source as part of a move; below a folder that is moved by
    /// copying, it is `false` and the folder removes the whole tree at the end.
    fn loc_node(&mut self, src: &Endpoint, mut dst: Endpoint, record: bool, move_now: bool) -> Step {
        let Some(entry) = self.guarded_loc(&src.display(), |_| src.stat().map_err(Fail::from))?
        else {
            self.count_skipped_end(src);
            return Ok(Flow::Partial);
        };
        let source_is_dir = is_walkable_dir(&entry);
        let name = src.name();
        let same = src.same_backend(&dst);

        if same && source_is_dir && dst.path != src.path && dst.path.starts_with(&src.path) {
            self.refuse(&src.display(), INTO_ITSELF);
            self.count_skipped_end(src);
            return Ok(Flow::Partial);
        }

        // Pasting where the item already is: a copy becomes «name (2)», a move
        // has nothing to do.
        if same && dst.path == src.path {
            if self.op == Op::Move {
                self.count_skipped_end(src);
                return Ok(Flow::Done);
            }
            dst = dst.unique_sibling();
        }

        let mut merge = false;
        let mut replace = false;
        // A destination the backend cannot answer for counts as free, as in
        // the local path: the write that follows never overwrites.
        while let Ok(existing) = dst.stat() {
            let kind = match (source_is_dir, is_walkable_dir(&existing)) {
                (false, false) => ConflictKind::FileOverFile,
                (false, true) => ConflictKind::FileOverDirectory,
                (true, false) => ConflictKind::DirectoryOverFile,
                (true, true) => ConflictKind::DirectoryOverDirectory,
            };
            let resolution = match self.decisions.decide(kind) {
                Some(known) => known.clone(),
                None => {
                    self.emit(Event::Conflict(ConflictPrompt {
                        kind,
                        name: name.clone(),
                        source: src.display(),
                        destination: dst.display(),
                    }));
                    match self.wait_for_answer() {
                        Some(Answer::Conflict {
                            resolution,
                            apply_to_all,
                        }) => {
                            if apply_to_all {
                                self.decisions.apply_to_all(kind, resolution.clone());
                            }
                            resolution
                        }
                        _ => return Err(Cancelled),
                    }
                }
            };
            if !kind.allows(&resolution) {
                return Err(Cancelled);
            }
            self.decisions.record(&resolution);

            match resolution {
                Resolution::Skip => {
                    self.count_skipped_end(src);
                    return Ok(Flow::Partial);
                }
                Resolution::KeepBoth => {
                    dst = dst.unique_sibling();
                    break;
                }
                Resolution::RenameTo(new_name) => match dst.sibling(&new_name) {
                    Some(renamed) => dst = renamed,
                    None => {
                        self.refuse(&dst.display(), BAD_NAME);
                        self.count_skipped_end(src);
                        return Ok(Flow::Partial);
                    }
                },
                Resolution::Merge => {
                    merge = true;
                    break;
                }
                Resolution::Replace => {
                    if let Location::Local(target) = &dst.location {
                        // Local: what is replaced goes to the trash first, as
                        // in the old path, and undo can bring it back.
                        let target = target.clone();
                        let trashed = self.guarded(&target, |_| {
                            trash_one(&target, &trash_policy()).map_err(trash_error_to_io)
                        })?;
                        match trashed {
                            Some(item) => {
                                self.actions.push(Action::Trashed {
                                    item: Box::new(item),
                                });
                                break;
                            }
                            None => {
                                self.count_skipped_end(src);
                                return Ok(Flow::Partial);
                            }
                        }
                    } else if kind == ConflictKind::FileOverFile {
                        replace = true;
                        break;
                    } else {
                        self.refuse(&dst.display(), REPLACE_WITHOUT_TRASH);
                        self.count_skipped_end(src);
                        return Ok(Flow::Partial);
                    }
                }
            }
        }

        self.tick(name, false);

        if source_is_dir {
            self.loc_directory(src, &dst, merge, record, move_now)
        } else {
            self.loc_leaf(src, &dst, &entry, replace, record, move_now)
        }
    }

    /// Whether a move of `src` onto `dst` is one backend rename.
    fn renames(&self, src: &Endpoint, dst: &Endpoint, move_now: bool) -> bool {
        self.op == Op::Move
            && move_now
            && src.same_backend(dst)
            && src.backend.capabilities().atomic_rename
    }

    /// A folder: renamed in one go on one drive, else created (or merged into)
    /// and filled; for a move, the source goes only once all of it arrived.
    fn loc_directory(
        &mut self,
        src: &Endpoint,
        dst: &Endpoint,
        merge: bool,
        record: bool,
        move_now: bool,
    ) -> Step {
        if !merge && self.renames(src, dst, move_now) {
            let renamed = self.guarded_loc(&src.display(), |_| {
                src.backend.rename(&src.path, &dst.path).map_err(Fail::from)
            })?;
            return match renamed {
                Some(()) => {
                    self.count_skipped_end(dst);
                    if record {
                        let action = self.move_record(src, dst, false);
                        self.actions.push(action);
                    }
                    Ok(Flow::Done)
                }
                None => {
                    self.count_skipped_end(src);
                    Ok(Flow::Partial)
                }
            };
        }

        if !merge {
            let made = self.guarded_loc(&dst.display(), |_| {
                dst.backend.create_dir(&dst.path).map_err(Fail::from)
            })?;
            if made.is_none() {
                self.count_skipped_end(src);
                return Ok(Flow::Partial);
            }
            if record {
                self.record_loc(src, dst, false);
            }
        }
        self.items_done = self.items_done.saturating_add(1);

        let token = self.token.clone();
        let Some(listing) = self.guarded_loc(&src.display(), |_| {
            src.backend.list(&src.path, &token).map_err(Fail::from)
        })?
        else {
            return Ok(Flow::Partial);
        };

        let mut whole = Flow::Done;
        if !listing.errors.is_empty() {
            // What could not be read is not copied, so the source must stay.
            self.refuse(&src.display(), INCOMPLETE_LISTING);
            whole = Flow::Partial;
        }
        let mut names: Vec<String> = Vec::with_capacity(listing.entries.len());
        for entry in listing.entries {
            match entry.name.to_str() {
                Some(name) => names.push(name.to_string()),
                None => whole = Flow::Partial,
            }
        }
        names.sort();

        // Children of a folder renamed child by child (a merge on one drive
        // with atomic rename) move themselves; anything else is copied first
        // and removed once all of it arrived.
        let children_move = self.renames(src, dst, move_now);
        let mark = self.copied_sources.len();
        for child in names {
            let (Some(child_src), Some(child_dst)) = (src.join(&child), dst.join(&child)) else {
                whole = Flow::Partial;
                continue;
            };
            let flow = self.loc_node(&child_src, child_dst, merge, children_move)?;
            if flow == Flow::Partial {
                whole = Flow::Partial;
            }
        }

        if self.op != Op::Move {
            return Ok(whole);
        }
        if !move_now {
            // Below a folder moved by copying: removed by that folder, after
            // its children.
            self.copied_sources.push((src.clone(), true));
            return Ok(whole);
        }
        if !children_move {
            let copied = self.copied_sources.split_off(mark);
            if whole == Flow::Partial {
                // Something did not arrive: every source stays.
                return Ok(whole);
            }
            for (end, is_dir) in copied {
                if is_dir {
                    // Empty by now unless something new appeared in it, and
                    // then it must stay.
                    let _ = end.backend.remove(&end.path);
                    continue;
                }
                let removed = self.guarded_loc(&end.display(), |_| {
                    end.backend.remove(&end.path).map_err(Fail::from)
                })?;
                if removed.is_none() {
                    whole = Flow::Partial;
                }
            }
        }
        if whole == Flow::Done {
            // Empty by now unless something else wrote into it; a failure to
            // remove it loses nothing.
            let _ = src.backend.remove(&src.path);
        }
        Ok(whole)
    }

    /// A file (or a link, which only a rename can move).
    fn loc_leaf(
        &mut self,
        src: &Endpoint,
        dst: &Endpoint,
        entry: &FileEntry,
        replace: bool,
        record: bool,
        move_now: bool,
    ) -> Step {
        let size = entry.size.unwrap_or(0);

        if !replace && self.renames(src, dst, move_now) {
            let renamed = self.guarded_loc(&src.display(), |_| {
                src.backend.rename(&src.path, &dst.path).map_err(Fail::from)
            })?;
            return match renamed {
                Some(()) => {
                    self.items_done = self.items_done.saturating_add(1);
                    self.bytes_done = self.bytes_done.saturating_add(size);
                    if record {
                        let action = self.move_record(src, dst, false);
                        self.actions.push(action);
                    }
                    Ok(Flow::Done)
                }
                None => {
                    self.count_skipped_end(src);
                    Ok(Flow::Partial)
                }
            };
        }

        if entry.is_symlink {
            // The trait has no way to create a link, and copying what it
            // points to would silently turn it into something else.
            self.refuse(&src.display(), LINK);
            self.count_skipped_end(src);
            return Ok(Flow::Partial);
        }

        let copied = self.guarded_loc(&src.display(), |worker| {
            worker.copy_file(src, dst, entry.size, replace)
        })?;
        if copied.is_none() {
            self.count_skipped_end(src);
            return Ok(Flow::Partial);
        }
        self.items_done = self.items_done.saturating_add(1);
        if record {
            self.record_loc(src, dst, replace);
        }
        if self.op == Op::Move && !move_now {
            // Removed by the folder being moved, once all of it arrived.
            self.copied_sources.push((src.clone(), false));
        }

        if self.op == Op::Move && move_now {
            // Only now: the copy is complete and its size checked. If removing
            // the original fails the user has two copies, the safe way to be
            // wrong.
            let removed = self.guarded_loc(&src.display(), |_| {
                src.backend.remove(&src.path).map_err(Fail::from)
            })?;
            if removed.is_none() {
                return Ok(Flow::Partial);
            }
        }
        Ok(Flow::Done)
    }

    /// Copies one file: server side when the drive can, streamed otherwise,
    /// then checks the size of what arrived.
    fn copy_file(
        &mut self,
        src: &Endpoint,
        dst: &Endpoint,
        source_size: Option<u64>,
        replace: bool,
    ) -> Result<(), Fail> {
        let server_side = !replace
            && !src.is_local()
            && src.same_backend(dst)
            && src.backend.capabilities().server_side_copy;
        if server_side {
            src.backend.copy_within(&src.path, &dst.path)?;
            let expected = match source_size {
                Some(size) => Some(size),
                None => src.stat().ok().and_then(|entry| entry.size),
            };
            verify(dst, expected, true)?;
            self.bytes_done = self.bytes_done.saturating_add(expected.unwrap_or(0));
            return Ok(());
        }

        let mut reader = src.backend.open_read(&src.path, 0)?;
        let mut session = dst.backend.begin_write(&dst.path, source_size, replace)?;
        let before = self.bytes_done;
        let name = src.name();
        let mut buffer = vec![0u8; CHUNK];
        let mut sent = 0u64;
        let streamed = loop {
            self.wait_while_paused();
            if self.cancelled() {
                break Err(Fail::cancelled());
            }
            let read = match reader.read(&mut buffer) {
                Ok(0) => break Ok(()),
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => break Err(Fail::from_io(error)),
            };
            let Some(chunk) = buffer.get(..read) else {
                break Err(Fail::other("lectura inválida"));
            };
            if let Err(error) = session.write_all(chunk) {
                break Err(Fail::from_io(error));
            }
            let read = u64::try_from(read).unwrap_or(u64::MAX);
            sent = sent.saturating_add(read);
            self.bytes_done = self.bytes_done.saturating_add(read);
            self.tick(name.clone(), false);
        };

        let committed = match streamed {
            Ok(()) => session.finish().map_err(Fail::from),
            Err(fail) => {
                // Leaves nothing behind; its own failure changes nothing.
                let _ = session.abort();
                Err(fail)
            }
        };
        // A retry starts over, so the counter goes back too.
        let verified = committed.and_then(|()| verify(dst, Some(sent), !replace));
        if verified.is_err() {
            self.bytes_done = before;
        }
        verified
    }

    /// The undo record of a finished copy or move of `src` onto `dst`.
    fn record_loc(&mut self, src: &Endpoint, dst: &Endpoint, replaced: bool) {
        let action = match self.op {
            Op::Delete => return,
            Op::Copy => match &dst.location {
                Location::Local(path) => Action::Copied {
                    created: path.clone(),
                },
                remote if replaced => Action::NotUndoable {
                    label: "copiar",
                    subject: remote.clone(),
                    reason: NO_UNDO_REMOTE_REPLACE.to_string(),
                },
                remote => Action::RemoteCopied {
                    created: remote.clone(),
                },
            },
            Op::Move => self.move_record(src, dst, replaced),
        };
        self.actions.push(action);
    }

    /// A move is undoable only inside one drive that declares `undo_move`.
    fn move_record(&self, src: &Endpoint, dst: &Endpoint, replaced: bool) -> Action {
        let not_undoable = |reason: &str| Action::NotUndoable {
            label: "mover",
            subject: dst.location.clone(),
            reason: reason.to_string(),
        };
        if !src.same_backend(dst) {
            not_undoable(NO_UNDO_CROSS_MOVE)
        } else if replaced && !dst.is_local() {
            not_undoable(NO_UNDO_REMOTE_REPLACE)
        } else if src.backend.capabilities().undo_move {
            Action::RemoteMoved {
                from: src.location.clone(),
                to: dst.location.clone(),
            }
        } else {
            not_undoable(NO_UNDO_MOVE)
        }
    }
}

/// Reads back the size of `dst` and compares it with what was sent. A copy
/// that does not match is removed when it was created fresh (never when it
/// replaced something: that would lose both).
fn verify(dst: &Endpoint, expected: Option<u64>, fresh: bool) -> Result<(), Fail> {
    let actual = dst.stat().map(|entry| entry.size);
    if let (Ok(Some(actual)), Some(expected)) = (&actual, expected)
        && *actual == expected
    {
        return Ok(());
    }
    if fresh {
        let _ = dst.backend.remove(&dst.path);
    }
    Err(match actual {
        Err(error) => Fail::from(error),
        Ok(_) => Fail::other(SIZE_MISMATCH),
    })
}
