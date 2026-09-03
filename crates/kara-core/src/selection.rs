//! Multi-selection over the visible listing.
//!
//! Reference: `ground/spec/02-seleccion.md` — "Seleccionar todo", "Deseleccionar
//! todo / quitar selección", "Invertir selección", "Selección con rubber-band
//! (marco elástico)", "Ctrl+clic (alternar selección individual)" and "Shift+clic
//! (selección de rango contiguo)".
//!
//! # Design decisions
//!
//! - **Selection lives on indices into the visible, sorted/filtered listing**, not
//!   on the raw scandir order. "Seleccionar todo" and "Invertir selección" are
//!   explicit about operating only on what is on screen, and an index-based model
//!   is what lets the same value serve as a row number for the view.
//! - **Selection, anchor and focus are three separate fields.** The spec requires
//!   Esc and "deseleccionar todo" to keep the focused (cursor) item so keyboard
//!   navigation can resume from there even with nothing selected; folding focus
//!   into the selected set would make that impossible to express.
//! - **The anchor only moves on a plain click or a Ctrl+click**, exactly as the
//!   spec states for Shift+clic: "El ancla solo debe cambiar con un clic simple o
//!   con Ctrl+clic, nunca con Shift+clic." [`Selection::select_range`] and
//!   [`Selection::add_range`] (Shift+clic and Ctrl+Shift+clic) compute their range
//!   from the current anchor without ever writing to it, even when there is no
//!   anchor yet — in that case the range collapses to the clicked index alone
//!   without leaving an anchor behind, matching the letter of the rule at the cost
//!   of the very first Shift+clic in a fresh listing acting like a plain click.
//! - **Ctrl+Espacio toggles the focused item without moving the anchor.** The
//!   spec's anchor-update clause is scoped to "el último elemento pulsado con
//!   Ctrl" (a click); Ctrl+Espacio is explicitly called out as *not* moving focus,
//!   and by the same reasoning it is not a "clic" either, so
//!   [`Selection::toggle_focused`] leaves the anchor untouched.
//! - **A plain click and a Ctrl+click move focus to the clicked item.** The spec
//!   only bothers to say Ctrl+Espacio does *not* move focus, which only makes
//!   sense as a carve-out from a default where clicking does.
//! - **Rubber-band geometry is the view's problem; only the covered index range
//!   crosses into this module.** [`Selection::apply_rubber_band`] takes the two
//!   row indices the marquee spans (already resolved by the view from pixels) and
//!   the additive flag for "Ctrl held". On release it sets the anchor to the low
//!   end and the focus to the high end of the band, so a Shift+clic right after
//!   releasing a marquee continues from where the drag left off — the spec is
//!   silent on this, but leaving both `None` would make the very next keyboard or
//!   Shift interaction behave as if nothing had just been selected.
//! - **Reordering and navigating away and back must not lose the selection**, but
//!   they invalidate indices in different ways: reordering permutes them
//!   ([`Selection::remap_after_sort`], built on [`crate::sort::sort_permutation`],
//!   [`crate::sort::invert_permutation`] and [`crate::sort::remap_selection`]),
//!   while leaving and returning to a folder can change *which entries exist at
//!   all*, so [`crate::history::ViewState`] persists the selection by name and
//!   [`Selection::to_view_state`] / [`Selection::from_view_state`] convert between
//!   the two, dropping names that no longer exist in the listing being restored
//!   into.
//! - **Every operation is total.** An index at or past the current listing length
//!   never panics; it is treated as a no-op rather than clamped, since clamping an
//!   out-of-range click to the last row would select something the caller never
//!   pointed at. [`Selection::remap_after_sort`] computes the remapped selection,
//!   anchor and focus into locals first and only commits them once all three
//!   succeed, so a [`crate::sort::SortError`] never leaves the selection half
//!   translated to the new order.

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsStr;

use crate::entry::FileEntry;
use crate::history::ViewState;
use crate::sort::{SortError, invert_permutation, remap_selection};

/// Multi-selection state over a visible listing: which rows are selected, which
/// row is the range anchor, and which row carries keyboard focus.
///
/// Indices are positions in whatever listing the caller is currently showing.
/// They are meaningless once that listing is re-sorted or replaced; see
/// [`Selection::remap_after_sort`] and [`Selection::to_view_state`] for the two
/// ways to carry a selection across such a change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    focused: Option<usize>,
}

impl Selection {
    /// An empty selection with no anchor and no focused item.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The selected indices, in ascending order.
    #[must_use]
    pub fn selected(&self) -> &BTreeSet<usize> {
        &self.selected
    }

    /// The range anchor: where a Shift+clic range starts from.
    #[must_use]
    pub fn anchor(&self) -> Option<usize> {
        self.anchor
    }

