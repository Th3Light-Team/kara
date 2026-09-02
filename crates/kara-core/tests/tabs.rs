//! Tab model: independent history per tab, open/close/reopen, reordering.
//!
//! Reference: `ground/spec/01-navegacion.md`, "Pestañas de carpetas",
//! "Reabrir pestaña cerrada", "Abrir carpeta en pestaña o ventana nueva".

use kara_core::{CloseOutcome, OpenMode, Tabs, REOPEN_CAPACITY};

fn paths(tabs: &Tabs) -> Vec<String> {
    tabs.tabs()
        .map(|tab| tab.path().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_new_bar_starts_with_one_active_tab() {
    let tabs = Tabs::new("/home/oliverv");
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs.active().path().to_str(), Some("/home/oliverv"));
    assert_eq!(tabs.active_id(), tabs.tabs().next().unwrap().id());
}

#[test]
fn each_tab_owns_an_independent_history() {
    let mut tabs = Tabs::new("/home");
    let first = tabs.active_id();
    let second = tabs.open("/tmp", OpenMode::Foreground);

    // Navigate the second tab's history; the first tab's history must not
    // see it, because they are not the same History instance.
    tabs.get_mut(second).unwrap().history_mut().visit("/tmp/a");
    assert_eq!(tabs.get(second).unwrap().path().to_str(), Some("/tmp/a"));
    assert_eq!(tabs.get(first).unwrap().path().to_str(), Some("/home"));

    // And back/forward on one tab does not touch the other's cursor.
    assert!(tabs.get_mut(second).unwrap().history_mut().back().is_some());
    assert_eq!(tabs.get(second).unwrap().path().to_str(), Some("/tmp"));
    assert_eq!(tabs.get(first).unwrap().path().to_str(), Some("/home"));
}

#[test]
fn opening_in_the_foreground_moves_focus_to_the_new_tab() {
    let mut tabs = Tabs::new("/home");
    let new_tab = tabs.open("/tmp", OpenMode::Foreground);
    assert_eq!(tabs.active_id(), new_tab);
}

#[test]
fn opening_in_the_background_leaves_focus_where_it_was() {
    // Spec: middle click and Ctrl+click open a background tab and the focus
    // stays on the current tab. This is the one case the spec fixes
    // explicitly, so it gets its own discriminating test.
    let mut tabs = Tabs::new("/home");
    let original = tabs.active_id();
    let new_tab = tabs.open("/tmp", OpenMode::Background);
    assert_eq!(tabs.active_id(), original);
    assert_ne!(tabs.active_id(), new_tab);
    assert_eq!(tabs.len(), 2);
}

#[test]
fn new_tabs_are_appended_at_the_end_of_the_bar() {
    let mut tabs = Tabs::new("/a");
    tabs.open("/b", OpenMode::Background);
    tabs.activate_at(0);
    tabs.open("/c", OpenMode::Background);
    // Opened while /a was active, but still lands at the end, not next to /a.
    assert_eq!(paths(&tabs), vec!["/a", "/b", "/c"]);
}

#[test]
fn closing_a_tab_that_is_not_active_does_not_move_the_focus() {
    let mut tabs = Tabs::new("/a");
    let active = tabs.active_id();
    let victim = tabs.open("/b", OpenMode::Background);

    let outcome = tabs.close(victim);
    assert_eq!(outcome, CloseOutcome::Closed { focus: active });
    assert_eq!(tabs.active_id(), active);
    assert_eq!(tabs.len(), 1);
}

#[test]
fn closing_the_active_tab_focuses_the_tab_that_slides_into_its_slot() {
    // Chrome-style: closing the middle tab of a/b/c focuses c (what used to
    // be "the next tab"), not a.
    let mut tabs = Tabs::new("/a");
    tabs.open("/b", OpenMode::Background);
    let c = tabs.open("/c", OpenMode::Background);
    let b = tabs.tabs().nth(1).unwrap().id();
    tabs.activate(b);

    let outcome = tabs.close(b);
    assert_eq!(outcome, CloseOutcome::Closed { focus: c });
    assert_eq!(tabs.active_id(), c);
}

#[test]
fn closing_the_active_last_tab_falls_back_to_the_new_last_tab() {
    // There is no "next" tab when the closed one was rightmost, so focus
    // falls back to whatever is now last.
    let mut tabs = Tabs::new("/a");
    let b = tabs.open("/b", OpenMode::Foreground);
    let a = tabs.tabs().next().unwrap().id();

    let outcome = tabs.close(b);
    assert_eq!(outcome, CloseOutcome::Closed { focus: a });
    assert_eq!(tabs.active_id(), a);
}

#[test]
fn closing_the_only_tab_is_refused_and_changes_nothing() {
    let mut tabs = Tabs::new("/a");
    let only = tabs.active_id();

    let outcome = tabs.close(only);
    assert_eq!(outcome, CloseOutcome::LastTab);
    // Nothing was mutated: still one tab, same one, still active.
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs.active_id(), only);
    assert_eq!(tabs.get(only).unwrap().path().to_str(), Some("/a"));
}

