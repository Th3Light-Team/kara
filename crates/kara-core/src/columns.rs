//! Configurable columns for the Details view.
//!
//! Reference: `ground/spec/03-vistas.md`, "Columnas configurables en Detalles"
//! (:122) and "Autoajustar ancho de columnas" (:134). Also keep "Recordar la
//! vista por carpeta" (:296) in mind: the column set and widths are meant to
//! persist per folder, the same way [`crate::view::ViewMemory`] and
//! [`crate::sort::SortOverrides`] already do for the mode and the sort.
//!
//! # Design decisions
//!
//! - **The catalog is not invented here.** [`crate::sort::ColumnId`] and
//!   [`crate::sort::sort_key_for_column`] already fix which column identifiers
//!   exist, including the three ([`"thumbnail"`], [`"preview"`], [`"icon"`])
//!   that are recognized but explicitly not sortable
//!   ([`crate::sort::SortError::UnsortableColumn`]). This module reuses that
//!   exact vocabulary: [`known_columns`] is a menu over the same identifiers
//!   [`crate::sort::sort_key_for_column`] already understands, not a second
//!   catalog that could drift from the first.
//! - **"Showable" and "sortable" are different questions.** A column that
//!   [`crate::sort::sort_key_for_column`] rejects with `UnsortableColumn`
//!   (`thumbnail`, `preview`, `icon`) is still a legitimate member of
//!   [`ColumnLayout`]: the Details view can display an icon or a preview
//!   swatch per row without being able to sort by it. This module never calls
//!   [`crate::sort::sort_key_for_column`] to decide whether a column may be
//!   shown — only the view, when it wants to draw a sort arrow, needs that.
//! - **No pixels, no fonts.** A width is a plain number the view scales and
//!   respects; measuring text against a font is the view's job, not this
//!   one's. What lives here is the *policy* around that number: the width a
//!   column returns to when asked to autofit, the floor it never goes below,
//!   and whether it can be resized by dragging at all. [`ColumnPolicy`] is
//!   that policy, and [`ColumnLayout::set_width`] is what enforces it — the
//!   view hands over whatever width a drag or a content measurement produced
//!   and this module clamps it, never the other way around.
//! - **The name column cannot be removed.** A file table without names shows
//!   nothing recognizable (03-vistas.md:130, "La columna de nombre no debe
//!   poder eliminarse"). [`is_removable`] is the single place that knows
//!   which identifier that is, and [`ColumnLayout::remove`] refuses instead of
//!   silently dropping it — a caller that tries anyway gets
//!   [`ColumnsError::NameColumnRequired`] rather than a details view with no
//!   way to tell rows apart.
//! - **Dragging a header cannot corrupt the layout.** `03-vistas.md:130` asks
//!   for columns reordered by dragging the header, with the same demand for
//!   totality the rest of the project holds itself to. [`ColumnLayout::reorder`]
//!   clamps both indices into range instead of panicking or leaving a gap: a
//!   stale index from a drag that started before another column was removed
//!   is exactly the kind of syscall-adjacent input the project's rules say
//!   must never crash.
//! - **Persistence is one full layout per folder, like the mode — not a diff
//!   like the sort.** [`crate::view::FolderView`] stores mode as
//!   `Option<ViewSettings>` (a folder either chose one fully, or inherits the
//!   global default) while it stores sort as [`crate::sort::SortOverrides`] (a
//!   folder can override just the direction and inherit the rest). A column
//!   layout is a set, an order and a width bundled together; there is no
//!   sensible way to override "just the order" independently of "what's even
//!   in the set" the way `SortOverrides` overrides direction independently of
//!   the criterion. So [`ColumnMemory`] follows the mode's pattern: a folder
//!   remembers a whole [`ColumnLayout`] or nothing, with the same bounded,
//!   in-RAM history as [`crate::view::MEMORY_CAPACITY`] — there is nowhere to
//!   put this on disk yet, and unbounded growth would mean a session that
//!   browses a large tree carrying a layout for every directory it passed
//!   through.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::sort::ColumnId;

/// One column in the Details view: which data it shows and how wide it
/// currently is.
///
/// Order is not a field here — it is the position of this `Column` inside
/// [`ColumnLayout::columns`]. Keeping order out of the element and in the
/// container is what makes [`ColumnLayout::reorder`] a slice operation
/// instead of a renumbering pass over every column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub id: ColumnId,
    /// Logical width the view scales and respects. Not a pixel measurement.
    pub width: u32,
}

