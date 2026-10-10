# Remote drives: what is built, what is left

Status after steps 1–4 (Rust side). Everything below the UI is done and tested in
the cloud container; what is left either needs Qt, a real server, or an owner
decision. Design: `remote-backends.md`. Tests and mutation tables:
`remote-backends-testing.md`. Known red tests: `known-test-failures.md`.
Job API for the UI: `remote-ops-integration.md`.

## Built

| Step | Crate | What |
|---|---|---|
| 1 | `kara-vfs` | `Location`, `RemotePath`, `DriveId`, `Backend`, `WriteSession`, errors, `Capabilities`, `MemoryBackend`, conformance suite |
| 2 | `kara-fs` | `LocalBackend` (fd-based delete, temp-file + atomic rename, parent fsync) |
| 3 | `kara-ops` | `LocationRequest`, `spawn_with`, streaming copy/move with size check, remote delete (confirmed), undo rules |
| 4 | `kara-remote` | `DriveConfig` <-> `settings.conf`, `SecretStore` (+ `oo7` keyring), groups with shared secrets, CSV / `ssh_config` import, `DriveRegistry`, `BackendFactory`, prompts |

### `kara-remote` in one page

- `DriveConfig { id, label, params }`. Stored as `[drive:<scheme>:<name>]` with
  `label` and `param.<key>`. A parameter whose key contains `password`, `passwd`,
  `passphrase`, `secret`, `token` or `apikey` is refused: secrets never reach the file.
- `SecretStore`: `MemorySecretStore` (this run only) and `KeyringSecretStore`
  (`--features keyring`, `oo7`, private tokio current-thread runtime). If no keyring
  is reachable the drive still connects and asks each time; `registry.secrets_are_persistent()`
  tells the UI whether to say so.
- `DriveRegistry`:
  `register_factory` · `add` / `add_all` / `remove` · `connect(id, &dyn PromptHandler, &Cancel)`
  (blocking, worker thread) · `disconnect` · `report_failure(id, &BackendError)`
  (an `Unavailable` marks the drive `Lost`) · `state` · `subscribe` ·
  `resolver()` (same type as `kara_ops::BackendResolver`; resolves only while `Ready`).
- Groups: `DriveConfig::with_group`; a group can share one secret (`Remember::ForGroup`,
  `remember_group_secret`); a drive's own secret wins over its group's; removing a
  drive leaves the group's secret for the others; `registry.groups()` lists members.
- Bulk add: `import::from_csv(text, scheme, default_group)` (`name,host[,user[,port[,key_file[,label[,group]]]]]`)
  and `import::from_ssh_config(text, scheme, group)` (concrete `Host` entries; wildcards,
  `Match`, `Include` skipped; `ProxyJump`/`ProxyCommand` hosts reported, not imported).
  Both return `ImportReport { drives, problems }`: a bad line never hides the good ones.
- States: `Disconnected → Connecting → Ready | Failed{reason}`, `Ready → Lost{reason}`.
- Prompts (`Prompt`): `Password`, `TrustHostKey`, `HostKeyChanged`. The registry
  asks for the password itself (up to 3 attempts; the secret is stored only after
  a connection with it worked and only if the user asked). Host-key prompts are
  asked by the factory through the same handler. The backend never draws a dialog.
- `--features memory` adds drive kind `mem` (`MemoryFactory`) for tests and demos.

## Use case: a Proxmox fleet of LXC containers

Goal: give an IP and credentials (password or key) and browse the container; do it
for dozens of containers without a form each. Two ways in, both over SFTP:

- **Per container**: `sftp` drive with `host`, `user`, `port`, `key_file` or a
  password. Needs `sshd` with the SFTP subsystem inside the container
  (`openssh-server`, on Debian/Ubuntu `openssh-sftp-server`) and, for root with a
  password, `PermitRootLogin yes`; not verified against Proxmox's own templates.
- **Through the node**: one `sftp` drive to the Proxmox host as root; running
  containers show up under `/var/lib/lxc/<vmid>/rootfs`. Unprivileged containers show
  shifted uids (100000+); ZFS/LVM rootfs must be mounted. No change to the containers.

Built for it: groups with one shared secret, CSV and `ssh_config` import, lazy
connection (`add` never connects; `connect` is per drive, on demand).

Not built, with the reason:

