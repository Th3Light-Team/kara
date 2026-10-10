//! Gated tests against real services, `#[ignore]`d by default: the
//! conformance suite inside a unique prefix of a real S3-compatible bucket
//! (MinIO, AWS, R2...) or a GCS bucket (or fake-gcs-server), and a
//! throughput measurement.
//!
//! ```sh
//! # MinIO in docker:
//! docker run --rm -d -p 9000:9000 -e MINIO_ROOT_USER=kara -e MINIO_ROOT_PASSWORD=karakara123 \
//!     minio/minio server /data
//! docker run --rm --network host --entrypoint sh minio/mc -c \
//!     "mc alias set l http://127.0.0.1:9000 kara karakara123 && mc mb l/kara-test"
//! KARA_TEST_S3="http://127.0.0.1:9000,kara-test,kara,karakara123" \
//!     cargo test -p kara-remote --test objstore_real_service -- --ignored --nocapture
//!
//! # GCS (a service-account key with storage.objectAdmin on the bucket):
//! KARA_TEST_GCS="my-bucket,/path/to/key.json" cargo test -p kara-remote \
//!     --test objstore_real_service gcs -- --ignored
//! # fake-gcs-server: add KARA_TEST_GCS_ENDPOINT=http://127.0.0.1:4443 and use a
//! # key file {"gcs_base_url": "http://127.0.0.1:4443", "disable_oauth": true,
//! #   "client_email": "", "private_key": "", "private_key_id": ""}
//! ```
//!
//! Optional: `KARA_TEST_S3_REGION` (default `us-east-1`),
//! `KARA_TEST_S3_MB` (size of the throughput object, default 256).
//! Nothing is left in the bucket: every run works under
//! `kara-test-<pid>-<time>/` and removes it.

mod objstore_support;
mod s3_mock_support;

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use kara_remote::objstore::{GcsFactory, ObjectStoreBackend, S3Factory};
use kara_remote::{DriveConfig, Secret};
use kara_vfs::conformance::{self, CaseOutcome};
use kara_vfs::{Backend, Cancel, RemotePath};
use objstore_support::rp;

fn unique_prefix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("kara-test-{}-{nanos}", std::process::id())
}

/// `endpoint,bucket,access_key_id,secret` from `KARA_TEST_S3`.
fn s3_from_env(extra: &[(&str, &str)]) -> io::Result<Option<(DriveConfig, Secret)>> {
    let Ok(spec) = std::env::var("KARA_TEST_S3") else {
        return Ok(None);
    };
    let parts: Vec<&str> = spec.splitn(4, ',').collect();
    let [endpoint, bucket, key_id, secret] = parts.as_slice() else {
        return Err(io::Error::other("KARA_TEST_S3 must be endpoint,bucket,access_key_id,secret"));
    };
    let region = std::env::var("KARA_TEST_S3_REGION").unwrap_or_else(|_| String::from("us-east-1"));
    let mut params: Vec<(String, String)> = vec![
        (String::from("bucket"), (*bucket).to_owned()),
        (String::from("access_key_id"), (*key_id).to_owned()),
        (String::from("region"), region),
        (String::from("prefix"), unique_prefix()),
    ];
    if !endpoint.is_empty() {
        params.push((String::from("endpoint"), (*endpoint).to_owned()));
        if endpoint.starts_with("http://") {
            params.push((String::from("allow_http"), String::from("true")));
        }
    }
    for (key, value) in extra {
        params.push(((*key).to_owned(), (*value).to_owned()));
    }
    let config = DriveConfig::new("s3", "real", "Real S3", params).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(Some((config, Secret::new(*secret))))
}

fn run_suite(backend: &dyn Backend) -> io::Result<()> {
    let scratch = rp("/scratch")?;
    backend.create_dir(&scratch).map_err(io::Error::other)?;
    let mut failures = Vec::new();
    for report in [
        conformance::run(backend, &scratch).map_err(|e| io::Error::other(e.to_string()))?,
        conformance::run_extra(backend, &scratch).map_err(|e| io::Error::other(e.to_string()))?,
    ] {
        for case in report.cases {
            if let CaseOutcome::Failed { detail } = case.outcome {
                failures.push(format!("{}: {detail}", case.id));
            }
        }
    }
    backend.remove_tree(&scratch, &Cancel::new()).map_err(io::Error::other)?;
    let left = backend.list(&RemotePath::root(), &Cancel::new()).map_err(io::Error::other)?;
    assert!(left.entries.is_empty(), "left behind: {:?}", left.entries);
    assert!(failures.is_empty(), "conformance failures:\n{}", failures.join("\n"));
    Ok(())
}

