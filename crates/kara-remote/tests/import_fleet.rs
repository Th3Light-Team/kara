//! Bulk import (CSV, ssh_config) and shared group secrets: the fleet case.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use kara_remote::config::{self, DriveConfig};
use kara_remote::import::{from_csv, from_ssh_config};
use kara_remote::memory::MemoryFactory;
use kara_remote::{
    ConnectionState, DriveRegistry, MemorySecretStore, Prompt, PromptAnswer, PromptHandler,
    Remember, Secret, SecretKey, SecretStore,
};
use kara_vfs::{Cancel, DriveId};

#[test]
fn csv_builds_one_drive_per_line() {
    let text = "name,host,user,port,key_file,label,group\n\
                # production fleet\n\
                web1,10.0.0.11,root,22,/home/u/.ssh/id_ed25519,Web 1,prod\n\
                web2,10.0.0.12,root\n\
                \n\
                db1,10.0.0.20,postgres,2222\n";
    let report = from_csv(text, "sftp", Some("fleet"));

    assert!(report.problems.is_empty(), "{:?}", report.problems);
    assert_eq!(report.drives.len(), 3);
    let web1 = &report.drives[0];
    assert_eq!(web1.id.scheme(), "sftp");
    assert_eq!(web1.label, "Web 1");
    assert_eq!(web1.group.as_deref(), Some("prod"), "a line's own group wins");
    assert_eq!(web1.param("key_file"), Some("/home/u/.ssh/id_ed25519"));
    let web2 = &report.drives[1];
    assert_eq!(web2.label, "web2", "label defaults to the name");
    assert_eq!(web2.group.as_deref(), Some("fleet"), "the default group fills the gaps");
    assert_eq!(web2.param("port"), None);
    assert_eq!(report.drives[2].param("port"), Some("2222"));
}

#[test]
fn csv_reports_bad_lines_and_keeps_the_rest() {
    let text = "ok1,10.0.0.1\n\
                ,10.0.0.2\n\
                no-host\n\
                bad_name,10.0.0.3\n\
                badport,10.0.0.4,root,99999\n\
                ok1,10.0.0.5\n\
                ok2,10.0.0.6,,,,,bad:group\n\
                ok3,10.0.0.7\n";
    let report = from_csv(text, "sftp", None);

    let names: Vec<_> = report.drives.iter().map(|d| d.id.name().to_owned()).collect();
    assert_eq!(names, ["ok1", "ok3"]);
    assert_eq!(report.problems.len(), 6, "{:?}", report.problems);
    assert!(report.problems.iter().any(|p| p.contains("duplicate")));
    assert!(report.problems.iter().any(|p| p.contains("line 5") && p.contains("port")));
}

#[test]
fn csv_never_puts_a_secret_in_a_param() {
    // The columns are fixed, so a password column does not exist; a key_file path
    // is a path, not a secret.
    let report = from_csv("a,h,u,22,/k\n", "sftp", None);
    for drive in &report.drives {
        for key in drive.params.keys() {
            assert!(!key.contains("pass"), "{key}");
        }
    }
}

const SSH_CONFIG: &str = "\
Host *
    ServerAliveInterval 30

Host web1 web2
    HostName 10.0.0.11
    User root
    Port 2200
    IdentityFile ~/.ssh/fleet
    IdentityFile ~/.ssh/other

Host Build_Box.local
    HostName build.example.org

Host behind-bastion
    HostName 10.9.9.9
    ProxyJump bastion

Host !blocked *.wild
    User nobody

Match host something
    User root

Host last
    HostName 192.168.1.5
";

#[test]
fn ssh_config_imports_the_concrete_hosts() {
    let report = from_ssh_config(SSH_CONFIG, "sftp", Some("lab"));

    let names: Vec<_> = report.drives.iter().map(|d| d.id.name().to_owned()).collect();
    assert_eq!(names, ["web1", "web2", "build-box-local", "last"]);
    let web1 = &report.drives[0];
    assert_eq!(web1.param("host"), Some("10.0.0.11"));
    assert_eq!(web1.param("user"), Some("root"));
    assert_eq!(web1.param("port"), Some("2200"));
    assert_eq!(web1.param("key_file"), Some("~/.ssh/fleet"), "the first IdentityFile");
    assert_eq!(web1.group.as_deref(), Some("lab"));
    assert_eq!(report.drives[1].param("host"), Some("10.0.0.11"), "aliases share the block");
    let last = &report.drives[3];
    assert_eq!(last.param("host"), Some("192.168.1.5"));
    assert_eq!(last.param("user"), None, "nothing leaks from Host * or a Match block");
    assert_eq!(report.drives[2].label, "Build_Box.local", "the alias stays as the label");
}

#[test]
fn ssh_config_reports_what_it_cannot_reach() {
    let report = from_ssh_config(SSH_CONFIG, "sftp", None);
    assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
    assert!(report.problems[0].contains("ProxyJump"));
}

#[test]
fn imported_drives_survive_the_settings_file() {
    let report = from_csv("a,h1,u\nb,h2,u\n", "sftp", Some("fleet"));
    let mut settings = kara_fs::settings::Settings::new();
    for drive in &report.drives {
        config::store(&mut settings, drive);
    }
    let text = kara_fs::settings::serialize(settings.sections());
    let back = kara_fs::settings::Settings::from_sections(kara_fs::settings::parse(&text));
    let (drives, problems) = config::load_all(&back);
    assert!(problems.is_empty());
    assert_eq!(drives, report.drives, "the group must come back too");
}

