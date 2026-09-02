//! Temas de iconos de FreeDesktop: troceo de `index.theme` y elección de talla.

use kara_fs::icons::{DirKind, IconTheme, Icons, parse_index_theme};
use kara_core::entry::EntryKind;

const TEMA: &str = "\
[Icon Theme]
Name=Vesper
Comment=algo
Inherits=breeze-dark,breeze,hicolor
Directories=places/16,mimetypes/32,apps/48

[places/16]
Size=16
Context=Places
Type=Scalable
MinSize=8
MaxSize=64

[mimetypes/32]
Size=32
Type=Fixed

[apps/48]
Size=48
Type=Threshold
Threshold=4
";

#[test]
fn la_herencia_se_lee_en_orden() {
    let index = parse_index_theme(TEMA);
    assert_eq!(index.inherits, ["breeze-dark", "breeze", "hicolor"]);
}

#[test]
fn cada_subdirectorio_trae_su_talla_y_su_tipo() {
    let index = parse_index_theme(TEMA);
    let nombres: Vec<&str> = index.dirs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(nombres, ["places/16", "mimetypes/32", "apps/48"]);

    assert_eq!(index.dirs[0].kind, DirKind::Scalable);
    assert_eq!(index.dirs[1].kind, DirKind::Fixed);
    assert_eq!(index.dirs[2].kind, DirKind::Threshold);
}

#[test]
fn sin_tipo_declarado_se_asume_umbral() {
    // Es el valor por defecto de la spec; asumir «Fixed» descartaria
    // subdirectorios perfectamente utilizables.
    let index = parse_index_theme("[Icon Theme]\nDirectories=a\n\n[a]\nSize=22\n");
    assert_eq!(index.dirs[0].kind, DirKind::Threshold);
    assert_eq!(index.dirs[0].threshold, 2);
}

#[test]
fn un_subdirectorio_listado_sin_seccion_se_descarta() {
    // Sin talla no hay forma de decidir si sirve.
    let index = parse_index_theme("[Icon Theme]\nDirectories=a,b\n\n[b]\nSize=16\n");
    assert_eq!(index.dirs.len(), 1);
    assert_eq!(index.dirs[0].name, "b");
}

#[test]
fn un_escalable_sirve_en_todo_su_rango_y_no_fuera() {
    let index = parse_index_theme(TEMA);
    let places = &index.dirs[0];

    assert!(places.matches(8));
    assert!(places.matches(16));
    assert!(places.matches(64));
    assert!(!places.matches(65));
    assert_eq!(places.distance(128), 64);
    assert_eq!(places.distance(16), 0);
}

#[test]
fn uno_fijo_solo_sirve_para_su_talla() {
    let index = parse_index_theme(TEMA);
    let mimetypes = &index.dirs[1];

    assert!(mimetypes.matches(32));
    assert!(!mimetypes.matches(31));
    assert_eq!(mimetypes.distance(16), 16);
}

#[test]
fn uno_de_umbral_admite_lo_cercano() {
    let index = parse_index_theme(TEMA);
    let apps = &index.dirs[2];

    assert!(apps.matches(44));
    assert!(apps.matches(52));
    assert!(!apps.matches(43));
    assert_eq!(apps.distance(40), 4);
}

#[test]
fn un_umbral_por_debajo_de_cero_no_desborda() {
    // `Size=8` con `Threshold=16` restaria por debajo de cero.
    let index = parse_index_theme("[Icon Theme]\nDirectories=a\n\n[a]\nSize=8\nThreshold=16\n");
    assert!(index.dirs[0].matches(1));
    assert_eq!(index.dirs[0].distance(1), 0);
}

#[test]
fn un_index_vacio_no_rompe_nada() {
    let index = parse_index_theme("");
    assert!(index.inherits.is_empty());
    assert!(index.dirs.is_empty());
}

#[test]
fn hicolor_siempre_tiene_la_carpeta_generica() {
    // Depende del sistema: sin ningun tema instalado no hay nada que probar.
    let theme = IconTheme::named("hicolor");
    if theme.is_empty() {
        return;
    }

    // `hicolor` es el minimo que la spec obliga a que exista; si el sistema
    // tiene temas, alguno de la cadena tiene que dar una carpeta.
    let carpeta = IconTheme::load().find("folder", 16);
    assert!(carpeta.is_some(), "ningun tema del sistema trae «folder»");
}

#[test]
fn el_tema_del_sistema_resuelve_lo_corriente() {
    let mut icons = Icons::load(16);
    if icons.of("x", EntryKind::Directory).is_none() {
        // Sin temas instalados no hay nada que comprobar.
        return;
    }

    assert!(icons.of("foto.png", EntryKind::File).is_some());
    assert!(icons.of("apuntes.txt", EntryKind::File).is_some());
    // Algo sin tipo reconocido tambien tiene que salir con algo.
    assert!(icons.of("cosa.qwertyuiop", EntryKind::File).is_some());
}
