//! View modes and icon zoom.
//!
//! Reference: `ground/spec/03-vistas.md`, "Modos de vista", "Zoom de tamaño de
//! icono con Ctrl+rueda" and "Recordar la vista por carpeta".
//!
//! # Design decisions
//!
//! - **Mode and zoom are one ladder, not two settings.** The spec asks for a
//!   wheel that scales icons *and* falls through into the denser modes once it
//!   runs out of sizes. Modelling that as a mode plus a number means every
//!   caller has to know when to switch one and when the other; as a single
//!   ordered list of rungs, zooming is moving one step and the transition at
//!   the ends is not a special case at all.
//! - **Nothing here knows about pixels on screen.** A rung carries the icon
//!   size it asks for; how wide a cell ends up being is layout, and layout is
//!   the view's problem.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::sort::SortOverrides;

/// How a folder's contents are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ViewMode {
    /// One row per entry, with columns.
    Details,
    /// Names only, flowing top to bottom and then to the right.
    List,
    /// A medium icon with name, type and size beside it.
    Tiles,
    /// A grid of thumbnails with the name underneath.
    Icons,
}

impl ViewMode {
    /// Every mode, in the order a mode picker should offer them.
    #[must_use]
    pub fn all() -> [Self; 4] {
        [Self::Details, Self::List, Self::Tiles, Self::Icons]
    }
}

/// A view mode together with the icon size it is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewSettings {
    pub mode: ViewMode,
    /// Longest side of the icon or thumbnail, in logical pixels.
    pub icon_size: u32,
}

impl ViewSettings {
    #[must_use]
    pub const fn new(mode: ViewMode, icon_size: u32) -> Self {
        Self { mode, icon_size }
    }

    /// The rung a mode starts at when it is picked from a menu or a shortcut.
    #[must_use]
    pub fn for_mode(mode: ViewMode) -> Self {
        LADDER
            .iter()
            .copied()
            .find(|rung| rung.mode == mode && rung.icon_size == default_size(mode))
            .unwrap_or(Self::new(mode, default_size(mode)))
    }

    /// One notch bigger. At the top it stays put rather than wrapping around.
    #[must_use]
    pub fn zoom_in(self) -> Self {
        let next = self.rung() + 1;
        LADDER.get(next).copied().unwrap_or(self)
    }

    /// One notch smaller.
    ///
    /// Past the smallest icons this falls through into the denser modes, which
    /// is what the wheel does in the Explorer: there is no "smaller than the
    /// smallest icon", there is a list.
    #[must_use]
    pub fn zoom_out(self) -> Self {
        match self.rung().checked_sub(1) {
            Some(previous) => LADDER[previous],
            None => self,
        }
    }

    /// Back to the size this mode normally shows, keeping the mode.
    #[must_use]
    pub fn reset_zoom(self) -> Self {
        Self::for_mode(self.mode)
    }

    #[must_use]
    pub fn is_smallest(self) -> bool {
        self.rung() == 0
    }

    #[must_use]
    pub fn is_largest(self) -> bool {
        self.rung() + 1 == LADDER.len()
    }

    /// Where these settings sit on the ladder.
    ///
    /// Settings that are not on it —a size nobody offers— are treated as their
    /// mode's normal rung instead of being rejected: the caller asked for
    /// something reasonable and zooming has to keep working.
    fn rung(self) -> usize {
        LADDER
            .iter()
            .position(|rung| *rung == self)
            .or_else(|| LADDER.iter().position(|rung| rung.mode == self.mode))
            .unwrap_or(0)
    }
}

impl Default for ViewSettings {
    /// Details, which is the mode that survives a folder with ten thousand
    /// files in it.
    fn default() -> Self {
        Self::for_mode(ViewMode::Details)
    }
}

/// The icon size each mode shows when it is chosen directly.
const fn default_size(mode: ViewMode) -> u32 {
    match mode {
        ViewMode::Details | ViewMode::List => 20,
        ViewMode::Tiles => 48,
        ViewMode::Icons => 96,
    }
}

