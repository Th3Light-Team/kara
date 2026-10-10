//! What Kara remembers between sessions.
//!
//! A thin, typed face over `kara_fs::settings`, which is a plain key-value
//! box. The names of the keys and how each value is written live here, in one
//! place, because they are a format: once a user has a settings file, changing
//! a key name silently loses whatever it held.
//!
//! # Design decisions
//!
//! - **Nothing here is fatal.** Settings that cannot be read leave every value
//!   at its default, and settings that cannot be written are reported once and
//!   the session carries on. A file manager that refuses to open because its
//!   preferences are unreadable would be worse than one that forgets them.
//! - **Values are written by name, never by ordinal.** `ViewMode` and
//!   `SortSpec` both serialize themselves; reordering an enum must not
//!   reinterpret a file already on disk.
//! - **Pinned folders are stored as values, not as keys.** A path may contain
//!   `=` and the parser splits a line on its first one, so a path used as a key
//!   would come back truncated. Numbered keys keep the order and put the path
//!   where escaping applies.

use std::path::{Path, PathBuf};

use kara_core::sort::SortSpec;
use kara_core::view::{ViewMode, ViewSettings};
use kara_fs::settings::{LoadOutcome, Settings, SettingsError};

const WINDOW: &str = "window";
const VIEW: &str = "view";
const PINNED: &str = "pinned";

/// Bounds on the sidebar width, so a corrupt or hand-edited value cannot leave
/// the panel invisible or wider than the window.
const SIDEBAR_MIN: i32 = 160;
const SIDEBAR_MAX: i32 = 480;
const SIDEBAR_DEFAULT: i32 = 240;

/// The settings file, already parsed.
#[derive(Debug)]
pub struct Prefs {
    /// `None` when there is nowhere to write: without `$HOME` there is no
    /// config directory, and the session simply does not remember anything.
    path: Option<PathBuf>,
    settings: Settings,
    /// Why the file on disk could not be used, if that is what happened. Kept
    /// so the window can say it once instead of failing silently.
    complaint: Option<String>,
}

impl Prefs {
    /// Reads the settings file, or starts empty if there is not one yet.
    #[must_use]
    pub fn load() -> Self {
        let path = match kara_fs::settings::default_path() {
            Ok(path) => path,
            Err(error) => {
                return Self {
                    path: None,
                    settings: Settings::new(),
                    complaint: Some(format!("No hay dónde guardar los ajustes: {error}")),
                };
            }
        };

        let loaded = kara_fs::settings::load(&path);
        // A corrupt file is left exactly as it is: overwriting it would throw
        // away whatever the user had, and they may want to fix it by hand.
        let complaint = match &loaded.outcome {
            LoadOutcome::Corrupt { reason } => Some(format!(
                "Los ajustes de {} no se pudieron leer ({reason}); esta sesión usa los valores por defecto",
                path.display()
            )),
            _ => None,
        };

        Self {
            path: Some(path),
            settings: loaded.settings,
            complaint,
        }
    }

    /// Builds a set of preferences over an existing store, for tests.
    #[cfg(test)]
    #[must_use]
    pub fn from_settings(settings: Settings) -> Self {
        Self {
            path: None,
            settings,
            complaint: None,
        }
    }

    /// What went wrong reading the file, if anything. Reported once.
    pub fn take_complaint(&mut self) -> Option<String> {
        self.complaint.take()
    }

    /// Writes the file. Failing to save is reported, never fatal.
    pub fn save(&self) -> Result<(), SettingsError> {
        match &self.path {
            Some(path) => {
                // The drives panel writes its own `[drive:*]` sections to this
                // file; keep what is on disk instead of erasing it with the
                // older copy held here.
                let mut merged = self.settings.clone();
                crate::remote_store::carry_drives(&kara_fs::settings::load(path).settings, &mut merged);
                kara_fs::settings::save(path, &merged)
            }
            None => Ok(()),
        }
    }

    fn number(&self, section: &str, key: &str) -> Option<i32> {
        self.settings.get(section, key)?.trim().parse().ok()
    }

    fn flag(&self, section: &str, key: &str) -> Option<bool> {
        match self.settings.get(section, key)?.trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }

    #[must_use]
    pub fn sidebar_width(&self) -> i32 {
        self.number(WINDOW, "sidebar_width")
            .unwrap_or(SIDEBAR_DEFAULT)
            .clamp(SIDEBAR_MIN, SIDEBAR_MAX)
    }

    pub fn set_sidebar_width(&mut self, width: i32) {
        let width = width.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        self.settings
            .set(WINDOW, "sidebar_width", width.to_string());
    }

    #[must_use]
    pub fn sidebar_visible(&self) -> bool {
        self.flag(WINDOW, "sidebar_visible").unwrap_or(true)
    }

    pub fn set_sidebar_visible(&mut self, visible: bool) {
        self.settings
            .set(WINDOW, "sidebar_visible", visible.to_string());
    }

