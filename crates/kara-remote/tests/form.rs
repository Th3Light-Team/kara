//! The «Add drive» form: descriptors, visibility, validation, editing.

use kara_remote::form::{self, FieldKind, Form, When, slug};

fn fill(form: &mut Form, key: &str, value: &str) {
    let index = form
        .kind()
        .fields
        .iter()
        .position(|f| f.key == key)
        .expect("field exists");
    assert!(form.set_value(index, value));
}

fn errors_of(form: &Form) -> Vec<(String, String)> {
    match form.build() {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|e| (e.field, e.message)).collect(),
    }
}

fn has_error(form: &Form, field: &str) -> bool {
    errors_of(form).iter().any(|(f, _)| f == field)
}

#[test]
fn every_protocol_has_a_descriptor_and_unique_keys() {
    let schemes: Vec<_> = form::kinds().iter().map(|k| k.scheme).collect();
    assert_eq!(schemes, ["sftp", "s3", "gcs"]);
    for kind in form::kinds() {
        let mut keys: Vec<_> = kind.fields.iter().map(|f| f.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), kind.fields.len(), "{} repeats a key", kind.scheme);
        // No descriptor may turn a secret into a stored parameter.
        for field in kind.fields {
            for word in ["password", "passwd", "passphrase", "secret", "token", "apikey"] {
                assert!(!field.key.contains(word), "{} is secret-looking", field.key);
            }
            // A condition must refer to a field of the same protocol.
            if let When::Equals(key, _) | When::StartsWith(key, _) = field.when {
                assert!(kind.fields.iter().any(|f| f.key == key), "{key} unknown");
            }
        }
    }
}

#[test]
fn unknown_scheme_has_no_form() {
    assert!(Form::new("ftp").is_none());
    assert!(form::kind("sftp").is_some());
}

#[test]
fn blank_sftp_form_asks_for_name_and_host() {
    let blank = Form::new("sftp").expect("sftp");
    let errors = errors_of(&blank);
    assert!(errors.iter().any(|(f, _)| f == "label"));
    assert!(errors.iter().any(|(f, m)| f == "host" && m == "Obligatorio"));
}

#[test]
fn sftp_builds_a_config_with_only_what_was_typed() {
    let mut f = Form::new("sftp").expect("sftp");
    f.label = String::from("Mi NAS");
    fill(&mut f, "host", "nas.local");
    fill(&mut f, "port", "2222");
    let built = f.build().expect("valid");
    assert_eq!(built.config.id.scheme(), "sftp");
    assert_eq!(built.config.id.name(), "mi-nas");
    assert_eq!(built.config.label, "Mi NAS");
    assert_eq!(built.config.param("host"), Some("nas.local"));
    assert_eq!(built.config.param("port"), Some("2222"));
    // Defaults are not written down: the adapter owns them.
    assert_eq!(built.config.param("timeout_s"), None);
    assert!(built.secret.is_none());
}

#[test]
fn numbers_are_range_checked() {
    let mut f = Form::new("sftp").expect("sftp");
    f.label = String::from("x");
    fill(&mut f, "host", "h");
    fill(&mut f, "port", "70000");
    assert!(has_error(&f, "port"));
    fill(&mut f, "port", "abc");
    assert!(has_error(&f, "port"));
    fill(&mut f, "port", "22");
    assert!(!has_error(&f, "port"));
}

#[test]
fn an_explicit_name_wins_and_is_validated() {
    let mut f = Form::new("sftp").expect("sftp");
    f.label = String::from("Servidor");
    fill(&mut f, "host", "h");
    f.name = String::from("Not Valid");
    assert!(has_error(&f, "name"));
    f.name = String::from("work-nas");
    assert_eq!(f.build().expect("valid").config.id.name(), "work-nas");
}

#[test]
fn a_label_with_no_latin_letters_asks_for_a_name() {
    let mut f = Form::new("sftp").expect("sftp");
    f.label = String::from("日本語");
    fill(&mut f, "host", "h");
    assert!(has_error(&f, "name"));
    f.name = String::from("nihongo");
    assert!(f.build().is_ok());
}

#[test]
fn slug_rules() {
    assert_eq!(slug("Mi NAS"), "mi-nas");
    assert_eq!(slug("  --Café  Servidor!! "), "caf-servidor");
    assert_eq!(slug("¡¡"), "");
    assert_eq!(slug(&"a".repeat(100)).len(), 63);
}

#[test]
fn s3_credentials_decide_which_fields_and_secret_apply() {
    let mut f = Form::new("s3").expect("s3");
    let key_index = f.kind().fields.iter().position(|x| x.key == "access_key_id").expect("key");
    assert!(f.field_visible(key_index), "credentials default to key");
    assert!(f.secret_visible());

    fill(&mut f, "credentials", "anonymous");
    assert!(!f.field_visible(key_index));
    assert!(!f.secret_visible());

    f.label = String::from("Datos");
    fill(&mut f, "bucket", "datos");
    // With the key hidden it is neither required nor stored.
    fill(&mut f, "access_key_id", "AKIA");
    let built = f.build().expect("anonymous needs no key");
    assert_eq!(built.config.param("access_key_id"), None);
    assert_eq!(built.config.param("credentials"), Some("anonymous"));
}

