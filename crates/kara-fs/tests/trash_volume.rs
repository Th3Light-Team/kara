//! Pruebas de las papeleras por volumen y del cruce de dispositivos.
//!
//! Aprovechan que `/tmp` es un tmpfs con un `st_dev` distinto al del árbol del
//! proyecto: eso da dos volúmenes reales sin necesidad de root. Cuando la
//! máquina no ofrece esa separación, la prueba falla en voz alta en vez de
//! pasar de vacío.
//!
//! Casos borde cubiertos: cb_05 (en disco), cb_10, cb_11, cb_12, cb_18, cb_24.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{
    Recorder, TempTree, TrashEnv, current_uid, entry_names, home_device_base, mode_of,
    on_different_devices, ok, read, snapshot, tmpfs_base,
};
use kara_fs::trash::{
    ConflictPolicy, RestoreError, TrashAvailability, TrashError, TrashKind, TrashPolicy,
    TrashedItem, UnavailableReason, probe_trash, read_trash_info, resolve_trash_dir, restore_item,
    trash_batch, trash_one,
};

/// Guard that removes the volume trash directories these tests create in the
/// shared tmpfs, and refuses to run if the machine already had one (deleting
/// somebody else's trash would itself be data loss).
struct TmpfsTrashGuard {
    volume_trash: PathBuf,
    dot_trash: PathBuf,
}

impl TmpfsTrashGuard {
    fn new(uid: u32) -> TmpfsTrashGuard {
        let volume_trash = tmpfs_base().join(format!(".Trash-{uid}"));
        let dot_trash = tmpfs_base().join(".Trash");
        assert!(
            !volume_trash.exists(),
            "{} ya existe: no la borro, ejecuta la prueba con /tmp limpio",
            volume_trash.display()
        );
        assert!(
            fs::symlink_metadata(&dot_trash).is_err(),
            "{} ya existe: no la toco, ejecuta la prueba con /tmp limpio",
            dot_trash.display()
        );
        TmpfsTrashGuard {
            volume_trash,
            dot_trash,
        }
    }
}

impl Drop for TmpfsTrashGuard {
    fn drop(&mut self) {
        common::restore_modes(&self.volume_trash);
        let _ = fs::remove_dir_all(&self.volume_trash);
        let _ = fs::remove_file(&self.dot_trash);
        let _ = fs::remove_dir_all(&self.dot_trash);
    }
}

fn require_two_devices() {
    assert!(
        on_different_devices(&home_device_base(), &tmpfs_base()),
        "estas pruebas necesitan que /tmp y el arbol del proyecto esten en dispositivos distintos"
    );
}

// ---------------------------------------------------------------------------
// cb_10 y cb_24: volumen sin papelera utilizable
// ---------------------------------------------------------------------------

/// `XDG_DATA_HOME` en tmpfs y el fichero en el volumen raíz: el volumen del
/// fichero (`/`) no tiene papelera y este usuario no puede crearla.
#[test]
fn cb_10_un_volumen_sin_papelera_avisa_y_jamas_borra_el_fichero() {
    require_two_devices();
    let env = TrashEnv::on_tmpfs();
    let tree = TempTree::on_home_device("cb10");
    let source = tree.write("x.txt", b"AAA");

    let result = trash_one(&source, &TrashPolicy::default());

    match result {
        Err(TrashError::NoTrashOnVolume { path, reason }) => {
            assert_eq!(path, source, "el aviso nombra el fichero");
            assert!(
                matches!(
                    reason,
                    UnavailableReason::TopDirNotWritable
                        | UnavailableReason::TopDirReadOnly
                        | UnavailableReason::VolumeTrashMissingAndCreationDisabled
                ),
                "razon inesperada: {reason:?}"
            );
        }
        other => panic!("expected NoTrashOnVolume, got {other:?}"),
    }

    assert!(source.exists(), "el fichero NO se borra: solo se avisa");
    assert_eq!(read(&source), b"AAA".to_vec());
    assert!(
        entry_names(&env.files()).is_empty(),
        "tampoco se cae a la papelera del hogar copiando bytes"
    );
}

