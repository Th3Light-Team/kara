//! Pruebas de «Enviar a la papelera» contra la papelera del hogar.
//!
//! Todas redirigen `XDG_DATA_HOME` a un directorio temporal y se serializan
//! entre sí (el entorno es del proceso). Casos borde cubiertos: cb_01, cb_02,
//! cb_03, cb_04, cb_08, cb_09, cb_13, cb_14, cb_15, cb_16, cb_17, cb_19,
//! cb_20, cb_21, cb_22, cb_23, cb_25, cb_26, cb_27, cb_28, cb_29, cb_31.

mod common;

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use common::{
    Recorder, RetryThenFix, Silent, TempTree, TrashEnv, entry_names, mode_of, ok, read, set_mode,
    snapshot,
};
use kara_fs::trash::{
    ConflictPolicy, ErrorDecision, RefusalReason, RestoreError, TrashError, TrashKind, TrashPolicy,
    TrashedItem, delete_permanently, home_trash_dir, probe_trash, read_trash_info, restore_item,
    trash_batch, trash_one,
};

/// EACCES (13) or EPERM (1), the two errnos the spec accepts for a refusal by
/// permissions.
fn eacces_or_eperm(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(1) | Some(13))
}

fn trashed(path: &Path) -> TrashedItem {
    match trash_one(path, &TrashPolicy::default()) {
        Ok(item) => item,
        Err(error) => panic!("trash_one({}) should succeed, got {error:?}", path.display()),
    }
}

// ---------------------------------------------------------------------------
// cb_01
// ---------------------------------------------------------------------------

#[test]
fn cb_01_supr_envia_la_seleccion_entera_a_la_papelera_en_orden() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb01");
    let paths = vec![
        tree.write("uno.txt", b"UNO"),
        tree.write("dos.txt", b"DOS"),
        tree.write("tres.txt", b"TRES"),
    ];
    let mut observer = Recorder::new();

    let outcome = trash_batch(&paths, &TrashPolicy::default(), &mut observer);

    assert_eq!(outcome.trashed.len(), 3, "los tres elementos deben moverse");
    assert!(outcome.skipped.is_empty(), "no debe omitirse ninguno");
    assert!(!outcome.cancelled);
    let reported: Vec<PathBuf> = outcome
        .trashed
        .iter()
        .map(|item| item.original_path.clone())
        .collect();
    assert_eq!(reported, paths, "el orden de entrada se conserva");
    for path in &paths {
        assert!(!path.exists(), "{} debe desaparecer del origen", path.display());
    }
    assert_eq!(
        entry_names(&env.files()),
        vec!["dos.txt".to_string(), "tres.txt".to_string(), "uno.txt".to_string()]
    );
    assert_eq!(read(&env.files().join("dos.txt")), b"DOS".to_vec());
    assert_eq!(observer.starts.len(), 3);
    assert_eq!(observer.totals, vec![3, 3, 3]);
    assert_eq!(observer.done.len(), 3);
}

// ---------------------------------------------------------------------------
// cb_02
// ---------------------------------------------------------------------------

#[test]
fn cb_02_un_directorio_con_muchos_hijos_se_mueve_sin_recorrer_bytes() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb02");
    let dir = tree.mkdir("grande");
    for index in 0..1_000 {
        let child = dir.join(format!("hijo-{index}.txt"));
        ok(fs::write(&child, b"x"), "write child");
    }
    let mut observer = Recorder::new();

    let outcome = trash_batch(&[dir.clone()], &TrashPolicy::default(), &mut observer);

    assert_eq!(outcome.trashed.len(), 1);
    assert_eq!(
        observer.byte_calls, 0,
        "dentro del mismo volumen no se recorre ni un byte"
    );
    let item = match outcome.trashed.first() {
        Some(item) => item,
        None => panic!("expected one trashed item"),
    };
    assert_eq!(item.bytes_copied, None, "rename(2) puro no copia bytes");
    assert!(!dir.exists());
    let moved = env.files().join("grande");
    assert_eq!(entry_names(&moved).len(), 1_000, "los hijos viajan con el padre");
}

#[test]
fn cb_02_el_fichero_movido_conserva_su_inodo_porque_fue_rename() {
    let _env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb02b");
    let source = tree.write("x.txt", b"AAA");
    let before = ok(fs::metadata(&source), "stat source");

    let item = trashed(&source);

    let after = ok(fs::metadata(&item.trashed_path), "stat trashed");
    assert_eq!(before.ino(), after.ino(), "mismo inodo: no hubo copia");
    assert_eq!(before.dev(), after.dev(), "mismo dispositivo");
    assert_eq!(item.bytes_copied, None);
    assert_eq!(item.kind, TrashKind::Home);
    assert_eq!(item.top_dir, None, "la papelera del hogar no lleva topdir");
}

// ---------------------------------------------------------------------------
// cb_03
// ---------------------------------------------------------------------------

#[test]
fn cb_03_registra_ruta_original_y_fecha_en_un_trashinfo_legible() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb03");
    let source = tree.write("nota.txt", b"AAA");

    let item = trashed(&source);

    assert_eq!(item.info_path, env.info().join("nota.txt.trashinfo"));
    let raw = read(&item.info_path);
    assert!(
        raw.starts_with(b"[Trash Info]\n"),
        "el fichero empieza por la cabecera literal"
    );
    let info = match read_trash_info(&item.info_path) {
        Ok(info) => info,
        Err(error) => panic!("read_trash_info should succeed, got {error:?}"),
    };
    assert_eq!(info.original_path, source);
    assert_eq!(info.deletion_date, item.deletion_date);
    let rendered = info.deletion_date.format();
    assert_eq!(rendered.len(), 19, "formato %Y-%m-%dT%H:%M:%S: {rendered}");
    assert!(info.deletion_date.year >= 2024, "fecha plausible: {rendered}");
}