/// Every rung of the zoom, densest first.
///
/// The order deviates from the Explorer's, which puts Tiles and Content *below*
/// Details in the wheel order. That reads as an accident of history rather than
/// a rule, and the spec only requires that running out of icon sizes drops into
/// the denser modes — which this does.
const LADDER: [ViewSettings; 9] = [
    ViewSettings::new(ViewMode::Details, 20),
    ViewSettings::new(ViewMode::List, 20),
    ViewSettings::new(ViewMode::Tiles, 48),
    ViewSettings::new(ViewMode::Icons, 48),
    ViewSettings::new(ViewMode::Icons, 64),
    ViewSettings::new(ViewMode::Icons, 96),
    ViewSettings::new(ViewMode::Icons, 128),
    ViewSettings::new(ViewMode::Icons, 176),
    ViewSettings::new(ViewMode::Icons, 256),
];

/// How many folders keep their own settings before the oldest are forgotten.
///
/// The spec asks for the history to be bounded: the Explorer kept a fixed
/// number of folders and let the oldest fall off. Growing without limit would
/// mean a session that browses a large tree carrying a setting for every
/// directory it ever passed through.
pub const MEMORY_CAPACITY: usize = 256;

// El límite se comprueba al compilar y no en una prueba: una prueba sobre una
// constante no ejerce nada, y lo que hay que impedir es que alguien la deje en
// cero —la memoria dejaría de recordar— o en un millón, que no es un límite.
const _: () = assert!(MEMORY_CAPACITY > 0 && MEMORY_CAPACITY <= 4096);

/// Everything one folder remembers about how it was shown.
///
/// Mode and sorting are stored differently on purpose. A mode is a choice or it
/// is nothing, so it is an `Option` over the global default; sorting already has
/// a type for "this folder decided some of it and inherits the rest", which is
/// what the spec asks for when it wants the folders-first toggle to persist
/// globally *and* per folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderView {
    pub settings: Option<ViewSettings>,
    pub sort: SortOverrides,
}

/// What each folder was left looking like.
#[derive(Debug, Clone)]
pub struct ViewMemory {
    fallback: ViewSettings,
    remembered: HashMap<PathBuf, FolderView>,
    /// Least recently remembered first, so eviction knows what to drop.
    order: Vec<PathBuf>,
    capacity: usize,
}

impl Default for ViewMemory {
    fn default() -> Self {
        Self::new(ViewSettings::default(), MEMORY_CAPACITY)
    }
}

impl ViewMemory {
    #[must_use]
    pub fn new(fallback: ViewSettings, capacity: usize) -> Self {
        Self {
            fallback,
            remembered: HashMap::new(),
            order: Vec::new(),
            capacity,
        }
    }

    /// The global default, used by every folder nobody has configured.
    #[must_use]
    pub fn fallback(&self) -> ViewSettings {
        self.fallback
    }

    pub fn set_fallback(&mut self, settings: ViewSettings) {
        self.fallback = settings;
    }

    /// How a folder should be shown.
    #[must_use]
    pub fn settings_for(&self, folder: &Path) -> ViewSettings {
        self.remembered
            .get(folder)
            .and_then(|remembered| remembered.settings)
            .unwrap_or(self.fallback)
    }

    /// What this folder decided about sorting. Empty means it follows the
    /// global default in everything.
    #[must_use]
    pub fn sort_for(&self, folder: &Path) -> SortOverrides {
        self.remembered
            .get(folder)
            .map(|remembered| remembered.sort.clone())
            .unwrap_or_default()
    }

    /// Records the mode and zoom a folder was left in, without disturbing what
    /// it had decided about sorting.
    pub fn remember(&mut self, folder: &Path, settings: ViewSettings) {
        self.touch(folder, |remembered| remembered.settings = Some(settings));
    }

    /// Records what a folder decided about sorting, without disturbing its mode.
    pub fn remember_sort(&mut self, folder: &Path, sort: SortOverrides) {
        self.touch(folder, |remembered| remembered.sort = sort);
    }

    /// Brings a folder to the front of the history and lets the caller change
    /// it, evicting the oldest if that overflows the bound.
    fn touch(&mut self, folder: &Path, change: impl FnOnce(&mut FolderView)) {
        if self.capacity == 0 {
            return;
        }

        self.order.retain(|kept| kept != folder);
        self.order.push(folder.to_path_buf());
        change(self.remembered.entry(folder.to_path_buf()).or_default());

        while self.order.len() > self.capacity {
            let oldest = self.order.remove(0);
            self.remembered.remove(&oldest);
        }
    }

    /// Forgets one folder, which then falls back to the global default.
    pub fn forget(&mut self, folder: &Path) {
        self.order.retain(|kept| kept != folder);
        self.remembered.remove(folder);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.remembered.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.remembered.is_empty()
    }
}