impl Column {
    #[must_use]
    pub fn new(id: ColumnId, width: u32) -> Self {
        Self { id, width }
    }
}

/// Errors a caller can get back from [`ColumnLayout`]. Every one of them is a
/// contract violation the caller can recover from — none of them corrupt the
/// layout, and none of them is reachable by ordinary drag or click input,
/// which is why [`ColumnLayout::reorder`] and [`ColumnLayout::set_width`]
/// clamp their input instead of returning one of these.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ColumnsError {
    /// `03-vistas.md:130`: the name column cannot be removed.
    #[error("the name column cannot be removed")]
    NameColumnRequired,
}

/// Whether a column may be taken out of the visible set.
///
/// The name column is the one exception the spec calls out explicitly: a
/// table of files without a name column shows nothing recognizable, so this
/// is a structural constraint, not a preference [`ColumnLayout::remove`] could
/// leave up to the caller.
#[must_use]
pub fn is_removable(id: &ColumnId) -> bool {
    id.0.as_ref() != "name"
}

/// What [`ColumnLayout`] falls back on for a column it has not been told
/// about explicitly: wide enough to be legible, resizable, and — because an
/// unknown identifier is either a `meta/<name>` column the user just added or
/// a future built-in this module has not been taught about yet — not assumed
/// to be anything narrower than ordinary text.
const FALLBACK_POLICY: ColumnPolicy = ColumnPolicy {
    default_width: 140,
    min_width: 40,
    fixed: false,
};

/// The width policy for one column: what it resets to, the floor it never
/// goes below, and whether it can be resized at all.
///
/// This is the "autoajustar" side the spec asks for
/// (`03-vistas.md:134`, "Debe respetar un ancho mínimo legible"). It is
/// deliberately *not* the "fit to the longest visible value" behaviour a
/// double-click on the header separator triggers — that needs the rendered
/// text's measured width, which only the view has. What this type fixes is
/// the part that does not depend on what is on screen: how far a column may
/// shrink, and where "Ajustar todas las columnas" puts it back to when there
/// is nothing to measure against (an empty folder, or a column the view has
/// not rendered yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnPolicy {
    /// Width `autofit` and `autofit_all` reset the column to.
    pub default_width: u32,
    /// Width `set_width` never goes below.
    pub min_width: u32,
    /// `true` for a column dragging the separator cannot resize at all.
    pub fixed: bool,
}

/// The policy for a known column, or [`FALLBACK_POLICY`] for anything this
/// module was not told about (a `meta/<name>` column, or an identifier from a
/// future built-in).
///
/// # Why the thumbnail-shaped columns are fixed
///
/// `thumbnail`, `preview` and `icon` show a square image at whatever size the
/// current zoom rung ([`crate::view::ViewSettings::icon_size`]) asks for, not
/// text that benefits from more horizontal room. Unlike a text column, where
/// dragging it wider reveals more of a name or a path before the ellipsis
/// kicks in, dragging one of these wider would only pad empty space around a
/// fixed-size image — so they are `fixed` rather than merely narrow.
#[must_use]
pub fn policy_for(id: &ColumnId) -> ColumnPolicy {
    match id.0.as_ref() {
        "name" => ColumnPolicy {
            default_width: 240,
            min_width: 80,
            fixed: false,
        },
        "extension" => ColumnPolicy {
            default_width: 80,
            min_width: 40,
            fixed: false,
        },
        "size" => ColumnPolicy {
            default_width: 90,
            min_width: 50,
            fixed: false,
        },
        "modified" | "created" | "accessed" => ColumnPolicy {
            default_width: 140,
            min_width: 90,
            fixed: false,
        },
        "kind" => ColumnPolicy {
            default_width: 140,
            min_width: 60,
            fixed: false,
        },
        "location" => ColumnPolicy {
            default_width: 220,
            min_width: 80,
            fixed: false,
        },
        "dimensions" => ColumnPolicy {
            default_width: 110,
            min_width: 60,
            fixed: false,
        },
        "duration" => ColumnPolicy {
            default_width: 90,
            min_width: 50,
            fixed: false,
        },
        "album" | "artist" => ColumnPolicy {
            default_width: 160,
            min_width: 60,
            fixed: false,
        },
        "tags" => ColumnPolicy {
            default_width: 160,
            min_width: 60,
            fixed: false,
        },
        "rating" => ColumnPolicy {
            default_width: 90,
            min_width: 50,
            fixed: false,
        },
        "thumbnail" | "preview" | "icon" => ColumnPolicy {
            default_width: 48,
            min_width: 48,
            fixed: true,
        },
        _ => FALLBACK_POLICY,
    }
}

