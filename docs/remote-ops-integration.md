# Remote drives in `kara-ops`: handoff for the UI

Step 3 of `remote-backends.md`, Rust side. `kara-ops` now runs copy, move,
permanent delete, rename and new folder on `kara_vfs::Location`s, so any of
them can involve a remote drive, with the same progress, conflicts, error
policy and undo stack as local work. **`kara-ui` was not touched**: Qt is not
installed in the environment where this was built, so the bridge could not be
compiled. This page says what the bridge has to change.

## New public API (`kara_ops`)

Everything that existed keeps its signature and behaviour. Additions:

| Item | What it is |
|---|---|
| `BackendResolver` | `Arc<dyn Fn(&DriveId) -> Option<Arc<dyn Backend>> + Send + Sync>`. Called on the worker thread. `no_drives()` serves none. |
| `LocationRequest { op, sources: Vec<Location>, dest_dir: Location, confirmed_permanent }` | The Location twin of `runner::Request`. `From<Request>` (a converted `Op::Delete` counts as confirmed, since the old API's only caller is the confirmation dialog), `is_all_local()`, `to_local()`. |
| `runner::spawn_with(request, resolver, sink) -> Result<Handle, RequestError>` | Starts the job. Refuses up front, touching nothing, with `RequestError::PermanentDeleteNotConfirmed` (a delete without `confirmed_permanent`) or `RequestError::DriveUnavailable(DriveId)` (a source or destination drive the resolver does not serve). An all-local request is handed to the old `spawn`: local behaviour is bit for bit the old one. |
| `Handle::pause()`, `resume()`, `is_paused()` | Holds a transfer at the next chunk boundary (256 KiB), local and remote. Cancel still works while paused. |
| `display_path(&Location) -> PathBuf`, `parse_display_path(&Path) -> Location` | How a remote item appears in `ConflictPrompt`, `FailurePrompt`, `Failure` and `BatchReport::skipped`: its canonical URI (`kara+sftp://nas/docs/a%20b.txt`) in the existing `PathBuf` fields. A local item is its plain path, as before. |
| `failure_kind(BackendErrorKind) -> FailureKind` | `PermissionDenied`/`AuthRequired` → `PermissionDenied`, `NoSpace` → `NoSpace`, `Unavailable` → `MediaGone` (so a blanket retry never loops), anything else → `Other`. |
| `rename_at(&Location, new_name, &resolver) -> Result<(Location, Action), LocationOpError>` | Never overwrites (`AlreadyExists`). Local: `kara_fs::rename(…, Fail)` and `Action::Renamed` as today. **Blocking**: call it from a worker for a remote location. |
| `create_dir_at(&Location parent, name, &resolver)` | «Nueva carpeta», «Nueva carpeta (2)»… on any drive (the `kara_core::unique_name` rule). Local: `kara_fs::create_directory(…, KeepBoth)` as today. Blocking. |
| `LocationOpError` | `DriveUnavailable`, `InvalidName`, `AlreadyExists(uri)`, `Failed { path: uri, kind, reason }`, `Local(TransferError)`. |
| `Action::RemoteCopied`, `RemoteMoved`, `RemoteRenamed`, `RemoteDirectoryCreated`, `NotUndoable { label, subject, reason }` | Undo records over `Location`s. |
| `Action::is_undoable()`, `not_undoable_reason()` | |
| `UndoStack::undo_with(&resolver)`, `redo_with(&resolver)` | Undo/redo that can reach drives. `undo()`/`redo()` still work for local records; on a remote one they fail with `UndoError::DriveUnavailable` and keep the record. |
| `UndoStack::undo_disabled_reason()` | `Some(reason)` when the top record is `NotUndoable`. `can_undo()` is then `false`. |
| `UndoError::NotUndoable(reason)`, `DriveUnavailable(uri)`, `Remote { path: uri, reason }` | New variants. |

`Op`, `Event`, `Answer`, `Outcome`, `ConflictPrompt`, `FailurePrompt`,
`Failure` and `BatchReport` are unchanged, so the bridge's existing matches keep
compiling.

## Routing (what the worker does)

| Source → destination | Route |
|---|---|
| local → local | the old code (`rename(2)` on one volume, trash on Replace). Also for a local item inside a mixed job. |
| same drive, a move, `atomic_rename` | `Backend::rename` |
| same drive, `server_side_copy` | `copy_within`, then the destination size is checked |
| anything else | `open_read` → `begin_write`, 256 KiB chunks, progress/cancel/pause per chunk, `finish` on success, `abort` on any failure or cancel, then `stat` of the destination must equal the bytes sent |

A move removes a file's source only after its copy is verified. A folder moved
across backends (or on a drive without atomic rename) is copied whole first;
only if every item under it arrived are the copied sources removed, children
first, files through the failure policy and folders only while empty. A file
that appears in the source meanwhile is never deleted.

Remote Replace is **file over file only**, via `begin_write(replace = true)`.
Replacing a folder (or a folder over a file) on a drive without trash would
destroy it, so it is reported as a failure and nothing is touched. Replace on a
*local* destination still sends the old item to the trash first.

## What `kara-ui` must change

**Who builds `Location`s.** Rust only. QML keeps receiving and sending strings:
`Location::to_uri()` / `Location::from_uri()` (`file:///…`, `kara+<scheme>://<drive>/…`).
Until tabs and history hold `Location`s (`present.rs` first, then the bridge,
per step 3 of the design), the bridge can wrap its `PathBuf`s in
`Location::Local`; nothing changes for local folders.

**Paste** (`paste`, `start_job`). Build a `LocationRequest` instead of a
`Request`: clipboard `file://` URIs become `Location::Local`, `kara+…` URIs
`Location::Remote`; the destination is the tab's location. Call
`runner::spawn_with(request, registry_resolver, sink)` and show a
`RequestError` with `report()` (nothing was started). The resolver comes from
the `DriveRegistry` of step 4; until it exists, `kara_ops::no_drives()`.

**Conflict dialog** (`on_paste_event`, `Event::Conflict`). `prompt.source` and
`prompt.destination` may be URIs. The current `describe(&prompt.source)` reads
local metadata; for a remote side it must `stat` through the backend
(`parse_display_path` → resolver), on a worker, or show the dialog without
size/date. The dialog text should show the drive label plus the remote path
rather than the raw URI when the registry has a label.

**Failure dialog** (`Event::Failure`). `prompt.path` is a URI for a remote item;
`prompt.kind` is already mapped. `MediaGone` (drive disconnected) gets no
«Reintentar todos»: `retry_may_help()` is `false`, as for vanished local media.
The final summary lists `report.skipped`/`failures` paths the same way.

**Permanent delete.** Supr on a location whose backend has
`capabilities().trash == false` must not trash: open the existing
permanent-delete dialog (focus on the safe button) with a text that says the
drive has no trash, and on confirm send
`LocationRequest { op: Op::Delete, confirmed_permanent: true, … }`. A request
without the flag is refused by `spawn_with`. Local Supr stays `trash_one` in
the bridge, unchanged; local Shift+Supr may keep using the old request.

**Rename and new folder** (`rename_entry`, `create_folder`). For a remote
location call `rename_at` / `create_dir_at` on a worker (they block on the
network) and `record()` the returned `Action` when the result comes back. On a
backend without `atomic_rename` (S3) a rename of a big folder is slow; it is
still one call today (see below).

**Undo.** `undo()`/`redo()` in the bridge must call
`undo_with(&resolver)`/`redo_with(&resolver)`, on a worker when the top record
is remote (`Action::Remote*`). `publish_undo` must also publish
`undo_disabled_reason()`: with a `NotUndoable` record on top, `can_undo()` is
`false` and the menu shows «Deshacer mover» disabled with the reason, as the
spec asks. Undo rules:

- remote copy: the copy is removed, permanently (no remote trash);
- remote new folder: removed only while still empty;
- move / rename inside one drive: renamed back, only while the drive declares
  `undo_move` / `undo_rename`;
- `NotUndoable`: a move between two backends, any move or rename on a drive
  without the capability, a copy that replaced a remote file;
- a permanent delete is never recorded.

**Pause.** The progress dialog's pause button can call `Handle::pause()` /
`resume()` and `Meter::pause()` / `resume()`.

## Decisions the owner should review

1. **Undo of a move between backends is not offered** (`NotUndoable`), even if
   the drive declares `undo_move`: undoing it means transferring everything back
   on the UI's undo path, with no progress and no conflict dialog. The
   capability applies to moves inside one drive.
2. **Symlinks are not copied to another backend** (the trait cannot create
   one, and copying the target would silently change what it is). Each link is
   reported and skipped, so a move keeps its source folder. A move inside one
   drive with atomic rename moves links like anything else.
3. **Remote Replace is file over file only** (see above).
4. **A copy that replaced a remote file is not undoable**: removing the copy
   would lose both versions.
5. **`confirmed_permanent` is required for every delete of the new API**,
   local ones included, because `Op::Delete` was already permanent.
6. A copied file does not keep its mtime or permissions on a remote drive (the
   trait has no call for it); the old local path still preserves both.
7. Rename on a drive without `atomic_rename` is one blocking `Backend::rename`
   (the adapter's copy+delete), without progress. A progress-reporting rename
   would be a job through `spawn_with`.

## Not done here

- `kara-ui` (`present.rs`, `bridge.rs`, QML): needs Qt to build.
- `Queue::push` still takes `PathBuf`s; nothing in the bridge uses the queue
  for paste jobs today.
- `DriveRegistry` and the drive list in the panel: step 4.
- Conflict dialog metadata (size, date) for a remote side: needs the bridge.