// ---------------------------------------------------------------------------
// cb_04
// ---------------------------------------------------------------------------

#[test]
fn cb_04_la_papelera_del_hogar_vive_en_xdg_data_home_con_files_e_info_0700() {
    let env = TrashEnv::on_home_device();

    let dir = match home_trash_dir() {
        Ok(dir) => dir,
        Err(error) => panic!("home_trash_dir should succeed, got {error:?}"),
    };

    assert_eq!(dir.root, env.trash_root());
    assert_eq!(dir.files, env.files());
    assert_eq!(dir.info, env.info());
    assert_eq!(dir.kind, TrashKind::Home);
    assert_eq!(dir.top_dir, None);
    assert!(dir.files.is_dir(), "files/ debe existir");
    assert!(dir.info.is_dir(), "info/ debe existir");
    assert_eq!(mode_of(&dir.files), 0o700, "files/ se crea en modo 0700");
    assert_eq!(mode_of(&dir.info), 0o700, "info/ se crea en modo 0700");
}

#[test]
fn cb_04_xdg_data_home_vacia_o_relativa_cae_a_home_local_share_trash() {
    let env = TrashEnv::on_home_device();
    let fake_home = TempTree::on_home_device("cb04-home");

    env.override_xdg_and_home("", &fake_home.root);
    let empty = match home_trash_dir() {
        Ok(dir) => dir,
        Err(error) => panic!("home_trash_dir should succeed with empty XDG, got {error:?}"),
    };
    assert_eq!(empty.root, fake_home.root.join(".local/share/Trash"));

    env.override_xdg_and_home("relativa/datos", &fake_home.root);
    let relative = match home_trash_dir() {
        Ok(dir) => dir,
        Err(error) => panic!("home_trash_dir should succeed with relative XDG, got {error:?}"),
    };
    assert_eq!(
        relative.root,
        fake_home.root.join(".local/share/Trash"),
        "una XDG_DATA_HOME relativa se ignora, no se usa a medias"
    );
    assert!(
        !PathBuf::from("relativa/datos").exists(),
        "no se crea nada relativo al directorio de trabajo"
    );
}

// ---------------------------------------------------------------------------
// cb_08
// ---------------------------------------------------------------------------

#[test]
fn cb_08_dos_ficheros_con_el_mismo_nombre_no_se_pisan() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb08");
    let first = tree.write("a/x.txt", b"AAA");
    let second = tree.write("b/x.txt", b"BBB");

    let first_item = trashed(&first);
    let second_item = trashed(&second);

    assert_eq!(
        entry_names(&env.files()),
        vec!["x.2.txt".to_string(), "x.txt".to_string()],
        "el sufijo se inserta antes de la extension"
    );
    assert_eq!(
        entry_names(&env.info()),
        vec!["x.2.txt.trashinfo".to_string(), "x.txt.trashinfo".to_string()]
    );
    let mut contents = vec![
        read(&first_item.trashed_path),
        read(&second_item.trashed_path),
    ];
    contents.sort();
    assert_eq!(
        contents,
        vec![b"AAA".to_vec(), b"BBB".to_vec()],
        "ningun fichero pierde su contenido"
    );
    let first_info = match read_trash_info(&first_item.info_path) {
        Ok(info) => info,
        Err(error) => panic!("read_trash_info should succeed, got {error:?}"),
    };
    let second_info = match read_trash_info(&second_item.info_path) {
        Ok(info) => info,
        Err(error) => panic!("read_trash_info should succeed, got {error:?}"),
    };
    assert_eq!(first_info.original_path, first);
    assert_eq!(second_info.original_path, second);
    assert_ne!(first_item.trashed_path, second_item.trashed_path);
}

#[test]
fn cb_08_la_desambiguacion_no_depende_de_que_el_lote_sea_uno_o_dos() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb08b");
    let paths = vec![tree.write("a/x.txt", b"AAA"), tree.write("b/x.txt", b"BBB")];
    let mut observer = Recorder::new();

    let outcome = trash_batch(&paths, &TrashPolicy::default(), &mut observer);

    assert_eq!(outcome.trashed.len(), 2);
    assert!(outcome.skipped.is_empty());
    assert_eq!(
        entry_names(&env.files()),
        vec!["x.2.txt".to_string(), "x.txt".to_string()]
    );
    let mut contents: Vec<Vec<u8>> = outcome
        .trashed
        .iter()
        .map(|item| read(&item.trashed_path))
        .collect();
    contents.sort();
    assert_eq!(contents, vec![b"AAA".to_vec(), b"BBB".to_vec()]);
}

#[test]
fn cb_08_un_nombre_sin_extension_recibe_el_sufijo_al_final() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb08c");
    let first = tree.write("a/LEEME", b"AAA");
    let second = tree.write("b/LEEME", b"BBB");

    let _ = trashed(&first);
    let _ = trashed(&second);

    assert_eq!(
        entry_names(&env.files()),
        vec!["LEEME".to_string(), "LEEME.2".to_string()]
    );
}