#[test]
fn cb_24_por_defecto_no_se_copia_ni_un_byte_entre_volumenes() {
    require_two_devices();
    let env = TrashEnv::on_tmpfs();
    let tree = TempTree::on_home_device("cb24");
    let source = tree.write("x.bin", &vec![3u8; 64 * 1024]);
    let mut observer = Recorder::new();

    let outcome = trash_batch(&[source.clone()], &TrashPolicy::default(), &mut observer);

    assert!(outcome.trashed.is_empty());
    assert_eq!(outcome.skipped.len(), 1, "se reporta, no se aborta el lote");
    assert_eq!(
        observer.byte_calls, 0,
        "allow_cross_device_copy = false no puede recorrer bytes"
    );
    assert!(source.exists());
    assert_eq!(read(&source).len(), 64 * 1024);
    assert!(entry_names(&env.files()).is_empty());
}

#[test]
#[ignore = "necesita un segundo volumen escribible montado con una papelera valida: \
            monta uno (p. ej. `sudo mount -t tmpfs tmpfs /mnt/kara`, `chmod 1777 /mnt/kara`) \
            y ejecuta `cargo test -p kara-fs -- --ignored cb_24_exdev`"]
fn cb_24_exdev_real_falla_con_cross_device_sin_tocar_el_original() {
    let env = TrashEnv::on_home_device();
    let source = PathBuf::from("/mnt/kara/x.txt");
    ok(fs::write(&source, b"AAA"), "write file on the second volume");

    match trash_one(&source, &TrashPolicy::default()) {
        Err(TrashError::CrossDevice { path, .. }) => assert_eq!(path, source),
        other => panic!("expected CrossDevice, got {other:?}"),
    }

    assert!(source.exists());
    assert!(entry_names(&env.files()).is_empty());
}

#[test]
fn cb_10_probe_trash_anticipa_el_volumen_sin_papelera_sin_crear_nada() {
    require_two_devices();
    let env = TrashEnv::on_tmpfs();
    let tree = TempTree::on_home_device("cb10b");
    let source = tree.write("x.txt", b"AAA");
    let tree_before = snapshot(&tree.root);
    let trash_before = snapshot(&env.trash_root());

    match probe_trash(&source, &TrashPolicy::default()) {
        Ok(TrashAvailability::Unavailable { reason }) => {
            assert!(
                matches!(
                    reason,
                    UnavailableReason::TopDirNotWritable
                        | UnavailableReason::TopDirReadOnly
                        | UnavailableReason::VolumeTrashMissingAndCreationDisabled
                ),
                "razon inesperada: {reason:?}"
            );
        }
        other => panic!("expected Unavailable, got {other:?}"),
    }

    assert_eq!(snapshot(&tree.root), tree_before);
    assert_eq!(snapshot(&env.trash_root()), trash_before);
    assert!(source.exists());
}

// ---------------------------------------------------------------------------
// cb_11: creacion de la papelera de volumen
// ---------------------------------------------------------------------------

#[test]
fn cb_11_la_papelera_de_volumen_se_crea_cuando_la_politica_lo_permite() {
    require_two_devices();
    let env = TrashEnv::on_home_device();
    let uid = current_uid(&env.data_home.root);
    let guard = TmpfsTrashGuard::new(uid);
    let tree = TempTree::on_tmpfs("cb11");
    let source = tree.write("datos/x.txt", b"AAA");

    let item = match trash_one(&source, &TrashPolicy::default()) {
        Ok(item) => item,
        Err(error) => panic!("trash_one should succeed on a writable volume, got {error:?}"),
    };

    assert_eq!(item.kind, TrashKind::Volume);
    assert_eq!(item.top_dir, Some(tmpfs_base()));
    assert_eq!(item.bytes_copied, None, "sigue siendo un rename(2)");
    assert!(
        item.trashed_path.starts_with(&guard.volume_trash),
        "el elemento vive en {}, no en {}",
        item.trashed_path.display(),
        guard.volume_trash.display()
    );
    assert!(guard.volume_trash.is_dir(), "se creo .Trash-<uid>");
    assert_eq!(mode_of(&guard.volume_trash), 0o700);
    assert!(guard.volume_trash.join("files").is_dir());
    assert!(guard.volume_trash.join("info").is_dir());
    assert!(!source.exists());
    assert!(
        entry_names(&env.files()).is_empty(),
        "no se usa la papelera del hogar para un fichero de otro volumen"
    );
}

