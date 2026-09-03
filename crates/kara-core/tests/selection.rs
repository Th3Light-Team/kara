//! Acceptance criteria for the multi-selection model
//! (`ground/spec/02-seleccion.md`): select all, deselect all, invert selection,
//! rubber-band, Ctrl+clic and Shift+clic. One test per edge case of the contract.

use std::collections::BTreeSet;
use std::ffi::OsString;

use kara_core::{EntryKind, FileEntry, MetadataBag, Selection, SortSpec, sort_permutation};

// ---------------------------------------------------------------- helpers ---

fn entry(name: &str) -> FileEntry {
    FileEntry {
        name: OsString::from(name),
        display: name.to_string(),
        kind: EntryKind::File,
        is_symlink: false,
        symlink_broken: false,
        is_hidden: false,
        size: None,
        modified: None,
        created: None,
        accessed: None,
        type_label: None,
        location: None,
        extra: MetadataBag::new(),
    }
}

fn indices(values: &[usize]) -> BTreeSet<usize> {
    values.iter().copied().collect()
}

// -------------------------------------------------------------- select all -

#[test]
fn select_all_marks_every_visible_index() {
    let mut selection = Selection::new();
    selection.select_all(5);
    assert_eq!(selection.selected(), &indices(&[0, 1, 2, 3, 4]));
    assert_eq!(selection.len(), 5);
}

#[test]
fn select_all_preserves_the_focused_item() {
    // The status bar counter changes, but keyboard navigation must not jump.
    let mut selection = Selection::new();
    selection.click(2, 5);
    selection.select_all(5);
    assert_eq!(selection.focused(), Some(2));
}

// ----------------------------------------------------------- deselect all --

#[test]
fn deselecting_all_empties_the_selection() {
    let mut selection = Selection::new();
    selection.select_all(5);
    selection.deselect_all();
    assert!(selection.is_empty());
    assert_eq!(selection.len(), 0);
}

#[test]
fn deselecting_all_keeps_the_focused_item() {
    // Esc (or a click on empty space) must leave keyboard navigation resuming
    // from the same cursor position, per "Deseleccionar todo / quitar selección".
    let mut selection = Selection::new();
    selection.click(3, 5);
    selection.select_all(5);
    selection.deselect_all();
    assert_eq!(selection.focused(), Some(3));
}

// --------------------------------------------------------------- invert ----

#[test]
fn inverting_swaps_membership_within_the_visible_range() {
    let mut selection = Selection::new();
    selection.ctrl_click(1, 5);
    selection.ctrl_click(3, 5);
    selection.invert(5);
    assert_eq!(selection.selected(), &indices(&[0, 2, 4]));
}

#[test]
fn inverting_twice_is_the_identity() {
    let mut selection = Selection::new();
    selection.ctrl_click(1, 5);
    selection.ctrl_click(3, 5);
    let before = selection.selected().clone();
    selection.invert(5);
    selection.invert(5);
    assert_eq!(selection.selected(), &before);
}

#[test]
fn inverting_preserves_the_focused_item() {
    let mut selection = Selection::new();
    selection.click(2, 5);
    selection.invert(5);
    assert_eq!(selection.focused(), Some(2));
}

#[test]
fn inverting_never_touches_indices_past_the_visible_length() {
    // A filter/search narrows what "select all"/"invert" can see (len), but a
    // previously selected index beyond it must not flip: it is not visible.
    let mut selection = Selection::new();
    selection.ctrl_click(0, 10);
    selection.ctrl_click(7, 10);
    selection.invert(5); // only indices 0..5 are "visible" now
    assert!(
        selection.is_selected(7),
        "index 7 was outside the visible range and must survive untouched"
    );
    assert!(!selection.is_selected(0), "index 0 was visible and must flip");
}

// ----------------------------------------------------------- plain click ---

#[test]
fn a_plain_click_replaces_the_selection_and_sets_anchor_and_focus() {
    let mut selection = Selection::new();
    selection.ctrl_click(1, 5);
    selection.ctrl_click(2, 5);
    selection.click(4, 5);
    assert_eq!(selection.selected(), &indices(&[4]));
    assert_eq!(selection.anchor(), Some(4));
    assert_eq!(selection.focused(), Some(4));
}