// ---------------------------------------------------------------------------
// cb_09
// ---------------------------------------------------------------------------

#[test]
fn cb_09_si_falla_el_rename_no_queda_ningun_trashinfo_huerfano() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb09");
    let source = tree.write("x.txt", b"AAA");
    let dir = match home_trash_dir() {
        Ok(dir) => dir,
        Err(error) => panic!("home_trash_dir should succeed, got {error:?}"),
    };
    set_mode(&dir.files, 0o500);
    let info_before = snapshot(&env.info());

    let result = trash_one(&source, &TrashPolicy::default());

    set_mode(&dir.files, 0o700);
    assert!(result.is_err(), "con files/ sin permiso el envio debe fallar");
    assert_eq!(
        snapshot(&env.info()),
        info_before,
        "info/ debe quedar exactamente como estaba"
    );
    assert!(source.exists(), "el fichero sigue en su sitio");
    assert_eq!(read(&source), b"AAA".to_vec());
    assert!(entry_names(&env.files()).is_empty());
}

// ---------------------------------------------------------------------------
// cb_13
// ---------------------------------------------------------------------------

#[test]
fn cb_13_un_elemento_mayor_que_la_capacidad_avisa_y_no_se_mueve() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb13");
    let source = tree.write("grande.bin", &vec![7u8; 4096]);
    let policy = TrashPolicy {
        max_item_bytes: Some(1024),
        ..TrashPolicy::default()
    };

    match trash_one(&source, &policy) {
        Err(TrashError::ExceedsTrashCapacity {
            path,
            needed_bytes,
            available_bytes,
        }) => {
            assert_eq!(path, source, "el error nombra el fichero");
            assert_eq!(needed_bytes, 4096);
            assert_eq!(available_bytes, 1024);
        }
        other => panic!("expected ExceedsTrashCapacity, got {other:?}"),
    }

    assert!(source.exists(), "el fichero no se toca");
    assert_eq!(read(&source), vec![7u8; 4096]);
    assert!(entry_names(&env.files()).is_empty());
    assert!(entry_names(&env.info()).is_empty());
}

#[test]
fn cb_13_sin_limite_de_tamano_el_mismo_fichero_se_envia_sin_recorrer_bytes() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb13b");
    let source = tree.write("grande.bin", &vec![7u8; 4096]);
    let mut observer = Recorder::new();

    let outcome = trash_batch(&[source.clone()], &TrashPolicy::default(), &mut observer);

    assert_eq!(outcome.trashed.len(), 1);
    assert_eq!(observer.byte_calls, 0, "no hay travesia para medir el tamano");
    assert!(!source.exists());
    assert_eq!(entry_names(&env.files()), vec!["grande.bin".to_string()]);
}

// ---------------------------------------------------------------------------
// cb_14
// ---------------------------------------------------------------------------

#[test]
fn cb_14_deshacer_devuelve_el_arbol_al_estado_previo() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb14");
    let source = tree.write("sub/x.txt", b"AAA");
    let before = snapshot(&tree.root);

    let item = trashed(&source);
    assert!(!source.exists());

    let restored = match restore_item(&item, ConflictPolicy::Fail) {
        Ok(path) => path,
        Err(error) => panic!("restore_item should succeed, got {error:?}"),
    };

    assert_eq!(restored, item.original_path);
    assert_eq!(restored, source);
    assert_eq!(read(&source), b"AAA".to_vec());
    assert!(entry_names(&env.files()).is_empty(), "files/ queda vacio");
    assert!(entry_names(&env.info()).is_empty(), "info/ queda vacio");
    let after = snapshot(&tree.root);
    assert_eq!(
        before.len(),
        after.len(),
        "el arbol vuelve a tener las mismas entradas"
    );
}

// ---------------------------------------------------------------------------
// cb_15
// ---------------------------------------------------------------------------

#[test]
fn cb_15_restaurar_recrea_las_carpetas_intermedias_desaparecidas() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb15");
    let source = tree.write("uno/dos/x.txt", b"AAA");

    let item = trashed(&source);
    ok(fs::remove_dir_all(tree.path("uno")), "remove parent chain");
    assert!(!tree.path("uno").exists());

    let restored = match restore_item(&item, ConflictPolicy::Fail) {
        Ok(path) => path,
        Err(error) => panic!("restore_item should recreate parents, got {error:?}"),
    };

    assert_eq!(restored, source);
    assert!(tree.path("uno/dos").is_dir(), "se recrea la cadena de padres");
    assert_eq!(read(&source), b"AAA".to_vec());
    assert!(entry_names(&env.files()).is_empty());
    assert!(entry_names(&env.info()).is_empty());
}

#[test]
fn cb_15_si_el_padre_no_se_puede_crear_la_entrada_sigue_en_la_papelera() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb15b");
    let source = tree.write("bloqueado/dos/x.txt", b"AAA");
    let item = trashed(&source);
    ok(fs::remove_dir_all(tree.path("bloqueado/dos")), "remove parent");
    set_mode(&tree.path("bloqueado"), 0o500);

    let result = restore_item(&item, ConflictPolicy::Fail);

    set_mode(&tree.path("bloqueado"), 0o700);
    match result {
        Err(RestoreError::ParentCreation { parent, source: io }) => {
            assert!(
                parent.starts_with(tree.path("bloqueado")),
                "el error nombra el padre que no se pudo crear: {}",
                parent.display()
            );
            assert!(eacces_or_eperm(&io), "se conserva el errno: {io:?}");
        }
        other => panic!("expected ParentCreation, got {other:?}"),
    }
    assert!(item.trashed_path.exists(), "el fichero sigue en files/");
    assert!(item.info_path.exists(), "el .trashinfo sigue en info/");
    assert_eq!(read(&item.trashed_path), b"AAA".to_vec());
    assert_eq!(entry_names(&env.files()), vec!["x.txt".to_string()]);
}

