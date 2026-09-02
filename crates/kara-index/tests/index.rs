//! Pruebas de travesía, búsqueda y tamaño (`ground/spec/04-busqueda.md` y
//! `05-operaciones.md`).

use std::fs;
use std::path::PathBuf;

use kara_index::{
    Cancel, Query, SearchEvent, SearchScope, WalkOptions, fold_for_search, folder_size, search,
    walk,
};
use tempfile::TempDir;

/// Árbol de prueba:
///   raiz/Árbol.txt  raiz/informe.pdf  raiz/sub/arbolito.txt  raiz/sub/hondo/x.txt
fn tree() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    fs::write(root.join("Árbol.txt"), b"12345").unwrap();
    fs::write(root.join("informe.pdf"), b"abc").unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub/arbolito.txt"), b"xy").unwrap();
    fs::create_dir(root.join("sub/hondo")).unwrap();
    fs::write(root.join("sub/hondo/x.txt"), b"z").unwrap();
    dir
}

fn collect(root: &std::path::Path, term: &str, scope: SearchScope) -> (Vec<PathBuf>, u64, bool) {
    let mut hits = Vec::new();
    let (mut total, mut cancelled) = (0, false);
    search(root, &Query::new(term), scope, &Cancel::new(), |ev| match ev {
        SearchEvent::Match(p) => hits.push(p),
        SearchEvent::Done { matches, cancelled: c } => {
            total = matches;
            cancelled = c;
        }
        _ => {}
    });
    (hits, total, cancelled)
}

#[test]
fn folding_removes_accents_and_case() {
    assert_eq!(fold_for_search("Árbol"), "arbol");
    assert_eq!(fold_for_search("ÑOÑO"), "nono");
    assert_eq!(fold_for_search("Straße"), "straße", "ss no es un plegado de acento");
}

#[test]
fn a_query_matches_by_substring_ignoring_accents() {
    let q = Query::new("arbol");
    assert!(q.matches("Árbol.txt"), "buscar sin acentos encuentra con acentos");
    assert!(q.matches("MiArbolito"), "coincide por subcadena, no solo por prefijo");
    assert!(!q.matches("informe.pdf"));
    assert!(Query::new("").is_empty());
}

#[test]
fn current_folder_scope_does_not_recurse() {
    let dir = tree();
    let (hits, total, _) = collect(dir.path(), "arbol", SearchScope::CurrentFolder);
    assert_eq!(total, 1, "solo el del nivel directo");
    assert!(hits[0].ends_with("Árbol.txt"));
}

#[test]
fn subfolders_scope_recurses() {
    let dir = tree();
    let (_, total, _) = collect(dir.path(), "arbol", SearchScope::Subfolders);
    assert_eq!(total, 2, "Árbol.txt y sub/arbolito.txt");
}

/// La spec pide mostrar antes las coincidencias de la carpeta actual.
#[test]
fn shallow_matches_arrive_before_deep_ones() {
    let dir = tree();
    let (hits, _, _) = collect(dir.path(), "arbol", SearchScope::Subfolders);
    assert!(hits[0].ends_with("Árbol.txt"), "primero el nivel directo");
    assert!(hits[1].ends_with("arbolito.txt"));
}

/// Vaciar la caja no debe lanzar ninguna travesía.
#[test]
fn an_empty_query_finishes_without_walking() {
    let dir = tree();
    let mut events = 0;
    search(dir.path(), &Query::new(""), SearchScope::Subfolders, &Cancel::new(), |ev| {
        events += 1;
        assert!(matches!(ev, SearchEvent::Done { matches: 0, .. }));
    });
    assert_eq!(events, 1, "solo el evento de fin");
}

/// Hay que distinguir «me pararon» de «no hay resultados».
#[test]
fn cancelling_is_reported_as_such() {
    let dir = tree();
    let cancel = Cancel::new();
    cancel.cancel();
    let mut done = None;
    search(dir.path(), &Query::new("arbol"), SearchScope::Subfolders, &cancel, |ev| {
        if let SearchEvent::Done { matches, cancelled } = ev {
            done = Some((matches, cancelled));
        }
    });
    assert_eq!(done, Some((0, true)), "cancelada, no vacia");
}

#[test]
fn folder_size_sums_the_tree() {
    let dir = tree();
    let report = folder_size(dir.path(), &Cancel::new(), |_| {});
    assert_eq!(report.totals.files, 4);
    assert_eq!(report.totals.directories, 2);
    assert_eq!(report.totals.logical, 5 + 3 + 2 + 1);
    assert!(!report.is_partial());
    assert!(report.totals.on_disk >= report.totals.logical, "los bloques redondean al alza");
}

/// Un fichero con dos nombres ocupa disco una sola vez.
#[test]
fn hard_links_are_counted_once() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a"), b"1234567890").unwrap();
    fs::hard_link(dir.path().join("a"), dir.path().join("b")).unwrap();

    let report = folder_size(dir.path(), &Cancel::new(), |_| {});
    assert_eq!(report.totals.files, 1, "dos nombres, un fichero");
    assert_eq!(report.totals.logical, 10);
}

/// Seguir un enlace a un ancestro no terminaria nunca.
#[test]
fn symlinks_are_not_followed() {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("real")).unwrap();
    fs::write(dir.path().join("real/f"), b"1234").unwrap();
    std::os::unix::fs::symlink(dir.path(), dir.path().join("real/bucle")).unwrap();

    let report = folder_size(dir.path(), &Cancel::new(), |_| {});

    // El enlace pesa lo que mide la ruta de su destino, igual que cuenta `du`.
    // Lo que importa es que NO se recorrio a traves de el: si se hubiera seguido,
    // el bucle no habria terminado y el fichero se contaria dos veces.
    let link_size = fs::symlink_metadata(dir.path().join("real/bucle")).unwrap().len();
    assert_eq!(report.totals.logical, 4 + link_size);
    assert_eq!(report.totals.files, 2, "el fichero y el enlace, cada uno una vez");
    assert!(!report.is_partial());
}

/// Una carpeta ilegible da totales parciales y se anota, no se miente.
#[test]
fn an_unreadable_directory_makes_the_total_partial() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("visible"), b"123").unwrap();
    let locked = dir.path().join("cerrada");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("oculto"), b"9999999999").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

    let report = folder_size(dir.path(), &Cancel::new(), |_| {});
    let partial = report.is_partial();
    let logical = report.totals.logical;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(partial, "debe declararse parcial");
    assert_eq!(logical, 3, "solo lo que se pudo leer");
}

#[test]
fn progress_is_reported_and_ends_with_the_totals() {
    let dir = tree();
    let mut last = None;
    let report = folder_size(dir.path(), &Cancel::new(), |p| last = Some(p));
    assert_eq!(last, Some(report.totals));
}

#[test]
fn max_depth_limits_the_walk() {
    let dir = tree();
    let mut deep = 0;
    let options = WalkOptions { max_depth: Some(1), ..WalkOptions::default() };
    walk(dir.path(), &options, &Cancel::new(), |item| {
        if item.depth > 1 {
            deep += 1;
        }
    });
    assert_eq!(deep, 0);
}
