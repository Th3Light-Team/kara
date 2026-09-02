//! View modes, the zoom ladder and the per-folder memory.
//!
//! Not in `view.rs`: that file already covers visibility, live filtering and
//! type-ahead.

use std::path::{Path, PathBuf};

use kara_core::view::{ViewMemory, ViewMode, ViewSettings};

fn icons(size: u32) -> ViewSettings {
    ViewSettings::new(ViewMode::Icons, size)
}

#[test]
fn picking_a_mode_lands_on_the_size_it_normally_shows() {
    assert_eq!(ViewSettings::for_mode(ViewMode::Icons), icons(96));
    assert_eq!(
        ViewSettings::for_mode(ViewMode::Tiles),
        ViewSettings::new(ViewMode::Tiles, 48)
    );
}

#[test]
fn details_is_the_default() {
    // The mode that survives a folder with ten thousand files in it.
    assert_eq!(ViewSettings::default().mode, ViewMode::Details);
}

#[test]
fn zooming_in_the_icon_view_changes_only_the_size() {
    let zoomed = icons(96).zoom_in();
    assert_eq!(zoomed.mode, ViewMode::Icons);
    assert_eq!(zoomed.icon_size, 128);
}

#[test]
fn zooming_out_past_the_smallest_icons_falls_into_the_denser_modes() {
    // This is the whole point of one ladder: there is no "smaller than the
    // smallest icon", there is a tile, then a list, then details.
    let smallest_icons = icons(48);
    let tiles = smallest_icons.zoom_out();
    assert_eq!(tiles.mode, ViewMode::Tiles);

    let list = tiles.zoom_out();
    assert_eq!(list.mode, ViewMode::List);

    let details = list.zoom_out();
    assert_eq!(details.mode, ViewMode::Details);
}

#[test]
fn the_ends_of_the_ladder_hold_instead_of_wrapping() {
    let densest = ViewSettings::for_mode(ViewMode::Details);
    assert!(densest.is_smallest());
    assert_eq!(densest.zoom_out(), densest);

    let largest = icons(256);
    assert!(largest.is_largest());
    assert_eq!(largest.zoom_in(), largest);
}

#[test]
fn the_whole_ladder_can_be_walked_up_and_back_down() {
    let mut settings = ViewSettings::for_mode(ViewMode::Details);
    let mut climbed = vec![settings];
    while !settings.is_largest() {
        settings = settings.zoom_in();
        climbed.push(settings);
    }

    let mut descended = vec![settings];
    while !settings.is_smallest() {
        settings = settings.zoom_out();
        descended.push(settings);
    }
    descended.reverse();

    assert_eq!(climbed, descended, "the ladder must be symmetric");
    assert!(climbed.len() > 4, "there has to be somewhere to zoom");
}

#[test]
fn resetting_the_zoom_keeps_the_mode() {
    // Ctrl+0 is "back to the normal size", not "back to the default view".
    assert_eq!(icons(256).reset_zoom(), icons(96));
    let tiles = ViewSettings::for_mode(ViewMode::Tiles);
    assert_eq!(tiles.reset_zoom(), tiles);
}

#[test]
fn a_size_that_is_not_on_the_ladder_still_zooms() {
    // Nothing offers 111 pixels, but arriving there must not freeze the wheel.
    let odd = icons(111);
    assert_eq!(odd.zoom_in().mode, ViewMode::Icons);
    assert_ne!(odd.zoom_in(), odd);
}

#[test]
fn a_folder_nobody_configured_gets_the_global_default() {
    let memory = ViewMemory::default();
    assert_eq!(
        memory.settings_for(Path::new("/home/ana")),
        ViewSettings::default()
    );
}

#[test]
fn a_folder_comes_back_the_way_it_was_left() {
    let mut memory = ViewMemory::default();
    memory.remember(Path::new("/home/ana/Fotos"), icons(256));

    assert_eq!(memory.settings_for(Path::new("/home/ana/Fotos")), icons(256));
    assert_eq!(
        memory.settings_for(Path::new("/home/ana/Documentos")),
        ViewSettings::default(),
        "remembering one folder must not touch its neighbours"
    );
}