// ---------------------------------------------------------------------------
// cb_16
// ---------------------------------------------------------------------------

#[test]
fn cb_16_restaurar_sobre_un_nombre_ocupado_no_sobrescribe() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb16");
    let source = tree.write("x.txt", b"AAA");
    let item = trashed(&source);
    ok(fs::write(&source, b"OTRO"), "recreate a different file");

    match restore_item(&item, ConflictPolicy::Fail) {
        Err(RestoreError::DestinationExists { destination }) => {
            assert_eq!(destination, source)
        }
        other => panic!("expected DestinationExists, got {other:?}"),
    }

    assert_eq!(read(&source), b"OTRO".to_vec(), "el existente no cambia");
    assert!(item.trashed_path.exists(), "la entrada sigue en la papelera");
    assert_eq!(read(&item.trashed_path), b"AAA".to_vec());
    assert_eq!(entry_names(&env.info()), vec!["x.txt.trashinfo".to_string()]);
}

#[test]
fn cb_16_keep_both_conserva_los_dos_ficheros() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb16b");
    let source = tree.write("x.txt", b"AAA");
    let item = trashed(&source);
    ok(fs::write(&source, b"OTRO"), "recreate a different file");

    let restored = match restore_item(&item, ConflictPolicy::KeepBoth) {
        Ok(path) => path,
        Err(error) => panic!("restore_item(KeepBoth) should succeed, got {error:?}"),
    };

    assert_ne!(restored, source, "KeepBoth no puede devolver la ruta ocupada");
    assert_eq!(restored, tree.path("x (2).txt"));
    assert_eq!(read(&source), b"OTRO".to_vec());
    assert_eq!(read(&restored), b"AAA".to_vec());
    assert!(entry_names(&env.files()).is_empty());
    assert!(entry_names(&env.info()).is_empty());
}

#[test]
fn cb_16_overwrite_solo_actua_cuando_el_llamador_lo_pide() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb16c");
    let source = tree.write("x.txt", b"AAA");
    let item = trashed(&source);
    ok(fs::write(&source, b"OTRO"), "recreate a different file");

    let restored = match restore_item(&item, ConflictPolicy::Overwrite) {
        Ok(path) => path,
        Err(error) => panic!("restore_item(Overwrite) should succeed, got {error:?}"),
    };

    assert_eq!(restored, source);
    assert_eq!(read(&source), b"AAA".to_vec());
    assert!(entry_names(&env.files()).is_empty());
    assert!(entry_names(&env.info()).is_empty());
}

// ---------------------------------------------------------------------------
// cb_17
// ---------------------------------------------------------------------------

#[test]
fn cb_17_si_la_entrada_ya_no_esta_en_la_papelera_se_avisa_sin_borrar_el_info() {
    let _env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb17");
    let source = tree.write("x.txt", b"AAA");
    let item = trashed(&source);
    ok(fs::remove_file(&item.trashed_path), "empty the trash by hand");

    match restore_item(&item, ConflictPolicy::Fail) {
        Err(RestoreError::TrashEntryMissing { trashed_path }) => {
            assert_eq!(trashed_path, item.trashed_path)
        }
        other => panic!("expected TrashEntryMissing, got {other:?}"),
    }

    assert!(
        item.info_path.exists(),
        "el .trashinfo queda como evidencia para el reporte"
    );
    assert!(!source.exists(), "no se inventa un fichero en el destino");
}

// ---------------------------------------------------------------------------
// cb_19
// ---------------------------------------------------------------------------

#[test]
fn cb_19_omitir_un_fallo_no_aborta_el_resto_del_lote() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb19");
    let locked_dir = tree.mkdir("bloqueado");
    let paths = vec![
        tree.write("uno.txt", b"1"),
        tree.write("bloqueado/dos.txt", b"2"),
        tree.write("tres.txt", b"3"),
        tree.write("cuatro.txt", b"4"),
        tree.write("cinco.txt", b"5"),
    ];
    set_mode(&locked_dir, 0o500);
    let mut observer = Recorder::deciding(ErrorDecision::Skip);

    let outcome = trash_batch(&paths, &TrashPolicy::default(), &mut observer);

    set_mode(&locked_dir, 0o700);
    assert_eq!(outcome.trashed.len(), 4, "los otros cuatro se envian igual");
    assert_eq!(outcome.skipped.len(), 1);
    assert!(!outcome.cancelled);
    let (failed_path, failed_error) = match outcome.skipped.first() {
        Some(entry) => entry,
        None => panic!("expected one skipped entry"),
    };
    assert_eq!(failed_path, &paths[1]);
    match failed_error {
        TrashError::PermissionDenied { path, source } => {
            assert_eq!(path, &paths[1]);
            assert!(eacces_or_eperm(source), "se conserva el errno: {source:?}");
        }
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
    assert!(paths[1].exists(), "el elemento fallido no se pierde");
    assert_eq!(read(&paths[1]), b"2".to_vec());
    assert_eq!(
        entry_names(&env.files()),
        vec![
            "cinco.txt".to_string(),
            "cuatro.txt".to_string(),
            "tres.txt".to_string(),
            "uno.txt".to_string()
        ]
    );
    assert_eq!(
        outcome.trashed.len() + outcome.skipped.len(),
        paths.len(),
        "sin cancelacion se intentan todos"
    );
}

