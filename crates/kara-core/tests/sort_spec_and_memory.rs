//! How a sort criterion is changed by a gesture, how a folder's own choices are
//! stored as differences from the global default, and how the per-folder memory
//! of view and sort is bounded.

use std::path::Path;

use kara_core::{
    DirectoryGrouping, SortError, SortKey, SortOrder, SortOverrides, SortSpec, ViewMemory,
    ViewMode, ViewSettings,
};
use kara_core::sort::ColumnId;

fn column(name: &'static str) -> ColumnId {
    ColumnId(std::borrow::Cow::Borrowed(name))
}

// ---- gestures ----------------------------------------------------------------

#[test]
fn activating_the_current_key_flips_the_direction() {
    let spec = SortSpec::default();
    let once = spec.activated_with(SortKey::Name);
    assert_eq!(once.order, SortOrder::Descending);
    let twice = once.activated_with(SortKey::Name);
    assert_eq!(twice.order, SortOrder::Ascending);
}

#[test]
fn activating_another_key_starts_ascending_even_after_a_descending_one() {
    let descending = SortSpec::default().activated_with(SortKey::Name);
    assert_eq!(descending.order, SortOrder::Descending);

    let other = descending.activated_with(SortKey::Size);
    assert_eq!(other.key, SortKey::Size);
    assert_eq!(other.order, SortOrder::Ascending);
}

#[test]
fn a_gesture_never_touches_grouping_or_collation() {
    let spec = SortSpec {
        grouping: DirectoryGrouping::Mixed,
        ..SortSpec::default()
    };
    for key in [SortKey::Name, SortKey::Size, SortKey::Modified] {
        let next = spec.activated_with(key);
        assert_eq!(next.grouping, DirectoryGrouping::Mixed);
        assert_eq!(next.collation, spec.collation);
    }
}

#[test]
fn a_menu_sets_the_key_without_flipping_when_picked_twice() {
    // The other verb: `with_key` *sets* a state, so repeating it changes nothing.
    let spec = SortSpec::default().with_order(SortOrder::Descending);
    let again = spec.with_key(SortKey::Name);
    assert_eq!(again.order, SortOrder::Descending);
}

#[test]
fn a_header_click_maps_columns_to_keys_and_rejects_the_unknown() {
    let spec = SortSpec::default();
    assert_eq!(spec.on_header_click(&column("size")).unwrap().key, SortKey::Size);
    assert_eq!(spec.on_header_click(&column("modified")).unwrap().key, SortKey::Modified);
    // Clicking the active column's header inverts it.
    assert_eq!(
        spec.on_header_click(&column("name")).unwrap().order,
        SortOrder::Descending
    );
    assert!(matches!(
        spec.on_header_click(&column("no-such-column")),
        Err(SortError::UnknownColumn(_))
    ));
}

// ---- overrides ----------------------------------------------------------------

fn spec(key: SortKey, order: SortOrder, grouping: DirectoryGrouping) -> SortSpec {
    SortSpec {
        key,
        order,
        grouping,
        ..SortSpec::default()
    }
}

#[test]
fn resolving_what_was_diffed_gives_back_the_original_for_every_combination() {
    let defaults = SortSpec::default();
    let keys = [SortKey::Name, SortKey::Size, SortKey::Modified, SortKey::Extension];
    let orders = [SortOrder::Ascending, SortOrder::Descending];
    let groupings = [
        DirectoryGrouping::First,
        DirectoryGrouping::Last,
        DirectoryGrouping::Mixed,
    ];
    for key in &keys {
        for order in orders {
            for grouping in groupings {
                let chosen = spec(key.clone(), order, grouping);
                let stored = SortOverrides::overriding(&chosen, &defaults);
                assert_eq!(stored.resolve(&defaults), chosen, "{chosen:?}");
            }
        }
    }
}

#[test]
fn a_folder_that_chose_nothing_stores_nothing() {
    let defaults = SortSpec::default();
    let stored = SortOverrides::overriding(&defaults, &defaults);
    assert!(stored.is_empty());
}

#[test]
fn only_the_fields_that_differ_are_stored() {
    let defaults = SortSpec::default();
    let chosen = SortSpec {
        order: SortOrder::Descending,
        ..SortSpec::default()
    };
    let stored = SortOverrides::overriding(&chosen, &defaults);
    assert_eq!(stored.order, Some(SortOrder::Descending));
    assert_eq!(stored.key, None);
    assert_eq!(stored.grouping, None);
}

#[test]
fn what_a_folder_inherited_follows_later_changes_to_the_default() {
    let mut defaults = SortSpec::default();
    let stored = SortOverrides::overriding(&SortSpec::default(), &defaults);

    defaults.grouping = DirectoryGrouping::Last;
    assert_eq!(stored.resolve(&defaults).grouping, DirectoryGrouping::Last);
}

