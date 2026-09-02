//! Pruebas de visibilidad, nombres, filtro en vivo y type-ahead
//! (`ground/spec/03-vistas.md` y `04-busqueda.md`).

use std::collections::BTreeSet;
use std::ffi::OsString;

use kara_core::{
    EntryKind, FileEntry, MetadataBag, NameDisplay, NameFilter, TypeAhead, Visibility,
    base_and_extension, fold_for_match,
};

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

fn file(name: &str) -> FileEntry {
    entry(name, EntryKind::File)
}

// ---------------------------------------------------------------- ocultos

#[test]
fn hidden_entries_are_filtered_unless_shown() {
    let mut v = vec![file("a.txt"), file(".bashrc"), file("b.txt")];
    let hide = Visibility::default();
    assert!(!hide.show_hidden, "el defecto es ocultarlos");

    let mut shown = v.clone();
    hide.retain_visible(&mut shown);
    assert_eq!(shown.len(), 2);

    let show = Visibility { show_hidden: true, ..Visibility::default() };
    show.retain_visible(&mut v);
    assert_eq!(v.len(), 3);
}

/// La spec pide respetar el `.hidden` de la carpeta en Linux.
#[test]
fn names_listed_in_dot_hidden_are_also_hidden() {
    let vis = Visibility {
        show_hidden: false,
        hidden_names: BTreeSet::from([OsString::from("notas.txt")]),
    };
    assert!(vis.is_hidden(&file("notas.txt")), "sin punto inicial, pero listado");
    assert!(!vis.is_visible(&file("notas.txt")));
    assert!(vis.is_visible(&file("otras.txt")));
}

// ------------------------------------------------------------- extensiones

#[test]
fn extensions_can_be_hidden_from_the_label() {
    let hide = NameDisplay { show_extensions: false, always_show_for_executables: true };
    assert_eq!(hide.label(&file("informe.pdf"), false), "informe");
    assert_eq!(NameDisplay::default().label(&file("informe.pdf"), false), "informe.pdf",
        "el defecto seguro es mostrarlas");
}

/// La salvaguarda que impide leer `factura.pdf.exe` como `factura.pdf`.
#[test]
fn executables_always_show_their_real_extension() {
    let hide = NameDisplay { show_extensions: false, always_show_for_executables: true };
    assert_eq!(hide.label(&file("factura.pdf.exe"), true), "factura.pdf.exe");
    assert_eq!(hide.label(&file("factura.pdf.exe"), false), "factura.pdf",
        "sin el bit de ejecucion, se recorta como cualquier otro");
}

#[test]
fn a_dot_in_a_folder_name_is_not_an_extension() {
    let hide = NameDisplay { show_extensions: false, always_show_for_executables: true };
    assert_eq!(hide.label(&entry("My.folder", EntryKind::Directory), false), "My.folder");
}

#[test]
fn extension_splitting_edge_cases() {
    assert_eq!(base_and_extension("a.tar.gz"), Some(("a.tar", "gz")));
    assert_eq!(base_and_extension("Makefile"), None, "sin punto");
    assert_eq!(base_and_extension("file."), None, "punto final");
    assert_eq!(base_and_extension(".bashrc"), None, "el punto marca oculto, no extension");
}

// ----------------------------------------------------------- filtro en vivo

#[test]
fn the_live_filter_matches_substrings_ignoring_case() {
    let f = NameFilter::new("FOR");
    assert!(f.matches(&file("informe.pdf")), "por subcadena y sin distinguir caja");
    assert!(!f.matches(&file("notas.txt")));
    assert!(NameFilter::new("").is_empty());
    assert!(NameFilter::new("").matches(&file("lo-que-sea")), "vacio no esconde nada");
}

/// El «N de M elementos» de la barra de estado: sin el, la carpeta parece vacia.
#[test]
fn the_filter_counts_what_survives() {
    let v = vec![file("informe.pdf"), file("informacion.txt"), file("notas.txt")];
    assert_eq!(NameFilter::new("info").count_matching(&v), 2);
    assert_eq!(NameFilter::new("zzz").count_matching(&v), 0);
}

/// El filtro no pliega acentos: eso es cosa de la busqueda recursiva.
#[test]
fn the_live_filter_does_not_fold_accents() {
    assert!(!NameFilter::new("arbol").matches(&file("Árbol.txt")));
    assert!(NameFilter::new("árbol").matches(&file("Árbol.txt")), "pero si la caja");
    assert_eq!(fold_for_match("ÁRBOL"), "árbol");
}

// --------------------------------------------------------------- type-ahead

#[test]
fn typing_letters_jumps_to_the_first_match() {
    let v = vec![file("apuntes.txt"), file("informe.pdf"), file("informacion.txt")];
    let mut ta = TypeAhead::new();
    assert_eq!(ta.push('i', &v, None), Some(1));
    assert_eq!(ta.push('n', &v, Some(1)), Some(1));
    assert_eq!(ta.push('f', &v, Some(1)), Some(1), "afinar mantiene la coincidencia");
    assert_eq!(ta.buffer(), "inf");
}

/// Pulsar la misma letra cicla entre los que empiezan por ella.
#[test]
fn repeating_a_letter_cycles_through_its_matches() {
    let v = vec![file("apuntes.txt"), file("informe.pdf"), file("informacion.txt")];
    let mut ta = TypeAhead::new();
    assert_eq!(ta.push('i', &v, None), Some(1));
    assert_eq!(ta.push('i', &v, Some(1)), Some(2), "salta al siguiente");
    assert_eq!(ta.push('i', &v, Some(2)), Some(1), "y da la vuelta");
}

#[test]
fn type_ahead_ignores_case_and_accepts_symbols() {
    let v = vec![file("Informe.PDF"), file("3-notas.txt")];
    let mut ta = TypeAhead::new();
    assert_eq!(ta.push('i', &v, None), Some(0), "sin distinguir mayusculas");
    ta.clear();
    assert_eq!(ta.push('3', &v, None), Some(1), "los numeros valen");
}

#[test]
fn a_miss_leaves_the_selection_alone_and_esc_clears_the_buffer() {
    let v = vec![file("apuntes.txt")];
    let mut ta = TypeAhead::new();
    assert_eq!(ta.push('z', &v, Some(0)), None, "nada coincide");
    ta.clear();
    assert!(ta.is_empty());
}