#[test]
fn closing_an_id_that_no_longer_exists_is_reported_and_does_nothing() {
    let mut tabs = Tabs::new("/a");
    let b = tabs.open("/b", OpenMode::Background);
    assert_eq!(tabs.close(b), CloseOutcome::Closed { focus: tabs.active_id() });

    // b is gone now; closing it again must not panic or resurrect it.
    assert_eq!(tabs.close(b), CloseOutcome::NotFound);
    assert_eq!(tabs.len(), 1);
}

#[test]
fn close_others_keeps_only_the_given_tab_and_focuses_it() {
    let mut tabs = Tabs::new("/a");
    tabs.open("/b", OpenMode::Background);
    let c = tabs.open("/c", OpenMode::Background);

    tabs.close_others(c);
    assert_eq!(paths(&tabs), vec!["/c"]);
    assert_eq!(tabs.active_id(), c);
}

#[test]
fn close_to_the_right_only_closes_tabs_after_the_given_one() {
    let mut tabs = Tabs::new("/a");
    let b = tabs.open("/b", OpenMode::Background);
    tabs.open("/c", OpenMode::Background);
    tabs.open("/d", OpenMode::Background);

    tabs.close_to_the_right(b);
    assert_eq!(paths(&tabs), vec!["/a", "/b"]);
}

#[test]
fn reopening_restores_the_full_history_not_just_the_path() {
    // This is what distinguishes "reopen" from "open a new tab at that path":
    // the back/forward stack has to come back too.
    let mut tabs = Tabs::new("/a");
    let b = tabs.open("/b", OpenMode::Foreground);
    tabs.get_mut(b).unwrap().history_mut().visit("/b/x");
    tabs.get_mut(b).unwrap().history_mut().visit("/b/y");
    tabs.close(b);

    let reopened = tabs.reopen().expect("a tab was just closed");
    let history = tabs.get(reopened).unwrap().history();
    assert_eq!(history.current_path().to_str(), Some("/b/y"));
    assert!(history.can_go_back());
}

#[test]
fn reopening_puts_the_tab_back_in_its_original_slot_and_focuses_it() {
    let mut tabs = Tabs::new("/a");
    let b = tabs.open("/b", OpenMode::Background);
    tabs.open("/c", OpenMode::Background);
    tabs.close(b); // closes the middle tab: a, c remain

    let reopened = tabs.reopen().unwrap();
    assert_eq!(paths(&tabs), vec!["/a", "/b", "/c"]);
    assert_eq!(tabs.active_id(), reopened);
}

#[test]
fn reopening_with_nothing_closed_returns_none() {
    let mut tabs = Tabs::new("/a");
    assert_eq!(tabs.reopen(), None);
}

#[test]
fn reopen_stack_replays_closures_in_reverse_order() {
    let mut tabs = Tabs::new("/a");
    let b = tabs.open("/b", OpenMode::Background);
    let c = tabs.open("/c", OpenMode::Background);
    tabs.close(b);
    tabs.close(c);

    // Most recently closed comes back first.
    let first = tabs.reopen().unwrap();
    assert_eq!(tabs.get(first).unwrap().path().to_str(), Some("/c"));
    let second = tabs.reopen().unwrap();
    assert_eq!(tabs.get(second).unwrap().path().to_str(), Some("/b"));
}

#[test]
fn the_reopen_stack_is_bounded_and_forgets_the_oldest_closure() {
    let mut tabs = Tabs::new("/base");
    // Open and immediately close more tabs than the stack can hold.
    for n in 0..(REOPEN_CAPACITY + 3) {
        let path = format!("/t{n}");
        let id = tabs.open(path, OpenMode::Background);
        tabs.close(id);
    }
    assert_eq!(tabs.reopenable_count(), REOPEN_CAPACITY);

    // The last tab reopened must be the most recent closure the bound could
    // still hold, i.e. the ones from the very start (t0, t1, t2) fell off.
    let mut recovered = Vec::new();
    while let Some(id) = tabs.reopen() {
        recovered.push(tabs.get(id).unwrap().path().to_string_lossy().into_owned());
    }
    assert_eq!(recovered.len(), REOPEN_CAPACITY);
    assert!(!recovered.contains(&"/t0".to_string()));
    assert!(!recovered.contains(&"/t1".to_string()));
    assert!(!recovered.contains(&"/t2".to_string()));
    assert!(recovered.contains(&format!("/t{}", REOPEN_CAPACITY + 2)));
}

