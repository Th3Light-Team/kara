//! Configurable Details columns: the visible set, its order, widths, the
//! catalog of what can still be added, and the per-folder memory.
//!
//! Reference: `ground/spec/03-vistas.md`, "Columnas configurables en
//! Detalles" (:122) and "Autoajustar ancho de columnas" (:134).

use std::path::{Path, PathBuf};

use kara_core::columns::{
    Column, ColumnLayout, ColumnMemory, ColumnsError, clamp_width, is_removable, known_columns,
    policy_for,
};
use kara_core::sort::ColumnId;

fn id(name: &'static str) -> ColumnId {
    ColumnId(name.into())
}

// --- Default layout ---------------------------------------------------------

#[test]
fn the_default_layout_is_explorers_own_four_columns_in_order() {
    // Name, date modified, type, size — the exact set and order Windows
    // Explorer shows a folder nobody has configured yet.
    let default = ColumnLayout::default();
    let ids: Vec<&str> = default.columns().iter().map(|c| c.id.0.as_ref()).collect();
    assert_eq!(ids, ["name", "modified", "kind", "size"]);
}

#[test]
fn the_default_layout_is_visible_through_is_visible() {
    let default = ColumnLayout::default();
    assert!(default.is_visible(&id("name")));
    assert!(default.is_visible(&id("size")));
    assert!(!default.is_visible(&id("tags")));
}

// --- The name column cannot be removed --------------------------------------

#[test]
fn removing_the_name_column_is_refused() {
    let mut layout = ColumnLayout::default();
    let result = layout.remove(&id("name"));

    assert_eq!(result, Err(ColumnsError::NameColumnRequired));
    // The refusal must leave the layout untouched, not just fail loudly.
    assert!(layout.is_visible(&id("name")));
    assert_eq!(layout.columns().len(), 4);
}

#[test]
fn removing_any_other_default_column_works() {
    let mut layout = ColumnLayout::default();
    layout.remove(&id("size")).expect("size is removable");

    assert!(!layout.is_visible(&id("size")));
    assert_eq!(layout.columns().len(), 3);
}

#[test]
fn is_removable_says_no_only_for_the_name_column() {
    assert!(!is_removable(&id("name")));
    for other in ["size", "modified", "kind", "extension", "tags", "icon"] {
        assert!(is_removable(&id(other)), "{other} must be removable");
    }
}

#[test]
fn removing_a_column_that_is_not_visible_is_a_harmless_no_op() {
    let mut layout = ColumnLayout::default();
    let before = layout.clone();
    layout.remove(&id("tags")).expect("tags is removable");

    assert_eq!(layout, before, "removing what wasn't shown changes nothing");
}

// --- Adding columns -----------------------------------------------------------

#[test]
fn adding_a_column_appends_it_at_its_policy_default_width() {
    let mut layout = ColumnLayout::default();
    layout.add(id("tags"));

    let last = layout.columns().last().expect("just added one");
    assert_eq!(last.id, id("tags"));
    assert_eq!(last.width, policy_for(&id("tags")).default_width);
}

#[test]
fn adding_an_already_visible_column_is_idempotent() {
    let mut layout = ColumnLayout::default();
    let before = layout.clone();
    layout.add(id("name"));

    assert_eq!(layout, before, "the column was already there");
    assert_eq!(
        layout.columns().iter().filter(|c| c.id == id("name")).count(),
        1,
        "adding it again must not duplicate it"
    );
}

// --- Reordering by dragging --------------------------------------------------

#[test]
fn reordering_moves_a_column_to_the_requested_position() {
    let mut layout = ColumnLayout::default();
    // Starts as name, modified, kind, size. Drag "size" (index 3) to the front.
    layout.reorder(3, 0);

    let ids: Vec<&str> = layout.columns().iter().map(|c| c.id.0.as_ref()).collect();
    assert_eq!(ids, ["size", "name", "modified", "kind"]);
}

#[test]
fn reordering_with_an_out_of_range_index_does_not_panic_or_lose_columns() {
    let mut layout = ColumnLayout::default();
    let original_len = layout.columns().len();

    // Neither index exists on a 4-column layout; this must clamp, not panic.
    layout.reorder(999, 1000);

    assert_eq!(layout.columns().len(), original_len, "no column may be dropped");
    // Clamping both to the last valid index (3) makes this a no-op move.
    let ids: Vec<&str> = layout.columns().iter().map(|c| c.id.0.as_ref()).collect();
    assert_eq!(ids, ["name", "modified", "kind", "size"]);
}