#[test]
fn changing_the_global_default_does_not_disturb_what_was_remembered() {
    let mut memory = ViewMemory::default();
    memory.remember(Path::new("/home/ana/Fotos"), icons(256));
    memory.set_fallback(ViewSettings::for_mode(ViewMode::List));

    assert_eq!(memory.settings_for(Path::new("/home/ana/Fotos")), icons(256));
    assert_eq!(
        memory.settings_for(Path::new("/home/ana")).mode,
        ViewMode::List
    );
}

#[test]
fn forgetting_a_folder_returns_it_to_the_default() {
    let mut memory = ViewMemory::default();
    let folder = Path::new("/home/ana/Fotos");
    memory.remember(folder, icons(256));
    memory.forget(folder);

    assert_eq!(memory.settings_for(folder), ViewSettings::default());
    assert!(memory.is_empty());
}

#[test]
fn the_history_stops_growing_and_drops_the_oldest() {
    // The spec asks for a bounded history: browsing a large tree must not carry
    // a setting for every directory it ever passed through.
    let mut memory = ViewMemory::new(ViewSettings::default(), 3);
    for n in 0..5 {
        memory.remember(&PathBuf::from(format!("/carpeta/{n}")), icons(128));
    }

    assert_eq!(memory.len(), 3);
    assert_eq!(
        memory.settings_for(Path::new("/carpeta/0")),
        ViewSettings::default(),
        "the oldest is the one that falls off"
    );
    assert_eq!(memory.settings_for(Path::new("/carpeta/4")), icons(128));
}

#[test]
fn revisiting_a_folder_keeps_it_from_falling_off() {
    let mut memory = ViewMemory::new(ViewSettings::default(), 2);
    memory.remember(Path::new("/a"), icons(128));
    memory.remember(Path::new("/b"), icons(128));
    // Touching /a again makes /b the oldest.
    memory.remember(Path::new("/a"), icons(256));
    memory.remember(Path::new("/c"), icons(128));

    assert_eq!(memory.settings_for(Path::new("/a")), icons(256));
    assert_eq!(
        memory.settings_for(Path::new("/b")),
        ViewSettings::default()
    );
}

#[test]
fn a_memory_with_no_room_simply_remembers_nothing() {
    let mut memory = ViewMemory::new(ViewSettings::default(), 0);
    memory.remember(Path::new("/a"), icons(128));

    assert!(memory.is_empty());
    assert_eq!(memory.settings_for(Path::new("/a")), ViewSettings::default());
}

#[test]
fn every_mode_is_reachable_from_the_picker() {
    assert_eq!(ViewMode::all().len(), 4);
    for mode in ViewMode::all() {
        assert_eq!(ViewSettings::for_mode(mode).mode, mode);
    }
}

// --- Sorting, remembered alongside the mode --------------------------------

use kara_core::sort::{SortKey, SortOrder, SortOverrides, SortSpec};

fn by_size_descending() -> SortOverrides {
    SortOverrides {
        key: Some(SortKey::Size),
        order: Some(SortOrder::Descending),
        ..SortOverrides::default()
    }
}

#[test]
fn a_folder_nobody_sorted_follows_the_global_default() {
    let memory = ViewMemory::default();
    let sort = memory.settings_for(Path::new("/home/ana"));
    assert_eq!(sort.mode, ViewMode::Details);
    assert!(memory.sort_for(Path::new("/home/ana")).is_empty());
}

#[test]
fn sorting_is_remembered_per_folder() {
    let mut memory = ViewMemory::default();
    let folder = Path::new("/home/ana/Descargas");
    memory.remember_sort(folder, by_size_descending());

    let resolved = memory.sort_for(folder).resolve(&SortSpec::default());
    assert_eq!(resolved.key, SortKey::Size);
    assert_eq!(resolved.order, SortOrder::Descending);
    // The folders-first grouping was never overridden, so it still follows the
    // global setting.
    assert_eq!(resolved.grouping, SortSpec::default().grouping);
}