#[test]
fn cb_19_reintentar_repite_el_mismo_elemento_hasta_que_funciona() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb19b");
    let locked_dir = tree.mkdir("bloqueado");
    let path = tree.write("bloqueado/dos.txt", b"2");
    set_mode(&locked_dir, 0o500);
    let mut observer = RetryThenFix {
        calls: 0,
        unlock_after: 2,
        unlock_dir: locked_dir.clone(),
    };

    let outcome = trash_batch(&[path.clone()], &TrashPolicy::default(), &mut observer);

    set_mode(&locked_dir, 0o700);
    assert_eq!(
        observer.calls, 2,
        "fallo dos veces y a la tercera funciono: dos consultas al observador"
    );
    assert_eq!(outcome.trashed.len(), 1, "el elemento acaba en la papelera");
    assert!(outcome.skipped.is_empty());
    assert!(!outcome.cancelled);
    assert!(!path.exists());
    assert_eq!(entry_names(&env.files()), vec!["dos.txt".to_string()]);
}

#[test]
fn cb_19_skip_all_no_vuelve_a_preguntar_por_los_siguientes_fallos() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb19c");
    let locked_dir = tree.mkdir("bloqueado");
    let paths = vec![
        tree.write("bloqueado/uno.txt", b"1"),
        tree.write("bloqueado/dos.txt", b"2"),
        tree.write("bloqueado/tres.txt", b"3"),
    ];
    set_mode(&locked_dir, 0o500);
    let mut observer = Recorder::deciding(ErrorDecision::SkipAll);

    let outcome = trash_batch(&paths, &TrashPolicy::default(), &mut observer);

    set_mode(&locked_dir, 0o700);
    assert_eq!(
        observer.errors.len(),
        1,
        "SkipAll silencia las consultas posteriores"
    );
    assert_eq!(outcome.skipped.len(), 3, "los tres quedan anotados como omitidos");
    assert!(outcome.trashed.is_empty());
    assert!(!outcome.cancelled);
    for path in &paths {
        assert!(path.exists(), "{} no se pierde", path.display());
    }
    assert!(entry_names(&env.files()).is_empty());
}

#[test]
fn cb_19_cancelar_desde_on_error_detiene_el_lote_sin_perder_nada() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb19d");
    let locked_dir = tree.mkdir("bloqueado");
    let paths = vec![
        tree.write("bloqueado/uno.txt", b"1"),
        tree.write("dos.txt", b"2"),
        tree.write("tres.txt", b"3"),
    ];
    set_mode(&locked_dir, 0o500);
    let mut observer = Recorder::deciding(ErrorDecision::Cancel);

    let outcome = trash_batch(&paths, &TrashPolicy::default(), &mut observer);

    set_mode(&locked_dir, 0o700);
    assert!(outcome.cancelled, "cancelar en on_error cancela el lote");
    assert!(outcome.trashed.is_empty());
    assert!(
        outcome.trashed.len() + outcome.skipped.len() < paths.len(),
        "los no intentados no aparecen en ninguna lista"
    );
    for path in &paths {
        assert!(path.exists(), "{} sigue en su sitio", path.display());
    }
    assert!(entry_names(&env.files()).is_empty());
}

// ---------------------------------------------------------------------------
// cb_20
// ---------------------------------------------------------------------------

#[test]
fn cb_20_cancelar_a_mitad_del_lote_no_deja_estados_a_medias() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb20");
    let paths = vec![
        tree.write("uno.txt", b"1"),
        tree.write("dos.txt", b"2"),
        tree.write("tres.txt", b"3"),
        tree.write("cuatro.txt", b"4"),
        tree.write("cinco.txt", b"5"),
    ];
    let mut observer = Recorder::cancelling_at(2);

    let outcome = trash_batch(&paths, &TrashPolicy::default(), &mut observer);

    assert_eq!(outcome.trashed.len(), 2, "solo los dos ya procesados");
    assert!(outcome.skipped.is_empty(), "cancelar no es omitir");
    assert!(outcome.cancelled);
    assert_eq!(
        observer.starts.len(),
        3,
        "no se consulta por los elementos posteriores al cancelado"
    );
    for path in paths.iter().skip(2) {
        assert!(path.exists(), "{} sigue en su ruta", path.display());
    }
    assert_eq!(
        entry_names(&env.files()),
        vec!["dos.txt".to_string(), "uno.txt".to_string()]
    );
    assert_eq!(
        entry_names(&env.info()),
        vec!["dos.txt.trashinfo".to_string(), "uno.txt.trashinfo".to_string()],
        "ni un .trashinfo suelto de los cancelados"
    );
}

// ---------------------------------------------------------------------------
// cb_21
// ---------------------------------------------------------------------------

