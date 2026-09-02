//! Pruebas de historial y breadcrumb (`ground/spec/01-navegacion.md`).

use kara_core::{
    History, SegmentKind, ViewState, collapse, segments,
};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

fn state(scroll: f64, selected: &[&str]) -> ViewState {
    ViewState {
        selection: selected.iter().map(OsString::from).collect::<BTreeSet<_>>(),
        focused: selected.first().map(OsString::from),
        scroll,
    }
}

// ---------------------------------------------------------------- historial

/// Atrás y Adelante recorren la pila; Adelante solo existe tras retroceder.
#[test]
fn back_and_forward_walk_the_stack() {
    let mut h = History::new("/a");
    assert!(!h.can_go_back() && !h.can_go_forward(), "una sola entrada no navega");

    h.visit("/a/b");
    h.visit("/a/b/c");
    assert!(h.can_go_back() && !h.can_go_forward());

    assert_eq!(h.back().map(|e| e.path.clone()), Some(PathBuf::from("/a/b")));
    assert!(h.can_go_forward(), "tras retroceder, Adelante se habilita");
    assert_eq!(h.back().map(|e| e.path.clone()), Some(PathBuf::from("/a")));
    assert!(!h.can_go_back(), "en el fondo de la pila, Atras se deshabilita");

    assert_eq!(h.forward().map(|e| e.path.clone()), Some(PathBuf::from("/a/b")));
}

/// Navegar tras retroceder trunca la pila de adelante.
#[test]
fn visiting_after_going_back_truncates_the_forward_stack() {
    let mut h = History::new("/a");
    h.visit("/a/b");
    h.visit("/a/b/c");
    h.back();

    h.visit("/a/z");
    assert!(!h.can_go_forward(), "/a/b/c ya no es alcanzable");
    assert_eq!(h.current_path(), Path::new("/a/z"));
    assert_eq!(h.len(), 3, "quedan /a, /a/b y /a/z");
}

/// Al volver se restauran selección y scroll: lo que la spec llama la diferencia
/// entre una buena implementación y una mediocre.
#[test]
fn going_back_restores_selection_and_scroll() {
    let mut h = History::new("/a");
    h.set_state(state(420.0, &["informe.pdf", "notas.txt"]));
    h.visit("/a/b");

    let restored = h.back().expect("hay historial atras");
    assert_eq!(restored.state.scroll, 420.0);
    assert_eq!(restored.state.focused.as_deref(), Some(OsString::from("informe.pdf").as_os_str()));
    assert!(restored.state.selection.contains(&OsString::from("notas.txt")));
}

/// Re-listar la carpeta en la que ya estás no es un movimiento de historial.
#[test]
fn revisiting_the_current_path_is_not_a_history_move() {
    let mut h = History::new("/a");
    h.set_state(state(99.0, &["x"]));
    h.visit("/a");

    assert_eq!(h.len(), 1, "no se apila un duplicado");
    assert_eq!(h.current().state.scroll, 99.0, "y no se pierde el estado");
}

/// Una carpeta que deja de existir se salta: se va a la mas cercana disponible.
#[test]
fn invalid_entries_are_skipped_in_both_directions() {
    let mut h = History::new("/a");
    h.visit("/borrada");
    h.visit("/tambien-borrada");
    h.visit("/c");
    h.invalidate(Path::new("/borrada"));
    h.invalidate(Path::new("/tambien-borrada"));

    assert_eq!(h.back().map(|e| e.path.clone()), Some(PathBuf::from("/a")),
        "salta las dos invalidas de golpe");
    assert_eq!(h.forward().map(|e| e.path.clone()), Some(PathBuf::from("/c")));
}

/// Si no queda ninguna entrada valida hacia atras, no se mueve nada.
#[test]
fn back_without_any_valid_entry_does_not_move() {
    let mut h = History::new("/a");
    h.visit("/b");
    h.invalidate(Path::new("/a"));

    assert!(!h.can_go_back(), "el boton debe verse atenuado");
    assert!(h.back().is_none());
    assert_eq!(h.current_path(), Path::new("/b"), "el cursor no se mueve");
}

/// Subir un nivel deja seleccionada la carpeta de la que se venia.
#[test]
fn going_up_preselects_the_folder_left_behind() {
    let s = ViewState::selecting("Proyectos");
    assert_eq!(s.focused.as_deref(), Some(OsString::from("Proyectos").as_os_str()));
    assert_eq!(s.selection.len(), 1);
}

// --------------------------------------------------------------- breadcrumb

#[test]
fn segments_split_an_absolute_path_and_mark_home() {
    let home = PathBuf::from("/home/oliverv");
    let s = segments(Path::new("/home/oliverv/Projects/kara"), Some(&home));

    let names: Vec<String> = s.iter().map(|x| x.name.to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["/", "home", "oliverv", "Projects", "kara"]);

    assert_eq!(s[0].kind, SegmentKind::Root);
    assert_eq!(s[2].kind, SegmentKind::Home, "la carpeta personal se marca");
    assert_eq!(s[1].kind, SegmentKind::Directory, "y sus ancestros siguen ahi");
    assert_eq!(s[3].path, PathBuf::from("/home/oliverv/Projects"),
        "cada segmento navega a su propio ancestro");
}

#[test]
fn segments_of_root_and_of_a_relative_path() {
    assert_eq!(segments(Path::new("/"), None).len(), 1);
    assert!(segments(Path::new("relativa/sin/raiz"), None).is_empty(),
        "una ruta relativa no tiene ancestros que ofrecer");
}

#[test]
fn collapse_hides_the_farthest_ancestors_first() {
    let s = segments(Path::new("/a/b/c/d/e"), None); // 6 segmentos con la raiz
    let c = collapse(&s, 3);

    assert_eq!(c.visible.len(), 3);
    assert_eq!(c.overflow.len(), 3);
    let visibles: Vec<String> = c.visible.iter().map(|x| x.name.to_string_lossy().into_owned()).collect();
    assert_eq!(visibles, vec!["c", "d", "e"], "se recorta por el principio");
    assert_eq!(c.overflow[0].name, OsString::from("/"), "la raiz es la primera en caer");
}

#[test]
fn collapse_never_hides_the_current_folder_nor_its_parent() {
    let s = segments(Path::new("/a/b/c/d/e"), None);
    for max in [0, 1] {
        let c = collapse(&s, max);
        assert_eq!(c.visible.len(), 2, "aunque se pida menos, quedan actual y padre");
        let visibles: Vec<String> = c.visible.iter().map(|x| x.name.to_string_lossy().into_owned()).collect();
        assert_eq!(visibles, vec!["d", "e"]);
    }
}

#[test]
fn collapse_does_nothing_when_everything_fits() {
    let s = segments(Path::new("/a/b"), None);
    let c = collapse(&s, 10);
    assert!(c.overflow.is_empty());
    assert_eq!(c.visible.len(), s.len());
}

#[test]
fn collapse_of_a_single_segment_keeps_it() {
    let s = segments(Path::new("/"), None);
    let c = collapse(&s, 0);
    assert_eq!(c.visible.len(), 1, "el minimo se acota a lo que hay");
    assert!(c.overflow.is_empty());
}
