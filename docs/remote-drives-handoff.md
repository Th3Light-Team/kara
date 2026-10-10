# Remote drives: what is built, what is left

Status after steps 1–6 (Rust side). Everything below the UI is done and tested in
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
| 6 | `kara-remote` (`--features s3`, `gcs`) | `objstore::ObjectStoreBackend` over any `object_store` 0.14 store, `S3Factory` (S3, MinIO, R2, B2, Ceph) and `GcsFactory`, fault-injecting store, hermetic S3 server that checks SigV4 |

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
- **No MinIO, no GCS, no network to a bucket**: the object-store adapter is
  tested over `object_store::memory::InMemory` (through a fault-injecting
  wrapper, `tests/objstore_support/`) and, for the real AWS client, against an
  in-process S3 server (`tests/s3_mock_support/`) that checks SigV4 the way AWS
  documents it. The GCS client (`object_store::gcp`) is built and its
  parameter/credential handling tested, but **no GCS request was ever made**.
- **Disk.** The container's disk filled up at 27 GB of `target/debug` (each
  `kara-remote` test binary is ~150 MB with full debug info). This step was
  built with `CARGO_PROFILE_DEV_DEBUG=line-tables-only` and an emptied
  `target/debug`; with full debug info the object-store test binaries add a
  few GB more.
- **`object_store` brings `reqwest` + `rustls`** and reuses `aws-lc-rs` (already
  there for `russh`). The dev self-dependency enables `s3` and `gcs`, so a plain
  `cargo test -p kara-remote` builds both clients.

## Left: step 4, UI side (needs Qt)

Wiring in `kara-ui` (nothing of this is business logic; it lives in Rust, QML only binds):

1. **Startup.** `Prefs`/settings: `config::load_all(&settings)` → `registry.add_all`.
   Report the returned problems once, like `Prefs` reports an unreadable file.
   Build the `SecretStore`: try `KeyringSecretStore::open()`, fall back to
   `MemorySecretStore` and show «Las contraseñas no se guardarán» when
   `secrets_are_persistent()` is false. Register the factories (SFTP, S3, GCS).
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

## Step 6, object storage: done (`kara-remote/src/objstore/`, features `s3`, `gcs`)

Module docs (`src/objstore/mod.rs`) list both factories' parameters; `backend.rs`,
`io.rs` and `keys.rs` say how each operation maps onto object requests. One
generic `ObjectStoreBackend` (`Arc<dyn object_store::ObjectStore>` + a key
prefix; internal feature `objectstore`) and two factories:

| Scheme | Factory | Service |
|---|---|---|
| `s3` | `objstore::S3Factory` (feature `s3`) | Amazon S3 and S3-compatible: MinIO, Cloudflare R2, Backblaze B2, Ceph RGW |
| `gcs` | `objstore::GcsFactory` (feature `gcs`) | Google Cloud Storage |

**Registering them in kara-ui:** build with `kara-remote/s3` and/or
`kara-remote/gcs`, then at startup, next to SFTP:

```rust
registry.register_factory(kara_remote::objstore::S3Factory::new());
registry.register_factory(kara_remote::objstore::GcsFactory::new());
```

Nothing else changes: an S3 drive's secret is asked for by the registry
(`AuthRequired`, then `AuthFailed` on a wrong one); a GCS drive never asks.
«Añadir unidad…» shows, for `s3`: bucket, endpoint (empty = AWS), region,
prefix, access key id, `allow_http` (only offered for an `http://` endpoint,
with the warning that credentials and data travel unencrypted), credentials
(key / environment / anonymous) and the secret («Clave secreta»; a session
token goes on a second line); for `gcs`: bucket, prefix, the key file path
(file chooser; the JSON content is never copied into the settings) or
«credenciales por defecto de la aplicación».

### S3 parameters

