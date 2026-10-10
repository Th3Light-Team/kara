//! Adding many drives at once: a CSV list, or an OpenSSH `config`.
//!
//! Both are pure text-to-config functions. They never connect, never touch the
//! disk and never see a secret: a fleet's shared password is set separately on
//! the group (`DriveRegistry::remember_group_secret`). A line that cannot be used
//! is reported and the rest are still imported.

use std::collections::BTreeSet;

use crate::config::DriveConfig;

/// What an import produced.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub drives: Vec<DriveConfig>,
    /// One line per entry that was skipped, with the reason.
    pub problems: Vec<String>,
}

impl ImportReport {
    fn push(&mut self, seen: &mut BTreeSet<String>, line: usize, config: Result<DriveConfig, String>) {
        match config {
            Ok(config) if seen.insert(config.id.name().to_owned()) => self.drives.push(config),
            Ok(config) => self
                .problems
                .push(format!("line {line}: duplicate name {:?}", config.id.name())),
            Err(reason) => self.problems.push(format!("line {line}: {reason}")),
        }
    }
}

fn valid_port(text: &str) -> bool {
    text.parse::<u16>().is_ok_and(|port| port != 0)
}

fn build(
    scheme: &str,
    name: &str,
    label: &str,
    group: Option<&str>,
    params: &[(&str, &str)],
) -> Result<DriveConfig, String> {
    let params = params
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()));
    let mut config = DriveConfig::new(scheme, name, label, params).map_err(|e| e.to_string())?;
    if let Some(group) = group.filter(|group| !group.is_empty()) {
        config = config.with_group(group).map_err(|e| e.to_string())?;
    }
    Ok(config)
}

/// Parses `name,host[,user[,port[,key_file[,label[,group]]]]]`, one drive per line.
///
/// Blank lines and lines starting with `#` are skipped, and so is a first line
/// that starts with `name,` (a header). Fields are split on commas without any
/// quoting, so a value cannot contain one. `default_group` applies to every line
/// whose own group field is empty.
#[must_use]
pub fn from_csv(text: &str, scheme: &str, default_group: Option<&str>) -> ImportReport {
    let mut report = ImportReport::default();
    let mut seen = BTreeSet::new();
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        if number == 1 && fields.first().is_some_and(|f| f.eq_ignore_ascii_case("name")) {
            continue;
        }
        let field = |i: usize| fields.get(i).copied().unwrap_or("");
        if field(0).is_empty() || field(1).is_empty() {
            report.problems.push(format!("line {number}: name and host are required"));
            continue;
        }
        if !field(3).is_empty() && !valid_port(field(3)) {
            report.problems.push(format!("line {number}: bad port {:?}", field(3)));
            continue;
        }
        let label = if field(5).is_empty() { field(0) } else { field(5) };
        let group = if field(6).is_empty() { default_group } else { Some(field(6)) };
        let config = build(
            scheme,
            field(0),
            label,
            group,
            &[
                ("host", field(1)),
                ("user", field(2)),
                ("port", field(3)),
                ("key_file", field(4)),
            ],
        );
        report.push(&mut seen, number, config);
    }
    report
}

/// A drive name (a DNS label) from an `ssh_config` alias: lowercase, anything
/// else becomes `-`, runs collapse, ends are trimmed, at most 63 characters.
fn name_from_alias(alias: &str) -> String {
    let mut name = String::new();
    for c in alias.chars().flat_map(char::to_lowercase) {
        let c = if c.is_ascii_alphanumeric() { c } else { '-' };
        if c == '-' && (name.is_empty() || name.ends_with('-')) {
            continue;
        }
        name.push(c);
    }
    let trimmed = name.trim_end_matches('-');
    trimmed.chars().take(63).collect::<String>().trim_end_matches('-').to_owned()
}

#[derive(Default)]
struct Block {
    start_line: usize,
    aliases: Vec<String>,
    host_name: Option<String>,
    user: Option<String>,
    port: Option<String>,
    identity: Option<String>,
    proxied: bool,
}

fn finish(block: &mut Block, scheme: &str, group: Option<&str>, report: &mut ImportReport, seen: &mut BTreeSet<String>) {
    let finished = std::mem::take(block);
    for alias in &finished.aliases {
        let line = finished.start_line;
        if finished.proxied {
            report
                .problems
                .push(format!("line {line}: {alias:?} uses ProxyJump/ProxyCommand, which is not supported"));
            continue;
        }
        let name = name_from_alias(alias);
        if name.is_empty() {
            report.problems.push(format!("line {line}: {alias:?} cannot be used as a drive name"));
            continue;
        }
        let port = finished.port.as_deref().unwrap_or("");
        if !port.is_empty() && !valid_port(port) {
            report.problems.push(format!("line {line}: bad port {port:?} for {alias:?}"));
            continue;
        }
        let host = finished.host_name.as_deref().unwrap_or(alias);
        let config = build(
            scheme,
            &name,
            alias,
            group,
            &[
                ("host", host),
                ("user", finished.user.as_deref().unwrap_or("")),
                ("port", port),
                ("key_file", finished.identity.as_deref().unwrap_or("")),
            ],
        );
        report.push(seen, line, config);
    }
}

/// Imports the concrete `Host` entries of an OpenSSH client config.
///
/// Understood: `Host` (several aliases allowed), `HostName`, `User`, `Port`,
/// `IdentityFile` (the first one). Skipped: wildcard and negated patterns,
/// `Match` blocks, `Include`, and hosts reached through `ProxyJump` or
/// `ProxyCommand` (reported, since Kara cannot reach them).
#[must_use]
pub fn from_ssh_config(text: &str, scheme: &str, group: Option<&str>) -> ImportReport {
    let mut report = ImportReport::default();
    let mut seen = BTreeSet::new();
    let mut block = Block::default();
    let mut inside_match = false;
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (keyword, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((keyword, value)) => (keyword, value.trim().trim_start_matches('=').trim()),
            None => (line, ""),
        };
        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                finish(&mut block, scheme, group, &mut report, &mut seen);
                inside_match = false;
                block.start_line = number;
                block.aliases = value
                    .split_whitespace()
                    .filter(|alias| !alias.contains(['*', '?', '!']))
                    .map(str::to_owned)
                    .collect();
            }
            "match" => {
                finish(&mut block, scheme, group, &mut report, &mut seen);
                inside_match = true;
            }
            _ if inside_match || block.aliases.is_empty() => {}
            "hostname" => block.host_name = Some(value.to_owned()),
            "user" => block.user = Some(value.to_owned()),
            "port" => block.port = Some(value.to_owned()),
            "identityfile" if block.identity.is_none() => {
                block.identity = Some(value.trim_matches('"').to_owned());
            }
            "proxyjump" | "proxycommand" => block.proxied = true,
            _ => {}
        }
    }
    finish(&mut block, scheme, group, &mut report, &mut seen);
    report
}
