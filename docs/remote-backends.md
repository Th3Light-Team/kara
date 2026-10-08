# Remote drives: one contract, interchangeable adapters

Status: **design, for review.** No code yet.

## Goal

The user adds a remote drive in the left panel (section «Red»). From then on it
behaves like any other place: browse, select, copy, move, rename, delete, drag,
progress, conflicts. **There is one UI.** The QML never learns which protocol is
underneath.

First adapters: **SFTP** and **S3**. The contract must not be shaped around
either of them.

## Where it goes

```
kara-ui → kara-ops → { kara-fs, kara-remote, kara-index } → kara-vfs → kara-core
```

| Crate | Role |
|---|---|
| `kara-vfs` (new) | `Location`, the `Backend` trait, `BackendError`, `Capabilities`, and the conformance suite (feature `conformance`). No protocol code. |
| `kara-fs` | Gains `LocalBackend`: the existing code behind the trait. Public functions stay. |
| `kara-remote` (new) | `MemoryBackend` (tests), `sftp` and `s3` behind cargo features. |

`kara-core` stays free of I/O, so the trait does not live there. `kara-vfs`
depends on `kara-core` only for `FileEntry`.

## `Location`

```rust
pub enum Location {
    Local(PathBuf),
    Remote { drive: DriveId, path: RemotePath },
}
```

- `RemotePath`: normalised, `/`-separated, no `.`/`..`, UTF-8. Not a `PathBuf`:
  S3 keys are not filesystem paths and must not inherit their semantics.
- Replaces `Path`/`PathBuf` in `kara-ops` (`Queue::push`, `Request`), in tabs and
  history, and in `FileEntry::location`. This is the large refactor; it goes in
  small steps, tests green at each one (see *Order of work*).
- QML receives a `Location` as a string URI (`file:///…`, `kara+sftp://drive-id/…`),
  produced and parsed in Rust only. Breadcrumbs, Ctrl+L and bookmarks use it.

## `Backend`

Blocking, `Send + Sync`. Listing and transfers already run on workers; an async
trait would force a runtime on every caller. An adapter that needs one (russh,
aws-sdk) owns a private runtime.

```rust
pub trait Backend: Send + Sync {
    fn capabilities(&self) -> Capabilities;

    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError>;
    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError>;

    fn open_read(&self, path: &RemotePath, from: u64) -> Result<Box<dyn Read + Send>, BackendError>;
    fn begin_write(&self, path: &RemotePath, size_hint: Option<u64>, replace: bool)
        -> Result<Box<dyn WriteSession>, BackendError>;

    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError>;
    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError>;
    fn remove(&self, path: &RemotePath) -> Result<(), BackendError>;      // file or empty dir
    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError>;

    /// Only called if `capabilities().server_side_copy`.
    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError>;
}

pub trait WriteSession: Write + Send {
    fn finish(self: Box<Self>) -> Result<(), BackendError>; // commit: rename temp, complete multipart
    fn abort(self: Box<Self>);                              // leave nothing behind
}
```

`WriteSession` exists because both adapters need a commit step: SFTP writes to a
temporary name and renames, S3 completes or aborts a multipart upload. Dropping
a session without `finish` aborts it. **A half-written destination is never
visible under its final name.**

### Errors

`BackendError { kind, path, source }`, with `kind` one of `NotFound`,
`AlreadyExists`, `PermissionDenied`, `NoSpace`, `Unavailable` (connection lost),
`AuthRequired`, `Unsupported`, `Cancelled`, `Other`. `kara-ops` maps it onto its
existing `FailureKind` (`Unavailable` → `MediaGone`, so no infinite retry loop;
`AuthRequired` → `needs_user_action_first`). No `unwrap`/`expect`/`panic!` in
`kara-vfs` or `kara-remote`, same rule and same scan as `kara-fs`.

### Capabilities

Adapters declare what they lack; the UI reads the flags and shows the warnings the
spec already asks for. It never branches on the protocol.

