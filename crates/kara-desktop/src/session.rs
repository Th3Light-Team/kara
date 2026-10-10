//! Which desktop Kara is running in.
//!
//! Only consulted where a standard leaves room for desktop-specific data — the
//! icon theme name — never to switch behaviour wholesale: the point of this
//! crate is that the same code serves GNOME and Plasma.

/// The desktops named in `XDG_CURRENT_DESKTOP`, lower-cased, most specific
/// first (`ubuntu:GNOME` → `["ubuntu", "gnome"]`).
#[must_use]
pub fn current_desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|value| parse_current_desktop(&value))
        .unwrap_or_default()
}

/// Splits an `XDG_CURRENT_DESKTOP` value.
#[must_use]
pub fn parse_current_desktop(value: &str) -> Vec<String> {
    value
        .split(':')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// Whether this is a Plasma session.
#[must_use]
pub fn is_kde() -> bool {
    current_desktops().iter().any(|desktop| desktop == "kde")
}

#[cfg(test)]
mod tests {
    use super::parse_current_desktop;

    #[test]
    fn ubuntu_lists_itself_before_gnome() {
        assert_eq!(parse_current_desktop("ubuntu:GNOME"), ["ubuntu", "gnome"]);
    }

    #[test]
    fn empty_entries_are_dropped() {
        assert_eq!(parse_current_desktop(":KDE::"), ["kde"]);
        assert!(parse_current_desktop("").is_empty());
    }
}
