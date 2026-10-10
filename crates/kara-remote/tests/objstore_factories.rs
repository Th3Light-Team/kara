//! `S3Factory` and `GcsFactory`: parameters and their defaults, the
//! `allow_http` rule, secrets (asked through the registry, never shown), the
//! connect-time check, and the real clients against endpoints that are not
//! there.

mod objstore_support;

use std::io;
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use kara_remote::objstore::object_store::aws::AmazonS3Builder;
use kara_remote::objstore::object_store::{ObjectStore, RetryConfig};
use kara_remote::objstore::{
    GcsCredentials, GcsFactory, GcsParams, ObjectStoreBackend, ObjectStoreOptions, S3Credentials,
    S3Factory, S3Params,
};
use kara_remote::{
    BackendFactory, ConfigError, ConnectError, ConnectOrRegistryError, ConnectionState,
    DriveConfig, DriveRegistry, MemorySecretStore, PromptAnswer, Remember, Secret, SecretKey,
    SecretStore,
};
use kara_vfs::{Backend, BackendErrorKind, Cancel};
use objstore_support::{
    Effect, Fault, Faulty, Op, RIGHT_SECRET, Scripted, fake_gcs_factory, fake_s3_factory,
    gcs_config, rp, s3_config,
};

fn s3_params(params: &[(&str, &str)]) -> Result<S3Params, ConnectError> {
    let config = s3_config("t", params).map_err(|e| ConnectError::Other(e.to_string()))?;
    S3Params::from_config(&config)
}