/// Clamps a requested width to what `id`'s policy allows: a fixed column
/// always stays at its `default_width`, and any other column never goes
/// below its `min_width`.
#[must_use]
pub fn clamp_width(id: &ColumnId, requested: u32) -> u32 {
    let policy = policy_for(id);
    if policy.fixed {
        policy.default_width
    } else {
        requested.max(policy.min_width)
    }
}

/// Every built-in column identifier, in the order `03-vistas.md:130`'s "Más…"
/// catalog should offer them and matching exactly the vocabulary
/// [`crate::sort::sort_key_for_column`] already fixed.
///
/// `meta/<name>` columns are not listed here: they are added by whatever
/// extractor produced them, not chosen from a fixed catalog, and cannot be
/// enumerated without knowing which ones a given folder's files carry.
#[must_use]
pub fn known_columns() -> Vec<ColumnId> {
    [
        "name",
        "extension",
        "size",
        "modified",
        "created",
        "accessed",
        "kind",
        "location",
        "dimensions",
        "duration",
        "album",
        "artist",
        "tags",
        "rating",
        "thumbnail",
        "preview",
        "icon",
    ]
    .into_iter()
    .map(|id| ColumnId(Cow::Borrowed(id)))
    .collect()
}

/// The column set, order and widths a Details view is currently showing.
///
/// Windows Explorer's own default — name, date modified, type, size — is
/// what a folder that never configured anything sees; see [`Default`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnLayout {
    columns: Vec<Column>,
}

impl Default for ColumnLayout {
    fn default() -> Self {
        Self {
            columns: [
                ("name", policy_for(&ColumnId(Cow::Borrowed("name"))).default_width),
                (
                    "modified",
                    policy_for(&ColumnId(Cow::Borrowed("modified"))).default_width,
                ),
                ("kind", policy_for(&ColumnId(Cow::Borrowed("kind"))).default_width),
                ("size", policy_for(&ColumnId(Cow::Borrowed("size"))).default_width),
            ]
            .into_iter()
            .map(|(id, width)| Column::new(ColumnId(Cow::Borrowed(id)), width))
            .collect(),
        }
    }
}

impl ColumnLayout {
    /// An empty layout — no columns at all, not even name.
    ///
    /// This exists only as a building block for callers that construct a
    /// layout column by column (persistence restoring a saved set, a test).
    /// It is not itself a valid "resting" state: nothing stops
    /// [`ColumnLayout::new`] from producing one with the name column absent,
    /// but every mutator that could remove it ([`ColumnLayout::remove`])
    /// refuses to, so an empty or name-less layout can only be reached by
    /// building it by hand or restoring corrupt state — never by the normal
    /// add/remove/reorder gestures this type exposes.
    #[must_use]
    pub fn new(columns: Vec<Column>) -> Self {
        Self { columns }
    }

    /// The visible columns, in display order.
    #[must_use]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    #[must_use]
    pub fn is_visible(&self, id: &ColumnId) -> bool {
        self.columns.iter().any(|column| &column.id == id)
    }

    fn position(&self, id: &ColumnId) -> Option<usize> {
        self.columns.iter().position(|column| &column.id == id)
    }

    /// Adds a column at the end of the layout, at its policy's default width.
    ///
    /// Idempotent: adding a column that is already visible does nothing —
    /// asking to show a column that is already shown is not an error, and
    /// treating it as one would make every "add from the catalog" call site
    /// check visibility first for no benefit.
    pub fn add(&mut self, id: ColumnId) {
        if self.is_visible(&id) {
            return;
        }
        let width = policy_for(&id).default_width;
        self.columns.push(Column::new(id, width));
    }

    /// Removes a column from the layout.
    ///
    /// Removing a column that is not visible is also idempotent and returns
    /// `Ok(())`: the caller asked for an end state, not for confirmation that
    /// something happened.
    ///
    /// # Errors
    ///
    /// [`ColumnsError::NameColumnRequired`] if `id` is the name column
    /// (`03-vistas.md:130`). The layout is left untouched.
    pub fn remove(&mut self, id: &ColumnId) -> Result<(), ColumnsError> {
        if !is_removable(id) {
            return Err(ColumnsError::NameColumnRequired);
        }
        self.columns.retain(|column| &column.id != id);
        Ok(())
    }

