//! Las ubicaciones del panel de navegación: troceo de `user-dirs.dirs` y de
//! `/proc/mounts`.

use std::path::{Path, PathBuf};

use kara_fs::places::{parse_mounts, parse_user_dirs};

fn home() -> PathBuf {
    PathBuf::from("/home/ana")
}

#[test]
fn una_carpeta_de_usuario_se_lee_con_su_home_expandido() {
    let dirs = parse_user_dirs(r#"XDG_DOWNLOAD_DIR="$HOME/Descargas""#, &home());

    assert_eq!(
        dirs.get("XDG_DOWNLOAD_DIR"),
        Some(&PathBuf::from("/home/ana/Descargas"))
    );
}

#[test]
fn los_comentarios_y_las_lineas_vacias_se_ignoran() {
    let text = "# Generado por xdg-user-dirs-update\n\nXDG_MUSIC_DIR=\"$HOME/Musica\"\n";
    let dirs = parse_user_dirs(text, &home());

    assert_eq!(dirs.len(), 1);
    assert_eq!(
        dirs.get("XDG_MUSIC_DIR"),
        Some(&PathBuf::from("/home/ana/Musica"))
    );
}

#[test]
fn una_ruta_absoluta_se_admite_tal_cual() {
    let dirs = parse_user_dirs(r#"XDG_VIDEOS_DIR="/mnt/discos/video""#, &home());

    assert_eq!(
        dirs.get("XDG_VIDEOS_DIR"),
        Some(&PathBuf::from("/mnt/discos/video"))
    );
}

#[test]
fn el_home_pelado_apunta_a_la_carpeta_personal() {
    // Es una configuracion real: escritorio sin carpeta propia.
    let dirs = parse_user_dirs(r#"XDG_DESKTOP_DIR="$HOME""#, &home());

    assert_eq!(dirs.get("XDG_DESKTOP_DIR"), Some(&home()));
}

#[test]
fn una_ruta_relativa_se_descarta_en_vez_de_inventarse_una_base() {
    let dirs = parse_user_dirs(r#"XDG_DESKTOP_DIR="Escritorio""#, &home());

    assert!(dirs.is_empty());
}

#[test]
fn lo_que_no_es_una_clave_xdg_se_descarta() {
    let dirs = parse_user_dirs("PATH=\"/usr/bin\"\nenabled=true\n", &home());

    assert!(dirs.is_empty());
}

#[test]
fn solo_salen_los_montajes_que_el_usuario_navega() {
    let mounts = "\
sysfs /sys sysfs rw,nosuid 0 0
proc /proc proc rw,nosuid 0 0
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
/dev/sdb1 /media/ana/USB vfat rw,nosuid 0 0
/dev/sdc1 /mnt/respaldo ext4 rw 0 0
tmpfs /run/user/1000 tmpfs rw 0 0
";

    assert_eq!(
        parse_mounts(mounts),
        vec![PathBuf::from("/media/ana/USB"), PathBuf::from("/mnt/respaldo")]
    );
}

#[test]
fn los_propios_prefijos_no_son_un_volumen() {
    // `/media` montado como tmpfs no es un disco que enseñar.
    let mounts = "tmpfs /media tmpfs rw 0 0\ntmpfs /mnt tmpfs rw 0 0\n";

    assert!(parse_mounts(mounts).is_empty());
}

#[test]
fn un_montaje_con_espacios_se_desescapa() {
    // El nucleo escribe `\040` por cada espacio; sin deshacerlo, la ruta que
    // sale del panel no existe.
    let mounts = "/dev/sdb1 /media/ana/Mis\\040cosas vfat rw 0 0\n";

    assert_eq!(
        parse_mounts(mounts),
        vec![PathBuf::from("/media/ana/Mis cosas")]
    );
}

#[test]
fn un_montaje_con_acentos_sobrevive_al_desescapado() {
    let mounts = "/dev/sdb1 /media/ana/Ámbar\\040azul vfat rw 0 0\n";

    assert_eq!(
        parse_mounts(mounts),
        vec![PathBuf::from("/media/ana/Ámbar azul")]
    );
}

#[test]
fn el_mismo_punto_de_montaje_dos_veces_es_una_sola_entrada() {
    let mounts = "\
/dev/sdb1 /media/ana/USB vfat rw 0 0
/dev/sdb1 /media/ana/USB vfat ro 0 0
";

    assert_eq!(parse_mounts(mounts), vec![PathBuf::from("/media/ana/USB")]);
}

#[test]
fn una_linea_a_medias_no_tumba_el_troceo() {
    let mounts = "solo-un-campo\n\n/dev/sdb1 /media/ana/USB vfat rw 0 0\n";

    assert_eq!(parse_mounts(mounts), vec![PathBuf::from("/media/ana/USB")]);
}

#[test]
fn el_acceso_rapido_sin_carpeta_personal_esta_vacio() {
    assert!(kara_fs::places::quick_access(None).is_empty());
}

#[test]
fn la_raiz_siempre_encabeza_este_equipo() {
    let places = kara_fs::places::this_computer();

    assert_eq!(places.first().map(|p| p.path.as_path()), Some(Path::new("/")));
}