    /// The item with keyboard focus (the cursor), independent of selection.
    #[must_use]
    pub fn focused(&self) -> Option<usize> {
        self.focused
    }

    /// How many items are selected, for the status bar's "N elementos
    /// seleccionados".
    #[must_use]
    pub fn len(&self) -> usize {
        self.selected.len()
    }

    /// `true` when nothing is selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    #[must_use]
    pub fn is_selected(&self, index: usize) -> bool {
        self.selected.contains(&index)
    }

    /// "Seleccionar todo" (Ctrl+A): selects every index in `0..len`.
    ///
    /// Operates only on `len`, the size of the visible/filtered listing — never
    /// on anything hidden by a filter, per the spec. Preserves the focused item,
    /// as the spec does not ask select-all to move the cursor.
    pub fn select_all(&mut self, len: usize) {
        self.selected = (0..len).collect();
    }

    /// "Deseleccionar todo" (a click on empty space, or Esc when no rename or
    /// rubber-band drag is in progress): empties the selection.
    ///
    /// Deliberately does not touch `focused`: the spec requires keyboard
    /// navigation to resume from the same item after this.
    pub fn deselect_all(&mut self) {
        self.selected.clear();
    }

    /// "Invertir selección": swaps selected and unselected within `0..len`.
    ///
    /// Only touches visible/filtered indices, same as `select_all`: anything
    /// selected at or past `len` (not currently visible under some filter) is
    /// carried over untouched rather than dropped. Leaves `focused` untouched,
    /// same as `select_all`.
    pub fn invert(&mut self, len: usize) {
        let mut result: BTreeSet<usize> = self.selected.range(len..).copied().collect();
        for index in 0..len {
            if !self.selected.contains(&index) {
                result.insert(index);
            }
        }
        self.selected = result;
    }

    /// A plain click on `index`: replaces the selection with just this item and
    /// moves both the anchor and the focus to it.
    ///
    /// No-op if `index` is past `len` (the listing the caller is showing).
    pub fn click(&mut self, index: usize, len: usize) {
        if index >= len {
            return;
        }
        self.selected.clear();
        self.selected.insert(index);
        self.anchor = Some(index);
        self.focused = Some(index);
    }

    /// Ctrl+clic: toggles `index` in the selection without touching the rest,
    /// and moves the anchor and the focus to it — "El ancla para rangos futuros
    /// pasa a ser el último elemento pulsado con Ctrl."
    ///
    /// No-op if `index` is past `len`.
    pub fn ctrl_click(&mut self, index: usize, len: usize) {
        if index >= len {
            return;
        }
        if !self.selected.remove(&index) {
            self.selected.insert(index);
        }
        self.anchor = Some(index);
        self.focused = Some(index);
    }

    /// Ctrl+Espacio: toggles the focused item's selection without moving the
    /// focus or the anchor.
    ///
    /// No-op if nothing is focused.
    pub fn toggle_focused(&mut self) {
        if let Some(index) = self.focused
            && !self.selected.remove(&index)
        {
            self.selected.insert(index);
        }
    }

    /// Shift+clic, and its keyboard equivalents Shift+Flecha / Shift+Inicio /
    /// Shift+Fin: replaces the selection with the contiguous range from the
    /// anchor to `index` and moves the focus to `index`.
    ///
    /// If there is no anchor yet, the range collapses to `index` alone; the
    /// anchor itself is never written here, per the spec.
    ///
    /// No-op if `index` is past `len`.
    pub fn select_range(&mut self, index: usize, len: usize) {
        if index >= len {
            return;
        }
        let start = self.anchor.unwrap_or(index);
        self.selected = span(start, index);
        self.focused = Some(index);
    }

    /// Ctrl+Shift+clic: adds the contiguous range from the anchor to `index` to
    /// the existing selection, without dropping what was already selected, and
    /// moves the focus to `index`.
    ///
    /// No-op if `index` is past `len`.
    pub fn add_range(&mut self, index: usize, len: usize) {
        if index >= len {
            return;
        }
        let start = self.anchor.unwrap_or(index);
        self.selected.extend(span(start, index));
        self.focused = Some(index);
    }

