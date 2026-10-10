//! The non-secret description of a drive, and its place in `settings.conf`.
//!
//! One section per drive, `[drive:<scheme>:<name>]`, with a `label` and one
//! `param.<key>` line per parameter (host, port, user, bucket, endpoint,
//! key-file path...). **Secrets never get here**: a parameter whose key looks
//! like one is refused when the config is built, so a password cannot reach the
//! file by a slip. Secrets go through [`crate::secrets::SecretStore`].

use std::collections::BTreeMap;

use kara_fs::settings::Settings;
use kara_vfs::{DriveId, DriveIdError};

const PREFIX: &str = "drive:";
const PARAM: &str = "param.";
const LABEL: &str = "label";
const GROUP: &str = "group";

/// Parameter keys that must be stored as secrets, not as settings.
const SECRET_WORDS: [&str; 6] = ["password", "passwd", "passphrase", "secret", "token", "apikey"];

/// Why a drive config cannot be built or stored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    Id(#[from] DriveIdError),
    #[error("the label is empty or has a line break")]
    BadLabel,
    #[error("the group name is empty, too long, or has ':' or a line break")]
    BadGroup,
    #[error("parameter {key:?} looks like a secret; store it with the SecretStore")]
    LooksLikeSecret { key: String },
    #[error("parameter key {key:?} is empty, has '=' or whitespace")]
    BadParamKey { key: String },
    #[error("parameter {key:?} has a line break in its value")]
    BadParamValue { key: String },
}

/// What is needed to reach a drive, minus its secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveConfig {
    pub id: DriveId,
    /// What the panel shows. Renaming it does not change the drive's identity.
    pub label: String,
    /// A fleet this drive belongs to. Drives of one group can share a secret
    /// and are listed together.
    pub group: Option<String>,
    pub params: BTreeMap<String, String>,
}

impl DriveConfig {
    /// Validates everything that would otherwise only fail on the next load.
    pub fn new(
        scheme: &str,
        name: &str,
        label: &str,
        params: impl IntoIterator<Item = (String, String)>,
    ) -> Result<DriveConfig, ConfigError> {
        let id = DriveId::new(scheme, name)?;
        let label = label.trim().to_owned();
        if label.is_empty() || label.contains(['\n', '\r']) {
            return Err(ConfigError::BadLabel);
        }
        let mut map = BTreeMap::new();
        for (key, value) in params {
            let lower = key.to_ascii_lowercase();
            if key.is_empty() || key.contains(['=', '\n', '\r']) || key.contains(char::is_whitespace)
            {
                return Err(ConfigError::BadParamKey { key });
            }
            if SECRET_WORDS.iter().any(|word| lower.contains(word)) {
                return Err(ConfigError::LooksLikeSecret { key });
            }
            if value.contains(['\n', '\r']) {
                return Err(ConfigError::BadParamValue { key });
            }
            map.insert(key, value);
        }
        Ok(DriveConfig {
            id,
            label,
            group: None,
            params: map,
        })
    }

    /// Puts the drive in a group. A group name is a short single-line label.
    pub fn with_group(mut self, group: &str) -> Result<DriveConfig, ConfigError> {
        let group = group.trim();
        if group.is_empty() || group.len() > 64 || group.contains(['\n', '\r', ':']) {
            return Err(ConfigError::BadGroup);
        }
        self.group = Some(group.to_owned());
        Ok(self)
    }

    /// A parameter by key.
    #[must_use]
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(String::as_str)
    }
}

fn section_name(id: &DriveId) -> String {
    format!("{PREFIX}{}:{}", id.scheme(), id.name())
}

/// Writes `config` into `settings`, replacing the drive's previous section.
pub fn store(settings: &mut Settings, config: &DriveConfig) {
    let section = section_name(&config.id);
    settings.remove_section(&section);
    settings.set(&section, LABEL, config.label.clone());
    if let Some(group) = &config.group {
        settings.set(&section, GROUP, group.clone());
    }
    for (key, value) in &config.params {
        settings.set(&section, &format!("{PARAM}{key}"), value.clone());
    }
}

/// Forgets a drive. `true` if there was something to forget.
pub fn forget(settings: &mut Settings, id: &DriveId) -> bool {
    settings.remove_section(&section_name(id))
}

/// Every drive in `settings`, in a stable order, plus a note for each section
/// that could not be used. A bad section never hides the good ones.
#[must_use]
pub fn load_all(settings: &Settings) -> (Vec<DriveConfig>, Vec<String>) {
    let mut drives = Vec::new();
    let mut problems = Vec::new();
    for (section, values) in settings.sections() {
        let Some(rest) = section.strip_prefix(PREFIX) else {
            continue;
        };
        let Some((scheme, name)) = rest.split_once(':') else {
            problems.push(format!("[{section}]: expected drive:<scheme>:<name>"));
            continue;
        };
        let label = values.get(LABEL).cloned().unwrap_or_else(|| name.to_owned());
        let params = values.iter().filter_map(|(key, value)| {
            key.strip_prefix(PARAM)
                .map(|stripped| (stripped.to_owned(), value.clone()))
        });
        let built = DriveConfig::new(scheme, name, &label, params).and_then(|config| {
            match values.get(GROUP) {
                Some(group) => config.with_group(group),
                None => Ok(config),
            }
        });
        match built {
            Ok(config) => drives.push(config),
            Err(error) => problems.push(format!("[{section}]: {error}")),
        }
    }
    (drives, problems)
}