#[test]
fn choosing_a_mode_does_not_forget_the_sorting() {
    // They are two separate gestures on the same folder, and one must not undo
    // the other.
    let mut memory = ViewMemory::default();
    let folder = Path::new("/home/ana/Descargas");
    memory.remember_sort(folder, by_size_descending());
    memory.remember(folder, icons(128));

    assert_eq!(memory.settings_for(folder), icons(128));
    assert_eq!(memory.sort_for(folder), by_size_descending());
}

#[test]
fn sorting_a_folder_does_not_pin_its_mode() {
    // A folder that only ever chose a sort still follows the global default
    // view; recording the sort must not silently freeze the mode too.
    let mut memory = ViewMemory::new(ViewSettings::for_mode(ViewMode::Icons), 8);
    let folder = Path::new("/home/ana/Fotos");
    memory.remember_sort(folder, by_size_descending());

    assert_eq!(
        memory.settings_for(folder),
        ViewSettings::for_mode(ViewMode::Icons)
    );
}

#[test]
fn sorting_counts_against_the_bounded_history_too() {
    let mut memory = ViewMemory::new(ViewSettings::default(), 2);
    memory.remember_sort(Path::new("/a"), by_size_descending());
    memory.remember_sort(Path::new("/b"), by_size_descending());
    memory.remember_sort(Path::new("/c"), by_size_descending());

    assert_eq!(memory.len(), 2);
    assert!(memory.sort_for(Path::new("/a")).is_empty());
}

#[test]
fn clicking_a_header_sorts_ascending_and_clicking_again_inverts() {
    // The exact gesture the Details view needs; the rule lives in `SortSpec`,
    // and this pins it down from the caller's side.
    use kara_core::sort::ColumnId;

    let by_name = SortSpec::default();
    let by_size = by_name
        .on_header_click(&ColumnId("size".into()))
        .expect("«size» is a sortable column");
    assert_eq!(by_size.key, SortKey::Size);
    assert_eq!(by_size.order, SortOrder::Ascending);

    let inverted = by_size
        .on_header_click(&ColumnId("size".into()))
        .expect("the same column again");
    assert_eq!(inverted.order, SortOrder::Descending);

    // Moving to another column starts over ascending, even from descending.
    let by_kind = inverted
        .on_header_click(&ColumnId("kind".into()))
        .expect("«kind» is sortable");
    assert_eq!(by_kind.key, SortKey::Kind);
    assert_eq!(by_kind.order, SortOrder::Ascending);
}

// --- The names settings write to disk --------------------------------------

#[test]
fn every_mode_survives_a_trip_through_its_written_name() {
    // Settings outlive the process, so the name has to round-trip exactly.
    for mode in ViewMode::all() {
        let written = mode.to_string();
        let read: ViewMode = written.parse().expect("a mode names itself");
        assert_eq!(read, mode, "{written} did not come back as itself");
    }
}

#[test]
fn the_written_names_are_not_the_ordinals() {
    // The ordinals are an accident of declaration order; writing them would
    // mean reordering the enum silently reinterprets everything on disk.
    for mode in ViewMode::all() {
        let written = mode.to_string();
        assert!(
            written.parse::<u32>().is_err(),
            "{written} is a number, which is exactly what must not be stored"
        );
    }
}

#[test]
fn a_settings_value_from_a_newer_version_is_rejected_not_guessed() {
    // A mode this build does not know must fall back to the default, not be
    // silently turned into whichever mode happens to be first.
    assert!("content".parse::<ViewMode>().is_err());
    assert!("".parse::<ViewMode>().is_err());
}

#[test]
fn surrounding_blanks_do_not_change_the_mode() {
    assert_eq!("  icons ".parse::<ViewMode>(), Ok(ViewMode::Icons));
}