#[test]
fn an_out_of_range_click_is_a_no_op() {
    // Total operation: an index past the listing must not panic or corrupt state.
    let mut selection = Selection::new();
    selection.click(1, 5);
    let before = selection.clone();
    selection.click(99, 5);
    assert_eq!(selection, before);
}

// ------------------------------------------------------------- ctrl+clic ---

#[test]
fn ctrl_click_on_an_unselected_item_adds_it_without_touching_the_rest() {
    let mut selection = Selection::new();
    selection.click(1, 5);
    selection.ctrl_click(3, 5);
    assert_eq!(selection.selected(), &indices(&[1, 3]));
}

#[test]
fn ctrl_click_on_a_selected_item_removes_only_that_item() {
    let mut selection = Selection::new();
    selection.ctrl_click(1, 5);
    selection.ctrl_click(3, 5);
    selection.ctrl_click(1, 5);
    assert_eq!(selection.selected(), &indices(&[3]));
}

#[test]
fn ctrl_click_moves_the_anchor_to_the_last_ctrl_clicked_item() {
    // "El ancla para rangos futuros pasa a ser el último elemento pulsado con
    // Ctrl."
    let mut selection = Selection::new();
    selection.click(0, 5);
    selection.ctrl_click(3, 5);
    assert_eq!(selection.anchor(), Some(3));
}

#[test]
fn ctrl_click_moves_focus_without_losing_the_rest_of_the_selection() {
    let mut selection = Selection::new();
    selection.click(1, 5);
    selection.ctrl_click(3, 5);
    assert_eq!(selection.focused(), Some(3));
    assert!(selection.is_selected(1), "ctrl+clic must not open or drop others");
}

// ------------------------------------------------------------ ctrl+space ---

#[test]
fn ctrl_space_toggles_the_focused_item_without_moving_it() {
    let mut selection = Selection::new();
    selection.click(2, 5);
    selection.deselect_all(); // focus stays on 2, but nothing is selected now
    selection.toggle_focused();
    assert!(selection.is_selected(2));
    assert_eq!(selection.focused(), Some(2), "focus must not move");
    selection.toggle_focused();
    assert!(!selection.is_selected(2));
}

#[test]
fn ctrl_space_does_not_move_the_anchor() {
    // Unlike a mouse Ctrl+clic, Ctrl+Espacio is explicitly the "without moving
    // it" keyboard variant, and only clicks are said to update the anchor.
    let mut selection = Selection::new();
    selection.click(0, 5);
    selection.click(2, 5); // anchor now 2
    selection.toggle_focused();
    assert_eq!(selection.anchor(), Some(2));
}

// --------------------------------------------------------------- shift+clic

#[test]
fn shift_click_selects_the_contiguous_range_from_the_anchor() {
    let mut selection = Selection::new();
    selection.click(1, 10);
    selection.select_range(4, 10);
    assert_eq!(selection.selected(), &indices(&[1, 2, 3, 4]));
}

#[test]
fn shift_click_replaces_the_previous_selection() {
    let mut selection = Selection::new();
    selection.click(1, 10);
    selection.ctrl_click(8, 10); // an unrelated far-away pick
    selection.click(1, 10); // re-anchor
    selection.select_range(3, 10);
    assert_eq!(selection.selected(), &indices(&[1, 2, 3]));
}

#[test]
fn repeated_shift_click_recomputes_from_the_same_anchor_instead_of_growing() {
    // "Volver a hacer Shift+clic recalcula el rango desde el mismo ancla (no
    // acumula ni crece indefinidamente)."
    let mut selection = Selection::new();
    selection.click(2, 10);
    selection.select_range(6, 10);
    selection.select_range(4, 10);
    assert_eq!(selection.selected(), &indices(&[2, 3, 4]));
}

#[test]
fn shift_click_never_moves_the_anchor() {
    let mut selection = Selection::new();
    selection.click(2, 10);
    selection.select_range(6, 10);
    assert_eq!(selection.anchor(), Some(2));
}

#[test]
fn shift_click_moves_the_focus_to_the_clicked_end() {
    let mut selection = Selection::new();
    selection.click(2, 10);
    selection.select_range(6, 10);
    assert_eq!(selection.focused(), Some(6));
}

