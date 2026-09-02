//! Pruebas del autocompletado de rutas (`ground/spec/01-navegacion.md`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use kara_core::{
    Completer, EntryKind, FileEntry, MetadataBag, PathInput, Source, split_input,
};

fn dir(name: &str) -> FileEntry {
    entry(name, EntryKind::Directory)
}

fn entry(name: &str, kind: EntryKind) -> FileEntry {
    FileEntry {
        name: OsString::from(name),
        display: name.to_string(),
        kind,
        is_symlink: false,
        symlink_broken: false,
        is_hidden: name.starts_with('.'),
        size: None,
        modified: None,
        created: None,
        accessed: None,
        type_label: None,
        location: None,
        extra: MetadataBag::new(),
    }
}

fn children() -> Vec<FileEntry> {
    vec![
        dir("Proyectos"),
        dir("proyectos-viejos"),
        dir("Descargas"),
        entry("proyecto.txt", EntryKind::File),
    ]
}

// ------------------------------------------------------- partir lo tecleado

/// Escribir el separador debe ofrecer el contenido del nuevo nivel: es lo que
/// hace que el completado avance segmento a segmento.
#[test]
fn the_separator_advances_to_the_next_level() {
    assert_eq!(
        split_input("/home/oli"),
        PathInput { directory: PathBuf::from("/home/"), prefix: "oli".into() }
    );
    assert_eq!(
        split_input("/home/"),
        PathInput { directory: PathBuf::from("/home/"), prefix: String::new() },
        "con el separador recien escrito, el prefijo queda vacio"
    );
    assert_eq!(
        split_input("/"),
        PathInput { directory: PathBuf::from("/"), prefix: String::new() }
    );
    assert_eq!(
        split_input("sin-barra"),
        PathInput { directory: PathBuf::new(), prefix: "sin-barra".into() }
    );
}

// ------------------------------------------------------------- sugerencias

#[test]
fn only_directories_are_offered() {
    let input = split_input("/home/pro");
    let s = Completer { case_sensitive: false }.suggest(&input, &children(), &[]);
    let names: Vec<String> = s.items().iter().map(|x| x.name.to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["Proyectos", "proyectos-viejos"],
        "proyecto.txt es un fichero: la barra navega, no abre");
}

#[test]
fn case_sensitivity_is_a_property_of_the_filesystem() {
    let input = split_input("/home/Pro");
    let sensitive = Completer { case_sensitive: true }.suggest(&input, &children(), &[]);
    assert_eq!(sensitive.items().len(), 1, "en ext4, 'Pro' no encuentra 'proyectos-viejos'");

    let insensitive = Completer { case_sensitive: false }.suggest(&input, &children(), &[]);
    assert_eq!(insensitive.items().len(), 2);
}

#[test]
fn suggested_paths_are_built_on_the_typed_directory() {
    let input = split_input("/home/oli/Pro");
    let s = Completer::default().suggest(&input, &children(), &[]);
    assert_eq!(s.items()[0].path, Path::new("/home/oli/Proyectos"));
}

/// El historial se mezcla, pero detras y sin repetir lo que el disco ya ofrece.
#[test]
fn history_complements_the_disk_without_duplicating_it() {
    let input = split_input("/home/oli/Pro");
    let history = vec![
        PathBuf::from("/home/oli/Proyectos"),
        PathBuf::from("/home/oli/Prototipos-borrados"),
    ];
    let s = Completer::default().suggest(&input, &children(), &history);

    assert_eq!(s.items().len(), 2, "Proyectos ya venia del disco: no se repite");
    assert_eq!(s.items()[0].source, Source::Disk);
    assert_eq!(s.items()[1].source, Source::History);
    assert_eq!(s.items()[1].path, Path::new("/home/oli/Prototipos-borrados"));
}

/// Una ruta que aun no existe tiene que poder escribirse.
#[test]
fn a_path_that_does_not_exist_yet_is_never_blocked() {
    let input = split_input("/home/carpeta-que-creare-luego");
    let s = Completer::default().suggest(&input, &children(), &[]);
    assert!(s.is_empty(), "no hay sugerencias");
    assert!(s.accept().is_none(), "y Enter navega lo escrito, no una sugerencia impuesta");
    assert!(s.inline_remainder().is_none());
}

// ------------------------------------------------- recorrer y completar

#[test]
fn tab_cycles_through_the_suggestions_both_ways() {
    let input = split_input("/home/pro");
    let mut s = Completer { case_sensitive: false }.suggest(&input, &children(), &[]);
    assert!(s.selected().is_none(), "nada resaltado hasta que se recorre");

    s.next();
    assert_eq!(s.selected().unwrap().name, OsString::from("Proyectos"));
    s.next();
    assert_eq!(s.selected().unwrap().name, OsString::from("proyectos-viejos"));
    s.next();
    assert_eq!(s.selected().unwrap().name, OsString::from("Proyectos"), "da la vuelta");
    s.previous();
    assert_eq!(s.selected().unwrap().name, OsString::from("proyectos-viejos"));
}

#[test]
fn accept_returns_the_highlighted_path() {
    let input = split_input("/home/Pro");
    let mut s = Completer::default().suggest(&input, &children(), &[]);
    s.next();
    assert_eq!(s.accept(), Some(Path::new("/home/Proyectos")));
}

#[test]
fn the_inline_remainder_is_what_is_left_to_type() {
    let input = split_input("/home/Pro");
    let s = Completer::default().suggest(&input, &children(), &[]);
    assert_eq!(s.inline_remainder(), Some("yectos"), "Pro + yectos = Proyectos");
}

/// Esc cierra la lista sin tocar lo escrito.
#[test]
fn dismissing_clears_the_list_only() {
    let input = split_input("/home/pro");
    let mut s = Completer { case_sensitive: false }.suggest(&input, &children(), &[]);
    s.next();
    s.dismiss();
    assert!(s.is_empty());
    assert!(s.selected().is_none());
    assert!(s.accept().is_none(), "tras Esc, Enter navega lo tecleado");
}