#[test]
fn what_a_folder_chose_survives_a_change_of_default() {
    let mut defaults = SortSpec::default();
    let chosen = SortSpec {
        key: SortKey::Size,
        ..SortSpec::default()
    };
    let stored = SortOverrides::overriding(&chosen, &defaults);

    defaults.key = SortKey::Modified;
    assert_eq!(stored.resolve(&defaults).key, SortKey::Size);
}

// ---- per-folder memory -----------------------------------------------------------

fn memory(capacity: usize) -> ViewMemory {
    ViewMemory::new(ViewSettings::for_mode(ViewMode::Details), capacity)
}

#[test]
fn an_unconfigured_folder_gets_the_global_view_and_an_empty_sort() {
    let memory = memory(8);
    assert_eq!(memory.settings_for(Path::new("/a")), memory.fallback());
    assert!(memory.sort_for(Path::new("/a")).is_empty());
    assert!(memory.is_empty());
}

#[test]
fn remembering_the_view_does_not_disturb_the_sort_and_the_other_way_round() {
    let mut memory = memory(8);
    let folder = Path::new("/a");
    let sort = SortOverrides {
        order: Some(SortOrder::Descending),
        ..SortOverrides::default()
    };

    memory.remember_sort(folder, sort.clone());
    memory.remember(folder, ViewSettings::for_mode(ViewMode::Icons));

    assert_eq!(memory.sort_for(folder), sort);
    assert_eq!(memory.settings_for(folder).mode, ViewMode::Icons);
    assert_eq!(memory.len(), 1, "one folder, not two entries");
}

#[test]
fn a_folder_that_only_chose_a_sort_still_follows_the_global_view() {
    let mut memory = memory(8);
    memory.remember_sort(Path::new("/a"), SortOverrides::default());
    assert_eq!(memory.settings_for(Path::new("/a")), memory.fallback());

    let new_global = ViewSettings::for_mode(ViewMode::Tiles);
    memory.set_fallback(new_global);
    assert_eq!(memory.settings_for(Path::new("/a")), new_global);
}

#[test]
fn changing_the_fallback_moves_only_the_folders_that_never_chose() {
    let mut memory = memory(8);
    let chosen = ViewSettings::for_mode(ViewMode::Icons);
    memory.remember(Path::new("/chose"), chosen);

    memory.set_fallback(ViewSettings::for_mode(ViewMode::List));

    assert_eq!(memory.settings_for(Path::new("/chose")), chosen);
    assert_eq!(memory.settings_for(Path::new("/never")).mode, ViewMode::List);
}

#[test]
fn the_memory_is_bounded_and_forgets_the_least_recently_touched_first() {
    let mut memory = memory(3);
    for folder in ["/a", "/b", "/c"] {
        memory.remember(Path::new(folder), ViewSettings::for_mode(ViewMode::Icons));
    }
    // Touching /a makes /b the oldest.
    memory.remember(Path::new("/a"), ViewSettings::for_mode(ViewMode::List));
    memory.remember(Path::new("/d"), ViewSettings::for_mode(ViewMode::Icons));

    assert_eq!(memory.len(), 3);
    assert_eq!(memory.settings_for(Path::new("/b")), memory.fallback(), "evicted");
    assert_eq!(memory.settings_for(Path::new("/a")).mode, ViewMode::List);
    assert_eq!(memory.settings_for(Path::new("/c")).mode, ViewMode::Icons);
}

#[test]
fn forgetting_a_folder_sends_it_back_to_the_defaults_and_frees_its_slot() {
    let mut memory = memory(2);
    memory.remember(Path::new("/a"), ViewSettings::for_mode(ViewMode::Icons));
    memory.remember(Path::new("/b"), ViewSettings::for_mode(ViewMode::Icons));

    memory.forget(Path::new("/a"));
    memory.remember(Path::new("/c"), ViewSettings::for_mode(ViewMode::Icons));

    assert_eq!(memory.settings_for(Path::new("/a")), memory.fallback());
    assert_eq!(memory.settings_for(Path::new("/b")).mode, ViewMode::Icons, "not evicted early");
    assert_eq!(memory.len(), 2);
}

#[test]
fn a_memory_with_no_room_remembers_nothing_instead_of_failing() {
    let mut memory = memory(0);
    memory.remember(Path::new("/a"), ViewSettings::for_mode(ViewMode::Icons));
    memory.remember_sort(Path::new("/a"), SortOverrides::default());
    assert!(memory.is_empty());
}