#[test]
fn reordering_a_single_column_layout_is_a_no_op() {
    let mut layout = ColumnLayout::new(vec![Column::new(id("name"), 240)]);
    layout.reorder(0, 5);
    assert_eq!(layout.columns().len(), 1);
    assert_eq!(layout.columns()[0].id, id("name"));
}

#[test]
fn reordering_an_empty_layout_does_not_panic() {
    let mut layout = ColumnLayout::new(Vec::new());
    layout.reorder(0, 3);
    assert!(layout.columns().is_empty());
}

// --- Width, minimum and fixed columns ----------------------------------------

#[test]
fn set_width_is_respected_above_the_minimum() {
    let mut layout = ColumnLayout::default();
    layout.set_width(&id("size"), 200);

    assert_eq!(layout.columns().iter().find(|c| c.id == id("size")).unwrap().width, 200);
}

#[test]
fn set_width_clamps_to_the_columns_minimum() {
    let policy = policy_for(&id("size"));
    let mut layout = ColumnLayout::default();
    layout.set_width(&id("size"), 1);

    let width = layout.columns().iter().find(|c| c.id == id("size")).unwrap().width;
    assert_eq!(width, policy.min_width, "must not shrink past the legible minimum");
}

#[test]
fn a_fixed_column_ignores_any_requested_width() {
    let mut layout = ColumnLayout::default();
    layout.add(id("icon"));
    let policy = policy_for(&id("icon"));
    assert!(policy.fixed, "the test assumes icon is a fixed column");

    layout.set_width(&id("icon"), 9000);

    let width = layout.columns().iter().find(|c| c.id == id("icon")).unwrap().width;
    assert_eq!(width, policy.default_width);
}

#[test]
fn clamp_width_is_the_same_rule_set_width_uses() {
    // Exercised directly since set_width is a thin wrapper over it.
    assert_eq!(clamp_width(&id("size"), 5), policy_for(&id("size")).min_width);
    assert_eq!(clamp_width(&id("size"), 500), 500);
    assert_eq!(clamp_width(&id("icon"), 500), policy_for(&id("icon")).default_width);
}

#[test]
fn setting_the_width_of_a_column_that_is_not_visible_does_nothing() {
    let mut layout = ColumnLayout::default();
    let before = layout.clone();
    layout.set_width(&id("tags"), 300);

    assert_eq!(layout, before);
}

// --- Autofit ------------------------------------------------------------------

#[test]
fn autofit_resets_one_column_to_its_default_width() {
    let mut layout = ColumnLayout::default();
    layout.set_width(&id("size"), 500);
    layout.autofit(&id("size"));

    let width = layout.columns().iter().find(|c| c.id == id("size")).unwrap().width;
    assert_eq!(width, policy_for(&id("size")).default_width);
}

#[test]
fn autofit_does_not_touch_other_columns() {
    let mut layout = ColumnLayout::default();
    layout.set_width(&id("size"), 500);
    layout.set_width(&id("kind"), 500);
    layout.autofit(&id("size"));

    let kind_width = layout.columns().iter().find(|c| c.id == id("kind")).unwrap().width;
    assert_eq!(kind_width, 500, "autofit on one column must leave its neighbours alone");
}

#[test]
fn autofit_all_resets_every_visible_column() {
    let mut layout = ColumnLayout::default();
    for column in layout.columns().to_vec() {
        layout.set_width(&column.id, 500);
    }
    layout.autofit_all();

    for column in layout.columns() {
        assert_eq!(column.width, policy_for(&column.id).default_width);
    }
}

// --- The catalog and what's left to add --------------------------------------

#[test]
fn known_columns_matches_the_vocabulary_sort_key_for_column_already_fixed() {
    use kara_core::sort::{SortError, sort_key_for_column};

    for column in known_columns() {
        match sort_key_for_column(&column) {
            // Sortable, or explicitly recognized-but-unsortable — either way
            // sort.rs already knows this identifier. An UnknownColumn error
            // would mean this module invented an id sort.rs never heard of.
            Ok(_) => {}
            Err(SortError::UnsortableColumn(_)) => {}
            Err(other) => panic!("{column:?} is not a known column identifier: {other}"),
        }
    }
}

#[test]
fn the_thumbnail_shaped_columns_are_offered_even_though_they_cannot_sort() {
    // "se puede enseñar" and "se puede ordenar" are different questions:
    // these three are legitimate columns despite sort.rs refusing to sort by
    // them.
    let catalog = known_columns();
    let known: Vec<&str> = catalog.iter().map(|c| c.0.as_ref()).collect();
    for unsortable in ["thumbnail", "preview", "icon"] {
        assert!(known.contains(&unsortable), "{unsortable} must still be offerable");
    }
}

