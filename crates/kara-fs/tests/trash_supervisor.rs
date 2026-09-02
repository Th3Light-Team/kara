//! Pruebas añadidas en la fase de supervisión, sobre huecos que ni el contrato
//! ni las 78 pruebas originales llegaron a cubrir.
//!
//! Cada una falla contra el código tal y como estaba antes de la supervisión;
//! la nota de cada prueba dice con qué fallo exacto.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{
    Silent, TempTree, TrashEnv, current_uid, entry_names, home_device_base, ok, read,
    restore_modes,
};
use kara_fs::trash::{
    RefusalReason, TrashAvailability, TrashError, TrashPolicy, UnavailableReason,
    delete_permanently, probe_trash, trash_one,
};

/// Second volume for these tests. Deliberately **not** `/tmp`: `trash_volume.rs`
/// already claims `/tmp/.Trash-<uid>`, and the environment mutex that
/// serialises those tests is per-process, so two test binaries running at the
/// same time would fight over it. `/dev/shm` is a separate tmpfs nobody else
/// here touches.
fn shm_base() -> PathBuf {
    PathBuf::from("/dev/shm")
}

fn shm_is_a_separate_writable_volume() -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Ok(shm), Ok(home)) = (fs::metadata(shm_base()), fs::metadata(home_device_base())) else {
        return false;
    };
    if shm.dev() == home.dev() {
        return false;
    }
    let probe = shm_base().join(format!("kara-shm-probe-{}", std::process::id()));
    let writable = fs::create_dir(&probe).is_ok();
    let _ = fs::remove_dir(&probe);
    writable
}

/// Removes whatever the test planted at `/dev/shm/.Trash-<uid>`, and refuses to
/// run if something was already there: deleting somebody else's trash would
/// itself be data loss.
struct ShmTrashGuard {
    volume_trash: PathBuf,
}

impl ShmTrashGuard {
    fn new(uid: u32) -> ShmTrashGuard {
        let volume_trash = shm_base().join(format!(".Trash-{uid}"));
        assert!(
            fs::symlink_metadata(&volume_trash).is_err(),
            "{} ya existe: no la toco, ejecuta la prueba con /dev/shm limpio",
            volume_trash.display()
        );
        ShmTrashGuard { volume_trash }
    }
}

impl Drop for ShmTrashGuard {
    fn drop(&mut self) {
        restore_modes(&self.volume_trash);
        let _ = fs::remove_file(&self.volume_trash);
        let _ = fs::remove_dir_all(&self.volume_trash);
    }
}

// ---------------------------------------------------------------------------
// cb_12 extendido: la desconfianza que se le aplica a `$topdir/.Trash` hay que
// aplicarsela tambien a la papelera en la que de verdad se escribe.
// ---------------------------------------------------------------------------

/// El `$topdir` de una unidad extraíble suele ser escribible por cualquiera
/// (por eso la `.Trash` compartida necesita el bit sticky). Cualquiera puede
/// dejar ahí un `.Trash-<uid>` que sea un enlace simbólico a un directorio
/// suyo: seguirlo entrega los ficheros borrados de este usuario, y sus
/// `.trashinfo`, a otra persona, y encima con la operación devolviendo `Ok`.
///
/// Antes del arreglo: `trash_one` devolvía `Ok` y el fichero acababa dentro del
/// directorio señuelo (`build_volume_trash_dir` usaba `Path::is_dir`, que sigue
/// los enlaces).
#[test]
fn sup_01_una_trash_uid_que_es_un_enlace_no_se_usa_ni_se_escribe_a_traves() {
    if !shm_is_a_separate_writable_volume() {
        return;
    }
    let env = TrashEnv::on_home_device();
    let uid = current_uid(&env.data_home.root);
    let guard = ShmTrashGuard::new(uid);
    let decoy = TempTree::new_in(&shm_base(), "sup01-decoy");
    let tree = TempTree::new_in(&shm_base(), "sup01");
    let source = tree.write("x.txt", b"AAA");
    ok(
        std::os::unix::fs::symlink(&decoy.root, &guard.volume_trash),
        "plant the symlinked volume trash",
    );

    let result = trash_one(&source, &TrashPolicy::default());

    match result {
        Err(TrashError::NoTrashOnVolume { path, reason }) => {
            assert_eq!(path, source, "el aviso nombra el fichero");
            assert_eq!(reason, UnavailableReason::VolumeTrashRejected);
        }
        other => panic!("expected NoTrashOnVolume(VolumeTrashRejected), got {other:?}"),
    }

    assert!(source.exists(), "el fichero sigue en su sitio");
    assert_eq!(read(&source), b"AAA".to_vec());
    assert!(
        entry_names(&decoy.root).is_empty(),
        "no se escribio nada a traves del enlace: {:?}",
        entry_names(&decoy.root)
    );
    assert!(
        fs::symlink_metadata(&guard.volume_trash)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false),
        "el enlace se deja como estaba, no se sustituye por una papelera"
    );
}

