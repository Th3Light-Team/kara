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
| 4 | `kara-remote` | `DriveConfig` <-> `settings.conf`, `SecretStore` (+ `oo7` keyring), `DriveRegistry`, `BackendFactory`, prompts |

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
- States: `Disconnected → Connecting → Ready | Failed{reason}`, `Ready → Lost{reason}`.
- Prompts (`Prompt`): `Password`, `TrustHostKey`, `HostKeyChanged`. The registry
  asks for the password itself (up to 3 attempts; the secret is stored only after
  a connection with it worked and only if the user asked). Host-key prompts are
  asked by the factory through the same handler. The backend never draws a dialog.
- `--features memory` adds drive kind `mem` (`MemoryFactory`) for tests and demos.

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