    /// Rubber-band release: applies the row range `[from, to]` (in either order)
    /// that the marquee covered.
    ///
    /// `additive` is "Ctrl held during the drag" — it adds the band to the
    /// existing selection instead of replacing it. A band that falls entirely
    /// past `len` (or `len == 0`) selects nothing, and — matching how a click on
    /// empty space deselects everything — still clears any previous selection
    /// in non-additive mode. On a non-empty band, moves the anchor to the low
    /// end and the focus to the high end.
    /// Applies a band that covers an arbitrary set of positions.
    ///
    /// A list view's band covers a contiguous run of rows, which
    /// [`Self::apply_rubber_band`] handles. A grid's does not: a rectangle
    /// drawn over a grid of icons covers, say, the last two cells of one row
    /// and the first two of the next, and the indices in between are outside
    /// it. Passing the range would select cells the rectangle never touched.
    ///
    /// Positions at or past `len` are ignored rather than rejected: the view
    /// works out what the rectangle covers from geometry, and geometry can
    /// name a cell that no longer has an entry.
    pub fn apply_band(&mut self, covered: &[usize], len: usize, additive: bool) {
        if !additive {
            self.selected.clear();
        }

        let mut low = None;
        let mut high = None;
        for position in covered.iter().copied().filter(|position| *position < len) {
            self.selected.insert(position);
            low = Some(low.map_or(position, |kept: usize| kept.min(position)));
            high = Some(high.map_or(position, |kept: usize| kept.max(position)));
        }

        // The anchor goes to the corner the band started from and the cursor to
        // the far one, so a Shift+click right after has somewhere to continue.
        if let (Some(low), Some(high)) = (low, high) {
            self.anchor = Some(low);
            self.focused = Some(high);
        }
    }

    pub fn apply_rubber_band(&mut self, from: usize, to: usize, len: usize, additive: bool) {
        let (low, high) = if from <= to { (from, to) } else { (to, from) };
        let covers_something = len > 0 && low < len;
        if !additive {
            self.selected.clear();
        }
        if covers_something {
            let high = high.min(len - 1);
            self.selected.extend(low..=high);
            self.anchor = Some(low);
            self.focused = Some(high);
        }
    }

    /// Carries the selection across a re-sort: `perm` is the permutation
    /// produced for the same listing by [`crate::sort::sort_permutation`]
    /// (`perm[new] == old`).
    ///
    /// Builds the remapped selection, anchor and focus in local variables and
    /// only assigns them once all three succeed, so an error never leaves the
    /// selection partially translated to the new order.
    ///
    /// # Errors
    ///
    /// Propagates [`SortError`] from [`crate::sort::invert_permutation`] /
    /// [`crate::sort::remap_selection`]: an invalid permutation, or a selected,
    /// anchored or focused index that does not exist in `perm`.
    pub fn remap_after_sort(&mut self, perm: &[usize]) -> Result<(), SortError> {
        let inverse = invert_permutation(perm)?;
        let selected = remap_selection(&self.selected, perm)?;
        let anchor = self
            .anchor
            .map(|index| remap_index(&inverse, index))
            .transpose()?;
        let focused = self
            .focused
            .map(|index| remap_index(&inverse, index))
            .transpose()?;
        self.selected = selected;
        self.anchor = anchor;
        self.focused = focused;
        Ok(())
    }

    /// Converts the current index-based selection into the name-based form that
    /// [`crate::history::History`] persists per visited folder, so it survives
    /// navigating away even if the folder's contents change while gone.
    ///
    /// `entries` must be the listing these indices were selected against.
    #[must_use]
    pub fn to_view_state(&self, entries: &[FileEntry], scroll: f64) -> ViewState {
        let selection = self
            .selected
            .iter()
            .filter_map(|&index| entries.get(index))
            .map(|entry| entry.name.clone())
            .collect();
        let focused = self
            .focused
            .and_then(|index| entries.get(index))
            .map(|entry| entry.name.clone());
        ViewState {
            selection,
            focused,
            scroll,
        }
    }

    /// Restores a [`crate::history::ViewState`] against `entries`, the listing of
    /// the folder being returned to.
    ///
    /// Names in `state` that no longer exist in `entries` are dropped rather
    /// than kept as dangling indices — the folder may have changed while the
    /// user was elsewhere. The anchor is not part of `ViewState` (nothing
    /// persists it), so it is initialized to the restored focus, giving a
    /// Shift+clic right after returning somewhere sensible to start from.
    #[must_use]
    pub fn from_view_state(state: &ViewState, entries: &[FileEntry]) -> Self {
        let index_of: HashMap<&OsStr, usize> = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.name.as_os_str(), index))
            .collect();
        let selected = state
            .selection
            .iter()
            .filter_map(|name| index_of.get(name.as_os_str()).copied())
            .collect();
        let focused = state
            .focused
            .as_ref()
            .and_then(|name| index_of.get(name.as_os_str()).copied());
        Self {
            selected,
            anchor: focused,
            focused,
        }
    }
}

/// The contiguous inclusive range between `a` and `b`, in whichever order.
fn span(a: usize, b: usize) -> BTreeSet<usize> {
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    (low..=high).collect()
}

/// Looks up a single index in an inverse permutation, as
/// [`crate::sort::remap_selection`] does for a whole set.
fn remap_index(inverse: &[usize], index: usize) -> Result<usize, SortError> {
    inverse
        .get(index)
        .copied()
        .ok_or(SortError::SelectionOutOfRange {
            index,
            len: inverse.len(),
        })
}