| Key | Default | Meaning |
|---|---|---|
| `bucket` | required | |
| `endpoint` | AWS | `https://…`; `http://` only with `allow_http=true` |
| `allow_http` | `false` | Refused unless the endpoint is `http://` (and an `http://` endpoint is refused without it) |
| `region` | `us-east-1` | Must match the bucket's on AWS |
| `prefix` | none | Key prefix the drive's root stands for |
| `access_key_id` | required with `credentials=key` | |
| `credentials` | `key` | `env`: the ambient AWS chain (`AWS_*` variables, web identity, ECS, EC2 metadata); **SSO and `~/.aws` profiles are not supported**. `anonymous`: public buckets |
| `path_style` | `true` with an endpoint | |
| `conditional_put` | `etag` | `disabled` for servers that reject `If-None-Match` |
| `copy_if_not_exists` | none | e.g. `multipart`, or R2's `header: cf-copy-destination-if-none-match: *` |
| `timeout_s` / `part_size_mb` / `upload_concurrency` | 30 / 8 / 4 | |

Secret: the secret access key, optionally `\n` + a session token. Neither is a
parameter (`DriveConfig` refuses secret-looking keys).

### GCS parameters

`bucket` (required), `prefix`, `service_account_file` (path, `~/` expanded) **or**
`credentials=adc` (`GOOGLE_APPLICATION_CREDENTIALS`, then gcloud's
`application_default_credentials.json`, then the GCE metadata server), `endpoint`
+ `allow_http` (emulators), `timeout_s`, `part_size_mb`, `upload_concurrency`.

### Decisions taken while building it

- **Crate:** `object_store` 0.14.2 (Apache Arrow) for both services; private
  tokio runtime per drive (2 workers, so upload parts move while the caller
  writes). A call from inside an async runtime is refused with `Other`.
- **Empty folders:** a hidden empty object `<folder>/.kara-dir`. Hidden from every
  listing, `stat`/`open_read` say `NotFound`, removed with its folder, and the
  name is **reserved** (writing, creating or renaming onto it is `Other`).
  `object_store` cannot write the S3 console's `folder/` markers (its paths
  never end in `/`); one written by another tool makes its folder exist, is
  never listed, but cannot be deleted through `object_store` (`remove` of such
  a folder answers `Unsupported`). Other tools see `.kara-dir` as a 0-byte file.
- **Object and folder with one name** (written by another tool): the object
  wins (`stat` is the file, `list` of it is `Other`), the listing of the parent
  reports the hidden folder as a per-entry error. Same rule as `MemoryBackend`.
- **No-overwrite:** small files are one `PUT`: a `HEAD` check, then
  `PutMode::Create` (`If-None-Match: *` / `ifGenerationMatch=0`); a store that
  refuses the condition falls back to the check alone. Multipart uploads are
  completed unconditionally by `object_store`, so the target is checked right
  before `complete` — **a small race window** for files above one part.
  Server-side copies use `CopyMode::Create` where configured, else
  check-then-copy (same window). S3 itself has no copy-if-not-exists by default.
- **Rename** (`atomic_rename=false`): copy every object (no-clobber, 32 at a
  time), delete the sources only after all copies exist, files before
  placeholders. A failed copy takes back the copies already made (every object
  keeps its old name only); if that clean-up fails, or a delete of the sources
  fails, objects exist under both names — never under neither — and the error
  is reported.