| Missing | Why / what it needs |
|---|---|
| Discovery through the Proxmox API (list containers and their IPs with an API token) | Needs an HTTPS/JSON client; none is in the dependency set yet (`ureq` or `reqwest` would be the first). Produces `DriveConfig`s; nothing else changes. |
| «Connect the whole group» with a concurrency limit | Trivial on top of `connect` (worker pool, N at a time); wait for the UI to know what it wants to show while 50 drives connect. |
| Bulk trust of host keys | Deliberately absent. Trusting 50 unknown keys in one click defeats the check. Proposal: one prompt per drive, plus an explicit «trust all keys in this import» that lists the fingerprints and is off by default. Decide with SFTP (step 5). |
| Per-group parameters (same user/key for every member) | The CSV/ssh_config columns cover it today; a group-level default param set would be a small addition to `DriveConfig`. |
| Incus/LXD file API adapter (no `sshd` needed) | A second `BackendFactory` (scheme `incus`); see below. |

### Incus / LXD adapter (optional, after SFTP)

REST file API over the unix socket or HTTPS with a client certificate:
`GET /1.0/instances/<name>/files?path=` (list a directory, read a file),
`POST` with `X-LXD-type: file|directory` and mode/uid/gid headers (write, mkdir),
`DELETE`. Capabilities: no atomic rename (copy + delete), no server-side copy,
POSIX permissions and symlinks yes. It would need the owner decision on preserving
mode/uid/gid (open decision 4 below) to be useful, since the API takes them on write.

## Stack and environment limits met so far

- **No Qt** in the cloud container: nothing under `kara-ui` builds, so the panel,
  the dialogs and the bridge are written from the specs in this file and
  `remote-ops-integration.md`, untested.
- **No `ssh`/`sshd` binaries** and an empty apt index: the SFTP adapter cannot be
  conformance-tested against OpenSSH here. Plan: `russh` also ships a server; an
  in-process `russh` + `russh-sftp` server on `127.0.0.1` over a tempdir gives a
  hermetic test (auth, host keys, the conformance suite), with a real `sshd`
  container as the second, ignored-by-default check.
- **No session bus / keyring**: `KeyringSecretStore` is compiled and linted only.
- **`oo7` is heavy**: with it the lockfile grows by ~750 lines (zbus, ashpd, crypto).
  It is behind `--features keyring`, but Cargo.lock lists it either way.
- **Async crates under a blocking trait**: `oo7`, and later `russh-sftp` and
  `object_store`, are async. Each adapter owns a private current-thread runtime and
  blocks on it; nothing async leaks into `kara-vfs`, `kara-ops` or the UI.
- **Root and no tmpfs** in the container make 19 older `kara-fs` tests fail
  (`known-test-failures.md`).

## Left: step 4, UI side (needs Qt)

Wiring in `kara-ui` (nothing of this is business logic; it lives in Rust, QML only binds):

1. **Startup.** `Prefs`/settings: `config::load_all(&settings)` → `registry.add_all`.
   Report the returned problems once, like `Prefs` reports an unreadable file.
   Build the `SecretStore`: try `KeyringSecretStore::open()`, fall back to
   `MemorySecretStore` and show «Las contraseñas no se guardarán» when
   `secrets_are_persistent()` is false. Register the factories (SFTP, later S3).
2. **Panel.** A «Red» section in the left panel listing `registry.configs()` with
   `registry.state(id)` (icon: connected / connecting / lost / failed) and
   `subscribe` feeding a QML model. Click on a disconnected drive → `connect` on a
   worker thread; click on a ready one → navigate to `Location::Remote { drive, path: root }`.
3. **«Añadir unidad…» dialog.** Fields: protocol (the registered schemes), name,
   label, protocol parameters, secret, «Recordar». On accept: `DriveConfig::new`
   (show its error text), `registry.add`, `config::store` + `settings::save`,
   `registry.remember_secret` if asked. Context menu: Conectar / Desconectar /
   Editar / Quitar (`registry.remove` + `config::forget` + save).
4. **Prompts.** Implement `PromptHandler` in the bridge: it posts the prompt to the
   UI thread (`qt_thread().queue`), blocks the worker on a channel until the dialog
   answers. Host-key dialogs: default button = Refuse; `HostKeyChanged` must read as
   a warning, never as a routine question.
5. **Lost drives.** On any `Unavailable` from a job or a listing call
   `registry.report_failure`; the panel shows the drive as lost with «Reconectar».
6. **Ops.** Pass `registry.resolver()` to `kara_ops::runner::spawn_with`
   (details and the delete-confirmation rule in `remote-ops-integration.md`).
7. **Breadcrumbs / Ctrl+L / bookmarks** use `Location::to_uri` / `from_uri`
   (`kara+sftp://work-nas/path`); drive the label from `DriveConfig::label`.
8. **Listing and tree** go through `Backend::list` on a worker with a request
   number, exactly like the local `request_listing`; no inotify: manual refresh only
   (`Capabilities::watch == false`). Thumbnails on remote drives: on demand only,
   cache key includes the drive.

