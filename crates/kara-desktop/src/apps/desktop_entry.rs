//! `.desktop` files, per the Desktop Entry Specification 1.5.
//!
//! Only what choosing and launching an application needs is read: the
//! `[Desktop Entry]` group of a `Type=Application`. Actions, keywords and the
//! rest are skipped.

use std::path::PathBuf;

/// An installed application, as its `.desktop` file describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    /// The desktop file ID: `org.gnome.TextEditor.desktop`. Unique, and what
    /// `mimeapps.list` refers to.
    pub id: String,
    /// The name in the first of the requested languages that has one.
    pub name: String,
    pub generic_name: Option<String>,
    /// An icon theme name, or an absolute path.
    pub icon: Option<String>,
    /// The `Exec` key, already unescaped as a string; [`super::exec`] splits it.
    pub exec: Option<String>,
    /// A program that must exist for the entry to count as installed.
    pub try_exec: Option<String>,
    /// The working directory to start it in.
    pub path: Option<PathBuf>,
    pub terminal: bool,
    /// Not meant for menus. It still handles the types it declares.
    pub no_display: bool,
    pub dbus_activatable: bool,
    pub only_show_in: Vec<String>,
    pub not_show_in: Vec<String>,
    pub mime_types: Vec<String>,
    /// Where the file is, for `%k`.
    pub source: PathBuf,
}

impl AppInfo {
    /// Whether the entry is meant to be shown on a desktop named by
    /// `desktops` (lower-case `XDG_CURRENT_DESKTOP` names).
    #[must_use]
    pub fn shown_in(&self, desktops: &[String]) -> bool {
        let named = |list: &[String]| {
            list.iter()
                .any(|entry| desktops.iter().any(|desktop| entry.eq_ignore_ascii_case(desktop)))
        };
        if !self.only_show_in.is_empty() && !named(&self.only_show_in) {
            return false;
        }
        !named(&self.not_show_in)
    }
}

/// The result of reading one `.desktop` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// An application.
    App(Box<AppInfo>),
    /// `Hidden=true`: the user deleted it. It also hides every file with the
    /// same ID further down the data directories.
    Deleted,
    /// Not an application (a link, a directory) or not a valid file. Ignored,
    /// and it does not hide anything.
    Other,
}

/// Parses a `.desktop` file. `languages` are locale names in order of
/// preference (`es_ES`, `es`); `id` and `source` say what the file is.
#[must_use]
pub fn parse(text: &str, id: &str, source: PathBuf, languages: &[String]) -> Parsed {
    let mut in_entry = false;
    let mut seen_entry = false;
    let mut keys: Vec<(&str, &str)> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            // The main group comes first; once it ends, nothing else matters.
            if in_entry {
                break;
            }
            in_entry = line == "[Desktop Entry]";
            seen_entry |= in_entry;
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            keys.push((key.trim_end(), value.trim_start()));
        }
    }
    if !seen_entry {
        return Parsed::Other;
    }

    let raw = |key: &str| keys.iter().rev().find(|(k, _)| *k == key).map(|(_, v)| *v);
    let string = |key: &str| raw(key).map(unescape);
    let boolean = |key: &str| raw(key).is_some_and(|v| v.trim() == "true");
    let list = |key: &str| raw(key).map(split_list).unwrap_or_default();

    if boolean("Hidden") {
        return Parsed::Deleted;
    }
    if raw("Type").map(str::trim) != Some("Application") {
        return Parsed::Other;
    }
    let Some(name) = localized(&keys, "Name", languages) else {
        return Parsed::Other;
    };

    Parsed::App(Box::new(AppInfo {
        id: id.to_string(),
        name,
        generic_name: localized(&keys, "GenericName", languages),
        icon: string("Icon").filter(|s| !s.is_empty()),
        exec: string("Exec").filter(|s| !s.trim().is_empty()),
        try_exec: string("TryExec").filter(|s| !s.trim().is_empty()),
        path: string("Path").filter(|s| !s.is_empty()).map(PathBuf::from),
        terminal: boolean("Terminal"),
        no_display: boolean("NoDisplay"),
        dbus_activatable: boolean("DBusActivatable"),
        only_show_in: list("OnlyShowIn"),
        not_show_in: list("NotShowIn"),
        mime_types: list("MimeType"),
        source,
    }))
}