- **Copies above 5 GiB** (`CopyObject`'s limit on S3) are streamed through the
  client by `copy_within` and `rename`; GCS has no limit set.
- **`list`/`stat` times** are cut to whole seconds (S3's `HEAD` has second
  precision, its listing millisecond). Folders have no time. The ETag and the
  version go into `extra` (`object.etag`, `object.version`); the storage class is
  not available (`object_store` drops it).
- **Timeouts:** no whole-request timeout (`object_store`'s default 30 s also
  bounds the response body and would cut every large download); a connection
  silent for `timeout_s` fails, connecting takes at most 10 s, 3 retries within
  `timeout_s`. Metadata calls are also bounded by 4×`timeout_s` + 5 s; an
  upload part, a single `PUT` and the completion by that plus the time the
  bytes in flight need at 16 KiB/s (`MIN_UPLOAD_RATE`: ~35 min for 4 × 8 MiB).
  A refused connection answers `Unavailable` in about a second; a silent
  endpoint in about `timeout_s`.
- **Requests per operation:** `begin_write` makes three requests at once (the
  target, a folder of that name, every ancestor), a small file's `finish` two
  (check, conditional `PUT`), `stat` of a folder two, `list` one per page plus a
  `HEAD`. Copying 10 000 small files to S3 is about 50 000 requests.
- **Errors:** listing and bulk delete wrap HTTP failures in `Generic` with the
  status only in the text (found by the S3 mock): 401/403 there map to
  `PermissionDenied`, 5xx/connection failures to `Unavailable`, 507 and quota
  messages to `NoSpace`. At connect time S3's code decides:
  `SignatureDoesNotMatch`/`InvalidAccessKeyId` → `AuthFailed` (prompt again),
  `AccessDenied` → a plain error, `NoSuchBucket` → «the bucket does not exist».
  The object key is replaced by the caller's path in the text of the cause; a
  percent-encoded key inside a URL is **not** (it is the user's own bucket path,
  no secret).
- **Capabilities:** fixed, `MemoryBackend::object_store_like()`'s:
  `server_side_copy` only; no trash, no atomic rename, no real directories, no
  permissions, no links, no watch, no undo of rename/move.

Tests (`cargo test -p kara-remote`, all hermetic): see
`remote-backends-testing.md` § «Object-store adapter (step 6)».

### Left for S3 and GCS

- **Never run against a real service.** MinIO/AWS:
  `KARA_TEST_S3="endpoint,bucket,access_key_id,secret" cargo test -p kara-remote
  --test objstore_real_service -- --ignored --nocapture` (MinIO docker one-liner
  in that file). GCS: `KARA_TEST_GCS="bucket,/path/key.json"` (fake-gcs-server:
  add `KARA_TEST_GCS_ENDPOINT`). Expect surprises around `If-None-Match` support
  (B2 and older Ceph/MinIO may reject or ignore it: `conditional_put=disabled`),
  region redirects on AWS, and R2's copy header.
- **Throughput not measured.** `s3_throughput` (same file, `--ignored`) prints
  upload/download MB/s for a 256 MiB object with 1/4/8 parts in flight; the only
  numbers taken are loopback ones against the in-process mock, which measure the
  mock and SHA-256 signing, not a link.
- The S3 mock decodes `+` in query strings as a space, as `object_store` signs
  it; that AWS does the same is assumed, not verified.
- Listing buckets (a drive with no bucket), storage classes, versioning, SSE
  options, the AWS CLI's profiles/SSO.
- A rename of a large folder is one blocking call with no progress (owner
  decision 5 applies here too, and more: it is one request per object).

## Left: needs a real environment

- **Keyring smoke test** (needs a session bus and an unlocked keyring):
  build with `--features keyring`; call `KeyringSecretStore::open()`, `set`, `get`,
  `delete` for a throwaway `DriveId`; check `secret-tool search application kara`.
  The code compiles and is clippy-clean but has never talked to a keyring.
- **SFTP conformance against a real sshd** (`KARA_TEST_SFTP`, above), S3 against
  MinIO/AWS (`KARA_TEST_S3`) and GCS (`KARA_TEST_GCS`), and the S3 throughput
  measurement.
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
7. Object-store folders: the `.kara-dir` placeholder name (other tools see it),
   and «the object wins» when an object and a folder share a name.
8. `replace=false` above one upload part is check-then-complete (a small race),
   because `object_store` completes multipart uploads unconditionally. Accept,
   or upload to a temporary key and `copy_if_not_exists` (doubles the work and
   needs per-service configuration).