    /// Moves the column at `from` to sit at `to`, the way dragging a header
    /// to a new position does.
    ///
    /// Both indices are clamped into `0..columns().len()` first. A stale
    /// index — the drag started before another column was removed, or a
    /// caller mis-tracked which row is which — lands on the nearest valid
    /// position instead of panicking or leaving the layout with a hole,
    /// matching the project-wide rule that a bad index must never corrupt
    /// state. With zero or one column this is a no-op, since there is
    /// nothing to reorder.
    pub fn reorder(&mut self, from: usize, to: usize) {
        let len = self.columns.len();
        if len < 2 {
            return;
        }
        let from = from.min(len - 1);
        let to = to.min(len - 1);
        if from == to {
            return;
        }
        let column = self.columns.remove(from);
        self.columns.insert(to, column);
    }

    /// Sets a column's width, clamped to what its [`ColumnPolicy`] allows —
    /// never below `min_width`, and never anything but `default_width` for a
    /// fixed column.
    ///
    /// A no-op if `id` is not currently visible: there is no width to set on
    /// a column that is not shown.
    pub fn set_width(&mut self, id: &ColumnId, width: u32) {
        if let Some(index) = self.position(id) {
            self.columns[index].width = clamp_width(id, width);
        }
    }

    /// Resets one column back to its policy's `default_width` — the single
    /// column version of "Ajustar todas las columnas".
    ///
    /// A no-op if `id` is not visible.
    pub fn autofit(&mut self, id: &ColumnId) {
        if let Some(index) = self.position(id) {
            let default_width = policy_for(id).default_width;
            self.columns[index].width = default_width;
        }
    }

    /// Resets every visible column back to its policy's `default_width`.
    /// `03-vistas.md:134`'s "Ajustar todas las columnas".
    pub fn autofit_all(&mut self) {
        for column in &mut self.columns {
            column.width = policy_for(&column.id).default_width;
        }
    }

    /// Built-in columns not currently visible, in [`known_columns`] order —
    /// the "Más…" catalog the header's context menu offers.
    #[must_use]
    pub fn available_to_add(&self) -> Vec<ColumnId> {
        known_columns()
            .into_iter()
            .filter(|id| !self.is_visible(id))
            .collect()
    }
}

/// How many folders keep their own column layout before the oldest is
/// forgotten. Same bound as [`crate::view::MEMORY_CAPACITY`], for the same
/// reason: a session that browses a large tree must not carry a layout for
/// every directory it ever passed through.
pub const MEMORY_CAPACITY: usize = 256;

// Checked at compile time rather than in a test, for the same reason
// `view.rs` checks its own bound this way: a test on a constant would not
// exercise anything, and what has to be prevented is someone leaving this at
// zero (the memory would stop remembering) or at a million (not a limit).
const _: () = assert!(MEMORY_CAPACITY > 0 && MEMORY_CAPACITY <= 4096);

/// What each folder was left showing in its Details view.
///
/// Mirrors [`crate::view::ViewMemory`]: a global fallback every unconfigured
/// folder resolves to, and a bounded, most-recently-used history of the
/// folders that chose their own layout.
#[derive(Debug, Clone)]
pub struct ColumnMemory {
    fallback: ColumnLayout,
    remembered: HashMap<PathBuf, ColumnLayout>,
    /// Least recently remembered first, so eviction knows what to drop.
    order: Vec<PathBuf>,
    capacity: usize,
}

impl Default for ColumnMemory {
    fn default() -> Self {
        Self::new(ColumnLayout::default(), MEMORY_CAPACITY)
    }
}

impl ColumnMemory {
    #[must_use]
    pub fn new(fallback: ColumnLayout, capacity: usize) -> Self {
        Self {
            fallback,
            remembered: HashMap::new(),
            order: Vec::new(),
            capacity,
        }
    }

    /// The global default, used by every folder nobody has configured.
    #[must_use]
    pub fn fallback(&self) -> &ColumnLayout {
        &self.fallback
    }

    pub fn set_fallback(&mut self, layout: ColumnLayout) {
        self.fallback = layout;
    }

    /// The layout a folder should be shown with.
    #[must_use]
    pub fn layout_for(&self, folder: &Path) -> ColumnLayout {
        self.remembered
            .get(folder)
            .cloned()
            .unwrap_or_else(|| self.fallback.clone())
    }

    /// Records the layout a folder was left with.
    pub fn remember(&mut self, folder: &Path, layout: ColumnLayout) {
        if self.capacity == 0 {
            return;
        }

        self.order.retain(|kept| kept != folder);
        self.order.push(folder.to_path_buf());
        self.remembered.insert(folder.to_path_buf(), layout);

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
