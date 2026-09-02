//! Pruebas de la parte pura del contrato: formato `.trashinfo`, fecha de
//! borrado y percent-encoding. No tocan el disco ni el entorno.
//!
//! Casos borde cubiertos: cb_03, cb_05, cb_06, cb_07, cb_30.

use std::ffi::OsStr;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use kara_fs::trash::{DeletionDate, TrashInfo, TrashInfoError};

/// 2026-09-02T17:06:24 UTC.
const KNOWN_EPOCH: u64 = 1_788_368_784;

fn known_date() -> DeletionDate {
    DeletionDate {
        year: 2026,
        month: 9,
        day: 2,
        hour: 17,
        minute: 6,
        second: 24,
    }
}

fn info_for(path: &str) -> TrashInfo {
    TrashInfo {
        original_path: PathBuf::from(path),
        deletion_date: known_date(),
    }
}

#[test]
fn cb_03_serializa_cabecera_y_claves_en_el_orden_exacto_de_freedesktop() {
    let rendered = match info_for("/home/u/nota.txt").serialize(None) {
        Ok(text) => text,
        Err(error) => panic!("serialize should succeed, got {error:?}"),
    };

    assert_eq!(
        rendered, "[Trash Info]\nPath=/home/u/nota.txt\nDeletionDate=2026-09-02T17:06:24\n",
        "el .trashinfo debe ser byte a byte el de la spec FreeDesktop"
    );
}

#[test]
fn cb_03_ida_y_vuelta_conserva_ruta_y_fecha() {
    let original = info_for("/home/u/sub dir/nota.txt");
    let rendered = match original.serialize(None) {
        Ok(text) => text,
        Err(error) => panic!("serialize should succeed, got {error:?}"),
    };
    let parsed = match TrashInfo::parse(rendered.as_bytes(), None) {
        Ok(info) => info,
        Err(error) => panic!("parse should succeed, got {error:?}"),
    };

    assert_eq!(parsed, original);
}

#[test]
fn cb_05_papelera_de_volumen_escribe_path_relativo_al_topdir() {
    let rendered = match info_for("/mnt/usb/data/x.txt").serialize(Some(Path::new("/mnt/usb"))) {
        Ok(text) => text,
        Err(error) => panic!("serialize should succeed, got {error:?}"),
    };

    assert!(
        rendered.contains("\nPath=data/x.txt\n"),
        "Path= debe ser relativo al topdir y sin barra inicial, fue: {rendered:?}"
    );
    assert!(
        !rendered.contains("Path=/mnt/usb"),
        "no debe quedar rastro del topdir absoluto: {rendered:?}"
    );
}

#[test]
fn cb_05_papelera_home_escribe_path_absoluto() {
    let rendered = match info_for("/home/u/x.txt").serialize(None) {
        Ok(text) => text,
        Err(error) => panic!("serialize should succeed, got {error:?}"),
    };

    assert!(
        rendered.contains("\nPath=/home/u/x.txt\n"),
        "sin topdir el Path= es absoluto, fue: {rendered:?}"
    );
}

#[test]
fn cb_05_parse_reconstruye_la_ruta_absoluta_con_el_topdir() {
    let raw = b"[Trash Info]\nPath=data/x.txt\nDeletionDate=2026-09-02T17:06:24\n";

    let parsed = match TrashInfo::parse(raw, Some(Path::new("/mnt/usb"))) {
        Ok(info) => info,
        Err(error) => panic!("parse should succeed, got {error:?}"),
    };

    assert_eq!(parsed.original_path, PathBuf::from("/mnt/usb/data/x.txt"));
    assert_eq!(parsed.deletion_date, known_date());
}

#[test]
fn cb_05_ruta_relativa_sin_topdir_es_error_no_ruta_a_medias() {
    let raw = b"[Trash Info]\nPath=data/x.txt\nDeletionDate=2026-09-02T17:06:24\n";

    match TrashInfo::parse(raw, None) {
        Err(TrashInfoError::NotAbsolute { raw }) => {
            assert!(raw.contains("data/x.txt"), "el error debe citar la ruta: {raw}")
        }
        other => panic!("expected NotAbsolute, got {other:?}"),
    }
}