#[test]
fn duplicating_a_tab_clones_its_history_next_to_the_original() {
    let mut tabs = Tabs::new("/a");
    let b = tabs.open("/b", OpenMode::Background);
    tabs.open("/c", OpenMode::Background); // sits after b, so "adjacent" and
                                            // "appended at the end" differ
    tabs.get_mut(b).unwrap().history_mut().visit("/b/x");

    let dup = tabs.duplicate(b, OpenMode::Background).unwrap();
    // Right next to the original (now at /b/x), not appended at the end like
    // `open` would put it.
    assert_eq!(paths(&tabs), vec!["/a", "/b/x", "/b/x", "/c"]);
    assert_eq!(tabs.get(dup).unwrap().path().to_str(), Some("/b/x"));
    // It is a copy: navigating the duplicate must not move the original.
    tabs.get_mut(dup).unwrap().history_mut().visit("/b/y");
    assert_eq!(tabs.get(b).unwrap().path().to_str(), Some("/b/x"));
}

#[test]
fn duplicating_an_id_that_does_not_exist_returns_none() {
    let mut tabs = Tabs::new("/a");
    let b = tabs.open("/b", OpenMode::Background);
    tabs.close(b);
    assert_eq!(tabs.duplicate(b, OpenMode::Foreground), None);
}

#[test]
fn dragging_a_tab_reorders_the_bar_and_focus_follows_the_moved_tab() {
    let mut tabs = Tabs::new("/a");
    tabs.open("/b", OpenMode::Background);
    tabs.open("/c", OpenMode::Background);
    let a = tabs.tabs().next().unwrap().id();
    tabs.activate(a);

    // Drag /a (index 0) to the end.
    tabs.move_tab(0, 2);
    assert_eq!(paths(&tabs), vec!["/b", "/c", "/a"]);
    // The focus is still on the tab that moved, not on whatever now sits at
    // index 0.
    assert_eq!(tabs.active_id(), a);
}

#[test]
fn moving_a_tab_with_an_out_of_range_source_is_a_total_no_op() {
    let mut tabs = Tabs::new("/a");
    tabs.open("/b", OpenMode::Background);
    tabs.move_tab(99, 0);
    assert_eq!(paths(&tabs), vec!["/a", "/b"]);
}

#[test]
fn moving_a_tab_to_an_out_of_range_destination_clamps_to_the_end() {
    let mut tabs = Tabs::new("/a");
    tabs.open("/b", OpenMode::Background);
    tabs.open("/c", OpenMode::Background);
    tabs.move_tab(0, 999);
    assert_eq!(paths(&tabs), vec!["/b", "/c", "/a"]);
}

#[test]
fn activate_at_clamps_an_out_of_range_index_to_the_last_tab() {
    let mut tabs = Tabs::new("/a");
    tabs.open("/b", OpenMode::Background);
    tabs.open("/c", OpenMode::Background);

    tabs.activate_at(50);
    assert_eq!(tabs.active().path().to_str(), Some("/c"));
}

#[test]
fn activate_last_jumps_to_the_rightmost_tab() {
    let mut tabs = Tabs::new("/a");
    tabs.open("/b", OpenMode::Background);
    let c = tabs.open("/c", OpenMode::Background);
    tabs.activate_at(0);

    tabs.activate_last();
    assert_eq!(tabs.active_id(), c);
}

#[test]
fn activate_next_and_previous_cycle_around_the_ends() {
    let mut tabs = Tabs::new("/a");
    tabs.open("/b", OpenMode::Background);
    tabs.open("/c", OpenMode::Background);
    let ids: Vec<_> = tabs.tabs().map(|tab| tab.id()).collect();
    tabs.activate(ids[0]);

    tabs.activate_next();
    assert_eq!(tabs.active_id(), ids[1]);
    tabs.activate_next();
    assert_eq!(tabs.active_id(), ids[2]);
    tabs.activate_next(); // wraps
    assert_eq!(tabs.active_id(), ids[0]);

    tabs.activate_previous(); // wraps the other way
    assert_eq!(tabs.active_id(), ids[2]);
}

#[test]
fn activating_an_id_that_no_longer_exists_is_a_no_op() {
    let mut tabs = Tabs::new("/a");
    let b = tabs.open("/b", OpenMode::Background);
    let a = tabs.tabs().next().unwrap().id();
    tabs.close(b);

    tabs.activate(b);
    assert_eq!(tabs.active_id(), a);
}

#[test]
fn the_bar_never_reports_as_empty() {
    let mut tabs = Tabs::new("/a");
    let only = tabs.active_id();
    tabs.close(only); // refused: it's the last tab
    assert!(!tabs.is_empty());
    assert_eq!(tabs.len(), 1);
}
