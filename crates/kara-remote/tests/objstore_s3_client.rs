//! The real S3 client (`S3Factory` → `object_store::aws`) against the
//! hermetic mock in `tests/s3_mock_support`: SigV4 checked on every request,
//! keys with awkward characters, XML listing with continuation tokens,
//! multipart, `If-None-Match`, server-side copy, bulk delete, retries.

mod s3_mock_support;
mod objstore_support;

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use kara_remote::objstore::{ObjectStoreBackend, S3Factory};
use kara_remote::{
    ConnectError, ConnectionState, DriveConfig, DriveRegistry, MemorySecretStore, PromptAnswer,
    Remember, Secret,
};
use kara_vfs::conformance::{self, CaseOutcome};
use kara_vfs::{Backend, BackendErrorKind, Cancel, RemotePath};
use s3_mock_support::{ACCESS_KEY, BUCKET, S3Mock, SECRET_KEY};
use objstore_support::{Scripted, rp};

fn config(mock: &S3Mock, extra: &[(&str, &str)]) -> io::Result<DriveConfig> {
    let endpoint = mock.endpoint();
    let mut params: Vec<(String, String)> = vec![
        (String::from("bucket"), String::from(BUCKET)),
        (String::from("endpoint"), endpoint),
        (String::from("allow_http"), String::from("true")),
        (String::from("access_key_id"), String::from(ACCESS_KEY)),
        (String::from("timeout_s"), String::from("5")),
    ];
    for (key, value) in extra {
        params.retain(|(k, _)| k != key);
        params.push(((*key).to_owned(), (*value).to_owned()));
    }
    DriveConfig::new("s3", "mock", "Mock", params).map_err(|e| io::Error::other(e.to_string()))
}

fn connect(mock: &S3Mock, extra: &[(&str, &str)]) -> io::Result<Arc<ObjectStoreBackend>> {
    S3Factory::new()
        .open(&config(mock, extra)?, Some(&Secret::new(SECRET_KEY)), &Cancel::new())
        .map_err(io::Error::other)
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(131) >> 3) as u8).collect()
}

fn put(backend: &dyn Backend, path: &RemotePath, data: &[u8]) -> io::Result<()> {
    let mut session = backend.begin_write(path, None, false).map_err(io::Error::other)?;
    session.write_all(data)?;
    session.finish().map_err(io::Error::other)
}

fn get(backend: &dyn Backend, path: &RemotePath, from: u64) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    backend
        .open_read(path, from)
        .map_err(io::Error::other)?
        .read_to_end(&mut out)?;
    Ok(out)
}

#[test]
fn the_suite_passes_through_the_real_client() -> io::Result<()> {
    let mock = S3Mock::start()?;
    mock.state.page_size.store(2, Ordering::SeqCst);
    let backend = connect(&mock, &[("prefix", "suite")])?;
    let scratch = rp("/scratch")?;
    backend.create_dir(&scratch).map_err(io::Error::other)?;
    let mut failures = Vec::new();
    for report in [
        conformance::run(backend.as_ref(), &scratch).map_err(|e| io::Error::other(e.to_string()))?,
        conformance::run_extra(backend.as_ref(), &scratch).map_err(|e| io::Error::other(e.to_string()))?,
    ] {
        for case in report.cases {
            if let CaseOutcome::Failed { detail } = case.outcome {
                failures.push(format!("{}: {detail}", case.id));
            }
        }
    }
    assert!(failures.is_empty(), "conformance failures:\n{}", failures.join("\n"));
    assert_eq!(mock.state.signature_failures.load(Ordering::SeqCst), 0);
    assert_eq!(mock.state.open_uploads(), 0);
    assert!(mock.state.count("GET list") > 0 && mock.state.count("POST bulk-delete") > 0);
    Ok(())
}