/// El aviso tiene que llegar *antes* de actuar: `probe_trash` debe señalar la
/// misma papelera indigna de confianza que `trash_one` rechaza, y sin tocar
/// nada.
///
/// Antes del arreglo: devolvía `Available { kind: Volume, would_create: false }`.
#[test]
fn sup_02_probe_avisa_de_la_trash_uid_que_es_un_enlace() {
    if !shm_is_a_separate_writable_volume() {
        return;
    }
    let env = TrashEnv::on_home_device();
    let uid = current_uid(&env.data_home.root);
    let guard = ShmTrashGuard::new(uid);
    let decoy = TempTree::new_in(&shm_base(), "sup02-decoy");
    let tree = TempTree::new_in(&shm_base(), "sup02");
    let source = tree.write("x.txt", b"AAA");
    ok(
        std::os::unix::fs::symlink(&decoy.root, &guard.volume_trash),
        "plant the symlinked volume trash",
    );

    match probe_trash(&source, &TrashPolicy::default()) {
        Ok(TrashAvailability::Unavailable { reason }) => {
            assert_eq!(reason, UnavailableReason::VolumeTrashRejected);
        }
        other => panic!("expected Unavailable(VolumeTrashRejected), got {other:?}"),
    }

    assert!(entry_names(&decoy.root).is_empty(), "probe no escribe nada");
    assert!(source.exists());
}

// ---------------------------------------------------------------------------
// cb_23 + cb_25: la consulta previa y la operacion tienen que coincidir.
// ---------------------------------------------------------------------------

/// `probe_trash` es lo que la UI mira para decidir si ofrecer «Enviar a la
/// papelera» o el aviso de borrado directo. Si dice que se puede con una ruta
/// que `trash_one` rechaza de plano, la UI ofrece una operación que va a
/// fallar.
///
/// Antes del arreglo: `probe_trash("/")` devolvía
/// `Ok(Available { kind: Home, .. })` mientras `trash_one("/")` devolvía
/// `Err(RefusedSpecialPath { reason: Root })`.
#[test]
fn sup_03_probe_rechaza_la_raiz_igual_que_trash_one() {
    let _env = TrashEnv::on_home_device();

    match probe_trash(Path::new("/"), &TrashPolicy::default()) {
        Err(TrashError::RefusedSpecialPath { path, reason }) => {
            assert_eq!(path, PathBuf::from("/"));
            assert_eq!(reason, RefusalReason::Root);
        }
        other => panic!("expected RefusedSpecialPath(Root), got {other:?}"),
    }
}

/// Misma exigencia con una ruta relativa: el `.trashinfo` necesita una ruta
/// absoluta, así que `trash_one` la rechaza; la consulta previa no puede
/// contestar otra cosa.
///
/// Antes del arreglo: `probe_trash` llegaba al `lstat` y contestaba
/// `Err(NotFound)` (o `Available`, si la ruta relativa existía), nunca
/// `RefusedSpecialPath { reason: Relative }`.
#[test]
fn sup_04_probe_rechaza_una_ruta_relativa_igual_que_trash_one() {
    let _env = TrashEnv::on_home_device();
    let relative = Path::new("sup04-relativa.txt");

    match probe_trash(relative, &TrashPolicy::default()) {
        Err(TrashError::RefusedSpecialPath { reason, .. }) => {
            assert_eq!(reason, RefusalReason::Relative);
        }
        other => panic!("expected RefusedSpecialPath(Relative), got {other:?}"),
    }
    match trash_one(relative, &TrashPolicy::default()) {
        Err(TrashError::RefusedSpecialPath { reason, .. }) => {
            assert_eq!(reason, RefusalReason::Relative, "y coincide con trash_one");
        }
        other => panic!("expected RefusedSpecialPath(Relative), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// cb_26: la mitad irreversible no puede tener menos guardas que la reversible.
// ---------------------------------------------------------------------------

/// `delete_permanently` no tenía ninguna guarda: sobre un punto de montaje
/// entraba a recorrerlo y a borrar su contenido entrada por entrada. `trash_one`
/// rechaza los puntos de montaje (cb_23) precisamente porque vaciarlos no es
/// reversible; en la operación que ya es irreversible la guarda hace más falta,
/// no menos.
///
/// Se prueba con `/proc`, que es un punto de montaje sobre el que `unlink(2)`
/// siempre devuelve `EPERM`: sin la guarda la llamada recorre el árbol y muere
/// con `PermissionDenied`, con la guarda ni siquiera lo abre. La prueba no se
/// ejecuta como root, donde ese colchón no valdría.
///
/// La guarda de `/` no se prueba a propósito: comprobar que una prueba falla
/// contra el código sin arreglar exigiría ejecutar `delete_permanently("/")`
/// sobre un árbol sin guarda, y eso empezaría a borrar el sistema.
#[test]
fn sup_05_delete_permanently_rechaza_un_punto_de_montaje() {
    let scratch = TempTree::on_home_device("sup05");
    if current_uid(&scratch.root) == 0 {
        return;
    }
    let proc_dir = Path::new("/proc");
    if !proc_dir.is_dir() {
        return;
    }

    match delete_permanently(proc_dir, &mut Silent) {
        Err(TrashError::RefusedSpecialPath { path, reason }) => {
            assert_eq!(path, proc_dir);
            assert_eq!(reason, RefusalReason::MountPoint);
        }
        other => panic!("expected RefusedSpecialPath(MountPoint), got {other:?}"),
    }

    assert!(proc_dir.join("self").exists(), "/proc sigue montado");
}