/// A localized key: `Name[es_ES]`, then `Name[es]`, …, then plain `Name`,
/// following the spec's matching order for each language in turn.
fn localized(keys: &[(&str, &str)], key: &str, languages: &[String]) -> Option<String> {
    let find = |wanted: &str| keys.iter().rev().find(|(k, _)| *k == wanted).map(|(_, v)| *v);
    for language in languages {
        for variant in locale_variants(language) {
            if let Some(value) = find(&format!("{key}[{variant}]")) {
                let value = unescape(value);
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
    }
    find(key).map(unescape).filter(|v| !v.is_empty())
}

/// `lang_COUNTRY.ENCODING@MODIFIER` → the forms to try, most specific first:
/// `lang_COUNTRY@MODIFIER`, `lang_COUNTRY`, `lang@MODIFIER`, `lang`. The
/// encoding never takes part in matching.
#[must_use]
pub fn locale_variants(locale: &str) -> Vec<String> {
    let (rest, modifier) = match locale.split_once('@') {
        Some((rest, modifier)) => (rest, Some(modifier)),
        None => (locale, None),
    };
    let rest = rest.split('.').next().unwrap_or(rest);
    let (lang, country) = match rest.split_once('_') {
        Some((lang, country)) => (lang, Some(country)),
        None => (rest, None),
    };
    if lang.is_empty() || lang == "C" || lang == "POSIX" {
        return Vec::new();
    }

    let mut out = Vec::new();
    if let (Some(country), Some(modifier)) = (country, modifier) {
        out.push(format!("{lang}_{country}@{modifier}"));
    }
    if let Some(country) = country {
        out.push(format!("{lang}_{country}"));
    }
    if let Some(modifier) = modifier {
        out.push(format!("{lang}@{modifier}"));
    }
    out.push(lang.to_string());
    out
}

/// Undoes the escapes a `string` value may contain: `\s`, `\n`, `\t`, `\r`
/// and `\\`. Anything else after a backslash is kept as written.
#[must_use]
pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Splits a `;`-separated list. `\;` is a literal semicolon inside an item.
#[must_use]
pub fn split_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&';') => {
                current.push(';');
                chars.next();
            }
            ';' => {
                items.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    items.push(current);
    items
        .into_iter()
        .map(|item| unescape(item.trim()))
        .filter(|item| !item.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(text: &str, languages: &[&str]) -> AppInfo {
        let languages: Vec<String> = languages.iter().map(|l| (*l).to_string()).collect();
        match parse(text, "test.desktop", PathBuf::from("/x/test.desktop"), &languages) {
            Parsed::App(app) => *app,
            other => panic!("expected an application, got {other:?}"),
        }
    }

    #[test]
    fn the_name_follows_the_language_and_falls_back_to_the_plain_key() {
        let text = "[Desktop Entry]\nType=Application\nName=Text Editor\nName[es]=Editor de texto\nName[es_MX]=Editor (MX)\nExec=x\n";
        assert_eq!(app(text, &["es_ES"]).name, "Editor de texto");
        assert_eq!(app(text, &["es_MX.UTF-8"]).name, "Editor (MX)");
        assert_eq!(app(text, &["de"]).name, "Text Editor");
        assert_eq!(app(text, &[]).name, "Text Editor");
    }

    #[test]
    fn keys_of_other_groups_do_not_leak_into_the_entry() {
        let text = "[Desktop Entry]\nType=Application\nName=App\nExec=app %U\n\n[Desktop Action new-window]\nName=New Window\nExec=app --new-window\n";
        let parsed = app(text, &[]);
        assert_eq!(parsed.name, "App");
        assert_eq!(parsed.exec.as_deref(), Some("app %U"));
    }

    #[test]
    fn mime_types_split_on_semicolons_and_ignore_the_trailing_one() {
        let text = "[Desktop Entry]\nType=Application\nName=A\nExec=a\nMimeType=text/plain;image/png;\n";
        assert_eq!(app(text, &[]).mime_types, ["text/plain", "image/png"]);
    }

    #[test]
    fn an_escaped_semicolon_stays_inside_its_item() {
        assert_eq!(split_list(r"a\;b;c;"), ["a;b", "c"]);
    }

    #[test]
    fn hidden_means_deleted_and_links_are_not_applications() {
        let hidden = "[Desktop Entry]\nType=Application\nName=A\nHidden=true\n";
        assert_eq!(parse(hidden, "a.desktop", PathBuf::new(), &[]), Parsed::Deleted);
        let link = "[Desktop Entry]\nType=Link\nName=A\nURL=https://example.org\n";
        assert_eq!(parse(link, "a.desktop", PathBuf::new(), &[]), Parsed::Other);
        assert_eq!(parse("garbage", "a.desktop", PathBuf::new(), &[]), Parsed::Other);
    }

    #[test]
    fn string_escapes_are_undone() {
        assert_eq!(unescape(r"a\sb\\c\nd"), "a b\\c\nd");
        assert_eq!(unescape(r"trailing\"), "trailing\\");
    }

    #[test]
    fn only_and_not_show_in_compare_case_insensitively() {
        let text = "[Desktop Entry]\nType=Application\nName=A\nExec=a\nOnlyShowIn=GNOME;Unity;\n";
        let parsed = app(text, &[]);
        assert!(parsed.shown_in(&["ubuntu".into(), "gnome".into()]));
        assert!(!parsed.shown_in(&["kde".into()]));

        let text = "[Desktop Entry]\nType=Application\nName=A\nExec=a\nNotShowIn=KDE;\n";
        let parsed = app(text, &[]);
        assert!(!parsed.shown_in(&["kde".into()]));
        assert!(parsed.shown_in(&["gnome".into()]));
    }

    #[test]
    fn c_and_posix_locales_add_no_variants() {
        assert!(locale_variants("C.UTF-8").is_empty());
        assert!(locale_variants("POSIX").is_empty());
        assert_eq!(locale_variants("sr_RS@latin"), ["sr_RS@latin", "sr_RS", "sr@latin", "sr"]);
    }
}