#[test]
fn group_names_are_validated() {
    let base = DriveConfig::new("sftp", "a", "A", []).expect("config");
    for bad in ["", "  ", "a:b", "x\ny"] {
        assert!(base.clone().with_group(bad).is_err(), "{bad:?}");
    }
    assert!(base.with_group("prod").is_ok());
}

// ---- shared group secret ---------------------------------------------------

struct CountingAsker {
    answer: &'static str,
    remember: Remember,
    asked: AtomicUsize,
}

impl PromptHandler for CountingAsker {
    fn ask(&self, prompt: &Prompt) -> PromptAnswer {
        self.asked.fetch_add(1, Ordering::SeqCst);
        assert!(matches!(prompt, Prompt::Password { .. }));
        PromptAnswer::Secret {
            secret: Secret::new(self.answer),
            remember: self.remember,
        }
    }
}

fn fleet(n: usize) -> (Arc<DriveRegistry>, Arc<MemorySecretStore>, Vec<DriveId>) {
    let secrets = Arc::new(MemorySecretStore::new());
    let registry = DriveRegistry::new(secrets.clone());
    registry.register_factory(MemoryFactory::new());
    let mut ids = Vec::new();
    for i in 0..n {
        let config = DriveConfig::new(
            "mem",
            &format!("node{i}"),
            &format!("Node {i}"),
            [("auth".to_owned(), "required".to_owned())],
        )
        .expect("config")
        .with_group("fleet")
        .expect("group");
        ids.push(config.id.clone());
        registry.add(config).expect("add");
    }
    (registry, secrets, ids)
}

#[test]
fn one_password_for_the_whole_group_is_asked_once() {
    let (registry, secrets, ids) = fleet(5);
    let asker = CountingAsker {
        answer: MemoryFactory::ACCEPTED,
        remember: Remember::ForGroup,
        asked: AtomicUsize::new(0),
    };

    for id in &ids {
        registry.connect(id, &asker, &Cancel::new()).expect("connect");
    }

    assert_eq!(asker.asked.load(Ordering::SeqCst), 1, "the other four reuse the group secret");
    assert!(secrets.get(&SecretKey::Group("fleet".into())).expect("get").is_some());
    for id in &ids {
        assert_eq!(registry.state(id), Some(ConnectionState::Ready));
        assert!(secrets.get(&SecretKey::Drive(id.clone())).expect("get").is_none());
    }
}

#[test]
fn a_drive_secret_beats_the_group_secret() {
    let (registry, _secrets, ids) = fleet(2);
    registry
        .remember_group_secret("fleet", &Secret::new("wrong for node0"))
        .expect("group secret");
    registry
        .remember_secret(&ids[0], &Secret::new(MemoryFactory::ACCEPTED))
        .expect("own secret");
    let asker = CountingAsker {
        answer: MemoryFactory::ACCEPTED,
        remember: Remember::No,
        asked: AtomicUsize::new(0),
    };

    registry.connect(&ids[0], &asker, &Cancel::new()).expect("own secret wins");
    assert_eq!(asker.asked.load(Ordering::SeqCst), 0);

    // node1 only has the group's secret, which is wrong: it is asked, then works.
    registry.connect(&ids[1], &asker, &Cancel::new()).expect("asked");
    assert_eq!(asker.asked.load(Ordering::SeqCst), 1);
}

#[test]
fn for_group_without_a_group_falls_back_to_the_drive() {
    let secrets = Arc::new(MemorySecretStore::new());
    let registry = DriveRegistry::new(secrets.clone());
    registry.register_factory(MemoryFactory::new());
    let config = DriveConfig::new("mem", "solo", "Solo", [("auth".to_owned(), "required".to_owned())])
        .expect("config");
    let id = config.id.clone();
    registry.add(config).expect("add");
    let asker = CountingAsker {
        answer: MemoryFactory::ACCEPTED,
        remember: Remember::ForGroup,
        asked: AtomicUsize::new(0),
    };
    registry.connect(&id, &asker, &Cancel::new()).expect("connect");
    assert!(secrets.get(&SecretKey::Drive(id)).expect("get").is_some());
}

#[test]
fn removing_a_drive_keeps_the_group_secret_for_the_others() {
    let (registry, secrets, ids) = fleet(2);
    registry.remember_group_secret("fleet", &Secret::new("shared")).expect("group secret");
    registry.remove(&ids[0]).expect("remove");
    assert!(secrets.get(&SecretKey::Group("fleet".into())).expect("get").is_some());
    registry.forget_group_secret("fleet").expect("forget");
    assert!(secrets.get(&SecretKey::Group("fleet".into())).expect("get").is_none());
}

#[test]
fn groups_list_their_members() {
    let (registry, _, ids) = fleet(3);
    let solo = DriveConfig::new("mem", "solo", "Solo", []).expect("config");
    registry.add(solo).expect("add");
    let groups = registry.groups();
    assert_eq!(groups.len(), 1, "ungrouped drives are not listed");
    assert_eq!(groups["fleet"], ids);
}
