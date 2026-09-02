//! Pruebas de la pila de deshacer (`ground/spec/05-operaciones.md`).

use std::fs;

use kara_fs::transfer::{ConflictPolicy, move_to, rename};
use kara_ops::{Action, UndoError, UndoStack};
use tempfile::TempDir;

#[test]
fn an_empty_stack_offers_nothing() {
    let mut s = UndoStack::new();
    assert!(!s.can_undo() && !s.can_redo());
    assert_eq!(s.undo_label(), None);
    assert!(matches!(s.undo(), Err(UndoError::Empty)));
}

/// La spec pide decir QUE se va a deshacer, no solo ofrecer deshacer.
#[test]
fn the_stack_says_what_it_would_undo() {
    let mut s = UndoStack::new();
    s.push(Action::Renamed { from: "/a".into(), to: "/b".into() });
    assert_eq!(s.undo_label(), Some("renombrar"));
    s.push(Action::Moved { from: "/a".into(), to: "/c/a".into() });
    assert_eq!(s.undo_label(), Some("mover"));
}

/// Debe ser una PILA de varias acciones, no solo la ultima.
#[test]
fn several_actions_stack_up() {
    let mut s = UndoStack::new();
    for i in 0..5 {
        s.push(Action::Copied { created: format!("/x/{i}").into() });
    }
    assert_eq!(s.depth(), 5);
}

/// Deshacer un renombrado recupera EXACTAMENTE el nombre previo.
#[test]
fn undoing_a_rename_restores_the_exact_previous_name() {
    let d = TempDir::new().unwrap();
    let from = d.path().join("informe final.txt");
    fs::write(&from, b"x").unwrap();
    let to = rename(&from, std::ffi::OsStr::new("otro.txt"), ConflictPolicy::Fail).unwrap();

    let mut s = UndoStack::new();
    s.push(Action::Renamed { from: from.clone(), to: to.clone() });
    s.undo().unwrap();

    assert!(from.exists(), "vuelve el nombre exacto, espacios incluidos");
    assert!(!to.exists());
    assert!(s.can_redo());
}

/// Deshacer un movimiento devuelve el fichero a su carpeta de origen.
#[test]
fn undoing_a_move_brings_it_back() {
    let d = TempDir::new().unwrap();
    fs::create_dir(d.path().join("destino")).unwrap();
    let from = d.path().join("a.txt");
    fs::write(&from, b"contenido").unwrap();
    let t = move_to(&from, &d.path().join("destino"), ConflictPolicy::Fail).unwrap();

    let mut s = UndoStack::new();
    s.push(Action::Moved { from: from.clone(), to: t.destination.clone() });
    s.undo().unwrap();

    assert_eq!(fs::read(&from).unwrap(), b"contenido");
    assert!(!t.destination.exists());
}

/// Rehacer vuelve a aplicar lo deshecho.
#[test]
fn redo_reapplies_what_was_undone() {
    let d = TempDir::new().unwrap();
    fs::create_dir(d.path().join("destino")).unwrap();
    let from = d.path().join("a.txt");
    fs::write(&from, b"x").unwrap();
    let t = move_to(&from, &d.path().join("destino"), ConflictPolicy::Fail).unwrap();

    let mut s = UndoStack::new();
    s.push(Action::Moved { from: from.clone(), to: t.destination.clone() });
    s.undo().unwrap();
    assert!(from.exists());

    s.redo().unwrap();
    assert!(!from.exists(), "vuelve a estar movido");
    assert!(t.destination.exists());
    assert!(s.can_undo());
}

/// Si el estado cambio, hay que AVISAR, no fallar en silencio ni tocar lo que
/// ocupe ahora ese sitio.
#[test]
fn undoing_something_that_vanished_reports_it() {
    let d = TempDir::new().unwrap();
    let to = d.path().join("se-fue.txt");
    let mut s = UndoStack::new();
    s.push(Action::Renamed { from: d.path().join("a.txt"), to: to.clone() });

    match s.undo() {
        Err(UndoError::Vanished(p)) => assert_eq!(p, to),
        other => panic!("esperaba Vanished, llego {other:?}"),
    }
    assert!(s.can_undo(), "la accion vuelve a la pila: no se pierde el registro");
}

/// Una operacion nueva invalida lo rehacible.
#[test]
fn a_new_action_clears_the_redo_stack() {
    let d = TempDir::new().unwrap();
    let from = d.path().join("a.txt");
    fs::write(&from, b"x").unwrap();
    let to = rename(&from, std::ffi::OsStr::new("b.txt"), ConflictPolicy::Fail).unwrap();

    let mut s = UndoStack::new();
    s.push(Action::Renamed { from, to });
    s.undo().unwrap();
    assert!(s.can_redo());

    s.push(Action::Copied { created: d.path().join("otra.txt") });
    assert!(!s.can_redo());
}

/// Deshacer una copia manda la copia a la PAPELERA, no la borra: la regla del
/// proyecto no tiene excepcion por que lo pida un deshacer.
#[test]
fn undoing_a_copy_sends_the_copy_to_the_trash() {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return;
    }
    let copia = std::path::PathBuf::from(&home).join("kara-undo-copia.txt");
    fs::write(&copia, b"copia de prueba").unwrap();

    let mut s = UndoStack::new();
    s.push(Action::Copied { created: copia.clone() });
    s.undo().unwrap();

    assert!(!copia.exists(), "la copia se fue");
    let en_papelera = std::path::PathBuf::from(&home)
        .join(".local/share/Trash/files/kara-undo-copia.txt");
    assert!(en_papelera.exists(), "y esta en la papelera, no borrada");
    let _ = fs::remove_file(&en_papelera);
    let _ = fs::remove_file(
        std::path::PathBuf::from(&home)
            .join(".local/share/Trash/info/kara-undo-copia.txt.trashinfo"),
    );
}
