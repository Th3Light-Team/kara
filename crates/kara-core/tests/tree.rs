//! El árbol del panel de navegación: aplanado, despliegue diferido y sincronía
//! con la carpeta que se está viendo.

use std::path::{Path, PathBuf};

use kara_core::tree::{Branch, Expandable, Row, RowKind, Section, SectionId, Tree};

fn quick_access(roots: &[&str]) -> Section {
    Section {
        id: SectionId::QuickAccess,
        roots: roots.iter().map(Branch::at).collect(),
    }
}

fn tree_with(roots: &[&str]) -> Tree {
    Tree::new(vec![quick_access(roots)])
}

/// Los nombres de las filas, con la sangría delante, para leer la forma del
/// árbol de un vistazo.
fn shape(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let sangria = "  ".repeat(row.depth);
            match &row.kind {
                RowKind::Section(_) => format!("{sangria}[seccion]"),
                RowKind::Folder { name, .. } => format!("{sangria}{}", name.to_string_lossy()),
            }
        })
        .collect()
}

fn children(names: &[&str]) -> Vec<Branch> {
    names.iter().map(Branch::at).collect()
}

#[test]
fn una_seccion_encabeza_sus_raices() {
    let tree = tree_with(&["/home/ana", "/home/ana/Documentos"]);
    assert_eq!(
        shape(&tree.rows()),
        ["[seccion]", "  ana", "  Documentos"]
    );
}

#[test]
fn una_rama_cerrada_no_enseña_sus_hijas() {
    let mut tree = tree_with(&["/home/ana"]);
    tree.set_children(Path::new("/home/ana"), children(&["/home/ana/Documentos"]));

    assert_eq!(shape(&tree.rows()), ["[seccion]", "  ana"]);
}

#[test]
fn desplegar_saca_las_hijas_sangradas() {
    let mut tree = tree_with(&["/home/ana"]);
    tree.set_children(
        Path::new("/home/ana"),
        children(&["/home/ana/Documentos", "/home/ana/Musica"]),
    );
    tree.expand(Path::new("/home/ana"));

    assert_eq!(
        shape(&tree.rows()),
        ["[seccion]", "  ana", "    Documentos", "    Musica"]
    );
}

#[test]
fn desplegar_una_rama_sin_leer_pide_leerla() {
    let mut tree = tree_with(&["/home/ana"]);
    assert!(
        tree.expand(Path::new("/home/ana")),
        "la primera vez hay que ir al disco"
    );

    tree.set_children(Path::new("/home/ana"), children(&["/home/ana/Documentos"]));
    tree.collapse(Path::new("/home/ana"));
    assert!(
        !tree.expand(Path::new("/home/ana")),
        "una vez leida, volver a abrirla no vuelve al disco"
    );
}

#[test]
fn una_rama_desplegada_pero_sin_leer_no_enseña_nada_y_no_rompe() {
    // Es el hueco entre que el usuario pulsa la flecha y el disco responde.
    let mut tree = tree_with(&["/home/ana"]);
    tree.expand(Path::new("/home/ana"));

    assert_eq!(shape(&tree.rows()), ["[seccion]", "  ana"]);
}

#[test]
fn la_flecha_se_ofrece_mientras_no_se_sepa_si_hay_hijas() {
    let mut tree = tree_with(&["/home/ana"]);
    assert_eq!(tree.expandable(Path::new("/home/ana")), Expandable::Unknown);

    tree.set_children(Path::new("/home/ana"), children(&["/home/ana/Documentos"]));
    assert_eq!(tree.expandable(Path::new("/home/ana")), Expandable::Yes);
}

#[test]
fn una_carpeta_leida_y_vacia_pierde_la_flecha() {
    let mut tree = tree_with(&["/home/ana"]);
    tree.set_children(Path::new("/home/ana"), Vec::new());

    assert_eq!(tree.expandable(Path::new("/home/ana")), Expandable::No);
}