#[test]
fn awkward_names_are_signed_listed_and_read_back() -> io::Result<()> {
    let mock = S3Mock::start()?;
    mock.state.page_size.store(3, Ordering::SeqCst);
    let backend = connect(&mock, &[])?;
    let names = [
        "100%.txt",
        "#hash",
        "why?",
        "[x]",
        "a b",
        "pl+us",
        "eq=ual",
        "and&amp",
        "quo'te",
        "\u{fc}n\u{ef}c\u{f8}d\u{e9}",
        "semi;colon",
        "com,ma",
        "til~de",
        "at@sign",
        "star*",
    ];
    for name in names {
        let path = rp(&format!("/odd/{name}"))?;
        put(backend.as_ref(), &path, name.as_bytes())?;
        assert_eq!(get(backend.as_ref(), &path, 0)?, name.as_bytes(), "{name}");
        assert_eq!(mock.state.bytes(&format!("odd/{name}")), Some(name.as_bytes().to_vec()), "{name}");
    }
    let listing = backend.list(&rp("/odd")?, &Cancel::new()).map_err(io::Error::other)?;
    let mut listed: Vec<String> = listing.entries.into_iter().map(|e| e.display).collect();
    listed.sort();
    let mut expected: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
    expected.sort();
    assert_eq!(listed, expected);
    backend.rename(&rp("/odd")?, &rp("/odd 2")?).map_err(io::Error::other)?;
    assert_eq!(get(backend.as_ref(), &rp("/odd 2/100%.txt")?, 0)?, b"100%.txt");
    assert_eq!(mock.state.signature_failures.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn big_files_go_up_in_parts_and_come_back_from_any_offset() -> io::Result<()> {
    let mock = S3Mock::start()?;
    let backend = connect(&mock, &[("part_size_mb", "5"), ("upload_concurrency", "2")])?;
    let path = rp("/video.mkv")?;
    let data = pattern(12 * 1024 * 1024 + 3);
    put(backend.as_ref(), &path, &data)?;
    assert_eq!(mock.state.count("PUT part"), 3);
    assert_eq!(mock.state.count("POST complete"), 1);
    assert_eq!(mock.state.open_uploads(), 0);
    assert_eq!(get(backend.as_ref(), &path, 0)?, data);
    assert_eq!(get(backend.as_ref(), &path, 7_000_000)?, data[7_000_000..]);
    assert_eq!(get(backend.as_ref(), &path, data.len() as u64)?, b"");

    // An abandoned upload is aborted on the service.
    let mut session = backend.begin_write(&rp("/left.bin")?, None, false).map_err(io::Error::other)?;
    session.write_all(&pattern(11 * 1024 * 1024))?;
    assert_eq!(mock.state.open_uploads(), 1);
    drop(session);
    assert_eq!(mock.state.open_uploads(), 0);
    assert!(mock.state.bytes("left.bin").is_none());
    assert_eq!(mock.state.count("DELETE abort"), 1);
    Ok(())
}

#[test]
fn if_none_match_makes_the_loser_of_a_race_already_exists() -> io::Result<()> {
    let mock = S3Mock::start()?;
    let backend = connect(&mock, &[])?;
    let path = rp("/race")?;
    let mut first = backend.begin_write(&path, None, false).map_err(io::Error::other)?;
    first.write_all(b"first")?;
    // Another client writes the name after our check: only the 412 saves us.
    mock.state.put_raw("race", b"theirs");
    let lost = first.finish().err().map(|e| (e.kind, e.path));
    assert_eq!(lost, Some((BackendErrorKind::AlreadyExists, Some(path))));
    assert_eq!(mock.state.bytes("race"), Some(b"theirs".to_vec()));
    Ok(())
}

#[test]
fn copies_inside_the_drive_never_download() -> io::Result<()> {
    let mock = S3Mock::start()?;
    let backend = connect(&mock, &[])?;
    put(backend.as_ref(), &rp("/a")?, &pattern(100_000))?;
    let gets = mock.state.count("GET object");
    backend.copy_within(&rp("/a")?, &rp("/b")?).map_err(io::Error::other)?;
    assert_eq!(mock.state.count("GET object"), gets);
    assert_eq!(mock.state.count("PUT copy"), 1);
    assert_eq!(mock.state.bytes("b"), Some(pattern(100_000)));
    let refused = backend.copy_within(&rp("/a")?, &rp("/b")?).err().map(|e| e.kind);
    assert_eq!(refused, Some(BackendErrorKind::AlreadyExists));
    Ok(())
}

#[test]
fn many_keys_are_listed_across_pages() -> io::Result<()> {
    let mock = S3Mock::start()?;
    for i in 0..25 {
        mock.state.put_raw(&format!("many/f{i:02}"), b"x");
        mock.state.put_raw(&format!("many/d{i:02}/inner"), b"y");
    }
    mock.state.page_size.store(4, Ordering::SeqCst);
    let backend = connect(&mock, &[])?;
    let before = mock.state.count("GET list");
    let listing = backend.list(&rp("/many")?, &Cancel::new()).map_err(io::Error::other)?;
    assert!(listing.errors.is_empty());
    assert_eq!(listing.entries.len(), 50);
    assert!(mock.state.count("GET list") - before >= 13, "listed in pages");
    backend.remove_tree(&rp("/many")?, &Cancel::new()).map_err(io::Error::other)?;
    assert!(mock.state.keys().is_empty(), "{:?}", mock.state.keys());
    Ok(())
}

#[test]
fn a_wrong_secret_or_key_id_is_auth_failed_and_the_registry_asks_again() -> io::Result<()> {
    let mock = S3Mock::start()?;
    let wrong = S3Factory::new().open(&config(&mock, &[])?, Some(&Secret::new("not the secret")), &Cancel::new());
    assert_eq!(wrong.err(), Some(ConnectError::AuthFailed));
    let unknown = S3Factory::new().open(
        &config(&mock, &[("access_key_id", "AKIAUNKNOWN")])?,
        Some(&Secret::new(SECRET_KEY)),
        &Cancel::new(),
    );
    assert_eq!(unknown.err(), Some(ConnectError::AuthFailed));

    let registry = DriveRegistry::new(Arc::new(MemorySecretStore::new()));
    registry.register_factory(S3Factory::new());
    let drive = config(&mock, &[])?;
    let id = drive.id.clone();
    registry.add(drive).map_err(io::Error::other)?;
    let prompts = Scripted::new([
        PromptAnswer::Secret {
            secret: Secret::new("typo"),
            remember: Remember::No,
        },
        PromptAnswer::Secret {
            secret: Secret::new(SECRET_KEY),
            remember: Remember::No,
        },
    ]);
    registry.connect(&id, &prompts, &Cancel::new()).map_err(io::Error::other)?;
    assert_eq!(prompts.asked().len(), 2);
    assert_eq!(registry.state(&id), Some(ConnectionState::Ready));
    Ok(())
}

#[test]
fn a_session_token_after_the_secret_is_sent_and_signed() -> io::Result<()> {
    let mock = S3Mock::start()?;
    let secret = Secret::new(format!("{SECRET_KEY}\nIQoJb3JpZ2luX2VjEXAMPLETOKEN"));
    let backend = S3Factory::new()
        .open(&config(&mock, &[])?, Some(&secret), &Cancel::new())
        .map_err(io::Error::other)?;
    put(backend.as_ref(), &rp("/t")?, b"t")?;
    assert_eq!(mock.state.signature_failures.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn a_few_503s_are_retried_and_a_lasting_outage_is_unavailable_promptly() -> io::Result<()> {
    let mock = S3Mock::start()?;
    let backend = connect(&mock, &[])?;
    let path = rp("/r")?;
    put(backend.as_ref(), &path, b"r")?;
    mock.state.fail_next.store(2, Ordering::SeqCst);
    assert_eq!(backend.stat(&path).map_err(io::Error::other)?.size, Some(1), "retried through two 503s");

    mock.state.fail_next.store(1000, Ordering::SeqCst);
    let started = Instant::now();
    let error = backend.stat(&path).err().ok_or_else(|| io::Error::other("answered"))?;
    assert_eq!((error.kind, error.path), (BackendErrorKind::Unavailable, Some(path)));
    assert!(started.elapsed() < Duration::from_secs(6), "{:?}", started.elapsed());
    Ok(())
}
