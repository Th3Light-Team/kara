//! Pruebas del fusionador y del watcher (`ground/spec/01-navegacion.md`).

use std::fs;
use std::path::PathBuf;

use kara_index::{Change, Coalescer, Watcher};
use tempfile::TempDir;

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

#[test]
fn an_empty_coalescer_has_nothing_to_apply() {
    let mut c = Coalescer::new();
    assert!(c.is_empty());
    assert!(c.drain().is_empty());
}

/// Aparecer y desaparecer en la misma rafaga no deja rastro: para la vista ese
/// fichero nunca existio.
#[test]
fn appearing_and_vanishing_cancel_out() {
    let mut c = Coalescer::new();
    c.push(Change::Appeared(p("/x/tmp.part")));
    c.push(Change::Vanished(p("/x/tmp.part")));
    assert!(c.drain().is_empty());
}

/// Descomprimir un tar produce miles de eventos; escribir un fichero, muchos
/// Touched sobre la misma ruta. Uno basta.
#[test]
fn repeated_touches_collapse_into_one() {
    let mut c = Coalescer::new();
    for _ in 0..500 {
        c.push(Change::Touched(p("/x/a.log")));
    }
    assert_eq!(c.drain(), vec![Change::Touched(p("/x/a.log"))]);
}

/// Un Touched sobre algo recien aparecido se absorbe: la vista va a hacer el
/// stat de todas formas.
#[test]
fn a_touch_on_something_just_created_is_absorbed() {
    let mut c = Coalescer::new();
    c.push(Change::Appeared(p("/x/nuevo.txt")));
    c.push(Change::Touched(p("/x/nuevo.txt")));
    assert_eq!(c.drain(), vec![Change::Appeared(p("/x/nuevo.txt"))]);
}

/// Releer la carpeta entera ya cubre cualquier cosa anterior.
#[test]
fn a_rescan_discards_everything_before_it() {
    let mut c = Coalescer::new();
    c.push(Change::Appeared(p("/x/a")));
    c.push(Change::Touched(p("/x/b")));
    c.push(Change::Rescan);
    c.push(Change::Appeared(p("/x/c")));
    assert_eq!(c.drain(), vec![Change::Rescan]);
    assert!(c.is_empty(), "y el rescan no se repite");
}

#[test]
fn a_rename_replaces_what_it_touches() {
    let mut c = Coalescer::new();
    c.push(Change::Touched(p("/x/viejo")));
    c.push(Change::Renamed { from: p("/x/viejo"), to: p("/x/nuevo") });
    assert_eq!(
        c.drain(),
        vec![Change::Renamed { from: p("/x/viejo"), to: p("/x/nuevo") }]
    );
}

#[test]
fn unrelated_changes_are_all_kept() {
    let mut c = Coalescer::new();
    c.push(Change::Appeared(p("/x/a")));
    c.push(Change::Vanished(p("/x/b")));
    c.push(Change::Touched(p("/x/c")));
    assert_eq!(c.drain().len(), 3);
}

// ------------------------------------------------------------- inotify real

/// Espera a que el watcher entregue algo, con limite: si inotify no funcionase
/// aqui, el test debe fallar y no colgarse.
fn wait_for(w: &Watcher) -> Vec<Change> {
    for _ in 0..100 {
        let changes = w.poll();
        if !changes.is_empty() {
            return changes;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Vec::new()
}

#[test]
fn a_new_file_is_noticed() {
    let d = TempDir::new().expect("tempdir");
    let w = Watcher::watch(d.path()).expect("inotify");
    assert!(w.poll().is_empty(), "sin cambios, nada que aplicar");

    fs::write(d.path().join("recien.txt"), b"x").expect("write");
    let changes = wait_for(&w);

    assert!(!changes.is_empty(), "inotify no entrego nada");
    assert!(
        changes.iter().any(|c| matches!(
            c,
            Change::Appeared(p) | Change::Touched(p) if p.ends_with("recien.txt")
        )) || changes.contains(&Change::Rescan),
        "no aparece el fichero nuevo: {changes:?}"
    );
}

#[test]
fn a_deletion_is_noticed() {
    let d = TempDir::new().expect("tempdir");
    let victim = d.path().join("se-va.txt");
    fs::write(&victim, b"x").expect("write");

    let w = Watcher::watch(d.path()).expect("inotify");
    fs::remove_file(&victim).expect("remove");
    let changes = wait_for(&w);

    assert!(
        changes.iter().any(|c| matches!(c, Change::Vanished(p) if p.ends_with("se-va.txt")))
            || changes.contains(&Change::Rescan),
        "no aparece el borrado: {changes:?}"
    );
}

/// Solo el nivel directo: vigilar recursivamente un $HOME grande costaria un
/// descriptor por subcarpeta.
#[test]
fn the_watch_is_not_recursive() {
    let d = TempDir::new().expect("tempdir");
    fs::create_dir(d.path().join("sub")).expect("mkdir");
    let w = Watcher::watch(d.path()).expect("inotify");
    let _ = w.poll();

    fs::write(d.path().join("sub/hondo.txt"), b"x").expect("write");
    std::thread::sleep(std::time::Duration::from_millis(120));

    let changes = w.poll();
    assert!(
        !changes.iter().any(|c| matches!(c, Change::Appeared(p) if p.ends_with("hondo.txt"))),
        "no debe vigilar dentro de las subcarpetas: {changes:?}"
    );
}