#[test]
fn cb_05_en_disco_la_papelera_de_volumen_registra_el_path_relativo_al_topdir() {
    require_two_devices();
    let env = TrashEnv::on_home_device();
    let uid = current_uid(&env.data_home.root);
    let _guard = TmpfsTrashGuard::new(uid);
    let tree = TempTree::on_tmpfs("cb05");
    let source = tree.write("datos/x.txt", b"AAA");
    let relative = match source.strip_prefix(tmpfs_base()) {
        Ok(relative) => relative.to_path_buf(),
        Err(error) => panic!("the source must live under /tmp: {error}"),
    };

    let item = match trash_one(&source, &TrashPolicy::default()) {
        Ok(item) => item,
        Err(error) => panic!("trash_one should succeed, got {error:?}"),
    };

    let raw = String::from_utf8_lossy(&read(&item.info_path)).into_owned();
    assert!(
        raw.contains(&format!("\nPath={}\n", relative.display())),
        "Path= debe ser relativo al topdir, fue: {raw:?}"
    );
    let info = match read_trash_info(&item.info_path) {
        Ok(info) => info,
        Err(error) => panic!("read_trash_info should succeed, got {error:?}"),
    };
    assert_eq!(
        info.original_path, source,
        "al leerlo se reconstruye la ruta absoluta"
    );
}

#[test]
fn cb_14_ida_y_vuelta_completa_tambien_en_la_papelera_de_volumen() {
    require_two_devices();
    let env = TrashEnv::on_home_device();
    let uid = current_uid(&env.data_home.root);
    let guard = TmpfsTrashGuard::new(uid);
    let tree = TempTree::on_tmpfs("cb14v");
    let source = tree.write("datos/x.txt", b"AAA");

    let item = match trash_one(&source, &TrashPolicy::default()) {
        Ok(item) => item,
        Err(error) => panic!("trash_one should succeed, got {error:?}"),
    };
    let restored = match restore_item(&item, ConflictPolicy::Fail) {
        Ok(path) => path,
        Err(error) => panic!("restore_item should succeed, got {error:?}"),
    };

    assert_eq!(restored, source);
    assert_eq!(read(&source), b"AAA".to_vec());
    assert!(entry_names(&guard.volume_trash.join("files")).is_empty());
    assert!(entry_names(&guard.volume_trash.join("info")).is_empty());
}

#[test]
fn cb_11_sin_permiso_de_creacion_se_avisa_y_no_se_crea_nada() {
    require_two_devices();
    let env = TrashEnv::on_home_device();
    let uid = current_uid(&env.data_home.root);
    let guard = TmpfsTrashGuard::new(uid);
    let tree = TempTree::on_tmpfs("cb11b");
    let source = tree.write("x.txt", b"AAA");
    let policy = TrashPolicy {
        create_volume_trash: false,
        ..TrashPolicy::default()
    };

    match trash_one(&source, &policy) {
        Err(TrashError::NoTrashOnVolume { path, reason }) => {
            assert_eq!(path, source);
            assert_eq!(
                reason,
                UnavailableReason::VolumeTrashMissingAndCreationDisabled
            );
        }
        other => panic!("expected NoTrashOnVolume, got {other:?}"),
    }

    assert!(source.exists(), "no se borra nada");
    assert_eq!(read(&source), b"AAA".to_vec());
    assert!(
        !guard.volume_trash.exists(),
        "con la creacion desactivada no aparece {}",
        guard.volume_trash.display()
    );
    assert!(entry_names(&env.files()).is_empty());
}

// ---------------------------------------------------------------------------
// cb_12: $topdir/.Trash sospechosa
// ---------------------------------------------------------------------------

#[test]
fn cb_12_un_topdir_trash_que_es_symlink_se_rechaza_y_no_se_escribe_a_traves() {
    require_two_devices();
    let env = TrashEnv::on_home_device();
    let uid = current_uid(&env.data_home.root);
    let guard = TmpfsTrashGuard::new(uid);
    let decoy = TempTree::on_home_device("cb12-decoy");
    ok(
        std::os::unix::fs::symlink(&decoy.root, &guard.dot_trash),
        "point /tmp/.Trash at a decoy",
    );
    let tree = TempTree::on_tmpfs("cb12");
    let source = tree.write("x.txt", b"AAA");

    let dir = match resolve_trash_dir(&source, &TrashPolicy::default()) {
        Ok(dir) => dir,
        Err(error) => panic!("resolve_trash_dir should succeed, got {error:?}"),
    };

    assert_eq!(dir.kind, TrashKind::Volume);
    assert_eq!(
        dir.root,
        guard.volume_trash,
        "una .Trash sospechosa se sustituye por .Trash-<uid>"
    );
    assert!(
        entry_names(&decoy.root).is_empty(),
        "no se escribe nada a traves del enlace"
    );
}

