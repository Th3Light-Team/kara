//! Pruebas de copiar, mover y renombrar.

use std::fs;
use std::path::PathBuf;

use kara_fs::transfer::{
    ConflictPolicy, Transfer, TransferError, copy_to, create_directory, move_to, rename,
    transfer_batch,
};
use tempfile::TempDir;

fn dir() -> TempDir {
    TempDir::new().expect("tempdir")
}

#[test]
fn renaming_moves_within_the_same_folder() {
    let d = dir();
    fs::write(d.path().join("viejo.txt"), b"x").unwrap();
    let out = rename(
        &d.path().join("viejo.txt"),
        std::ffi::OsStr::new("nuevo.txt"),
        ConflictPolicy::Fail,
    )
    .unwrap();
    assert_eq!(out, d.path().join("nuevo.txt"));
    assert!(!d.path().join("viejo.txt").exists());
}

/// Renombrar a un nombre ocupado no pisa nada por defecto.
#[test]
fn renaming_onto_an_existing_name_fails_by_default() {
    let d = dir();
    fs::write(d.path().join("a.txt"), b"a").unwrap();
    fs::write(d.path().join("b.txt"), b"b").unwrap();
    let e = rename(
        &d.path().join("a.txt"),
        std::ffi::OsStr::new("b.txt"),
        ConflictPolicy::Fail,
    );
    assert!(matches!(e, Err(TransferError::DestinationExists(_))));
    assert_eq!(fs::read(d.path().join("b.txt")).unwrap(), b"b", "no se piso");
}

#[test]
fn keep_both_adds_a_numeric_suffix_before_the_extension() {
    let d = dir();
    fs::write(d.path().join("informe.pdf"), b"1").unwrap();
    fs::write(d.path().join("copia.pdf"), b"2").unwrap();
    let out = rename(
        &d.path().join("copia.pdf"),
        std::ffi::OsStr::new("informe.pdf"),
        ConflictPolicy::KeepBoth,
    )
    .unwrap();
    assert_eq!(out.file_name().unwrap(), "informe (2).pdf");
    assert!(d.path().join("informe.pdf").exists(), "el original sigue");
}

#[test]
fn invalid_names_are_refused() {
    let d = dir();
    fs::write(d.path().join("a.txt"), b"x").unwrap();
    for bad in ["", "..", "con/barra"] {
        let r = rename(
            &d.path().join("a.txt"),
            std::ffi::OsStr::new(bad),
            ConflictPolicy::Fail,
        );
        assert!(matches!(r, Err(TransferError::InvalidName(_))), "{bad:?}");
    }
}

#[test]
fn moving_within_a_volume_does_not_copy_bytes() {
    let d = dir();
    fs::create_dir(d.path().join("destino")).unwrap();
    fs::write(d.path().join("a.txt"), b"12345").unwrap();
    let t = move_to(
        &d.path().join("a.txt"),
        &d.path().join("destino"),
        ConflictPolicy::Fail,
    )
    .unwrap();
    assert_eq!(t.bytes_copied, None, "un rename(2) no recorre bytes");
    assert!(d.path().join("destino/a.txt").exists());
    assert!(!d.path().join("a.txt").exists());
}

#[test]
fn copying_leaves_the_original_in_place() {
    let d = dir();
    fs::create_dir(d.path().join("destino")).unwrap();
    fs::write(d.path().join("a.txt"), b"12345").unwrap();
    let t = copy_to(
        &d.path().join("a.txt"),
        &d.path().join("destino"),
        ConflictPolicy::Fail,
    )
    .unwrap();
    assert_eq!(t.bytes_copied, Some(5));
    assert!(d.path().join("a.txt").exists(), "el original se queda");
    assert_eq!(fs::read(d.path().join("destino/a.txt")).unwrap(), b"12345");
}