fn other_text(result: Result<S3Params, ConnectError>) -> String {
    match result {
        Err(ConnectError::Other(text)) => text,
        other => format!("not an Other error: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Parameters.

#[test]
fn s3_defaults_for_aws_and_for_an_endpoint() -> Result<(), ConnectError> {
    let aws = s3_params(&[("bucket", "photos"), ("access_key_id", "AKIA1")])?;
    assert_eq!(aws.endpoint, None);
    assert_eq!(aws.region, "us-east-1");
    assert!(!aws.path_style, "AWS uses virtual-hosted URLs");
    assert!(!aws.allow_http);
    assert_eq!(aws.credentials, S3Credentials::Key);
    assert_eq!(aws.prefix, "");
    assert_eq!(aws.tuning.timeout, Duration::from_secs(30));
    assert_eq!(aws.tuning.part_size, 8 * 1024 * 1024);
    assert_eq!(aws.tuning.upload_concurrency, 4);
    assert_eq!(aws.label(), "s3://photos");

    let minio = s3_params(&[
        ("bucket", "photos"),
        ("access_key_id", "AKIA1"),
        ("endpoint", "https://minio.lan:9000/"),
        ("prefix", "team/2024"),
    ])?;
    assert_eq!(minio.endpoint.as_deref(), Some("https://minio.lan:9000"));
    assert_eq!(minio.region, "us-east-1");
    assert!(minio.path_style, "an endpoint defaults to path-style URLs");
    assert_eq!(minio.label(), "s3://photos/team/2024 at https://minio.lan:9000");

    let tuned = s3_params(&[
        ("bucket", "b"),
        ("access_key_id", "k"),
        ("endpoint", "https://r2.example"),
        ("path_style", "false"),
        ("region", "auto"),
        ("timeout_s", "5"),
        ("part_size_mb", "16"),
        ("upload_concurrency", "8"),
    ])?;
    assert!(!tuned.path_style);
    assert_eq!(tuned.region, "auto");
    assert_eq!(tuned.tuning.timeout, Duration::from_secs(5));
    assert_eq!(tuned.tuning.part_size, 16 * 1024 * 1024);
    assert_eq!(tuned.tuning.upload_concurrency, 8);
    Ok(())
}

#[test]
fn s3_parameters_are_checked() {
    assert!(other_text(s3_params(&[("access_key_id", "k")])).contains("no bucket"));
    assert!(other_text(s3_params(&[("bucket", "a/b"), ("access_key_id", "k")])).contains("not a bucket"));
    assert!(other_text(s3_params(&[("bucket", "b")])).contains("access_key_id"));
    assert!(other_text(s3_params(&[("bucket", "b"), ("credentials", "sso")])).contains("credentials"));
    assert!(other_text(s3_params(&[("bucket", "b"), ("access_key_id", "k"), ("prefix", "a/../b")])).contains("prefix"));
    for (key, value) in [
        ("timeout_s", "0"),
        ("timeout_s", "x"),
        ("part_size_mb", "4"),
        ("part_size_mb", "513"),
        ("upload_concurrency", "0"),
        ("upload_concurrency", "33"),
        ("path_style", "maybe"),
    ] {
        let result = s3_params(&[("bucket", "b"), ("access_key_id", "k"), (key, value)]);
        assert!(other_text(result).contains(key), "{key}={value} should be refused");
    }
    // The ambient chain and anonymous access need no key id.
    assert!(s3_params(&[("bucket", "b"), ("credentials", "env")]).is_ok());
    let anonymous = s3_params(&[("bucket", "b"), ("credentials", "anonymous")]);
    assert!(matches!(anonymous, Ok(p) if p.credentials == S3Credentials::Anonymous));
}

#[test]
fn plain_http_needs_allow_http_and_allow_http_needs_plain_http() {
    let base = [("bucket", "b"), ("access_key_id", "k")];
    let with = |extra: &[(&'static str, &'static str)]| {
        let mut all = base.to_vec();
        all.extend_from_slice(extra);
        s3_params(&all)
    };
    let refused = other_text(with(&[("endpoint", "http://minio.lan:9000")]));
    assert!(refused.contains("allow_http=true") && refused.contains("unencrypted"), "{refused}");
    let accepted = with(&[("endpoint", "http://minio.lan:9000"), ("allow_http", "true")]);
    assert!(matches!(accepted, Ok(p) if p.allow_http));
    for extra in [
        &[("allow_http", "true")][..],
        &[("endpoint", "https://minio.lan"), ("allow_http", "true")][..],
    ] {
        assert!(other_text(with(extra)).contains("http://"), "{extra:?}");
    }
    assert!(other_text(with(&[("endpoint", "minio.lan:9000")])).contains("https://"));
    assert!(other_text(with(&[("endpoint", "ftp://minio.lan")])).contains("https://"));
    // allow_http=false with https is just the default.
    assert!(with(&[("endpoint", "https://minio.lan"), ("allow_http", "false")]).is_ok());
}

#[test]
fn secrets_are_never_parameters() {
    for key in ["secret_access_key", "session_token", "aws_secret", "SecretKey", "token"] {
        let result = DriveConfig::new("s3", "d", "d", [(key.to_owned(), String::from("x"))]);
        assert!(
            matches!(result, Err(ConfigError::LooksLikeSecret { .. })),
            "{key} must be refused, got {result:?}"
        );
    }
    // The non-secret ones are fine.
    let ok = DriveConfig::new(
        "s3",
        "d",
        "d",
        ["access_key_id", "bucket", "service_account_file", "credentials"]
            .map(|k| (k.to_owned(), String::from("x"))),
    );
    assert!(ok.is_ok(), "{ok:?}");
}

#[test]
fn gcs_parameters_and_the_key_file_path() -> io::Result<()> {
    let params = |extra: &[(&str, &str)]| -> Result<GcsParams, ConnectError> {
        let config = gcs_config("g", extra).map_err(|e| ConnectError::Other(e.to_string()))?;
        GcsParams::from_config(&config)
    };
    let home = std::env::home_dir().ok_or_else(|| io::Error::other("no home"))?;
    let expanded = params(&[("bucket", "b"), ("service_account_file", "~/keys/sa.json")])
        .map_err(io::Error::other)?;
    assert_eq!(
        expanded.credentials,
        GcsCredentials::ServiceAccountFile(home.join("keys/sa.json"))
    );
    assert_eq!(expanded.label(), "gs://b");
    let absolute = params(&[("bucket", "b"), ("service_account_file", "/etc/sa.json"), ("prefix", "x/y")])
        .map_err(io::Error::other)?;
    assert_eq!(absolute.credentials, GcsCredentials::ServiceAccountFile("/etc/sa.json".into()));
    assert_eq!(absolute.label(), "gs://b/x/y");
    let adc = params(&[("bucket", "b"), ("credentials", "adc")]).map_err(io::Error::other)?;
    assert_eq!(adc.credentials, GcsCredentials::ApplicationDefault);

    for (extra, needle) in [
        (&[("bucket", "b")][..], "service_account_file"),
        (&[("bucket", "b"), ("credentials", "adc"), ("service_account_file", "/k.json")][..], "not both"),
        (&[("bucket", "b"), ("credentials", "oauth")][..], "credentials"),
        (&[("service_account_file", "/k.json")][..], "bucket"),
        (&[("bucket", "b"), ("credentials", "adc"), ("endpoint", "http://fake:4443")][..], "allow_http"),
    ] {
        match params(extra) {
            Err(ConnectError::Other(text)) => assert!(text.contains(needle), "{extra:?}: {text}"),
            other => panic!("{extra:?} should be refused, got {other:?}"),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Secrets through the registry, with a fake service.

fn registry_with(factory: Arc<dyn BackendFactory>, store: Arc<MemorySecretStore>) -> Arc<DriveRegistry> {
    let registry = DriveRegistry::new(store);
    registry.register_factory(factory);
    registry
}

#[test]
fn a_missing_secret_is_asked_for_a_wrong_one_again_and_the_right_one_kept_if_asked() -> io::Result<()> {
    let service = Faulty::new();
    let secrets = Arc::new(MemorySecretStore::new());
    let registry = registry_with(fake_s3_factory(&service), Arc::clone(&secrets));
    let config = s3_config("bucket", &[("bucket", "b"), ("access_key_id", "AKIA1")])?;
    let id = config.id.clone();
    registry.add(config).map_err(io::Error::other)?;

    let prompts = Scripted::new([
        PromptAnswer::Secret {
            secret: Secret::new("not-it"),
            remember: Remember::ForDrive,
        },
        PromptAnswer::Secret {
            secret: Secret::new(RIGHT_SECRET),
            remember: Remember::ForDrive,
        },
    ]);
    registry.connect(&id, &prompts, &Cancel::new()).map_err(io::Error::other)?;
    assert_eq!(prompts.asked().len(), 2, "asked once for the missing secret, once after the wrong one");
    assert_eq!(registry.state(&id), Some(ConnectionState::Ready));
    let kept = secrets.get(&SecretKey::Drive(id.clone())).map_err(io::Error::other)?;
    assert_eq!(kept.as_ref().map(Secret::expose), Some(RIGHT_SECRET), "the secret that worked is kept");

    // Next time nothing is asked.
    registry.disconnect(&id).map_err(io::Error::other)?;
    let silent = Scripted::default();
    registry.connect(&id, &silent, &Cancel::new()).map_err(io::Error::other)?;
    assert!(silent.asked().is_empty());
    Ok(())
}

#[test]
fn a_refused_prompt_is_auth_required_and_three_wrong_secrets_are_auth_failed() -> io::Result<()> {
    let service = Faulty::new();
    let secrets = Arc::new(MemorySecretStore::new());
    let registry = registry_with(fake_s3_factory(&service), Arc::clone(&secrets));
    let config = s3_config("bucket", &[("bucket", "b"), ("access_key_id", "AKIA1")])?;
    let id = config.id.clone();
    registry.add(config).map_err(io::Error::other)?;

    let refused = registry.connect(&id, &Scripted::default(), &Cancel::new());
    assert_eq!(refused, Err(ConnectOrRegistryError::Connect(ConnectError::AuthRequired)));

    let wrong = || PromptAnswer::Secret {
        secret: Secret::new("nope"),
        remember: Remember::ForDrive,
    };
    let prompts = Scripted::new([wrong(), wrong(), wrong(), wrong()]);
    let failed = registry.connect(&id, &prompts, &Cancel::new());
    assert_eq!(failed, Err(ConnectOrRegistryError::Connect(ConnectError::AuthFailed)));
    assert!(matches!(registry.state(&id), Some(ConnectionState::Failed { .. })));
    assert_eq!(secrets.get(&SecretKey::Drive(id)).map_err(io::Error::other)?, None, "a wrong secret is never kept");
    Ok(())
}

#[test]
fn gcs_drives_never_ask_for_a_secret() -> io::Result<()> {
    let service = Faulty::new();
    service.inject(Fault::on(Op::List, Effect::Unauthenticated).always());
    let registry = registry_with(fake_gcs_factory(&service), Arc::new(MemorySecretStore::new()));
    let config = gcs_config("g", &[("bucket", "b"), ("credentials", "adc")])?;
    let id = config.id.clone();
    registry.add(config).map_err(io::Error::other)?;
    let prompts = Scripted::default();
    let result = registry.connect(&id, &prompts, &Cancel::new());
    assert!(
        matches!(&result, Err(ConnectOrRegistryError::Connect(ConnectError::Other(text))) if text.contains("refused")),
        "{result:?}"
    );
    assert!(prompts.asked().is_empty(), "{:?}", prompts.asked());

    service.clear();
    registry.connect(&id, &prompts, &Cancel::new()).map_err(io::Error::other)?;
    assert_eq!(registry.state(&id), Some(ConnectionState::Ready));
    Ok(())
}

#[test]
fn connect_time_errors_come_out_of_connect() -> io::Result<()> {
    let config = s3_config("b", &[("bucket", "b"), ("access_key_id", "k")])?;
    let secret = Secret::new(RIGHT_SECRET);
    for (effect, check) in [
        (Effect::Refused, "unreachable"),
        (Effect::ServerError, "unreachable"),
        (Effect::Denied, "denied"),
        (Effect::Unauthenticated, "failed"),
    ] {
        let service = Faulty::new();
        service.inject(Fault::on(Op::List, effect.clone()).always());
        let factory = fake_s3_factory(&service);
        let result = factory.open(&config, Some(&secret), &Cancel::new());
        let ok = match (&result, check) {
            (Err(ConnectError::Unreachable(_)), "unreachable") => true,
            (Err(ConnectError::Other(text)), "denied") => text.contains("may not list"),
            (Err(ConnectError::AuthFailed), "failed") => true,
            _ => false,
        };
        assert!(ok, "{effect:?}: {result:?}");
    }
    // A bucket that does not exist.
    let service = Faulty::new();
    service.inject(Fault::on(Op::List, Effect::NoSuchBucket).always());
    let result = fake_s3_factory(&service).open(&config, Some(&secret), &Cancel::new());
    assert!(
        matches!(&result, Err(ConnectError::Other(text)) if text.contains("bucket does not exist")),
        "{result:?}"
    );

    // A cancelled connect never reaches the service.
    let service = Faulty::new();
    let factory = fake_s3_factory(&service);
    let cancel = Cancel::new();
    cancel.cancel();
    assert_eq!(factory.open(&config, Some(&secret), &cancel).err(), Some(ConnectError::Cancelled));
    assert_eq!(service.counts.get(Op::List), 0);

    // A cancel while the check hangs answers at once.
    let service = Faulty::new();
    service.inject(Fault::on(Op::List, Effect::Stall(Duration::from_secs(20))));
    let factory = fake_s3_factory(&service);
    let cancel = Cancel::new();
    let canceller = {
        let cancel = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            cancel.cancel();
        })
    };
    let started = Instant::now();
    let result = factory.open(&config, Some(&secret), &cancel);
    let _ = canceller.join();
    assert_eq!(result.err(), Some(ConnectError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    Ok(())
}

// ---------------------------------------------------------------------------
// The real clients, against endpoints that are not there.

/// A port nobody listens on.
fn closed_port() -> io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

#[test]
fn a_dead_endpoint_is_unreachable_at_connect_promptly_and_says_nothing_secret() -> io::Result<()> {
    let port = closed_port()?;
    let endpoint = format!("http://127.0.0.1:{port}");
    let config = s3_config(
        "dead",
        &[
            ("bucket", "b"),
            ("access_key_id", "AKIAEXAMPLEID"),
            ("endpoint", &endpoint),
            ("allow_http", "true"),
            ("timeout_s", "5"),
        ],
    )?;
    let secret = Secret::new(format!("{RIGHT_SECRET}\nFwoGZXIvYXdzEXAMPLESESSIONTOKEN"));
    let started = Instant::now();
    let result = S3Factory::new().open(&config, Some(&secret), &Cancel::new());
    let elapsed = started.elapsed();
    let error = result.err().ok_or_else(|| io::Error::other("connected to nothing"))?;
    assert!(matches!(error, ConnectError::Unreachable(_)), "{error:?}");
    assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");
    let text = format!("{error} {error:?}");
    assert!(!text.contains(RIGHT_SECRET) && !text.contains("SESSIONTOKEN"), "{text}");
    Ok(())
}

#[test]
fn a_silent_endpoint_times_out_instead_of_hanging() -> io::Result<()> {
    // Accepts connections and never answers.
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let holder = thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().take(16) {
            held.push(stream);
        }
        thread::sleep(Duration::from_secs(30));
    });
    let endpoint = format!("http://127.0.0.1:{port}");
    let config = s3_config(
        "silent",
        &[
            ("bucket", "b"),
            ("access_key_id", "k"),
            ("endpoint", &endpoint),
            ("allow_http", "true"),
            ("timeout_s", "1"),
        ],
    )?;
    let started = Instant::now();
    let result = S3Factory::new().open(&config, Some(&Secret::new("s")), &Cancel::new());
    let elapsed = started.elapsed();
    assert!(matches!(result, Err(ConnectError::Unreachable(_))), "{result:?}");
    assert!(elapsed < Duration::from_secs(8), "took {elapsed:?}");
    drop(holder);
    Ok(())
}

#[test]
fn every_call_on_a_drive_whose_endpoint_died_is_unavailable_at_once() -> io::Result<()> {
    let port = closed_port()?;
    let store = AmazonS3Builder::new()
        .with_bucket_name("b")
        .with_region("us-east-1")
        .with_endpoint(format!("http://127.0.0.1:{port}"))
        .with_allow_http(true)
        .with_access_key_id("k")
        .with_secret_access_key("s")
        .with_retry(RetryConfig {
            max_retries: 2,
            retry_timeout: Duration::from_secs(2),
            ..RetryConfig::default()
        })
        .build()
        .map_err(io::Error::other)?;
    let store: Arc<dyn ObjectStore> = Arc::new(store);
    let backend = ObjectStoreBackend::new(store, ObjectStoreOptions::default()).map_err(io::Error::other)?;
    let path = rp("/docs/a.txt")?;
    let started = Instant::now();
    let kinds = [
        backend.stat(&path).err().map(|e| (e.kind, e.path)),
        backend.list(&rp("/docs")?, &Cancel::new()).err().map(|e| (e.kind, e.path)),
        backend.open_read(&path, 0).err().map(|e| (e.kind, e.path)),
        backend.begin_write(&path, None, false).err().map(|e| (e.kind, e.path)),
        backend.remove(&path).err().map(|e| (e.kind, e.path)),
    ];
    for found in kinds {
        let (kind, named) = found.ok_or_else(|| io::Error::other("a call succeeded"))?;
        assert_eq!(kind, BackendErrorKind::Unavailable);
        assert!(named.is_some_and(|p| p.as_str().starts_with("/docs")));
    }
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    Ok(())
}

#[test]
fn a_gcs_key_file_that_is_missing_or_broken_is_reported_without_its_content() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let missing = dir.path().join("nope.json");
    let config = gcs_config(
        "g",
        &[("bucket", "b"), ("service_account_file", &missing.to_string_lossy())],
    )?;
    match GcsFactory::new().open(&config, &Cancel::new()) {
        Err(ConnectError::Other(text)) => assert!(text.contains("nope.json"), "{text}"),
        other => panic!("{other:?}"),
    }
    let broken = dir.path().join("broken.json");
    std::fs::write(
        &broken,
        "{\"private_key\": \"-----BEGIN PRIVATE KEY-----\\nTOPSECRETKEYMATERIAL\\n\", \"client_email\": 42",
    )?;
    let config = gcs_config(
        "g",
        &[("bucket", "b"), ("service_account_file", &broken.to_string_lossy())],
    )?;
    match GcsFactory::new().open(&config, &Cancel::new()) {
        Err(error) => {
            let text = format!("{error} {error:?}");
            assert!(!text.contains("TOPSECRETKEYMATERIAL"), "{text}");
        }
        Ok(_) => panic!("a broken key file connected"),
    }
    Ok(())
}

#[test]
fn debug_output_holds_no_secret() -> io::Result<()> {
    let service = Faulty::new();
    let factory = fake_s3_factory(&service);
    let config = s3_config("b", &[("bucket", "b"), ("access_key_id", "AKIA1"), ("prefix", "p")])?;
    let secret = Secret::new(RIGHT_SECRET);
    let backend = factory.open(&config, Some(&secret), &Cancel::new()).map_err(io::Error::other)?;
    let params = S3Params::from_config(&config).map_err(io::Error::other)?;
    let text = format!("{factory:?} {backend:?} {params:?} {secret:?} {config:?}");
    assert!(!text.contains(RIGHT_SECRET), "{text}");
    assert!(text.contains("s3://b/p"), "{text}");
    Ok(())
}