#[test]
fn available_to_add_excludes_what_is_already_visible() {
    let layout = ColumnLayout::default();
    let available = layout.available_to_add();

    for visible in layout.columns() {
        assert!(
            !available.contains(&visible.id),
            "{:?} is already shown, it should not be offered again",
            visible.id
        );
    }
    // Something genuinely absent from the default four must still show up.
    assert!(available.contains(&id("tags")));
}

#[test]
fn available_to_add_follows_known_columns_order() {
    let layout = ColumnLayout::new(vec![Column::new(id("name"), 240)]);
    let available = layout.available_to_add();
    let catalog: Vec<ColumnId> = known_columns().into_iter().filter(|c| *c != id("name")).collect();

    assert_eq!(available, catalog);
}

#[test]
fn an_empty_layout_offers_the_whole_catalog() {
    let layout = ColumnLayout::new(Vec::new());
    assert_eq!(layout.available_to_add(), known_columns());
}

// --- Per-folder memory --------------------------------------------------------

fn only_name_and_tags() -> ColumnLayout {
    ColumnLayout::new(vec![
        Column::new(id("name"), 240),
        Column::new(id("tags"), 160),
    ])
}

#[test]
fn a_folder_nobody_configured_gets_the_global_default() {
    let memory = ColumnMemory::default();
    assert_eq!(memory.layout_for(Path::new("/home/ana")), ColumnLayout::default());
}

#[test]
fn a_folder_comes_back_the_way_it_was_left() {
    let mut memory = ColumnMemory::default();
    memory.remember(Path::new("/home/ana/Musica"), only_name_and_tags());

    assert_eq!(memory.layout_for(Path::new("/home/ana/Musica")), only_name_and_tags());
    assert_eq!(
        memory.layout_for(Path::new("/home/ana/Documentos")),
        ColumnLayout::default(),
        "remembering one folder must not touch its neighbours"
    );
}

#[test]
fn changing_the_global_default_does_not_disturb_what_was_remembered() {
    let mut memory = ColumnMemory::default();
    memory.remember(Path::new("/home/ana/Musica"), only_name_and_tags());

    let mut new_default = ColumnLayout::default();
    new_default.add(id("dimensions"));
    memory.set_fallback(new_default.clone());

    assert_eq!(memory.layout_for(Path::new("/home/ana/Musica")), only_name_and_tags());
    assert_eq!(memory.layout_for(Path::new("/home/ana/Otra")), new_default);
}

#[test]
fn forgetting_a_folder_returns_it_to_the_default() {
    let mut memory = ColumnMemory::default();
    let folder = Path::new("/home/ana/Musica");
    memory.remember(folder, only_name_and_tags());
    memory.forget(folder);

    assert_eq!(memory.layout_for(folder), ColumnLayout::default());
    assert!(memory.is_empty());
}

#[test]
fn the_history_stops_growing_and_drops_the_oldest() {
    let mut memory = ColumnMemory::new(ColumnLayout::default(), 3);
    for n in 0..5 {
        memory.remember(&PathBuf::from(format!("/carpeta/{n}")), only_name_and_tags());
    }

    assert_eq!(memory.len(), 3);
    assert_eq!(
        memory.layout_for(Path::new("/carpeta/0")),
        ColumnLayout::default(),
        "the oldest is the one that falls off"
    );
    assert_eq!(memory.layout_for(Path::new("/carpeta/4")), only_name_and_tags());
}

#[test]
fn revisiting_a_folder_keeps_it_from_falling_off() {
    let mut memory = ColumnMemory::new(ColumnLayout::default(), 2);
    let mut variant = only_name_and_tags();
    variant.add(id("rating"));

    memory.remember(Path::new("/a"), only_name_and_tags());
    memory.remember(Path::new("/b"), only_name_and_tags());
    // Touching /a again makes /b the oldest.
    memory.remember(Path::new("/a"), variant.clone());
    memory.remember(Path::new("/c"), only_name_and_tags());

    assert_eq!(memory.layout_for(Path::new("/a")), variant);
    assert_eq!(memory.layout_for(Path::new("/b")), ColumnLayout::default());
}

#[test]
fn a_memory_with_no_room_simply_remembers_nothing() {
    let mut memory = ColumnMemory::new(ColumnLayout::default(), 0);
    memory.remember(Path::new("/a"), only_name_and_tags());

    assert!(memory.is_empty());
    assert_eq!(memory.layout_for(Path::new("/a")), ColumnLayout::default());
}