## Left: step 5, SFTP adapter (`russh` + `russh-sftp`)

New module `kara-remote/src/sftp/` behind `--features sftp`; a `SftpFactory`
implementing `BackendFactory` for scheme `sftp`, a `SftpBackend` implementing
`kara_vfs::Backend`. Decisions already taken: pure Rust, async inside, a private
runtime per backend, blocking trait outside.

- **Params:** `host`, `port` (22), `user`, `key_file` (optional), `known_hosts`
  (optional, default `~/.ssh/known_hosts`), `root` (optional start path).
- **Auth order:** agent → `key_file` (passphrase = secret) → password (secret).
  Return `ConnectError::AuthRequired` when a secret is needed and none was given,
  `AuthFailed` when it was rejected.
- **Host keys:** known and equal → connect; unknown → `Prompt::TrustHostKey`
  (remember → append to known_hosts); different → `Prompt::HostKeyChanged`, refuse
  unless the user insists, never auto-update.
- **Capabilities:** `trash=false`, `atomic_rename=true` (posix-rename@openssh.com
  when offered, otherwise rename + report), `server_side_copy=false` (copy-data
  extension is optional: enable only after probing), `real_directories=true`,
  `posix_permissions=true`, `symlinks=true`, `watch=false`, undo flags true.
- **Writes:** `begin_write` → `.<name>.<pid>.<n>.kara-part` in the same directory,
  `finish` = fsync (extension if present) + rename (no-replace: link/`ssh_rename`
  semantics, never silent overwrite), `abort`/`Drop` = remove. Same invariants as
  `LocalBackend`; the conformance suite and the mutation checklist in
  `remote-backends-testing.md` apply unchanged.
- **Reads:** `open_read(path, from)` with an offset; chunked 256 KiB
  (`TRANSFER_CHUNK`); map `SSH_FX_*` to `BackendErrorKind` (NO_SUCH_FILE → NotFound,
  PERMISSION_DENIED → PermissionDenied, FAILURE on write → NoSpace only when the
  server says so, connection reset / EOF → `Unavailable`).
- **Session health:** keepalive every 30 s; on `Unavailable` the backend returns
  errors without retrying forever and the caller calls `report_failure`.
- **Cancellation:** check the `Cancel` token per chunk and per directory entry.
- **Tests:** `Backend` conformance via `kara_vfs::conformance::run` against a real
  `sshd` (container) gated by `KARA_TEST_SFTP=host:port,user,key` and `#[ignore]`
  otherwise; unit tests for the error mapping and the known_hosts logic with
  fixtures; the registry/prompt flow is already covered with `MemoryFactory`.

## Left: step 6, S3 adapter (`object_store`)

Deferred by decision. Same shape: `S3Factory` (scheme `s3`), params `endpoint`,
`bucket`, `prefix`, `region`; credentials access key id as a param and the secret
key through the `SecretStore`. Capabilities: `server_side_copy=true`,
`real_directories=false`, `atomic_rename=false` (copy + delete),
`posix_permissions=false`, `symlinks=false`, undo of move/rename false. «Nueva
carpeta» writes a `key/` marker. Listing: `delimiter=/`, paginated, streamed.
Upload: multipart above a threshold, aborted on cancel/drop (maps to `WriteSession`).
Conformance against MinIO (`KARA_TEST_S3`), and measure throughput before trusting it
(concurrent ranged GETs / multipart parts are the knobs).

## Left: needs a real environment

- **Keyring smoke test** (needs a session bus and an unlocked keyring):
  build with `--features keyring`; call `KeyringSecretStore::open()`, `set`, `get`,
  `delete` for a throwaway `DriveId`; check `secret-tool search application kara`.
  The code compiles and is clippy-clean but has never talked to a keyring.
- **SFTP and S3 conformance** as above.
- **Qt build** of all UI work; `qmllint` per `CLAUDE.md`; run `scripts/kara-e2e` once
  and treat a red run as unverified (see CLAUDE.md).
- fsync durability and the no-clobber fallback on NFS/FUSE: not testable in a
  container (see `remote-backends-testing.md`).

## Owner decisions still open

1. Retire or re-pin the two obsolete guards `cb_30` / `cb_48` (`known-test-failures.md` §3).
2. Keep the old mode and owner when a file is replaced (contradicts pinned `cb_17`).
3. Symlink size and `FileEntry.location` under `with_root` (pinned `cb_07`).
4. Remote copies do not keep mtime/permissions: add a `set_times`/`set_mode` call to
   `Backend` (additive, capability-gated) or accept it.
5. Remote rename on drives without atomic rename is one blocking call with no
   progress; accept or add chunked progress.
6. A move between two different backends is never undoable; accept or record the
   reverse transfer.