#[test]
fn cb_21_sin_permisos_el_error_nombra_el_fichero_y_conserva_el_errno() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb21");
    let locked_dir = tree.mkdir("bloqueado");
    let path = tree.write("bloqueado/x.txt", b"AAA");
    set_mode(&locked_dir, 0o500);

    let result = trash_one(&path, &TrashPolicy::default());

    set_mode(&locked_dir, 0o700);
    match result {
        Err(TrashError::PermissionDenied {
            path: reported,
            source,
        }) => {
            assert_eq!(reported, path, "el error nombra exactamente la ruta pedida");
            assert!(eacces_or_eperm(&source), "el errno no se pierde: {source:?}");
        }
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
    assert!(path.exists());
    assert_eq!(read(&path), b"AAA".to_vec());
    assert!(entry_names(&env.files()).is_empty());
    assert!(entry_names(&env.info()).is_empty());
}

#[test]
fn cb_21_una_ruta_inexistente_es_not_found_con_su_errno() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb21b");
    let missing = tree.path("no-existe.txt");

    match trash_one(&missing, &TrashPolicy::default()) {
        Err(TrashError::NotFound { path, source }) => {
            assert_eq!(path, missing);
            assert_eq!(source.raw_os_error(), Some(2), "ENOENT se conserva");
        }
        other => panic!("expected NotFound, got {other:?}"),
    }

    assert!(entry_names(&env.files()).is_empty());
    assert!(entry_names(&env.info()).is_empty());
}

// ---------------------------------------------------------------------------
// cb_22
// ---------------------------------------------------------------------------

#[test]
fn cb_22_un_enlace_simbolico_se_envia_sin_seguir_su_destino() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb22");
    let target = tree.write("destino.txt", b"INTACTO");
    let link = tree.path("enlace");
    ok(
        std::os::unix::fs::symlink(&target, &link),
        "create symlink",
    );

    let item = trashed(&link);

    assert!(target.exists(), "el destino no se toca");
    assert_eq!(read(&target), b"INTACTO".to_vec());
    let trashed_link = env.files().join("enlace");
    let metadata = ok(fs::symlink_metadata(&trashed_link), "lstat trashed link");
    assert!(
        metadata.file_type().is_symlink(),
        "en la papelera hay un enlace, no una copia del destino"
    );
    assert_eq!(
        ok(fs::read_link(&trashed_link), "read trashed link"),
        target,
        "el enlace conserva su destino"
    );
    let info = match read_trash_info(&item.info_path) {
        Ok(info) => info,
        Err(error) => panic!("read_trash_info should succeed, got {error:?}"),
    };
    assert_eq!(info.original_path, link, "el .trashinfo apunta al enlace");
    assert!(!link.exists() && fs::symlink_metadata(&link).is_err());
}

#[test]
fn cb_22_un_enlace_roto_tambien_se_envia_sin_error() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb22b");
    let link = tree.path("roto");
    ok(
        std::os::unix::fs::symlink(tree.path("no-existe"), &link),
        "create broken symlink",
    );

    let item = trashed(&link);

    assert_eq!(item.original_path, link);
    let trashed_link = env.files().join("roto");
    let metadata = ok(fs::symlink_metadata(&trashed_link), "lstat trashed link");
    assert!(metadata.file_type().is_symlink());
    assert!(fs::symlink_metadata(&link).is_err(), "ya no esta en el origen");
}

// ---------------------------------------------------------------------------
// cb_23
// ---------------------------------------------------------------------------

#[test]
fn cb_23_la_raiz_del_sistema_se_rechaza_sin_efecto() {
    let env = TrashEnv::on_home_device();

    match trash_one(Path::new("/"), &TrashPolicy::default()) {
        Err(TrashError::RefusedSpecialPath { path, reason }) => {
            assert_eq!(path, PathBuf::from("/"));
            assert_eq!(reason, RefusalReason::Root);
        }
        other => panic!("expected RefusedSpecialPath(Root), got {other:?}"),
    }

    assert!(Path::new("/etc").is_dir(), "el sistema sigue en pie");
    assert!(entry_names(&env.files()).is_empty());
}

#[test]
fn cb_23_una_ruta_relativa_o_vacia_se_rechaza() {
    let env = TrashEnv::on_home_device();

    match trash_one(Path::new("relativa.txt"), &TrashPolicy::default()) {
        Err(TrashError::RefusedSpecialPath { reason, .. }) => {
            assert_eq!(reason, RefusalReason::Relative)
        }
        other => panic!("expected RefusedSpecialPath(Relative), got {other:?}"),
    }

    match trash_one(Path::new(""), &TrashPolicy::default()) {
        Err(TrashError::RefusedSpecialPath { reason, .. }) => {
            assert_eq!(reason, RefusalReason::Empty)
        }
        other => panic!("expected RefusedSpecialPath(Empty), got {other:?}"),
    }

    assert!(entry_names(&env.files()).is_empty());
    assert!(entry_names(&env.info()).is_empty());
}

#[test]
fn cb_23_un_punto_de_montaje_se_rechaza_como_tal() {
    let env = TrashEnv::on_home_device();

    // /proc siempre es un punto de montaje y nunca es del usuario: si la
    // implementacion no lo rechazase, el rename fallaria por permisos y este
    // test lo delataria igualmente.
    match trash_one(Path::new("/proc"), &TrashPolicy::default()) {
        Err(TrashError::RefusedSpecialPath { path, reason }) => {
            assert_eq!(path, PathBuf::from("/proc"));
            assert_eq!(reason, RefusalReason::MountPoint);
        }
        other => panic!("expected RefusedSpecialPath(MountPoint), got {other:?}"),
    }

    assert!(Path::new("/proc/self").exists(), "/proc sigue montado");
    assert!(entry_names(&env.files()).is_empty());
}