| Flag | SFTP | S3 | Effect in the UI |
|---|---|---|---|
| `trash` | no | no | Supr warns and offers permanent delete, confirmed, focus on the safe button |
| `atomic_rename` | yes | no (copy+delete) | Rename of big trees shows progress |
| `server_side_copy` | no | yes | Copy inside the drive moves no bytes |
| `real_directories` | yes | no (prefixes) | «Nueva carpeta» creates a `key/` marker |
| `posix_permissions` | yes | no | Permissions tab hidden |
| `symlinks` | yes | no | |
| `watch` | no | no | Manual refresh only; no inotify |
| `undo_rename`/`undo_move` | yes | no | Undo disabled with a reason, as the spec says |

## Transfers

`kara-ops` decides the route from the two `Location`s:

1. Local → local: unchanged (`rename(2)`, existing code).
2. Same remote drive and `server_side_copy`: `copy_within`.
3. Anything else: stream `open_read` → `begin_write`, with progress, speed/ETA,
   cancel and pause at chunk boundaries.

A move across backends is copy, verify size, then delete. **The source is never
deleted if the copy failed or the sizes differ.** This is the invariant that
`kara-fs` already guards for local moves, extended.

## Drives

- `DriveConfig { id, kind, label, params }` is persisted in `settings.conf`
  (non-secret fields only). Secrets go through a `SecretStore` trait; the
  implementation is an open question below.
- `DriveRegistry` maps `DriveId` to `Arc<dyn Backend>`, and tracks state
  (`Connecting`, `Ready`, `Lost`, `Reconnecting`) so the panel can draw it.
- Connecting never blocks the UI. Anything that needs the user (SFTP unknown
  host key, password, MFA) comes out as an event the UI answers; the backend
  does not draw dialogs.

## Adapter notes

**SFTP.** Auth by agent, key file or password. Host key checked against
`~/.ssh/known_hosts`; an unknown key is a prompt (trust once / always / cancel),
a *changed* key is a hard refusal. Keepalive plus reconnect. Writes go to a temp
name then rename. mtime preserved when the server allows it.

**S3.** One drive = endpoint + bucket (+ optional prefix) + credentials, so MinIO,
R2 and B2 work. Listing uses `delimiter=/` and pagination, streaming entries so
a 100 000-object prefix does not block. No `created`/`accessed`; `modified` is
`LastModified`; storage class and ETag go in the `MetadataBag`. Upload is
multipart above a threshold, aborted on cancel. `remove_tree` lists and batch-deletes.
A drive with no bucket (listing buckets) is out of scope for now.

## Conformance suite

One generic function in `kara-vfs`, run against every backend:

- list / stat round trip, names with spaces, unicode and `#`/`%`/`?`
- write → read back, empty file, file larger than one chunk
- `replace = false` on an existing target fails with `AlreadyExists`
- aborted or dropped `WriteSession` leaves nothing visible
- rename, remove, `remove_tree`
- cancel mid-list and mid-transfer
- every error kind reachable, and only declared capabilities are exercised

`MemoryBackend` and `LocalBackend` run it in CI. SFTP and S3 run it against a
real server (OpenSSH container, MinIO) behind `KARA_TEST_SFTP`/`KARA_TEST_S3`
environment variables and `#[ignore]` otherwise. Add them in new test files;
`crates/kara-fs/tests/*` is hash-checked.

## Order of work

1. `kara-vfs`: `Location`, `RemotePath`, errors, trait, `MemoryBackend`, conformance suite.
2. `LocalBackend` in `kara-fs`, passing the suite. Existing tests untouched.
3. `Location` through `kara-ops`, then `kara-ui` (`present.rs` first, then the bridge).
4. `DriveRegistry`, config, and the panel entry «Añadir unidad…» (QML is presentation only).
5. SFTP adapter.
6. S3 adapter.

Steps 1–2 and the Rust side of 3–4 can be built and tested without Qt. Only the
panel and the bridge need a Qt build.

## Open questions

1. **Secrets:** system keyring (Secret Service) or not stored (ask every time)?
   Never plain text in `settings.conf`.
2. **Crates:** SFTP via `russh-sftp` (pure Rust, async, private runtime) or `ssh2`
   (libssh2, blocking, C dependency in CI). S3 via `aws-sdk-s3` (standard, heavy) or
   `rust-s3`. Decide at steps 5 and 6, after the trait is frozen.
3. **Search and folder size** (`kara-index`) over a remote drive: not in scope. They
   report «no disponible» until a backend declares a cheap way to do it.
4. **Thumbnails** on remote drives: on demand only, per the spec; cache key must
   include the drive.