#[test]
fn cb_12_un_topdir_trash_sin_bit_sticky_se_rechaza() {
    require_two_devices();
    let env = TrashEnv::on_home_device();
    let uid = current_uid(&env.data_home.root);
    let guard = TmpfsTrashGuard::new(uid);
    ok(fs::create_dir(&guard.dot_trash), "create /tmp/.Trash");
    common::set_mode(&guard.dot_trash, 0o700);
    let tree = TempTree::on_tmpfs("cb12b");
    let source = tree.write("x.txt", b"AAA");

    let dir = match resolve_trash_dir(&source, &TrashPolicy::default()) {
        Ok(dir) => dir,
        Err(error) => panic!("resolve_trash_dir should succeed, got {error:?}"),
    };

    assert_eq!(
        dir.root,
        guard.volume_trash,
        "sin bit sticky no se usa $topdir/.Trash"
    );
    assert!(
        entry_names(&guard.dot_trash).is_empty(),
        "no se crea el subdirectorio del uid dentro de la .Trash rechazada"
    );

    match probe_trash(&source, &TrashPolicy::default()) {
        Ok(TrashAvailability::Available { kind, .. }) => assert_eq!(kind, TrashKind::Volume),
        other => panic!("expected Available(Volume), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// cb_18: la ubicacion original ya no es alcanzable
// ---------------------------------------------------------------------------

#[test]
#[ignore = "necesita una unidad realmente desconectada: monta un tmpfs en /media/kara, \
            envia un fichero suyo a la papelera, desmontalo y ejecuta \
            `cargo test -p kara-fs -- --ignored cb_18_unidad_desconectada`"]
fn cb_18_unidad_desconectada_se_distingue_de_un_fallo_de_creacion() {
    let env = TrashEnv::on_home_device();
    let item = TrashedItem {
        original_path: PathBuf::from("/media/kara/x.txt"),
        trashed_path: env.files().join("x.txt"),
        info_path: env.info().join("x.txt.trashinfo"),
        deletion_date: kara_fs::trash::DeletionDate {
            year: 2026,
            month: 9,
            day: 2,
            hour: 17,
            minute: 6,
            second: 24,
        },
        kind: TrashKind::Home,
        top_dir: None,
        bytes_copied: None,
    };

    match restore_item(&item, ConflictPolicy::Fail) {
        Err(RestoreError::DestinationVolumeUnavailable { destination, .. }) => {
            assert_eq!(destination, item.original_path)
        }
        other => panic!("expected DestinationVolumeUnavailable, got {other:?}"),
    }
}

/// Cobertura no ignorada del mismo principio: si el destino no es alcanzable,
/// la entrada sobrevive íntegra en la papelera y no se crean directorios
/// sueltos por el camino.
#[test]
fn cb_18_un_destino_inalcanzable_conserva_la_entrada_en_la_papelera() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("cb18");
    let source = tree.write("x.txt", b"AAA");
    let item = match trash_one(&source, &TrashPolicy::default()) {
        Ok(item) => item,
        Err(error) => panic!("trash_one should succeed, got {error:?}"),
    };
    let unreachable = Path::new("/proc/kara-inexistente/x.txt");
    let broken = TrashedItem {
        original_path: unreachable.to_path_buf(),
        ..item.clone()
    };

    let result = restore_item(&broken, ConflictPolicy::Fail);

    assert!(
        result.is_err(),
        "restaurar a un destino inalcanzable no puede devolver Ok"
    );
    assert!(
        !Path::new("/proc/kara-inexistente").exists(),
        "no se crean directorios sueltos por el camino"
    );
    assert!(item.trashed_path.exists(), "la entrada sigue en files/");
    assert!(item.info_path.exists(), "el .trashinfo sigue en info/");
    assert_eq!(read(&item.trashed_path), b"AAA".to_vec());
    assert_eq!(entry_names(&env.files()), vec!["x.txt".to_string()]);
}