#[test]
#[ignore = "needs KARA_TEST_S3=endpoint,bucket,access_key_id,secret"]
fn s3_conformance_against_a_real_service() -> io::Result<()> {
    let Some((config, secret)) = s3_from_env(&[])? else {
        eprintln!("KARA_TEST_S3 is not set: nothing to do");
        return Ok(());
    };
    let backend = S3Factory::new()
        .open(&config, Some(&secret), &Cancel::new())
        .map_err(io::Error::other)?;
    run_suite(backend.as_ref())
}

#[test]
#[ignore = "needs KARA_TEST_GCS=bucket,service_account_file"]
fn gcs_conformance_against_a_real_service() -> io::Result<()> {
    let Ok(spec) = std::env::var("KARA_TEST_GCS") else {
        eprintln!("KARA_TEST_GCS is not set: nothing to do");
        return Ok(());
    };
    let Some((bucket, key_file)) = spec.split_once(',') else {
        return Err(io::Error::other("KARA_TEST_GCS must be bucket,service_account_file"));
    };
    let mut params = vec![
        (String::from("bucket"), bucket.to_owned()),
        (String::from("service_account_file"), key_file.to_owned()),
        (String::from("prefix"), unique_prefix()),
    ];
    if let Ok(endpoint) = std::env::var("KARA_TEST_GCS_ENDPOINT") {
        if endpoint.starts_with("http://") {
            params.push((String::from("allow_http"), String::from("true")));
        }
        params.push((String::from("endpoint"), endpoint));
    }
    let config = DriveConfig::new("gcs", "real", "Real GCS", params).map_err(|e| io::Error::other(e.to_string()))?;
    let backend = GcsFactory::new().open(&config, &Cancel::new()).map_err(io::Error::other)?;
    run_suite(backend.as_ref())
}

// ---------------------------------------------------------------------------
// Throughput.

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(131) >> 3) as u8).collect()
}

/// Uploads and downloads one object of `megabytes`, returns (up, down) MB/s.
fn measure(backend: &ObjectStoreBackend, megabytes: usize, name: &str) -> io::Result<(f64, f64)> {
    let path = rp(&format!("/{name}"))?;
    let chunk = pattern(1024 * 1024);
    let started = Instant::now();
    let mut session = backend.begin_write(&path, None, false).map_err(io::Error::other)?;
    for _ in 0..megabytes {
        session.write_all(&chunk)?;
    }
    session.finish().map_err(io::Error::other)?;
    let up = megabytes as f64 / started.elapsed().as_secs_f64();

    let started = Instant::now();
    let mut reader = backend.open_read(&path, 0).map_err(io::Error::other)?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut total = 0usize;
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total += n;
    }
    let down = megabytes as f64 / started.elapsed().as_secs_f64();
    assert_eq!(total, megabytes * 1024 * 1024);
    backend.remove(&path).map_err(io::Error::other)?;
    Ok((up, down))
}

fn megabytes() -> usize {
    std::env::var("KARA_TEST_S3_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256)
}

#[test]
#[ignore = "needs KARA_TEST_S3; prints MB/s for 1, 4 and 8 parts in flight"]
fn s3_throughput() -> io::Result<()> {
    let size = megabytes();
    for concurrency in ["1", "4", "8"] {
        let Some((config, secret)) = s3_from_env(&[("upload_concurrency", concurrency), ("part_size_mb", "8")])? else {
            eprintln!("KARA_TEST_S3 is not set: nothing to do");
            return Ok(());
        };
        let backend = S3Factory::new()
            .open(&config, Some(&secret), &Cancel::new())
            .map_err(io::Error::other)?;
        let (up, down) = measure(&backend, size, "throughput.bin")?;
        eprintln!("{size} MiB, {concurrency} part(s) in flight: upload {up:.1} MB/s, download {down:.1} MB/s");
    }
    Ok(())
}

#[test]
#[ignore = "loopback numbers against the in-process mock: checks the benchmark, says nothing about a real link"]
fn throughput_harness_against_the_local_mock() -> io::Result<()> {
    let mock = s3_mock_support::S3Mock::start()?;
    let size = megabytes().min(64);
    for concurrency in ["1", "4", "8"] {
        let config = DriveConfig::new(
            "s3",
            "mock",
            "Mock",
            [
                ("bucket", s3_mock_support::BUCKET),
                ("endpoint", mock.endpoint().as_str()),
                ("allow_http", "true"),
                ("access_key_id", s3_mock_support::ACCESS_KEY),
                ("upload_concurrency", concurrency),
                ("part_size_mb", "8"),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned())),
        )
        .map_err(|e| io::Error::other(e.to_string()))?;
        let backend: Arc<ObjectStoreBackend> = S3Factory::new()
            .open(&config, Some(&Secret::new(s3_mock_support::SECRET_KEY)), &Cancel::new())
            .map_err(io::Error::other)?;
        let (up, down) = measure(&backend, size, "throughput.bin")?;
        eprintln!("mock, {size} MiB, {concurrency} part(s) in flight: upload {up:.1} MB/s, download {down:.1} MB/s");
    }
    Ok(())
}
