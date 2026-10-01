//! The family icon a MIME type falls back to (`generic-icons`).

#[test]
fn generic_icons_are_read_as_type_to_icon_pairs_and_skip_comments() {
    let map = kara_fs::mime::parse_generic_icons(
        "# comment\ntext/plain:text-x-generic\napplication/zip:package-x-generic\nbroken line\n",
    );
    assert_eq!(map.get("text/plain").map(String::as_str), Some("text-x-generic"));
    assert_eq!(map.get("application/zip").map(String::as_str), Some("package-x-generic"));
    assert_eq!(map.len(), 2);
}

#[test]
fn the_generic_icon_of_a_type_is_looked_up_by_its_exact_name() {
    // `load()` fills the table from the system; whatever it holds, a type that
    // does not exist has no entry and one that does is non-empty.
    let db = kara_fs::MimeDatabase::load();
    assert_eq!(db.generic_icon("application/x-kara-test-nonexistent"), None);
    if let Some(icon) = db.generic_icon("text/plain") {
        assert!(!icon.is_empty());
    }
}