#[test]
fn shift_click_with_no_anchor_collapses_to_the_clicked_item() {
    let mut selection = Selection::new();
    selection.select_range(4, 10);
    assert_eq!(selection.selected(), &indices(&[4]));
    assert_eq!(
        selection.anchor(),
        None,
        "the anchor is never written by a shift-driven selection"
    );
}

#[test]
fn shift_click_works_backwards_toward_a_lower_index() {
    let mut selection = Selection::new();
    selection.click(6, 10);
    selection.select_range(3, 10);
    assert_eq!(selection.selected(), &indices(&[3, 4, 5, 6]));
}

#[test]
fn ctrl_shift_click_adds_a_second_range_without_losing_the_first() {
    // "Ctrl+Shift+clic añade un segundo rango sin perder el anterior." Pick a far
    // range with plain clicks, then re-anchor with ctrl+clic and grow a second
    // range with add_range: both must survive.
    let mut selection = Selection::new();
    selection.click(0, 10);
    selection.select_range(1, 10); // first range: 0..=1
    selection.ctrl_click(5, 10); // re-anchor to 5, keeping 0..=1 selected
    selection.add_range(7, 10); // second range: 5..=7, unioned with the first
    assert_eq!(selection.selected(), &indices(&[0, 1, 5, 6, 7]));
}

#[test]
fn add_range_unions_with_a_range_from_the_same_anchor() {
    let mut selection = Selection::new();
    selection.click(0, 10);
    selection.select_range(1, 10); // anchor 0, range 0..=1
    selection.add_range(3, 10); // still anchored at 0: adds 0..=3
    assert_eq!(
        selection.selected(),
        &indices(&[0, 1, 2, 3]),
        "add_range must union with, not replace, the previous selection"
    );
}

#[test]
fn add_range_never_moves_the_anchor_either() {
    let mut selection = Selection::new();
    selection.click(2, 10);
    selection.add_range(5, 10);
    assert_eq!(selection.anchor(), Some(2));
}

#[test]
fn range_selection_ignores_an_out_of_range_target() {
    let mut selection = Selection::new();
    selection.click(1, 10);
    let before = selection.clone();
    selection.select_range(99, 10);
    assert_eq!(selection, before);
}

// ------------------------------------------------------------- rubber-band -

#[test]
fn rubber_band_selects_the_covered_range() {
    let mut selection = Selection::new();
    selection.apply_rubber_band(2, 5, 10, false);
    assert_eq!(selection.selected(), &indices(&[2, 3, 4, 5]));
}

#[test]
fn rubber_band_replaces_the_selection_by_default() {
    let mut selection = Selection::new();
    selection.click(9, 10);
    selection.apply_rubber_band(2, 4, 10, false);
    assert_eq!(selection.selected(), &indices(&[2, 3, 4]));
}

#[test]
fn rubber_band_with_ctrl_held_adds_to_the_existing_selection() {
    // "Mantener Ctrl mientras se dibuja el marco añade a la selección existente
    // en vez de reemplazarla."
    let mut selection = Selection::new();
    selection.click(9, 10);
    selection.apply_rubber_band(2, 4, 10, true);
    assert_eq!(selection.selected(), &indices(&[2, 3, 4, 9]));
}

#[test]
fn rubber_band_handles_corners_drawn_in_either_direction() {
    let mut selection = Selection::new();
    selection.apply_rubber_band(5, 2, 10, false);
    assert_eq!(selection.selected(), &indices(&[2, 3, 4, 5]));
}

#[test]
fn rubber_band_clamps_to_the_visible_length() {
    let mut selection = Selection::new();
    selection.apply_rubber_band(3, 999, 5, false);
    assert_eq!(selection.selected(), &indices(&[3, 4]));
}

#[test]
fn rubber_band_entirely_past_the_listing_selects_nothing() {
    let mut selection = Selection::new();
    selection.apply_rubber_band(20, 30, 5, false);
    assert!(selection.is_empty());
}

#[test]
fn rubber_band_entirely_past_the_listing_still_clears_in_replace_mode() {
    // Releasing an empty band is, in effect, a click on empty space: it clears
    // any prior selection when not additive.
    let mut selection = Selection::new();
    selection.click(1, 5);
    selection.apply_rubber_band(20, 30, 5, false);
    assert!(selection.is_empty());
}