#[test]
fn cb_06_percent_encoding_escapa_espacio_almohadilla_y_porcentaje() {
    let rendered = match info_for("/home/u/a b#c%d.txt").serialize(None) {
        Ok(text) => text,
        Err(error) => panic!("serialize should succeed, got {error:?}"),
    };

    assert!(
        rendered.contains("Path=/home/u/a%20b%23c%25d.txt"),
        "los reservados se escapan y `/` no: {rendered:?}"
    );
}

#[test]
fn cb_06_percent_encoding_es_reversible_byte_a_byte() {
    let original = info_for("/home/u/a b#c%d.txt");
    let rendered = match original.serialize(None) {
        Ok(text) => text,
        Err(error) => panic!("serialize should succeed, got {error:?}"),
    };
    let parsed = match TrashInfo::parse(rendered.as_bytes(), None) {
        Ok(info) => info,
        Err(error) => panic!("parse should succeed, got {error:?}"),
    };

    assert_eq!(parsed.original_path, original.original_path);
}

#[test]
fn cb_06_nombres_no_utf8_sobreviven_la_ida_y_vuelta_sin_perdida() {
    let raw_name: Vec<u8> = b"/home/u/rar\xff\xfe.txt".to_vec();
    let original = TrashInfo {
        original_path: PathBuf::from(std::ffi::OsString::from_vec(raw_name.clone())),
        deletion_date: known_date(),
    };

    let rendered = match original.serialize(None) {
        Ok(text) => text,
        Err(error) => panic!("serialize should succeed for non-UTF-8 names, got {error:?}"),
    };
    assert!(
        rendered.is_ascii(),
        "el .trashinfo escrito debe ser ASCII puro: {rendered:?}"
    );
    assert!(
        rendered.to_uppercase().contains("%FF%FE"),
        "los bytes >= 0x80 se escapan: {rendered:?}"
    );

    let parsed = match TrashInfo::parse(rendered.as_bytes(), None) {
        Ok(info) => info,
        Err(error) => panic!("parse should succeed, got {error:?}"),
    };
    assert_eq!(
        parsed.original_path.as_os_str().as_bytes(),
        raw_name.as_slice(),
        "la ruta debe volver byte a byte identica"
    );
}

#[test]
fn cb_06_percent_encoding_invalido_se_reporta_sin_panico() {
    let raw = b"[Trash Info]\nPath=/home/u/%zz.txt\nDeletionDate=2026-09-02T17:06:24\n";

    match TrashInfo::parse(raw, None) {
        Err(TrashInfoError::InvalidPercentEncoding { .. }) => {}
        other => panic!("expected InvalidPercentEncoding, got {other:?}"),
    }
}

#[test]
fn cb_07_fecha_local_sin_zona_con_desplazamiento_inyectado() {
    let instant = UNIX_EPOCH + Duration::from_secs(KNOWN_EPOCH);

    let utc = match DeletionDate::from_system_time_local(instant, 0) {
        Ok(date) => date,
        Err(error) => panic!("conversion should succeed, got {error:?}"),
    };
    assert_eq!(utc.format(), "2026-09-02T17:06:24");
    assert_eq!(utc, known_date());

    let shifted = match DeletionDate::from_system_time_local(instant, 3600) {
        Ok(date) => date,
        Err(error) => panic!("conversion should succeed, got {error:?}"),
    };
    assert_eq!(
        shifted.format(),
        "2026-09-02T18:06:24",
        "el desplazamiento se aplica a la hora local"
    );

    let behind = match DeletionDate::from_system_time_local(instant, -4 * 3600) {
        Ok(date) => date,
        Err(error) => panic!("conversion should succeed, got {error:?}"),
    };
    assert_eq!(behind.format(), "2026-09-02T13:06:24");
}

#[test]
fn cb_07_formato_rellena_con_ceros_todos_los_campos() {
    let instant = UNIX_EPOCH + Duration::from_secs(1_767_323_045);

    let date = match DeletionDate::from_system_time_local(instant, 0) {
        Ok(date) => date,
        Err(error) => panic!("conversion should succeed, got {error:?}"),
    };

    assert_eq!(date.format(), "2026-01-02T03:04:05");
}

