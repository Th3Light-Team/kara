//! Tab model: several independent folder views inside one window.
//!
//! Reference conveniences: `ground/spec/01-navegacion.md`, "Pestañas de
//! carpetas", "Reabrir pestaña cerrada" and "Abrir carpeta en pestaña o
//! ventana nueva". Shortcuts: `ground/spec/07-atajos-teclado.md`, section
//! "Pestañas y paneles".
//!
//! Pure logic, like the rest of `kara-core`: no I/O and no windows. [`Tabs`]
//! models the tab bar of a **single** window; "open in a new window" is, from
//! here, simply constructing another [`Tabs`] elsewhere — this module
//! neither creates nor closes windows.
//!
//! # Design decisions
//!
//! - **Each tab owns its own [`History`].** That's already the piece that
//!   carries back/forward and the view state for a location; a tab is
//!   nothing more than a `History` with an identity. It isn't shared between
//!   tabs — duplicating a tab clones its `History`, it doesn't reference it.
//! - **Tabs are identified by [`TabId`], not by position.** Closing,
//!   reordering or navigating another tab keeps shifting indices around; if
//!   "the active tab" were an index, every one of those operations would
//!   have to recompute it by hand, and one slip would leave the focus
//!   jumping to the wrong tab. With an opaque identifier, "focus follows the
//!   moved tab, not the position" (the rule drag-to-reorder asks for) comes
//!   for free: moving never touches `active` at all.
//! - **The collection is never empty.** Just like [`History`] never runs out
//!   of a current entry, [`Tabs`] always has at least one tab. Closing the
//!   last one doesn't remove it: it returns [`CloseOutcome::LastTab`] without
//!   mutating anything. This module doesn't close windows, so "what closing
//!   the last tab means" (close the window? fall back to Home?) is a
//!   decision for whoever actually manages those; this module only signals
//!   the edge case explicitly so that layer can decide.
//! - **Closing the active tab focuses whatever slides into its slot —
//!   browser style.** If there was a tab to the right, it inherits focus; if
//!   the closed one was the last, focus falls back to the new last tab. The
//!   spec already leans on browser conventions for this family of shortcuts
//!   (Ctrl+T/Ctrl+W, Ctrl+Shift+T, Ctrl+9 "browser style"), so that's the
//!   most consistent reading for this case too, which the spec leaves open.
//! - **The "reopen" stack is bounded** ([`REOPEN_CAPACITY`]), with the same
//!   trimming pattern as [`crate::view::ViewMemory`]: without a limit, a long
//!   session that opens and closes tabs would keep piling up history
//!   forever. Reopening restores the closed tab's full `History`, not just
//!   its path — that's what distinguishes it from opening a new tab there —
//!   and leaves it focused, just like Ctrl+Shift+T in a browser.
//! - **Out-of-range indices never panic.** They're clamped to the nearest
//!   valid end (`activate_at`, `move_tab`) or ignored when there's no
//!   reasonable end to fall back to (a [`TabId`] that no longer exists).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::history::History;

/// Opaque identifier for a tab. Stable for as long as the tab lives,
/// independent of its position in the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(u64);

/// A tab: a location with its own navigation history.
#[derive(Debug, Clone)]
pub struct Tab {
    id: TabId,
    history: History,
}

impl Tab {
    #[must_use]
    pub fn id(&self) -> TabId {
        self.id
    }

    #[must_use]
    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn history_mut(&mut self) -> &mut History {
        &mut self.history
    }

    /// The folder the tab is currently showing.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.history.current_path()
    }
}

/// Whether a new tab opens in the foreground (it receives focus) or in the
/// background (the bar gains a tab, but the active one stays the same). The
/// spec asks for this explicitly for middle click and Ctrl+click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    Foreground,
    Background,
}