#[test]
fn cb_23_no_se_puede_enviar_a_la_papelera_algo_que_ya_esta_dentro() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb23");
    let source = tree.write("x.txt", b"AAA");
    let item = trashed(&source);
    let before = snapshot(&env.trash_root());

    match trash_one(&item.trashed_path, &TrashPolicy::default()) {
        Err(TrashError::PathIsInsideTrash { path }) => assert_eq!(path, item.trashed_path),
        other => panic!("expected PathIsInsideTrash, got {other:?}"),
    }

    assert_eq!(snapshot(&env.trash_root()), before, "la papelera no cambia");
    assert_eq!(read(&item.trashed_path), b"AAA".to_vec());
}

#[test]
fn cb_23_no_se_puede_enviar_a_la_papelera_un_ancestro_de_la_propia_papelera() {
    let env = TrashEnv::on_home_device();
    let before = snapshot(&env.data_home.root);

    match trash_one(&env.data_home.root, &TrashPolicy::default()) {
        Err(TrashError::RefusedSpecialPath { reason, .. }) => {
            assert_eq!(reason, RefusalReason::TrashAncestor)
        }
        Err(TrashError::PathIsInsideTrash { .. }) => {
            panic!("un ancestro de la papelera no esta dentro de ella")
        }
        other => panic!("expected RefusedSpecialPath(TrashAncestor), got {other:?}"),
    }

    assert_eq!(snapshot(&env.data_home.root), before);
}

// ---------------------------------------------------------------------------
// cb_25
// ---------------------------------------------------------------------------

#[test]
fn cb_25_probe_trash_no_tiene_ningun_efecto_secundario() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb25");
    let source = tree.write("x.txt", b"AAA");
    if let Err(error) = home_trash_dir() {
        panic!("home_trash_dir should succeed, got {error:?}");
    }
    let trash_before = snapshot(&env.trash_root());
    let tree_before = snapshot(&tree.root);

    let availability = match probe_trash(&source, &TrashPolicy::default()) {
        Ok(value) => value,
        Err(error) => panic!("probe_trash should succeed, got {error:?}"),
    };

    match availability {
        kara_fs::trash::TrashAvailability::Available {
            kind, would_create, ..
        } => {
            assert_eq!(kind, TrashKind::Home);
            assert!(!would_create, "la papelera del hogar ya existia");
        }
        other => panic!("expected Available, got {other:?}"),
    }
    assert_eq!(snapshot(&env.trash_root()), trash_before, "la papelera intacta");
    assert_eq!(snapshot(&tree.root), tree_before, "el origen intacto");
    assert!(source.exists());
}

#[test]
fn cb_25_probe_trash_anticipa_el_exceso_de_capacidad_sin_mover_nada() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb25b");
    let source = tree.write("grande.bin", &vec![7u8; 4096]);
    let policy = TrashPolicy {
        max_item_bytes: Some(1024),
        ..TrashPolicy::default()
    };

    match probe_trash(&source, &policy) {
        Ok(kara_fs::trash::TrashAvailability::ExceedsCapacity {
            needed_bytes,
            available_bytes,
        }) => {
            assert_eq!(needed_bytes, 4096);
            assert_eq!(available_bytes, 1024);
        }
        other => panic!("expected ExceedsCapacity, got {other:?}"),
    }

    assert!(source.exists());
    assert_eq!(read(&source), vec![7u8; 4096]);
    assert!(entry_names(&env.files()).is_empty());
}

// ---------------------------------------------------------------------------
// cb_26
// ---------------------------------------------------------------------------

#[test]
fn cb_26_delete_permanently_borra_recursivamente_y_cuenta_las_entradas() {
    let _env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb26");
    let root = tree.mkdir("arbol");
    let _ = tree.write("arbol/a.txt", b"a");
    let _ = tree.write("arbol/uno/b.txt", b"b");
    let _ = tree.write("arbol/uno/dos/c.txt", b"c");
    let mut observer = Recorder::new();

    let removed = match delete_permanently(&root, &mut observer) {
        Ok(count) => count,
        Err(error) => panic!("delete_permanently should succeed, got {error:?}"),
    };

    assert_eq!(
        removed, 6,
        "3 ficheros + 2 subdirectorios + la raiz del arbol"
    );
    assert!(!root.exists(), "el arbol desaparece");
    assert!(tree.root.is_dir(), "solo se borra lo pedido");
}

#[test]
fn cb_26_delete_permanently_borra_el_enlace_no_su_destino() {
    let _env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb26b");
    let target = tree.write("fuera/destino.txt", b"INTACTO");
    let root = tree.mkdir("arbol");
    ok(
        std::os::unix::fs::symlink(&target, root.join("enlace")),
        "create symlink inside tree",
    );
    let mut observer = Silent;

    let removed = match delete_permanently(&root, &mut observer) {
        Ok(count) => count,
        Err(error) => panic!("delete_permanently should succeed, got {error:?}"),
    };

    assert_eq!(removed, 2, "el enlace y el directorio que lo contiene");
    assert!(!root.exists());
    assert!(target.exists(), "el destino del enlace no se sigue");
    assert_eq!(read(&target), b"INTACTO".to_vec());
}