#[test]
fn copying_a_directory_is_recursive() {
    let d = dir();
    fs::create_dir_all(d.path().join("origen/hondo")).unwrap();
    fs::write(d.path().join("origen/a.txt"), b"12").unwrap();
    fs::write(d.path().join("origen/hondo/b.txt"), b"345").unwrap();
    fs::create_dir(d.path().join("destino")).unwrap();

    let t = copy_to(
        &d.path().join("origen"),
        &d.path().join("destino"),
        ConflictPolicy::Fail,
    )
    .unwrap();
    assert_eq!(t.bytes_copied, Some(5));
    assert_eq!(
        fs::read(d.path().join("destino/origen/hondo/b.txt")).unwrap(),
        b"345"
    );
}

/// Seguir un enlace al copiar duplicaria el destino y, con uno a un ancestro,
/// no terminaria nunca.
#[test]
fn symlinks_are_recreated_not_followed() {
    let d = dir();
    fs::create_dir(d.path().join("destino")).unwrap();
    fs::write(d.path().join("real.txt"), b"12345").unwrap();
    std::os::unix::fs::symlink(d.path().join("real.txt"), d.path().join("enlace")).unwrap();

    let t = copy_to(
        &d.path().join("enlace"),
        &d.path().join("destino"),
        ConflictPolicy::Fail,
    )
    .unwrap();
    assert_eq!(t.bytes_copied, Some(0), "un enlace no pesa lo que su destino");
    let copiado = d.path().join("destino/enlace");
    assert!(fs::symlink_metadata(&copiado).unwrap().file_type().is_symlink());
}

/// Mover una carpeta dentro de si misma dejaria el arbol inalcanzable.
#[test]
fn moving_a_directory_into_itself_is_refused() {
    let d = dir();
    fs::create_dir_all(d.path().join("origen/dentro")).unwrap();
    let r = move_to(
        &d.path().join("origen"),
        &d.path().join("origen/dentro"),
        ConflictPolicy::Fail,
    );
    assert!(matches!(r, Err(TransferError::IntoItself(_))));
    assert!(d.path().join("origen/dentro").exists(), "nada se movio");
}

#[test]
fn overwrite_replaces_the_destination() {
    let d = dir();
    fs::create_dir(d.path().join("destino")).unwrap();
    fs::write(d.path().join("a.txt"), b"nuevo").unwrap();
    fs::write(d.path().join("destino/a.txt"), b"viejo").unwrap();

    move_to(
        &d.path().join("a.txt"),
        &d.path().join("destino"),
        ConflictPolicy::Overwrite,
    )
    .unwrap();
    assert_eq!(fs::read(d.path().join("destino/a.txt")).unwrap(), b"nuevo");
}

/// Un fallo no aborta el lote: el resto se procesa igual.
#[test]
fn one_failure_does_not_abort_the_batch() {
    let d = dir();
    fs::create_dir(d.path().join("destino")).unwrap();
    fs::write(d.path().join("a.txt"), b"a").unwrap();
    fs::write(d.path().join("c.txt"), b"c").unwrap();

    let sources: Vec<PathBuf> = vec![
        d.path().join("a.txt"),
        d.path().join("no-existe.txt"),
        d.path().join("c.txt"),
    ];
    let out = transfer_batch(
        &sources,
        &d.path().join("destino"),
        Transfer::Copy,
        ConflictPolicy::Fail,
    );
    assert_eq!(out.done.len(), 2, "los dos buenos pasan");
    assert_eq!(out.failed.len(), 1);
    assert!(out.failed[0].0.ends_with("no-existe.txt"));
}

#[test]
fn creating_a_directory_respects_the_conflict_policy() {
    let d = dir();
    let first = create_directory(d.path(), std::ffi::OsStr::new("nueva"), ConflictPolicy::Fail).unwrap();
    assert!(first.is_dir());
    assert!(matches!(
        create_directory(d.path(), std::ffi::OsStr::new("nueva"), ConflictPolicy::Fail),
        Err(TransferError::DestinationExists(_))
    ));
    let second =
        create_directory(d.path(), std::ffi::OsStr::new("nueva"), ConflictPolicy::KeepBoth).unwrap();
    assert_eq!(second.file_name().unwrap(), "nueva (2)");
}
