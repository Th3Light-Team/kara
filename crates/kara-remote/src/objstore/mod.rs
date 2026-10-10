//! Object-storage drives: [`ObjectStoreBackend`] implements
//! [`kara_vfs::Backend`] over any `object_store::ObjectStore` (Apache Arrow's
//! crate), and two factories put real services behind it:
//!
//! - [`S3Factory`] (feature `s3`, scheme `s3`): Amazon S3 and S3-compatible
//!   services — MinIO, Cloudflare R2, Backblaze B2, Ceph RGW;
//! - [`GcsFactory`] (feature `gcs`, scheme `gcs`): Google Cloud Storage.
//!
//! Both features enable the shared internal feature `objectstore`, which on
//! its own gives the generic backend (tests run it over
//! `object_store::memory::InMemory`). The trait stays blocking: each connected
//! drive owns a private tokio runtime ([`runtime`]); nothing async leaks out.
//!
//! # Registering them (kara-ui)
//!
//! ```ignore
//! registry.register_factory(kara_remote::objstore::S3Factory::new());
//! registry.register_factory(kara_remote::objstore::GcsFactory::new());
//! ```
//!
//! Build `kara-ui` with `kara-remote/s3` and/or `kara-remote/gcs`. The registry
//! asks for the S3 secret access key itself (`ConnectError::AuthRequired`,
//! then `AuthFailed` on a wrong one); GCS drives never ask. On any
//! `Unavailable` from a listing or a job, call `DriveRegistry::report_failure`.
//!
//! # `s3` parameters
//!
//! | Key | Default | Meaning |
//! |---|---|---|
//! | `bucket` | required | |
//! | `endpoint` | AWS | `https://…` of MinIO, R2, B2, Ceph… `http://` only with `allow_http=true` |
//! | `allow_http` | `false` | Accept a plain-http endpoint (unencrypted: credentials and data travel in clear). Refused when the endpoint is not `http://` |
//! | `region` | `us-east-1` | Must match the bucket's on AWS |
//! | `prefix` | none | Key prefix the drive's root stands for |
//! | `access_key_id` | required with `credentials=key` | |
//! | `credentials` | `key` | `key` (id + secret), `env` (ambient AWS chain: `AWS_*` variables, web identity, ECS, EC2 metadata; **no SSO, no profiles**), `anonymous` (public buckets) |
//! | `path_style` | `true` with an endpoint, `false` on AWS | `endpoint/bucket/key` URLs |
//! | `conditional_put` | `etag` | `object_store`'s `aws_conditional_put`; `disabled` for servers that reject `If-None-Match` |
//! | `copy_if_not_exists` | none (check, then copy) | `object_store`'s `aws_copy_if_not_exists`, e.g. `multipart`, or `header: cf-copy-destination-if-none-match: *` on R2 |
//! | `timeout_s` | `30` | Per request; a dead endpoint answers in about a second (3 quick retries) |
//! | `part_size_mb` | `8` | Upload part size, 5–512 |
//! | `upload_concurrency` | `4` | Parts in flight per upload, 1–32 |
//!
//! The secret ([`Secret`](crate::Secret)) is the **secret access key**,
//! optionally followed by a line break and a session token (temporary STS
//! credentials). `secret_access_key` and `session_token` are deliberately not
//! parameters: `DriveConfig` refuses secret-looking keys.
//!
//! # `gcs` parameters
//!
//! | Key | Default | Meaning |
//! |---|---|---|
//! | `bucket` | required | |
//! | `prefix` | none | |
//! | `service_account_file` | — | Path of a service-account JSON key (`~/` expanded). Only the path is stored |
//! | `credentials` | `file` | `adc`: application default credentials instead of a file |
//! | `endpoint`, `allow_http` | Google | Another JSON API base URL (an emulator); same `http://` rule as S3 |
//! | `timeout_s`, `part_size_mb`, `upload_concurrency` | as S3 | |
//!
//! # Semantics
//!
//! See [`backend`] and [`keys`]: folders are prefixes, an empty folder is a
//! hidden `.kara-dir` placeholder object, rename is copy + delete (never
//! overwrites, sources deleted only after every copy exists), copy inside the
//! drive is server side (streamed through the client above 5 GiB on S3),
//! uploads are multipart above one part and aborted on cancel. Capabilities:
//! [`ObjectStoreBackend::CAPABILITIES`], fixed.
//!
//! # Not supported
//!
//! - listing buckets (a drive is one bucket);
//! - the storage class (`object_store` does not report it); the ETag and the
//!   version go into `FileEntry::extra` as `object.etag` / `object.version`;
//! - keys `object_store` cannot parse (empty segments, `.`/`..` segments,
//!   control characters) make the listing of their folder fail as a whole;
//! - the AWS CLI's profiles and SSO (use `credentials=env` with exported
//!   variables, or a key).

mod backend;
#[cfg(any(feature = "s3", feature = "gcs"))]
mod connect;
mod error;
mod io;
pub mod keys;
mod runtime;

#[cfg(feature = "gcs")]
mod gcs;
#[cfg(feature = "s3")]
mod s3;

pub use backend::{
    DEFAULT_PAGE_SIZE, DEFAULT_PART_SIZE, DEFAULT_TIMEOUT_S, DEFAULT_UPLOAD_CONCURRENCY,
    MIN_PART_SIZE, ObjectStoreBackend, ObjectStoreOptions,
};
#[cfg(any(feature = "s3", feature = "gcs"))]
pub use connect::Tuning;
pub use keys::PLACEHOLDER;
/// The crate underneath, so callers can name its types without a version skew.
pub use object_store;

#[cfg(feature = "gcs")]
pub use gcs::{GcsConnector, GcsCredentials, GcsFactory, GcsParams};
#[cfg(feature = "s3")]
pub use s3::{S3_MAX_SERVER_COPY, S3Connector, S3Credentials, S3Factory, S3Params};
