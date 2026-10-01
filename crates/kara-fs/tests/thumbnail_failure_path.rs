//! Where a failed thumbnail attempt is remembered.

use std::path::Path;

use kara_fs::{ThumbnailSize, Thumbnails};

#[test]
fn a_failed_attempt_is_recorded_under_fail_and_this_application_not_beside_the_thumbnails() -> std::io::Result<()> {
    let root = tempfile::tempdir()?;
    let store = Thumbnails::at(root.path(), ThumbnailSize::Normal);
    let file = Path::new("/home/ana/Pictures/broken.png");

    let failure = store.failure_path_for(file);
    let thumbnail = store.path_for(file);

    assert!(
        failure.starts_with(root.path().join("fail").join("kara")),
        "{}",
        failure.display()
    );
    // Same name the thumbnail would have, so a lookup needs only the file.
    assert_eq!(failure.file_name(), thumbnail.file_name());
    assert_ne!(failure.parent(), thumbnail.parent());
    Ok(())
}

#[test]
fn two_different_files_never_share_a_failure_record() -> std::io::Result<()> {
    let root = tempfile::tempdir()?;
    let store = Thumbnails::at(root.path(), ThumbnailSize::Normal);
    assert_ne!(
        store.failure_path_for(Path::new("/a/one.png")),
        store.failure_path_for(Path::new("/a/two.png"))
    );
    Ok(())
}
