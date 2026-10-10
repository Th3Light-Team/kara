# Remote drives: what is built, what is left

Status after steps 1–5 (Rust side). Everything below the UI is done and tested in
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
| 5 | `kara-remote` (`--features sftp`) | `sftp::SftpFactory` / `SftpBackend` on `russh` 0.64 + `russh-sftp` 3.0, `known_hosts` handling, hermetic test server |

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
| Bulk trust of host keys | Deliberately absent. Trusting 50 unknown keys in one click defeats the check. Proposal: one prompt per drive, plus an explicit «trust all keys in this import» that lists the fingerprints and is off by default. Still open after step 5: SFTP asks once per drive (and «trust once» lasts for the run). |
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
- **No `ssh`/`sshd` binaries** and an empty apt index: the SFTP adapter is tested
  against an in-process `russh` + `russh-sftp` server on `127.0.0.1` over a
  tempdir (`crates/kara-remote/tests/support/`), written to answer like
  OpenSSH's `sftp-server`; the real `sshd` check is ignored by default.
- **`russh` pulls `aws-lc-rs`** (its default crypto backend; needs cmake and a C
  compiler, both present). Behind `--features sftp` only.
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

## Step 5, SFTP adapter: done (`kara-remote/src/sftp/`, feature `sftp`)

Module docs (`src/sftp/mod.rs`) list the parameters, the auth order, the host-key
rules and how `kara-ui` registers it (`registry.register_factory(SftpFactory::new())`,
build with `kara-remote/sftp`). Decisions taken while building it:

- **Runtime:** one private tokio multi-thread runtime (1 worker) per connected
  drive; the trait stays blocking. A call from inside an async runtime is refused
  with `Other` instead of panicking.
- **Params:** `host`, `port`, `user` (default `$USER`), `key_file`, `known_hosts`,
  `root` (absolute, or `~`/relative resolved by the server; default the server's
  `/`), `agent` (`$SSH_AUTH_SOCK` by default, `none`, or a socket path),
  `timeout_s` (30, per request; connect gets twice that), `keepalive_s` (30).
- **Auth:** agent → key file → password. With `key_file` the secret is the
  passphrase, there is no password fallback, and only the agent identity of that
  key is offered (a crowded agent cannot exhaust `MaxAuthTries`).
  Keyboard-interactive is **not** implemented (servers with
  `PasswordAuthentication no` + PAM keyboard-interactive will answer `AuthFailed`).
- **Host keys:** an untrusted key ends the handshake; the factory asks on the
  connecting thread and connects again accepting exactly that key (no blocking
  inside the runtime). «Trust once» lasts for the life of the factory (so the
  registry's password retry does not ask about the key again). A changed key is
  never written, even on `Trust { remember: true }`. New lines are plain
  (`[host]:port type key`), not hashed. A failed append still connects.
- **Bulk trust of host keys:** still **not built**, deliberately. One prompt per
  drive; the «trust all keys in this import» proposal stays open for the owner.
- **Capabilities:** probed once (`posix-rename@openssh.com`, `fsync@openssh.com`);
  `atomic_rename`/`undo_*` follow posix-rename.
- **Writes:** the target directory is resolved once with `realpath` at
  `begin_write` (as `LocalBackend` opens its directory fd); no-replace commit is
  plain `SSH_FXP_RENAME` after an `lstat` check (OpenSSH links + unlinks, so it is
  race-free there; on servers whose rename replaces, the check closes all but a
  tiny race). Replace without posix-rename is remove + rename, **not atomic**.
- **Errors:** OpenSSH folds `ENOTDIR`/`ELOOP` into `NO_SUCH_FILE` and most errnos
  into «Failure»; the backend looks (`lstat`/`stat`) after a failure to give the
  same kinds as `LocalBackend` (differential test). `NoSpace` only when the
  server's message says so (OpenSSH never does: a full disk is `Other` there).
- **Throughput:** 32 KiB requests, 16 in flight per reader/writer. Not measured
  against a real link.

Tests (all hermetic, `cargo test -p kara-remote`): see
`remote-backends-testing.md` § «SFTP adapter (step 5)».

### Left for SFTP

- **Real-sshd run:** `KARA_TEST_SFTP="host:port,user,keyfile" cargo test -p
  kara-remote --test sftp_real_server -- --ignored` (password/passphrase in
  `KARA_TEST_SFTP_PASSWORD`, known_hosts in `KARA_TEST_SFTP_KNOWN_HOSTS`). Never
  run: no sshd and no network here. Expect possible differences in error kinds
  where OpenSSH's realpath or rename differs from the test server.
- **Proxmox:** not tried. Things to check on a real node/container: Debian's
  `sftp-server` path (`Subsystem sftp /usr/lib/openssh/sftp-server`),
  `PermitRootLogin` for root with a password, and that unprivileged containers
  show shifted uids (`posix.uid` in `FileEntry::extra`).
- Keyboard-interactive auth; `copy-data` (server-side copy) probing; `limits@openssh.com`
  for bigger requests; preserving mtime/permissions (owner decision 4).
- `russh-sftp` decodes names and handles as (lossy) UTF-8: non-UTF-8 names are
  listed with a per-entry error and cannot be opened; servers whose handles are
  not UTF-8 (OpenSSH's are 4 binary bytes, fine below 128 open handles) could
  misbehave.
- With a dead connection a temporary cannot be removed: a hidden
  `.name.<pid>.<n>.kara-part` stays on the server.

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
- **SFTP conformance against a real sshd** (`KARA_TEST_SFTP`, above) and S3 against MinIO.
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