    /// Whether the window opens in focus mode, with the tab bar hidden.
    ///
    /// Default is on: a window that has never had a second tab should look
    /// exactly like one that cannot have them.
    #[must_use]
    pub fn focus_mode(&self) -> bool {
        self.flag(WINDOW, "focus_mode").unwrap_or(true)
    }

    pub fn set_focus_mode(&mut self, on: bool) {
        self.settings.set(WINDOW, "focus_mode", on.to_string());
    }

    /// Whether hidden files (leading dot, or listed in a folder's `.hidden`)
    /// are shown. Off by default, like every desktop file manager.
    #[must_use]
    pub fn show_hidden(&self) -> bool {
        self.flag(VIEW, "show_hidden").unwrap_or(false)
    }

    pub fn set_show_hidden(&mut self, on: bool) {
        self.settings.set(VIEW, "show_hidden", on.to_string());
    }

    /// Whether file names are shown with their extension. On by default: it is
    /// what Dolphin and Nautilus do, and hiding them is an opt-in.
    #[must_use]
    pub fn show_extensions(&self) -> bool {
        self.flag(VIEW, "show_extensions").unwrap_or(true)
    }

    pub fn set_show_extensions(&mut self, on: bool) {
        self.settings.set(VIEW, "show_extensions", on.to_string());
    }

    /// The view every folder that has not been configured is shown with.
    #[must_use]
    pub fn default_view(&self) -> ViewSettings {
        let mode = self
            .settings
            .get(VIEW, "mode")
            .and_then(|text| text.parse::<ViewMode>().ok());
        let Some(mode) = mode else {
            return ViewSettings::default();
        };

        match self.number(VIEW, "icon_size") {
            // A size nobody offers is not rejected: the ladder treats it as its
            // mode's normal rung, which is what a hand-edited file deserves.
            Some(size) if size > 0 => ViewSettings::new(mode, size.unsigned_abs()),
            _ => ViewSettings::for_mode(mode),
        }
    }

    pub fn set_default_view(&mut self, view: ViewSettings) {
        self.settings.set(VIEW, "mode", view.mode.to_string());
        self.settings
            .set(VIEW, "icon_size", view.icon_size.to_string());
    }

    /// The sort every folder that has not chosen one inherits.
    ///
    /// Read from the file but never written yet: the only gesture that changes
    /// a sort today is a click on a column header, and that one is *per
    /// folder*. Writing it to the global default as well would make sorting
    /// one folder by size quietly re-sort every folder that had not chosen —
    /// `SortOverrides` stores "same as the default" as "inherit", by design.
    /// A deliberate "apply to all folders" action is what should write it.
    #[must_use]
    pub fn sort_defaults(&self) -> SortSpec {
        self.settings
            .get(VIEW, "sort")
            .and_then(|text| text.parse::<SortSpec>().ok())
            .unwrap_or_default()
    }

    /// Folders the user pinned to Quick Access, in their order.
    #[must_use]
    pub fn pinned(&self) -> Vec<PathBuf> {
        let Some(values) = self.settings.sections().get(PINNED) else {
            return Vec::new();
        };

        let mut numbered: Vec<(u32, &String)> = values
            .iter()
            .filter_map(|(key, value)| Some((key.trim().parse::<u32>().ok()?, value)))
            .collect();
        numbered.sort_by_key(|(order, _)| *order);
        numbered
            .into_iter()
            .map(|(_, value)| PathBuf::from(value))
            .filter(|path| path.is_absolute())
            .collect()
    }

    pub fn set_pinned(&mut self, paths: &[PathBuf]) {
        // The whole section is rewritten rather than patched: leaving stale
        // numbered keys behind would resurrect folders the user unpinned.
        self.settings.remove_section(PINNED);
        for (order, path) in paths.iter().enumerate() {
            self.settings
                .set(PINNED, &order.to_string(), path.to_string_lossy().into_owned());
        }
    }

    /// Adds a folder to Quick Access. Pinning something already pinned does
    /// nothing rather than listing it twice.
    pub fn pin(&mut self, path: &Path) {
        let mut pinned = self.pinned();
        if pinned.iter().any(|kept| kept == path) {
            return;
        }
        pinned.push(path.to_path_buf());
        self.set_pinned(&pinned);
    }

    pub fn unpin(&mut self, path: &Path) {
        let pinned: Vec<PathBuf> = self
            .pinned()
            .into_iter()
            .filter(|kept| kept != path)
            .collect();
        self.set_pinned(&pinned);
    }

