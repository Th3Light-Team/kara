# Remote-drive fixtures for end-to-end tests

`scripts/kara-remote-fixtures` starts throwaway local servers so the SFTP, S3 and
GCS adapters of `kara-remote` can be tested against real wire protocols without
cloud accounts: the gated `cargo test` suites, and the UI e2e (`scripts/kara-e2e`).

```bash
scripts/kara-remote-fixtures up        # ~10 s the first time (image pull, moto install)
scripts/kara-remote-fixtures status
scripts/kara-remote-fixtures run cargo test -p kara-remote \
    --test sftp_real_server --test objstore_real_service -- --ignored --test-threads=1
eval "$(scripts/kara-remote-fixtures env)"   # or just export the variables yourself
scripts/kara-remote-fixtures down      # stops everything, deletes the state dir
```

## What each fixture is

| Drive | Server | Mimics | Endpoint |
|---|---|---|---|
| SFTP | `lscr.io/linuxserver/openssh-server` in docker (real OpenSSH `sshd`) | a Linux box with key login | `127.0.0.1:22222`, user `kara`, ed25519 key generated per run |
| S3 | `moto_server` (`uv tool run --from 'moto[server]'`), in memory | an S3-compatible service (MinIO/AWS style, path-style URLs) | `http://127.0.0.1:9000`, bucket `kara-test`, key `kara` / `karakara123` |
| GCS | the same moto, behind `scripts/kara-fixture-proxy` | GCS's XML API | `http://127.0.0.1:4443`, bucket `kara-test-gcs`, no-auth key file |

Every fixture holds the `scripts/kara-sample` tree (without the `Errors/`
permission traps, which mean nothing remotely) under `sample/`: for SFTP in the
login's home (`~/sample`), for S3 and GCS under the key prefix `sample/`.

Credentials are throwaway and loopback-only. Nothing is written to `~/.ssh`,
Kara's config or the keyring. The SFTP `known_hosts` file starts empty, so a
client sees an **unknown host key** (the gated test trusts it for the run only).

## State

`$KARA_FIXTURES_DIR`, default `$XDG_RUNTIME_DIR/kara-remote-fixtures`
(else `/tmp/kara-remote-fixtures-<uid>`):

```
fixtures.env    export lines for the gated tests (what `env` prints)
fixtures.json   drives for the UI e2e (below)
ssh/            id_ed25519(.pub), known_hosts (empty)
sftp/           the container's /config = the login's home, with sample/
gcs/key.json    service-account stand-in {"gcs_base_url", "disable_oauth": true, ...}
s3/ seed/       moto pid and log; the seeded tree
```

Knobs (environment): `KARA_FIX_SFTP_PORT`, `KARA_FIX_S3_PORT`, `KARA_FIX_GCS_PORT`,
`KARA_FIX_ONLY="sftp s3 gcs"` (a subset; gcs needs the s3 server and starts it),
`KARA_FIX_S3_BACKEND=minio` plus `KARA_FIX_MINIO_IMAGE=<pullable image>` to use
MinIO instead of moto (the public `minio/minio` image is gone from Docker Hub
and quay.io answered 401 on 2026-10-10, hence moto as the default).

## For the UI e2e

Run `up` before `scripts/kara-e2e`, read `$KARA_FIXTURES_JSON` (also exported by
`env`), register each entry, `down` afterwards. The file looks like:

```json
{ "state_dir": "...",
  "drives": [
    { "kind": "sftp", "id": "fixture-sftp", "name": "Fixture SFTP",
      "params": { "host": "127.0.0.1", "port": "22222", "user": "kara",
                  "key_file": ".../ssh/id_ed25519", "known_hosts": ".../ssh/known_hosts",
                  "root": "~/sample" }, "secret": null },
    { "kind": "s3", "id": "fixture-s3", "name": "Fixture S3",
      "params": { "endpoint": "http://127.0.0.1:9000", "allow_http": "true",
                  "bucket": "kara-test", "region": "us-east-1",
                  "access_key_id": "kara", "prefix": "sample" }, "secret": "karakara123" },
    { "kind": "gcs", "id": "fixture-gcs", "name": "Fixture GCS",
      "params": { "endpoint": "http://127.0.0.1:4443", "allow_http": "true",
                  "bucket": "kara-test-gcs", "service_account_file": ".../gcs/key.json",
                  "prefix": "sample" }, "secret": null } ] }
```

`params` are exactly the `DriveConfig` parameters of each factory (all values
strings); `secret` is the value to hand over as the `Secret` (S3 secret key, or
key passphrase / password for SFTP), `null` when none. Only the drives that
started appear. An e2e that must not depend on the host key prompt should answer
`Prompt::TrustHostKey` with trust-once, as `tests/sftp_real_server.rs` does.
The fixture files must not be committed; the e2e should treat a missing
`fixtures.json` as "skip remote checks", not as a failure.

## Limits

- moto is a faithful-enough S3, not MinIO: no real durability, in memory, and
  it enforces no IAM. Throughput numbers (`s3_throughput`) are loopback numbers
  of a Python server, not a network.
- The GCS fixture speaks the XML API that `object_store` uses (S3-like list,
  plain PUT, `x-goog-copy-source`). `fake-gcs-server` was tried and **cannot**
  serve this adapter: it only implements the JSON API and answers
  `400 invalid uploadType` to the XML PUT. The proxy exists because the GCS
  client still sends an `Authorization` header moto cannot parse (500) and,
  without one, moto treats the caller as anonymous (403 on delete); it swaps in
  a well-formed unchecked SigV4 header and renames the copy-source header. So
  OAuth, signed URLs and real GCS error bodies are not exercised.
- SFTP is a real OpenSSH `sshd`, but password and keyboard-interactive login are
  off (key only), and the home is `/config` inside the container. The server has
  no trash and no server-side copy (`capabilities` reflect that).
- Needs `docker` (SFTP), `uv` (moto), `python3`, `curl` (>= 7.75, `--aws-sigv4`),
  `ssh-keygen`/`ssh`. No sudo. `down` also uses a throwaway `busybox` container
  to hand root-owned files created by the sshd image back before deleting.

## Last run (2026-10-10, Ubuntu 26.04)

All gated suites pass: SFTP conformance against real `sshd`; S3 conformance and
throughput (32 MiB: ~140 / ~175 / ~190 MB/s up with 1 / 4 / 8 parts, 1+ GB/s
down); GCS conformance.