/// What happened when asked to close a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    /// It closed. `focus` is the tab that should be active afterward — it
    /// may be the same one that already was, if the closed tab wasn't
    /// active.
    Closed { focus: TabId },
    /// It was the only tab left: nothing was touched. See the module notes
    /// on why this isn't an error.
    LastTab,
    /// The id doesn't match any open tab (already closed, for instance).
    /// Nothing was touched.
    NotFound,
}

/// How many closed tabs the "reopen" stack remembers.
///
/// The spec's own text suggests "the last 10" as an example of a reasonable
/// limit; adopted as-is.
pub const REOPEN_CAPACITY: usize = 10;

// Same as `view::MEMORY_CAPACITY`: the limit is fixed at compile time, not
// with a test that would exercise nothing but the constant's exact value.
const _: () = assert!(REOPEN_CAPACITY > 0 && REOPEN_CAPACITY <= 64);

/// What's needed to reopen a closed tab the way it was.
#[derive(Debug, Clone)]
struct ClosedTab {
    history: History,
    /// Position it held in the bar when closed, to restore it there if it
    /// still fits. The spec calls this "desirable", not mandatory, but it
    /// costs the same to carry as not to.
    position: usize,
}

/// A window's tab bar.
#[derive(Debug, Clone)]
pub struct Tabs {
    tabs: Vec<Tab>,
    active: TabId,
    next_tab_id: u64,
    /// Most recently closed tabs, for Ctrl+Shift+T. Most recent at the end;
    /// reopening does `pop_back`, closing does `push_back`.
    closed: VecDeque<ClosedTab>,
}