    #[must_use]
    pub fn is_pinned(&self, path: &Path) -> bool {
        self.pinned().iter().any(|kept| kept == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kara_core::sort::{SortKey, SortOrder};

    fn empty() -> Prefs {
        Prefs::from_settings(Settings::new())
    }

    #[test]
    fn an_empty_file_gives_every_default() {
        let prefs = empty();
        assert_eq!(prefs.sidebar_width(), SIDEBAR_DEFAULT);
        assert!(prefs.sidebar_visible());
        assert_eq!(prefs.default_view(), ViewSettings::default());
        assert_eq!(prefs.sort_defaults(), SortSpec::default());
        assert!(prefs.pinned().is_empty());
    }

    #[test]
    fn hidden_files_are_off_and_extensions_on_until_the_user_says_otherwise() {
        let mut prefs = empty();
        assert!(!prefs.show_hidden());
        assert!(prefs.show_extensions());

        prefs.set_show_hidden(true);
        prefs.set_show_extensions(false);
        assert!(prefs.show_hidden());
        assert!(!prefs.show_extensions());
    }

    #[test]
    fn the_view_survives_a_round_trip_by_name() {
        let mut prefs = empty();
        let chosen = ViewSettings::new(ViewMode::Icons, 176);
        prefs.set_default_view(chosen);

        assert_eq!(prefs.default_view(), chosen);
        assert_eq!(prefs.settings.get(VIEW, "mode"), Some("icons"));
    }

    #[test]
    fn a_hand_written_sort_criterion_is_read_back() {
        let sort = SortSpec {
            key: SortKey::Size,
            order: SortOrder::Descending,
            ..SortSpec::default()
        };

        let mut settings = Settings::new();
        settings.set(VIEW, "sort", sort.to_string());

        assert_eq!(Prefs::from_settings(settings).sort_defaults(), sort);
    }

    #[test]
    fn a_hand_edited_mode_nobody_knows_falls_back_instead_of_guessing() {
        let mut settings = Settings::new();
        settings.set(VIEW, "mode", "contenido");
        let prefs = Prefs::from_settings(settings);

        assert_eq!(prefs.default_view(), ViewSettings::default());
    }

    #[test]
    fn an_absurd_sidebar_width_is_clamped_not_obeyed() {
        // A hand-edited or corrupt value must not be able to leave the panel
        // invisible or wider than the window.
        let mut settings = Settings::new();
        settings.set(WINDOW, "sidebar_width", "20000");
        assert_eq!(Prefs::from_settings(settings).sidebar_width(), SIDEBAR_MAX);

        let mut settings = Settings::new();
        settings.set(WINDOW, "sidebar_width", "-5");
        assert_eq!(Prefs::from_settings(settings).sidebar_width(), SIDEBAR_MIN);

        let mut settings = Settings::new();
        settings.set(WINDOW, "sidebar_width", "no soy un número");
        assert_eq!(
            Prefs::from_settings(settings).sidebar_width(),
            SIDEBAR_DEFAULT
        );
    }

    #[test]
    fn pinned_folders_keep_the_order_they_were_pinned_in() {
        let mut prefs = empty();
        prefs.pin(Path::new("/home/ana/Proyectos"));
        prefs.pin(Path::new("/home/ana/Fotos"));

        assert_eq!(
            prefs.pinned(),
            vec![
                PathBuf::from("/home/ana/Proyectos"),
                PathBuf::from("/home/ana/Fotos")
            ]
        );
    }

    #[test]
    fn pinning_twice_does_not_list_it_twice() {
        let mut prefs = empty();
        prefs.pin(Path::new("/home/ana/Fotos"));
        prefs.pin(Path::new("/home/ana/Fotos"));

        assert_eq!(prefs.pinned().len(), 1);
        assert!(prefs.is_pinned(Path::new("/home/ana/Fotos")));
    }

    #[test]
    fn unpinning_shortens_the_list_without_leaving_a_stale_tail() {
        // The failure this guards against: rewriting the numbered keys in
        // place leaves the last entry of the longer list behind, and it comes
        // back on the next load.
        let mut prefs = empty();
        prefs.pin(Path::new("/a"));
        prefs.pin(Path::new("/b"));
        prefs.pin(Path::new("/c"));
        prefs.unpin(Path::new("/b"));

        assert_eq!(prefs.pinned(), vec![PathBuf::from("/a"), PathBuf::from("/c")]);

        prefs.unpin(Path::new("/a"));
        prefs.unpin(Path::new("/c"));
        assert!(prefs.pinned().is_empty(), "nothing may survive the last unpin");
    }

    #[test]
    fn a_relative_pinned_path_is_dropped_rather_than_resolved() {
        // It could only have come from a hand-edited file, and resolving it
        // against the current directory would point somewhere arbitrary.
        let mut settings = Settings::new();
        settings.set(PINNED, "0", "Documentos");
        settings.set(PINNED, "1", "/home/ana/Fotos");

        assert_eq!(
            Prefs::from_settings(settings).pinned(),
            vec![PathBuf::from("/home/ana/Fotos")]
        );
    }

    #[test]
    fn pinned_entries_are_read_in_numeric_order_not_alphabetical() {
        // With ten or more, "10" sorts before "2" as text.
        let mut settings = Settings::new();
        settings.set(PINNED, "2", "/segundo");
        settings.set(PINNED, "10", "/decimo");
        settings.set(PINNED, "1", "/primero");

        assert_eq!(
            Prefs::from_settings(settings).pinned(),
            vec![
                PathBuf::from("/primero"),
                PathBuf::from("/segundo"),
                PathBuf::from("/decimo")
            ]
        );
    }
}
