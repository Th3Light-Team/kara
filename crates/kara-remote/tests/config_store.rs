//! DriveConfig <-> settings.conf: round trip, stability, and no secrets on disk.

use kara_fs::settings::{self, Settings};
use kara_remote::config::{self, ConfigError, DriveConfig};

fn cfg(scheme: &str, name: &str, label: &str, params: &[(&str, &str)]) -> DriveConfig {
    DriveConfig::new(
        scheme,
        name,
        label,
        params.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    )
    .expect("valid config")
}

#[test]
fn a_config_survives_serialize_and_parse() {
    let nas = cfg(
        "sftp",
        "work-nas",
        "Work NAS",
        &[("host", "nas.example.org"), ("port", "2222"), ("key_file", "/home/u/.ssh/id=ed25519")],
    );
    let bucket = cfg("s3", "backups", "Backups", &[("bucket", "my-bucket"), ("endpoint", "https://s3.example")]);
    let mut settings = Settings::new();
    config::store(&mut settings, &nas);
    config::store(&mut settings, &bucket);

    let text = settings::serialize(settings.sections());
    let back = Settings::from_sections(settings::parse(&text));
    let (drives, problems) = config::load_all(&back);

    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(drives.len(), 2);
    assert!(drives.contains(&nas));
    assert!(drives.contains(&bucket));
}

#[test]
fn storing_twice_gives_identical_text() {
    let one = cfg("sftp", "a", "A", &[("host", "h")]);
    let mut settings = Settings::new();
    config::store(&mut settings, &one);
    let first = settings::serialize(settings.sections());
    config::store(&mut settings, &one);
    assert_eq!(settings::serialize(settings.sections()), first);
}

#[test]
fn storing_replaces_removed_parameters() {
    let mut settings = Settings::new();
    config::store(&mut settings, &cfg("sftp", "a", "A", &[("host", "h"), ("port", "22")]));
    config::store(&mut settings, &cfg("sftp", "a", "A", &[("host", "h")]));
    let (drives, _) = config::load_all(&settings);
    assert_eq!(drives.len(), 1);
    assert_eq!(drives[0].param("port"), None, "a dropped parameter must not linger");
}

#[test]
fn secret_looking_parameters_are_refused() {
    for key in ["password", "Password", "ssh_passphrase", "client_secret", "access_token", "apikey"] {
        let result = DriveConfig::new("sftp", "a", "A", [(key.to_owned(), "x".to_owned())]);
        assert_eq!(
            result,
            Err(ConfigError::LooksLikeSecret { key: key.to_owned() }),
            "{key}"
        );
    }
}

#[test]
fn nothing_secret_reaches_the_file() {
    let mut settings = Settings::new();
    config::store(&mut settings, &cfg("sftp", "a", "A", &[("host", "h"), ("user", "oli")]));
    let text = settings::serialize(settings.sections()).to_lowercase();
    for word in ["password", "passphrase", "secret", "token"] {
        assert!(!text.contains(word), "{word} must not be stored: {text}");
    }
}

#[test]
fn bad_input_is_rejected_when_the_config_is_built() {
    let ok = |params: Vec<(String, String)>| DriveConfig::new("sftp", "a", "A", params);
    assert!(matches!(DriveConfig::new("SFTP", "a", "A", []), Err(ConfigError::Id(_))));
    assert!(matches!(DriveConfig::new("sftp", "Bad_Name", "A", []), Err(ConfigError::Id(_))));
    assert_eq!(DriveConfig::new("sftp", "a", "  ", []), Err(ConfigError::BadLabel));
    assert_eq!(DriveConfig::new("sftp", "a", "A\nB", []), Err(ConfigError::BadLabel));
    assert!(matches!(ok(vec![("a=b".into(), "x".into())]), Err(ConfigError::BadParamKey { .. })));
    assert!(matches!(ok(vec![("a b".into(), "x".into())]), Err(ConfigError::BadParamKey { .. })));
    assert!(matches!(ok(vec![(String::new(), "x".into())]), Err(ConfigError::BadParamKey { .. })));
    assert!(matches!(ok(vec![("k".into(), "x\ny".into())]), Err(ConfigError::BadParamValue { .. })));
}

#[test]
fn a_broken_section_does_not_hide_the_good_ones() {
    let text = "[drive:sftp:good]\nlabel=Good\nparam.host=h\n\n[drive:nonsense]\nlabel=x\n\n[drive:SFTP:Bad]\nlabel=y\n\n[other]\nk=v\n";
    let settings = Settings::from_sections(settings::parse(text));
    let (drives, problems) = config::load_all(&settings);
    assert_eq!(drives.len(), 1);
    assert_eq!(drives[0].label, "Good");
    assert_eq!(problems.len(), 2, "{problems:?}");
}

#[test]
fn a_missing_label_falls_back_to_the_name() {
    let settings = Settings::from_sections(settings::parse("[drive:sftp:nas]\nparam.host=h\n"));
    let (drives, problems) = config::load_all(&settings);
    assert!(problems.is_empty());
    assert_eq!(drives[0].label, "nas");
}

#[test]
fn forgetting_removes_only_that_drive() {
    let a = cfg("sftp", "a", "A", &[("host", "h")]);
    let b = cfg("sftp", "b", "B", &[("host", "h")]);
    let mut settings = Settings::new();
    config::store(&mut settings, &a);
    config::store(&mut settings, &b);
    assert!(config::forget(&mut settings, &a.id));
    assert!(!config::forget(&mut settings, &a.id), "second time there is nothing to forget");
    let (drives, _) = config::load_all(&settings);
    assert_eq!(drives, vec![b]);
}