#[test]
fn s3_key_credentials_require_the_key_id() {
    let mut f = Form::new("s3").expect("s3");
    f.label = String::from("Datos");
    fill(&mut f, "bucket", "datos");
    assert!(has_error(&f, "access_key_id"));
    fill(&mut f, "access_key_id", "AKIA123");
    f.secret = String::from("s3cr3t");
    let built = f.build().expect("valid");
    assert_eq!(built.secret.expect("secret").expose(), "s3cr3t");
    // The secret never becomes a parameter.
    assert!(built.config.params.keys().all(|k| !k.contains("secret")));
}

#[test]
fn http_endpoint_needs_the_unencrypted_toggle() {
    let mut f = Form::new("s3").expect("s3");
    f.label = String::from("Local");
    fill(&mut f, "bucket", "b");
    fill(&mut f, "access_key_id", "k");
    let toggle = f.kind().fields.iter().position(|x| x.key == "allow_http").expect("toggle");
    assert!(!f.field_visible(toggle));

    fill(&mut f, "endpoint", "https://s3.example.com");
    assert!(!f.field_visible(toggle));
    assert!(f.build().is_ok());

    fill(&mut f, "endpoint", "HTTP://127.0.0.1:9000");
    assert!(f.field_visible(toggle));
    assert!(has_error(&f, "allow_http"));
    fill(&mut f, "allow_http", "true");
    assert!(f.build().is_ok());
}

#[test]
fn gcs_has_no_secret_and_needs_a_key_file_unless_adc() {
    let mut f = Form::new("gcs").expect("gcs");
    assert!(!f.secret_visible());
    f.label = String::from("Backups");
    fill(&mut f, "bucket", "backups");
    assert!(has_error(&f, "service_account_file"));
    fill(&mut f, "credentials", "adc");
    let built = f.build().expect("adc needs no file");
    assert_eq!(built.config.param("credentials"), Some("adc"));
}

#[test]
fn a_secret_for_a_protocol_without_one_is_dropped() {
    let mut f = Form::new("gcs").expect("gcs");
    f.label = String::from("B");
    fill(&mut f, "bucket", "b");
    fill(&mut f, "credentials", "adc");
    f.secret = String::from("ignored");
    assert!(f.build().expect("valid").secret.is_none());
}

#[test]
fn choices_and_toggles_reject_unknown_values() {
    let mut f = Form::new("s3").expect("s3");
    f.label = String::from("x");
    fill(&mut f, "bucket", "b");
    fill(&mut f, "access_key_id", "k");
    fill(&mut f, "conditional_put", "maybe");
    assert!(has_error(&f, "conditional_put"));
    fill(&mut f, "conditional_put", "disabled");
    fill(&mut f, "path_style", "yes");
    assert!(has_error(&f, "path_style"));
    fill(&mut f, "path_style", "false");
    assert!(f.build().is_ok());
}

#[test]
fn group_is_validated() {
    let mut f = Form::new("sftp").expect("sftp");
    f.label = String::from("x");
    fill(&mut f, "host", "h");
    f.group = String::from("a:b");
    assert!(has_error(&f, "group"));
    f.group = String::from("Proxmox");
    assert_eq!(f.build().expect("valid").config.group.as_deref(), Some("Proxmox"));
}

#[test]
fn editing_round_trips_and_keeps_unknown_parameters() {
    let mut f = Form::new("sftp").expect("sftp");
    f.label = String::from("NAS");
    fill(&mut f, "host", "nas.local");
    let mut config = f.build().expect("valid").config;
    config.params.insert(String::from("handmade"), String::from("1"));

    let mut edit = Form::editing(&config).expect("sftp");
    assert!(edit.is_editing());
    assert_eq!(edit.name, "nas");
    assert_eq!(edit.label, "NAS");
    assert!(edit.secret.is_empty(), "the stored secret is never shown");

    edit.label = String::from("NAS del salón");
    fill(&mut edit, "port", "2200");
    let rebuilt = edit.build().expect("valid").config;
    assert_eq!(rebuilt.id, config.id, "identity is kept");
    assert_eq!(rebuilt.label, "NAS del salón");
    assert_eq!(rebuilt.param("port"), Some("2200"));
    assert_eq!(rebuilt.param("handmade"), Some("1"));
    assert_eq!(rebuilt.param("host"), Some("nas.local"));
}

#[test]
fn the_remember_flag_travels_with_the_secret() {
    let mut f = Form::new("sftp").expect("sftp");
    f.label = String::from("x");
    fill(&mut f, "host", "h");
    f.secret = String::from("pw");
    f.remember = false;
    let built = f.build().expect("valid");
    assert!(!built.remember);
    assert!(built.secret.is_some());
}

#[test]
fn path_fields_are_marked_for_the_file_chooser() {
    let sftp = form::kind("sftp").expect("sftp");
    let key_file = sftp.fields.iter().find(|f| f.key == "key_file").expect("key_file");
    assert_eq!(key_file.kind, FieldKind::Path);
}