#[test]
fn cb_07_parse_es_el_inverso_de_format() {
    let date = known_date();

    let parsed = match DeletionDate::parse(&date.format()) {
        Ok(value) => value,
        Err(error) => panic!("parse should succeed, got {error:?}"),
    };

    assert_eq!(parsed, date);
}

#[test]
fn cb_07_fecha_con_sufijo_z_es_malformada_no_un_valor_a_medias() {
    match DeletionDate::parse("2026-09-02T17:06:24Z") {
        Err(TrashInfoError::MalformedDate { raw }) => {
            assert!(raw.contains("2026-09-02T17:06:24Z"), "el error cita el valor: {raw}")
        }
        other => panic!("expected MalformedDate, got {other:?}"),
    }
}

#[test]
fn cb_07_fechas_truncadas_o_con_basura_son_malformadas() {
    for raw in ["", "2026-09-02", "2026-09-02 17:06:24", "26-9-2T17:06:24", "abcd-ef-ghTij:kl:mn"] {
        match DeletionDate::parse(raw) {
            Err(TrashInfoError::MalformedDate { .. }) => {}
            other => panic!("expected MalformedDate for {raw:?}, got {other:?}"),
        }
    }
}

#[test]
fn cb_30_info_sin_cabecera_es_missing_header() {
    let raw = b"Path=/home/u/x.txt\nDeletionDate=2026-09-02T17:06:24\n";

    match TrashInfo::parse(raw, None) {
        Err(TrashInfoError::MissingHeader) => {}
        other => panic!("expected MissingHeader, got {other:?}"),
    }
}

#[test]
fn cb_30_info_con_path_vacio_es_missing_path() {
    let raw = b"[Trash Info]\nPath=\nDeletionDate=2026-09-02T17:06:24\n";

    match TrashInfo::parse(raw, None) {
        Err(TrashInfoError::MissingPath) => {}
        other => panic!("expected MissingPath, got {other:?}"),
    }
}

#[test]
fn cb_30_info_sin_clave_path_es_missing_path() {
    let raw = b"[Trash Info]\nDeletionDate=2026-09-02T17:06:24\n";

    match TrashInfo::parse(raw, None) {
        Err(TrashInfoError::MissingPath) => {}
        other => panic!("expected MissingPath, got {other:?}"),
    }
}

#[test]
fn cb_30_info_sin_fecha_es_missing_deletion_date() {
    let raw = b"[Trash Info]\nPath=/home/u/x.txt\n";

    match TrashInfo::parse(raw, None) {
        Err(TrashInfoError::MissingDeletionDate) => {}
        other => panic!("expected MissingDeletionDate, got {other:?}"),
    }
}

#[test]
fn cb_30_info_con_fecha_basura_es_malformed_date_y_no_entra_en_panico() {
    let raw = b"[Trash Info]\nPath=/home/u/x.txt\nDeletionDate=ayer por la tarde\n";

    match TrashInfo::parse(raw, None) {
        Err(TrashInfoError::MalformedDate { .. }) => {}
        other => panic!("expected MalformedDate, got {other:?}"),
    }
}

#[test]
fn cb_30_info_con_bytes_binarios_no_entra_en_panico() {
    let raw: &[u8] = &[0x00, 0xff, 0xfe, b'\n', 0x80, 0x81];

    match TrashInfo::parse(raw, None) {
        Err(_) => {}
        Ok(info) => panic!("basura binaria no puede parsear a {info:?}"),
    }
}

#[test]
fn cb_28_una_ruta_larga_cabe_entera_en_el_path_del_info() {
    let long_component = "n".repeat(255);
    let path = PathBuf::from(format!("/home/u/{long_component}"));
    let original = TrashInfo {
        original_path: path.clone(),
        deletion_date: known_date(),
    };

    let rendered = match original.serialize(None) {
        Ok(text) => text,
        Err(error) => panic!("serialize should succeed, got {error:?}"),
    };
    let parsed = match TrashInfo::parse(rendered.as_bytes(), None) {
        Ok(info) => info,
        Err(error) => panic!("parse should succeed, got {error:?}"),
    };

    assert_eq!(parsed.original_path, path);
    assert_eq!(
        parsed.original_path.file_name(),
        Some(OsStr::new(long_component.as_str()))
    );
}