#[test]
fn cb_26_delete_permanently_es_cancelable_entre_entradas() {
    let _env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb26c");
    let root = tree.mkdir("arbol");
    for index in 0..50 {
        let _ = tree.write(&format!("arbol/f-{index}.txt"), b"x");
    }
    let mut observer = Recorder::new();
    observer.cancel_after_bytes = Some(2);

    match delete_permanently(&root, &mut observer) {
        Err(TrashError::Cancelled) => {}
        other => panic!("expected Cancelled, got {other:?}"),
    }

    assert!(observer.byte_calls >= 1, "hubo progreso antes de cancelar");
    assert!(
        root.exists(),
        "cancelar detiene el borrado: la raiz sobrevive"
    );
}

// ---------------------------------------------------------------------------
// cb_28
// ---------------------------------------------------------------------------

#[test]
fn cb_28_un_nombre_de_255_bytes_se_envia_y_conserva_su_ruta_original_completa() {
    let _env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb28");
    let name = "n".repeat(255);
    let source = tree.write(&name, b"AAA");

    let item = trashed(&source);

    assert!(!source.exists());
    assert_eq!(read(&item.trashed_path), b"AAA".to_vec());
    let info = match read_trash_info(&item.info_path) {
        Ok(info) => info,
        Err(error) => panic!("read_trash_info should succeed, got {error:?}"),
    };
    assert_eq!(
        info.original_path, source,
        "la ruta completa vive en el Path=, no en el nombre del info"
    );
    let info_name = match item.info_path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => panic!("info path must have a file name"),
    };
    assert!(
        info_name.len() <= 255,
        "el nombre del .trashinfo respeta NAME_MAX: {} bytes",
        info_name.len()
    );
    assert!(info_name.ends_with(".trashinfo"));
}

// ---------------------------------------------------------------------------
// cb_29
// ---------------------------------------------------------------------------

#[test]
fn cb_29_un_lote_grande_consulta_la_cancelacion_una_vez_por_elemento() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb29");
    let mut paths = Vec::new();
    for index in 0..2_000 {
        paths.push(tree.write(&format!("f-{index}.txt"), b"x"));
    }
    let mut observer = Recorder::new();

    let outcome = trash_batch(&paths, &TrashPolicy::default(), &mut observer);

    assert_eq!(outcome.trashed.len(), 2_000);
    assert_eq!(
        observer.starts.len(),
        2_000,
        "ningun bucle corre sin consultar cancelacion"
    );
    assert_eq!(entry_names(&env.files()).len(), 2_000);
}

#[test]
fn cb_29_cancelar_en_el_primer_elemento_detiene_el_lote_entero() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb29b");
    let mut paths = Vec::new();
    for index in 0..500 {
        paths.push(tree.write(&format!("f-{index}.txt"), b"x"));
    }
    let mut observer = Recorder::cancelling_at(0);

    let outcome = trash_batch(&paths, &TrashPolicy::default(), &mut observer);

    assert!(outcome.trashed.is_empty());
    assert!(outcome.skipped.is_empty());
    assert!(outcome.cancelled);
    assert_eq!(observer.starts.len(), 1, "no se consulta por los demas");
    assert!(entry_names(&env.files()).is_empty());
    assert_eq!(entry_names(&tree.root).len(), 500, "nada se movio");
}

// ---------------------------------------------------------------------------
// cb_27 y cb_31: reglas sobre el propio codigo fuente
// ---------------------------------------------------------------------------

fn source_lines() -> Vec<(PathBuf, usize, String)> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rust_files(&src, &mut files);
    assert!(!files.is_empty(), "deberia haber codigo en {}", src.display());
    let mut lines = Vec::new();
    for file in files {
        let text = ok(fs::read_to_string(&file), "read source file");
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            lines.push((file.clone(), index + 1, line.to_string()));
        }
    }
    lines
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn cb_27_el_codigo_de_kara_fs_no_usa_unwrap_expect_ni_panicos() {
    let forbidden = [
        ".unwrap(",
        ".expect(",
        "panic!",
        "unreachable!",
        "todo!",
        "unimplemented!",
    ];

    let offenders: Vec<String> = source_lines()
        .into_iter()
        .filter(|(_, _, line)| forbidden.iter().any(|needle| line.contains(needle)))
        .map(|(file, number, line)| format!("{}:{number}: {}", file.display(), line.trim()))
        .collect();

    assert!(
        offenders.is_empty(),
        "ninguna ruta que toque ficheros del usuario puede entrar en panico:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn cb_31_kara_fs_no_decide_ni_muestra_confirmaciones() {
    let forbidden = ["confirm", "Confirm", "Dialog", "dialog"];

    let offenders: Vec<String> = source_lines()
        .into_iter()
        .filter(|(_, _, line)| forbidden.iter().any(|needle| line.contains(needle)))
        .map(|(file, number, line)| format!("{}:{number}: {}", file.display(), line.trim()))
        .collect();

    assert!(
        offenders.is_empty(),
        "el dialogo de confirmacion vive por encima de kara-fs:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn cb_31_kara_fs_no_escribe_en_la_salida_estandar() {
    let forbidden = ["println!", "eprintln!", "print!", "eprint!", "dbg!"];

    let offenders: Vec<String> = source_lines()
        .into_iter()
        .filter(|(_, _, line)| forbidden.iter().any(|needle| line.contains(needle)))
        .map(|(file, number, line)| format!("{}:{number}: {}", file.display(), line.trim()))
        .collect();

    assert!(offenders.is_empty(), "kara-fs no escribe por consola:\n{}", offenders.join("\n"));
}