impl Tabs {
    /// Starts the bar with a single tab at `path`, active.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let id = TabId(0);
        Self {
            tabs: vec![Tab {
                id,
                history: History::new(path),
            }],
            active: id,
            next_tab_id: 1,
            closed: VecDeque::new(),
        }
    }

    fn allocate_id(&mut self) -> TabId {
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        id
    }

    fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    /// Inserts an already-built tab at `position` (clamped to a valid slot)
    /// and, if `mode` asks for it, focuses it.
    fn insert_tab(&mut self, history: History, position: usize, mode: OpenMode) -> TabId {
        let id = self.allocate_id();
        let position = position.min(self.tabs.len());
        self.tabs.insert(position, Tab { id, history });
        if mode == OpenMode::Foreground {
            self.active = id;
        }
        id
    }

    fn push_closed(&mut self, closed: ClosedTab) {
        self.closed.push_back(closed);
        while self.closed.len() > REOPEN_CAPACITY {
            self.closed.pop_front();
        }
    }

    /// Opens `path` in a new tab, at the end of the bar.
    ///
    /// At the end, not next to the active one: that way the Ctrl+1…8
    /// shortcuts for tabs that already existed don't change target just
    /// because one more was opened. `duplicate` is the deliberate exception
    /// — see its doc.
    pub fn open(&mut self, path: impl Into<PathBuf>, mode: OpenMode) -> TabId {
        self.insert_tab(History::new(path), self.tabs.len(), mode)
    }

    /// Duplicates a tab: same `History` (location and navigation history), a
    /// new independent tab from there on.
    ///
    /// Unlike `open`, it's inserted right next to the original, to its
    /// right: it's a copy of it, not an unrelated new tab, and the context
    /// menu offers it on the tab being duplicated.
    ///
    /// `None` if `id` no longer exists.
    pub fn duplicate(&mut self, id: TabId, mode: OpenMode) -> Option<TabId> {
        let index = self.index_of(id)?;
        let history = self.tabs[index].history.clone();
        Some(self.insert_tab(history, index + 1, mode))
    }

    /// Closes a tab. See [`CloseOutcome`] for the three outcomes.
    pub fn close(&mut self, id: TabId) -> CloseOutcome {
        let Some(index) = self.index_of(id) else {
            return CloseOutcome::NotFound;
        };
        if self.tabs.len() == 1 {
            return CloseOutcome::LastTab;
        }

        let tab = self.tabs.remove(index);
        self.push_closed(ClosedTab {
            history: tab.history,
            position: index,
        });

        if self.active == id {
            // The tab that now occupies the vacated slot (what used to be
            // "the next one"), or the new last tab if the closed one was the
            // last.
            let landing = index.min(self.tabs.len() - 1);
            self.active = self.tabs[landing].id;
        }

        CloseOutcome::Closed { focus: self.active }
    }

    /// Closes every tab except `keep`. No-op if `keep` doesn't exist.
    pub fn close_others(&mut self, keep: TabId) {
        if self.index_of(keep).is_none() {
            return;
        }
        let victims: Vec<TabId> = self
            .tabs
            .iter()
            .map(|tab| tab.id)
            .filter(|&id| id != keep)
            .collect();
        for id in victims {
            self.close(id);
        }
        self.active = keep;
    }

    /// Closes the tabs to the right of `id`. No-op if `id` doesn't exist.
    pub fn close_to_the_right(&mut self, id: TabId) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        let victims: Vec<TabId> = self.tabs[index + 1..].iter().map(|tab| tab.id).collect();
        for victim in victims {
            self.close(victim);
        }
    }

    /// Reopens the most recently closed tab, with its full `History`
    /// restored, at its original position if it still fits, and focused —
    /// just like Ctrl+Shift+T in a browser. `None` if there's nothing to
    /// reopen.
    pub fn reopen(&mut self) -> Option<TabId> {
        let closed = self.closed.pop_back()?;
        Some(self.insert_tab(closed.history, closed.position, OpenMode::Foreground))
    }

    /// Moves the tab at `from` to position `to`, clamped to the end of the
    /// bar if it overshoots. Total: `from` out of range is a no-op. Focus
    /// isn't touched because it's tracked by [`TabId`], not position — which
    /// is the very definition of "focus follows the moved tab".
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() {
            return;
        }
        let to = to.min(self.tabs.len() - 1);
        if from == to {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
    }

    /// Focuses `id`. No-op if it no longer exists.
    pub fn activate(&mut self, id: TabId) {
        if self.index_of(id).is_some() {
            self.active = id;
        }
    }

    /// Focuses the tab at position `index` (0-based). Total: an out-of-range
    /// index is clamped to the last tab, rather than doing nothing — same as
    /// `view`'s zoom staying at the end instead of ignoring the gesture.
    pub fn activate_at(&mut self, index: usize) {
        let index = index.min(self.tabs.len() - 1);
        self.active = self.tabs[index].id;
    }

    /// Focuses the rightmost tab in the bar (Ctrl+9).
    pub fn activate_last(&mut self) {
        self.activate_at(usize::MAX);
    }

    /// Ctrl+Tab: the next tab, cyclic.
    pub fn activate_next(&mut self) {
        let current = self.index_of(self.active).unwrap_or(0);
        let next = (current + 1) % self.tabs.len();
        self.active = self.tabs[next].id;
    }

    /// Ctrl+Shift+Tab: the previous tab, cyclic.
    pub fn activate_previous(&mut self) {
        let current = self.index_of(self.active).unwrap_or(0);
        let previous = if current == 0 {
            self.tabs.len() - 1
        } else {
            current - 1
        };
        self.active = self.tabs[previous].id;
    }

    #[must_use]
    pub fn active_id(&self) -> TabId {
        self.active
    }

    /// The active tab. Never fails: the bar always has at least one.
    #[must_use]
    pub fn active(&self) -> &Tab {
        self.get(self.active).unwrap_or(&self.tabs[0])
    }

    pub fn active_mut(&mut self) -> &mut Tab {
        let index = self.index_of(self.active).unwrap_or(0);
        &mut self.tabs[index]
    }

    #[must_use]
    pub fn get(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn get_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    /// The tabs in the order they're shown in the bar.
    pub fn tabs(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// Always `false`: the bar is born with one tab and never empties.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// How many closed tabs are in the "reopen" stack. For tests, and so the
    /// view can dim Ctrl+Shift+T when it's empty.
    #[must_use]
    pub fn reopenable_count(&self) -> usize {
        self.closed.len()
    }
}
