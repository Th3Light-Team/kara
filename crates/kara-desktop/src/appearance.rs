//! How the desktop looks: light or dark, its accent colour and its icon theme.
//!
//! Read from the Settings portal, `org.freedesktop.appearance`, which GNOME
//! and Plasma both implement, and followed live through `SettingChanged`.
//! Qt's own guess goes through the platform theme, and outside Plasma there
//! usually is none: on GNOME the palette Qt reports is light whatever the user
//! chose.
//!
//! Every value is optional. No portal at all gives `None`, and a portal
//! without a key leaves that field unset; the window then falls back to what
//! Qt can tell, which is right on Plasma.

use std::sync::Arc;
use std::thread;

use ashpd::desktop::settings::{ColorScheme as PortalScheme, Settings};
use futures_lite::StreamExt;
use futures_lite::future::block_on;

use crate::session;

const APPEARANCE: &str = "org.freedesktop.appearance";
const GNOME_INTERFACE: &str = "org.gnome.desktop.interface";

/// The user's light/dark preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    /// No preference, which every desktop renders as light.
    NoPreference,
    Dark,
    Light,
}

impl ColorScheme {
    /// Whether windows should be dark.
    #[must_use]
    pub fn is_dark(self) -> bool {
        self == Self::Dark
    }
}

impl From<PortalScheme> for ColorScheme {
    fn from(scheme: PortalScheme) -> Self {
        match scheme {
            PortalScheme::PreferDark => Self::Dark,
            PortalScheme::PreferLight => Self::Light,
            PortalScheme::NoPreference => Self::NoPreference,
        }
    }
}

/// An sRGB colour with components in `[0, 1]`, as the portal sends it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accent {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
}

impl Accent {
    /// The portal says "no accent" with components outside `[0, 1]`.
    #[must_use]
    pub fn from_portal(red: f64, green: f64, blue: f64) -> Option<Self> {
        let valid = |c: f64| c.is_finite() && (0.0..=1.0).contains(&c);
        (valid(red) && valid(green) && valid(blue)).then_some(Self { red, green, blue })
    }

    /// `#rrggbb`, which QML reads as a colour.
    #[must_use]
    pub fn to_hex(self) -> String {
        // In range by construction, so the cast cannot truncate.
        let byte = |c: f64| (c * 255.0).round().clamp(0.0, 255.0) as u8;
        format!("#{:02x}{:02x}{:02x}", byte(self.red), byte(self.green), byte(self.blue))
    }
}

/// What the desktop said, field by field.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Appearance {
    pub color_scheme: Option<ColorScheme>,
    pub accent: Option<Accent>,
    /// The icon theme, where the desktop publishes it through the portal
    /// (GNOME and the GTK desktops). `None` on Plasma on purpose: there the
    /// theme is in `kdeglobals`, which `kara-fs` already reads first.
    pub icon_theme: Option<String>,
}

/// Reads the current values. Blocks for one D-Bus round trip, or up to the
/// bus timeout if a portal is registered but hung: call it off the UI thread.
#[must_use]
pub fn read() -> Option<Appearance> {
    block_on(async {
        let settings = Settings::new().await.ok()?;
        Some(read_with(&settings).await)
    })
}

async fn read_with(settings: &Settings) -> Appearance {
    let color_scheme = settings.color_scheme().await.ok().map(ColorScheme::from);
    let accent = settings
        .accent_color()
        .await
        .ok()
        .and_then(|c| Accent::from_portal(c.red(), c.green(), c.blue()));
    let icon_theme = if session::is_kde() {
        None
    } else {
        settings
            .read::<String>(GNOME_INTERFACE, "icon-theme")
            .await
            .ok()
            .filter(|name| !name.trim().is_empty())
    };
    Appearance {
        color_scheme,
        accent,
        icon_theme,
    }
}

/// Calls `sink` with the new values every time the desktop changes any of
/// them, from a thread of its own, for as long as the session lasts. Returns
/// without doing anything when there is no portal to listen to.
pub fn watch(sink: Arc<dyn Fn(Appearance) + Send + Sync>) {
    let spawned = thread::Builder::new()
        .name("kara-appearance".into())
        .spawn(move || {
            let _ = block_on(async {
                let settings = Settings::new().await?;
                let mut changes = settings.receive_setting_changed().await?;
                while let Some(setting) = changes.next().await {
                    let relevant = setting.namespace() == APPEARANCE
                        || (setting.namespace() == GNOME_INTERFACE && setting.key() == "icon-theme");
                    if relevant {
                        sink(read_with(&settings).await);
                    }
                }
                Ok::<(), ashpd::Error>(())
            });
        });
    // Without a thread there are no live updates; the values read at start-up
    // stay, which is what a desktop without a portal gets anyway.
    drop(spawned);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_outside_the_unit_range_mean_no_accent() {
        assert!(Accent::from_portal(-1.0, -1.0, -1.0).is_none());
        assert!(Accent::from_portal(0.5, 1.5, 0.5).is_none());
        assert!(Accent::from_portal(f64::NAN, 0.0, 0.0).is_none());
    }

    #[test]
    fn gnome_slate_renders_as_its_hex() {
        // What GNOME 50 sends for its «slate» accent on this machine.
        let slate = Accent::from_portal(0.435_294_12, 0.513_725_52, 0.588_235_32);
        assert_eq!(slate.map(Accent::to_hex).as_deref(), Some("#6f8396"));
    }

    #[test]
    fn the_extremes_are_black_and_white() {
        assert_eq!(Accent::from_portal(0.0, 0.0, 0.0).map(Accent::to_hex).as_deref(), Some("#000000"));
        assert_eq!(Accent::from_portal(1.0, 1.0, 1.0).map(Accent::to_hex).as_deref(), Some("#ffffff"));
    }

    #[test]
    fn only_a_dark_preference_is_dark() {
        assert!(ColorScheme::Dark.is_dark());
        assert!(!ColorScheme::Light.is_dark());
        assert!(!ColorScheme::NoPreference.is_dark());
    }
}