#[test]
fn alternar_abre_y_cierra() {
    let mut tree = tree_with(&["/home/ana"]);
    let ana = Path::new("/home/ana");

    assert!(tree.toggle(ana), "abrirla por primera vez pide leerla");
    assert!(tree.is_expanded(ana));

    assert!(!tree.toggle(ana), "cerrarla nunca pide leer nada");
    assert!(!tree.is_expanded(ana));
}

#[test]
fn olvidar_lo_leido_no_cierra_la_rama() {
    // Refrescar no debe deshacer lo que el usuario abrio a mano.
    let mut tree = tree_with(&["/home/ana"]);
    let ana = Path::new("/home/ana");
    tree.set_children(ana, children(&["/home/ana/Documentos"]));
    tree.expand(ana);

    tree.forget(ana);

    assert!(tree.is_expanded(ana));
    assert_eq!(tree.expandable(ana), Expandable::Unknown);
    assert_eq!(shape(&tree.rows()), ["[seccion]", "  ana"]);
}

#[test]
fn revelar_devuelve_los_ancestros_de_fuera_hacia_dentro() {
    let tree = tree_with(&["/home/ana"]);

    assert_eq!(
        tree.path_to_reveal(Path::new("/home/ana/a/b")),
        vec![PathBuf::from("/home/ana"), PathBuf::from("/home/ana/a")]
    );
}

#[test]
fn revelar_la_propia_raiz_no_despliega_nada() {
    let tree = tree_with(&["/home/ana"]);
    assert!(tree.path_to_reveal(Path::new("/home/ana")).is_empty());
}

#[test]
fn revelar_algo_fuera_de_toda_raiz_no_despliega_nada() {
    let tree = tree_with(&["/home/ana"]);
    assert!(tree.path_to_reveal(Path::new("/var/log")).is_empty());
}

#[test]
fn revelar_entra_por_la_raiz_mas_profunda() {
    // Con `/` y la carpeta personal en el panel, `/home/ana/a` debe abrirse por
    // la rama de la carpeta personal: entrar por `/` obliga a bajar `home` y
    // `ana` para acabar en el mismo sitio.
    let tree = Tree::new(vec![
        quick_access(&["/home/ana"]),
        Section {
            id: SectionId::ThisComputer,
            roots: vec![Branch::at("/")],
        },
    ]);

    assert_eq!(
        tree.path_to_reveal(Path::new("/home/ana/a")),
        vec![PathBuf::from("/home/ana")]
    );
}

#[test]
fn cambiar_las_secciones_conserva_lo_desplegado() {
    // Montar un volumen cambia las raices; no debe cerrar lo que estaba abierto.
    let mut tree = tree_with(&["/home/ana"]);
    let ana = Path::new("/home/ana");
    tree.set_children(ana, children(&["/home/ana/Documentos"]));
    tree.expand(ana);

    tree.set_sections(vec![
        quick_access(&["/home/ana"]),
        Section {
            id: SectionId::ThisComputer,
            roots: vec![Branch::at("/")],
        },
    ]);

    assert_eq!(
        shape(&tree.rows()),
        [
            "[seccion]",
            "  ana",
            "    Documentos",
            "[seccion]",
            "  /"
        ]
    );
}

#[test]
fn un_ciclo_de_enlaces_no_cuelga_el_aplanado() {
    // `/home/ana/bucle` es un enlace a `/home/ana`: el arbol se recorreria sin
    // fin. Se corta por profundidad en vez de resolver enlaces, que es I/O.
    let mut tree = tree_with(&["/home/ana"]);
    let mut path = PathBuf::from("/home/ana");
    for _ in 0..200 {
        let child = path.join("bucle");
        tree.set_children(&path, vec![Branch::at(&child)]);
        tree.expand(&path);
        path = child;
    }

    let rows = tree.rows();
    assert!(rows.len() < 200, "el aplanado se corta en algun punto");
    assert!(rows.iter().all(|row| row.depth <= 64));
}

#[test]
fn la_raiz_se_llama_barra_y_no_cadena_vacia() {
    let tree = tree_with(&["/"]);
    assert_eq!(shape(&tree.rows()), ["[seccion]", "  /"]);
}
