//! Which icon theme Kara picks, and that it can load one from a folder it was
//! pointed at. Everything here changes process-wide environment variables, so it
//! is a single test in its own binary: nothing else runs beside it.

use std::fs;
use std::path::Path;

use kara_fs::icons::{IconTheme, current_theme_name};

fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)
}

#[test]
fn the_theme_comes_from_kde_then_gtk_and_a_theme_on_disk_can_be_loaded() -> std::io::Result<()> {
    let home = tempfile::tempdir()?;
    let config = home.path().join("config");
    let data = home.path().join("share");
    // SAFETY: the only test in this binary, so no other thread reads the
    // environment while it is being changed.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_DATA_DIRS", &data);
        std::env::set_var("XDG_DATA_HOME", home.path().join("none"));
    }

    // GTK alone.
    write(
        &config.join("gtk-3.0/settings.ini"),
        "[Settings]\ngtk-icon-theme-name = FromGtk\n",
    )?;
    assert_eq!(current_theme_name(), "FromGtk");

    // KDE wins when both are there.
    write(&config.join("kdeglobals"), "[General]\nx=1\n[Icons]\nTheme=FromKde\n")?;
    assert_eq!(current_theme_name(), "FromKde");

    // A kdeglobals without an [Icons] section does not hide GTK's answer.
    write(&config.join("kdeglobals"), "[General]\nTheme=NotAnIconTheme\n")?;
    assert_eq!(current_theme_name(), "FromGtk");

    // A key in the wrong section is not an answer either; an unreadable pair of
    // files leaves a usable name, never a panic or an empty string.
    write(&config.join("gtk-3.0/settings.ini"), "garbage with no sections")?;
    assert!(!current_theme_name().is_empty());

    // A theme laid out on disk is found through the data directories.
    write(
        &data.join("icons/Fake/index.theme"),
        "[Icon Theme]\nName=Fake\nDirectories=16x16/places\n\n[16x16/places]\nSize=16\nType=Fixed\n",
    )?;
    write(&data.join("icons/Fake/16x16/places/folder.png"), "not really a png")?;

    let theme = IconTheme::named("Fake");
    assert!(!theme.is_empty());
    let found = theme.find("folder", 16).expect("the theme has a folder icon");
    assert!(found.ends_with("Fake/16x16/places/folder.png"), "{}", found.display());
    assert_eq!(theme.find("no-such-icon", 16), None);

    // A theme nobody installed is empty rather than an error.
    assert!(IconTheme::named("NotInstalledAnywhere").is_empty());
    Ok(())
}