#[test]
fn rubber_band_entirely_past_the_listing_does_not_clear_in_additive_mode() {
    let mut selection = Selection::new();
    selection.click(1, 5);
    selection.apply_rubber_band(20, 30, 5, true);
    assert!(selection.is_selected(1), "ctrl-held band over nothing must not drop prior selection");
}

#[test]
fn rubber_band_moves_anchor_and_focus_to_the_ends_of_the_band() {
    let mut selection = Selection::new();
    selection.apply_rubber_band(6, 2, 10, false);
    assert_eq!(selection.anchor(), Some(2));
    assert_eq!(selection.focused(), Some(6));
}

// --------------------------------------------------------- reordering -----

#[test]
fn reordering_preserves_the_same_selected_entries() {
    // "Reordenar no puede perder la selección": the *entries*, not the indices,
    // must still read as selected once the permutation is applied.
    let entries = vec![entry("charlie"), entry("alpha"), entry("bravo")];
    let mut selection = Selection::new();
    selection.click(0, 3); // "charlie"
    selection.ctrl_click(2, 3); // "bravo"

    let perm = sort_permutation(&entries, &SortSpec::default()); // alpha, bravo, charlie
    selection.remap_after_sort(&perm).expect("valid permutation");

    let sorted: Vec<&str> = perm.iter().map(|&old| entries[old].display.as_str()).collect();
    assert_eq!(sorted, vec!["alpha", "bravo", "charlie"]);

    let selected_names: BTreeSet<&str> = selection
        .selected()
        .iter()
        .map(|&new_index| sorted[new_index])
        .collect();
    assert_eq!(
        selected_names,
        BTreeSet::from(["charlie", "bravo"]),
        "the same two entries must still be selected after reordering"
    );
}

#[test]
fn reordering_remaps_anchor_and_focus_too() {
    let entries = vec![entry("charlie"), entry("alpha"), entry("bravo")];
    let mut selection = Selection::new();
    selection.click(0, 3); // anchor and focus both on "charlie"

    let perm = sort_permutation(&entries, &SortSpec::default()); // alpha, bravo, charlie
    selection.remap_after_sort(&perm).unwrap();

    // "charlie" ends up last (index 2) in the alphabetical order.
    assert_eq!(selection.anchor(), Some(2));
    assert_eq!(selection.focused(), Some(2));
}

#[test]
fn remap_after_sort_rejects_a_permutation_shorter_than_the_selection() {
    let mut selection = Selection::new();
    selection.ctrl_click(0, 3);
    selection.ctrl_click(2, 3);
    let before = selection.clone();

    // A permutation of length 2 cannot map index 2.
    let result = selection.remap_after_sort(&[1, 0]);
    assert!(result.is_err());
    assert_eq!(
        selection, before,
        "a failed remap must leave the selection exactly as it was, not half-translated"
    );
}

#[test]
fn remap_after_sort_leaves_selected_untouched_when_only_a_stale_anchor_is_out_of_range() {
    // Deselecting or select-all deliberately leave anchor/focus stale (that is
    // the point of preserving the cursor), so anchor/focus can point past a
    // permutation that is perfectly valid for `selected`. That must still fail
    // the whole call rather than silently commit the remapped `selected` while
    // erroring out on the stale anchor/focus.
    let mut selection = Selection::new();
    selection.click(9, 10); // anchor and focus land on 9
    selection.deselect_all(); // selected empty; anchor/focus stay at the stale 9
    selection.select_all(1); // selected = {0}; anchor/focus are still the stale 9
    let before = selection.clone();

    // Valid for `selected` ({0} -> {1}), but too short for the stale anchor/focus (9).
    let result = selection.remap_after_sort(&[1, 0]);
    assert!(result.is_err());
    assert_eq!(
        selection, before,
        "a permutation valid for `selected` alone must not partially apply when anchor/focus reject it"
    );
}

#[test]
fn remap_after_sort_rejects_a_corrupt_permutation() {
    let mut selection = Selection::new();
    selection.click(1, 3);
    let before = selection.clone();

    // Not a permutation: index 1 repeated, index 2 missing.
    let result = selection.remap_after_sort(&[0, 1, 1]);
    assert!(result.is_err());
    assert_eq!(selection, before);
}

