//! Pruebas del listado de directorios.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use kara_core::EntryKind;
use kara_fs::{describe, list_directory, read_hidden_file};
use tempfile::TempDir;

fn tree() -> TempDir {
    let d = TempDir::new().unwrap();
    let r = d.path();
    fs::write(r.join("a.txt"), b"12345").unwrap();
    fs::write(r.join(".oculto"), b"x").unwrap();
    fs::create_dir(r.join("sub")).unwrap();
    std::os::unix::fs::symlink(r.join("sub"), r.join("enlace-a-carpeta")).unwrap();
    std::os::unix::fs::symlink(r.join("no-existe"), r.join("enlace-roto")).unwrap();
    d
}

fn find<'a>(l: &'a kara_fs::Listing, name: &str) -> &'a kara_core::FileEntry {
    l.entries.iter().find(|e| e.display == name).expect(name)
}

#[test]
fn lists_every_entry_including_hidden_ones() {
    let d = tree();
    let l = list_directory(d.path()).unwrap();
    assert_eq!(l.entries.len(), 5, "listar no filtra: eso es de kara-core");
    assert!(l.errors.is_empty());
    assert!(find(&l, ".oculto").is_hidden);
    assert!(!find(&l, "a.txt").is_hidden);
}

/// Un enlace a carpeta agrupa con las carpetas; uno roto, con los ficheros.
#[test]
fn symlinks_resolve_their_kind_through_the_target() {
    let d = tree();
    let l = list_directory(d.path()).unwrap();

    let ok = find(&l, "enlace-a-carpeta");
    assert_eq!(ok.kind, EntryKind::Directory);
    assert!(ok.is_symlink && !ok.symlink_broken);

    let roto = find(&l, "enlace-roto");
    assert_eq!(roto.kind, EntryKind::File, "un enlace roto no es una carpeta");
    assert!(roto.is_symlink && roto.symlink_broken);
}

/// El tamano de una carpeta es None, no cero: calcularlo es recursivo.
#[test]
fn a_directory_has_no_size_rather_than_zero() {
    let d = tree();
    let l = list_directory(d.path()).unwrap();
    assert_eq!(find(&l, "sub").size, None);
    assert_eq!(find(&l, "a.txt").size, Some(5));
}

#[test]
fn entries_carry_their_location_and_timestamps() {
    let d = tree();
    let l = list_directory(d.path()).unwrap();
    let a = find(&l, "a.txt");
    assert_eq!(a.location.as_deref(), Some(d.path()));
    assert!(a.modified.is_some());
}

/// Un directorio que no se puede abrir si es error de nivel superior.
#[test]
fn an_unreadable_directory_is_a_top_level_error() {
    let d = TempDir::new().unwrap();
    let cerrada = d.path().join("cerrada");
    fs::create_dir(&cerrada).unwrap();
    fs::set_permissions(&cerrada, fs::Permissions::from_mode(0o000)).unwrap();

    let r = list_directory(&cerrada);
    fs::set_permissions(&cerrada, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(r.is_err());
}

/// El .hidden de la carpeta se lee; que no exista no es un error.
#[test]
fn the_dot_hidden_file_is_read_when_present() {
    let d = TempDir::new().unwrap();
    assert!(read_hidden_file(d.path()).is_empty(), "sin .hidden no oculta nada");

    fs::write(d.path().join(".hidden"), "notas.txt\n\n  otro.txt  \n").unwrap();
    let h = read_hidden_file(d.path());
    assert_eq!(h.len(), 2, "lineas vacias fuera, espacios recortados");
    assert!(h.contains(std::ffi::OsStr::new("otro.txt")));
}

#[test]
fn describe_works_on_a_single_path() {
    let d = tree();
    let e = describe(&d.path().join("a.txt")).unwrap();
    assert_eq!(e.display, "a.txt");
    assert_eq!(e.size, Some(5));
    assert!(describe(&d.path().join("no-existe")).is_err());
}