// ------------------------------------------------------- navigation state -

#[test]
fn to_view_state_stores_the_selection_by_name() {
    let entries = vec![entry("a"), entry("b"), entry("c")];
    let mut selection = Selection::new();
    selection.click(0, 3);
    selection.ctrl_click(2, 3);

    let state = selection.to_view_state(&entries, 42.0);
    assert_eq!(
        state.selection,
        BTreeSet::from([OsString::from("a"), OsString::from("c")])
    );
    assert_eq!(state.focused, Some(OsString::from("c")));
    assert_eq!(state.scroll, 42.0);
}

#[test]
fn from_view_state_round_trips_when_nothing_changed() {
    let entries = vec![entry("a"), entry("b"), entry("c")];
    let mut original = Selection::new();
    original.click(0, 3);
    original.ctrl_click(2, 3);

    let state = original.to_view_state(&entries, 0.0);
    let restored = Selection::from_view_state(&state, &entries);
    assert_eq!(restored.selected(), original.selected());
    assert_eq!(restored.focused(), original.focused());
}

#[test]
fn from_view_state_drops_names_that_no_longer_exist() {
    // The user navigated away, a file got deleted elsewhere, and now returns:
    // the vanished name must not resurrect as a dangling index.
    let entries_before = vec![entry("a"), entry("b"), entry("c")];
    let mut selection = Selection::new();
    selection.ctrl_click(1, 3); // "b"
    selection.ctrl_click(0, 3); // "a", clicked last so focus ends up here
    let state = selection.to_view_state(&entries_before, 0.0);

    // "z" sits at index 0 on purpose: a bug that maps a dropped name to index 0
    // by default (instead of dropping it) would wrongly resurrect "z" here.
    let entries_after = vec![entry("z"), entry("b"), entry("c")]; // "a" was deleted
    let restored = Selection::from_view_state(&state, &entries_after);

    assert_eq!(restored.selected(), &indices(&[1])); // only "b" survives, at index 1
    assert!(!restored.is_selected(0), "a dropped name must not resurrect as index 0");
    assert_eq!(restored.focused(), None, "the focused name was the one that vanished");
}

// --- A band over a grid, which covers no contiguous run --------------------

#[test]
fn a_band_selects_exactly_the_positions_it_covers() {
    // The case a range cannot express: a rectangle over a grid covering the
    // tail of one row and the head of the next. Passing the range 2..=5 would
    // select 3 and 4, which the rectangle never touched.
    let mut selection = Selection::new();
    selection.apply_band(&[2, 5], 8, false);

    assert!(selection.is_selected(2));
    assert!(selection.is_selected(5));
    assert!(!selection.is_selected(3));
    assert!(!selection.is_selected(4));
    assert_eq!(selection.len(), 2);
}

#[test]
fn a_band_replaces_the_previous_selection_unless_it_adds() {
    let mut selection = Selection::new();
    selection.apply_band(&[0, 1], 8, false);
    selection.apply_band(&[4], 8, false);
    assert_eq!(selection.len(), 1);

    selection.apply_band(&[6], 8, true);
    assert_eq!(selection.len(), 2);
    assert!(selection.is_selected(4));
    assert!(selection.is_selected(6));
}

#[test]
fn a_band_leaves_the_anchor_and_the_cursor_at_its_ends() {
    let mut selection = Selection::new();
    selection.apply_band(&[5, 2, 7], 8, false);

    assert_eq!(selection.anchor(), Some(2));
    assert_eq!(selection.focused(), Some(7));
}

#[test]
fn a_band_over_positions_that_no_longer_exist_ignores_them() {
    // The view computes what the rectangle covers from geometry, and geometry
    // can name a cell whose entry is gone.
    let mut selection = Selection::new();
    selection.apply_band(&[1, 99], 3, false);

    assert!(selection.is_selected(1));
    assert_eq!(selection.len(), 1);
}

#[test]
fn an_empty_band_clears_without_moving_the_cursor() {
    // Dragging over nothing must not leave the cursor pointing at a corner
    // that was never touched.
    let mut selection = Selection::new();
    selection.click(3, 8);
    let cursor = selection.focused();

    selection.apply_band(&[], 8, false);

    assert!(selection.is_empty());
    assert_eq!(selection.focused(), cursor);
}
