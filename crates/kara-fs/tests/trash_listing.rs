//! Pruebas de «Listar la papelera», base de «Restaurar desde la papelera» y
//! «Vaciar la papelera» (`ground/spec/05-operaciones.md`).
//!
//! `volume_top_dirs` is always injected as a `TempTree` on the home device
//! (never a real mount): `list_trash_in`/`empty_trash_in` never check the
//! device a top dir lives on, only that `$top/.Trash-$uid` exists, is a real
//! directory and is owned by this user, so an ordinary temp directory is
//! enough to exercise the volume-trash path deterministically and without
//! racing the real `/tmp`/`/dev/shm` other trash test binaries already claim
//! (see `trash_volume.rs`, `trash_supervisor.rs`). Only `list_trash`/
//! `empty_trash` — the wrappers that read real mounted volumes — are never
//! called with the bare (destructive) `empty_trash`: doing so in a test could
//! delete a real external volume's trash if the machine happened to have one
//! for this uid.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{Recorder, Silent, TempTree, TrashEnv, current_uid, ok, set_mode};
use kara_fs::trash::{
    ConflictPolicy, ErrorDecision, TrashEntry, TrashError, TrashKind, TrashPolicy, delete_trash_entry,
    empty_trash_in, list_trash, list_trash_in, restore_item, trash_one,
};

/// Creates `<root>/files` and `<root>/info`, empty, and returns their paths.
fn make_trash_dirs(root: &std::path::Path) -> (PathBuf, PathBuf) {
    let files = root.join("files");
    let info = root.join("info");
    ok(fs::create_dir_all(&files), "create trash files/");
    ok(fs::create_dir_all(&info), "create trash info/");
    (files, info)
}

/// Builds `<top>/.Trash-<uid>/{files,info}` by hand, exactly like a previous
/// `trash_one` call against that volume would have, without going through
/// `resolve_trash_dir` (which requires a real different device).
fn make_volume_trash(top_dir: &std::path::Path, uid: u32) -> (PathBuf, PathBuf) {
    make_trash_dirs(&top_dir.join(format!(".Trash-{uid}")))
}

/// Writes a `.trashinfo` by hand for a *volume* trash, where `Path=` is
/// recorded relative to `top_dir` (see `TrashInfo::serialize`'s `Some(top)`
/// branch) — unlike the home trash, an absolute `Path=` there is rejected as
/// malformed on read.
fn write_trashinfo(
    info_dir: &std::path::Path,
    top_dir: &std::path::Path,
    basename: &str,
    original_path: &std::path::Path,
) {
    let relative = match original_path.strip_prefix(top_dir) {
        Ok(relative) => relative,
        Err(_) => panic!(
            "test setup: {} is not under {}",
            original_path.display(),
            top_dir.display()
        ),
    };
    let content = format!(
        "[Trash Info]\nPath={}\nDeletionDate=2026-01-02T03:04:05\n",
        relative.display()
    );
    ok(
        fs::write(info_dir.join(format!("{basename}.trashinfo")), content),
        "write trashinfo",
    );
}

// ---------------------------------------------------------------------------
// Empty trash: no failure, no entries.
// ---------------------------------------------------------------------------

#[test]
fn una_papelera_nunca_usada_se_lista_vacia_sin_fallos() {
    let _env = TrashEnv::on_home_device();

    let listing = list_trash_in(&[]);

    assert!(listing.entries.is_empty(), "nada que listar todavia");
    assert!(listing.unreadable_records.is_empty());
    assert!(listing.unreadable_roots.is_empty());
}

// ---------------------------------------------------------------------------
// Paired entry: the happy path, and the tie-in with restore_item.
// ---------------------------------------------------------------------------

#[test]
fn una_entrada_emparejada_trae_lo_que_restore_item_necesita() {
    let _env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("paired");
    let source = tree.write("nota.txt", b"AAA");
    let trashed = match trash_one(&source, &TrashPolicy::default()) {
        Ok(item) => item,
        Err(error) => panic!("trash_one should succeed, got {error:?}"),
    };

    let listing = list_trash_in(&[]);

    assert_eq!(listing.entries.len(), 1, "un solo elemento en la papelera");
    let item = match &listing.entries[0] {
        TrashEntry::Item(item) => item.clone(),
        other => panic!("expected TrashEntry::Item, got {other:?}"),
    };
    assert_eq!(item.original_path, source);
    assert_eq!(item.trashed_path, trashed.trashed_path);
    assert_eq!(item.info_path, trashed.info_path);
    assert_eq!(item.deletion_date, trashed.deletion_date);
    assert_eq!(item.kind, TrashKind::Home);
    assert_eq!(item.top_dir, None);

    // La lista no es solo para enseñar: lo que devuelve entra directo en
    // restore_item, que es la razon de ser de "Restaurar desde la papelera".
    let restored = match restore_item(&item, ConflictPolicy::Fail) {
        Ok(path) => path,
        Err(error) => panic!("restore_item should succeed from a listed item, got {error:?}"),
    };
    assert_eq!(restored, source);
    assert_eq!(ok(fs::read(&source), "read restored"), b"AAA".to_vec());
}

// ---------------------------------------------------------------------------
// Unpaired: a .trashinfo without its files/ entry.
// ---------------------------------------------------------------------------

#[test]
fn un_trashinfo_sin_su_fichero_se_enseña_como_no_disponible_no_como_error() {
    let _env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("missing-file");
    let source = tree.write("x.txt", b"AAA");
    let trashed = match trash_one(&source, &TrashPolicy::default()) {
        Ok(item) => item,
        Err(error) => panic!("trash_one should succeed, got {error:?}"),
    };
    ok(fs::remove_file(&trashed.trashed_path), "delete the files/ entry by hand");

    let listing = list_trash_in(&[]);

    assert!(listing.unreadable_records.is_empty(), "no es un fallo de parseo");
    assert!(listing.unreadable_roots.is_empty());
    assert_eq!(listing.entries.len(), 1, "el .trashinfo huerfano sigue contando como una entrada");
    match &listing.entries[0] {
        TrashEntry::MissingFile {
            info_path,
            original_path,
            ..
        } => {
            assert_eq!(info_path, &trashed.info_path);
            assert_eq!(original_path, &source);
        }
        other => panic!("expected MissingFile, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Unpaired: a files/ entry without its .trashinfo.
// ---------------------------------------------------------------------------

#[test]
fn un_fichero_sin_su_trashinfo_se_enseña_como_no_disponible_y_no_rompe_el_resto() {
    let _env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("missing-info");
    let orphan_source = tree.write("huerfano.txt", b"AAA");
    let sane_source = tree.write("normal.txt", b"BBB");
    let orphan = match trash_one(&orphan_source, &TrashPolicy::default()) {
        Ok(item) => item,
        Err(error) => panic!("trash_one should succeed, got {error:?}"),
    };
    if let Err(error) = trash_one(&sane_source, &TrashPolicy::default()) {
        panic!("trash_one should succeed, got {error:?}");
    }
    ok(fs::remove_file(&orphan.info_path), "delete the .trashinfo by hand");

    let listing = list_trash_in(&[]);

    assert!(listing.unreadable_records.is_empty());
    assert!(listing.unreadable_roots.is_empty());
    assert_eq!(listing.entries.len(), 2, "el fichero sano no se pierde por culpa del huerfano");

    let missing_info = listing
        .entries
        .iter()
        .find(|entry| matches!(entry, TrashEntry::MissingInfo { .. }))
        .unwrap_or_else(|| panic!("expected one MissingInfo entry in {:?}", listing.entries));
    match missing_info {
        TrashEntry::MissingInfo { file_path, .. } => assert_eq!(file_path, &orphan.trashed_path),
        other => panic!("expected MissingInfo, got {other:?}"),
    }
    let paired = listing
        .entries
        .iter()
        .find(|entry| matches!(entry, TrashEntry::Item(_)))
        .unwrap_or_else(|| panic!("expected one paired entry in {:?}", listing.entries));
    match paired {
        TrashEntry::Item(item) => assert_eq!(item.original_path, sane_source),
        other => panic!("expected Item, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// An unreadable (unparsable) .trashinfo does not abort the listing, and its
// paired files/ entry is not double-reported as MissingInfo.
// ---------------------------------------------------------------------------

#[test]
fn un_trashinfo_ilegible_no_aborta_el_listado_ni_duplica_la_entrada() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("corrupt");
    let sane_source = tree.write("normal.txt", b"BBB");
    if let Err(error) = trash_one(&sane_source, &TrashPolicy::default()) {
        panic!("trash_one should succeed, got {error:?}");
    }
    // Un fichero en files/ con un .trashinfo que existe pero no parsea.
    let corrupt_file = env.files().join("roto.bin");
    ok(fs::write(&corrupt_file, b"CCC"), "write orphaned-looking file");
    ok(
        fs::write(env.info().join("roto.bin.trashinfo"), b"esto no es un trashinfo valido"),
        "write unparsable trashinfo",
    );

    let listing = list_trash_in(&[]);

    assert_eq!(listing.unreadable_records.len(), 1, "se reporta el fallo de parseo");
    assert_eq!(listing.unreadable_records[0].0, env.info().join("roto.bin.trashinfo"));
    assert!(listing.unreadable_roots.is_empty());
    assert_eq!(
        listing.entries.len(),
        1,
        "el roto no aparece como MissingInfo: ya tiene un .trashinfo, solo que ilegible"
    );
    match &listing.entries[0] {
        TrashEntry::Item(item) => assert_eq!(item.original_path, sane_source),
        other => panic!("expected the one sane Item, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// A root that cannot even be opened is reported, not swallowed.
// ---------------------------------------------------------------------------

#[test]
fn info_sin_permiso_de_lectura_se_reporta_en_unreadable_roots() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("locked-root");
    let source = tree.write("x.txt", b"AAA");
    if let Err(error) = trash_one(&source, &TrashPolicy::default()) {
        panic!("trash_one should succeed, got {error:?}");
    }
    set_mode(&env.info(), 0o300); // write+execute, no read: read_dir fails.

    let listing = list_trash_in(&[]);

    set_mode(&env.info(), 0o700);
    assert_eq!(listing.unreadable_roots.len(), 1);
    assert_eq!(listing.unreadable_roots[0].0, env.info());
    match &listing.unreadable_roots[0].1 {
        TrashError::PermissionDenied { .. } => {}
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Volume trashes: injected top dirs, and the symlink-hijack refusal.
// ---------------------------------------------------------------------------

#[test]
fn una_papelera_de_volumen_inyectada_se_lista_con_su_topdir() {
    let _env = TrashEnv::on_home_device();
    let volume = TempTree::on_home_device("volume-mount");
    let uid = current_uid(&volume.root);
    let (files, info) = make_volume_trash(&volume.root, uid);
    let original = volume.path("some/where/x.txt");
    ok(fs::write(files.join("x.txt"), b"AAA"), "write volume trash payload");
    write_trashinfo(&info, &volume.root, "x.txt", &original);

    let listing = list_trash_in(&[volume.root.clone()]);

    assert_eq!(listing.entries.len(), 1);
    match &listing.entries[0] {
        TrashEntry::Item(item) => {
            assert_eq!(item.kind, TrashKind::Volume);
            assert_eq!(item.top_dir, Some(volume.root.clone()));
            assert_eq!(item.original_path, original);
            assert_eq!(item.trashed_path, files.join("x.txt"));
        }
        other => panic!("expected Item, got {other:?}"),
    }
}

#[test]
fn un_topdir_de_volumen_sin_trash_no_es_un_fallo() {
    let _env = TrashEnv::on_home_device();
    let volume = TempTree::on_home_device("volume-unused");

    let listing = list_trash_in(&[volume.root.clone()]);

    assert!(listing.entries.is_empty());
    assert!(listing.unreadable_roots.is_empty(), "un volumen sin usar no es un error");
}

#[test]
fn un_volumen_cuya_papelera_es_un_enlace_simbolico_se_ignora_no_se_sigue() {
    let _env = TrashEnv::on_home_device();
    let volume = TempTree::on_home_device("volume-hijack");
    let uid = current_uid(&volume.root);
    let attacker_dir = TempTree::on_home_device("attacker");
    // Un `files/`+`info/` completo detras del enlace: si la proteccion contra
    // el secuestro fallase, esta entrada apareceria en el listado. Con solo
    // un fichero suelto (sin `files/`/`info/`) la prueba pasaria igual sin
    // que la proteccion hiciera nada, que es justo el fallo que hay que
    // descartar (una entrada rota "se enseña como no disponible", no un
    // volumen vacio que ya lo estaba de por si).
    let (attacker_files, attacker_info) = make_trash_dirs(&attacker_dir.root);
    let attacker_secret = attacker_files.join("secreto.txt");
    ok(fs::write(&attacker_secret, b"NO TOCAR"), "plant attacker payload");
    write_trashinfo(&attacker_info, &volume.root, "secreto.txt", &volume.path("cosa.txt"));
    let hijacked_root = volume.root.join(format!(".Trash-{uid}"));
    ok(
        std::os::unix::fs::symlink(&attacker_dir.root, &hijacked_root),
        "plant a symlinked .Trash-<uid>",
    );

    let listing = list_trash_in(&[volume.root.clone()]);

    assert!(
        listing.entries.is_empty(),
        "una papelera de volumen enlazada se trata como inexistente, no se lee: {:?}",
        listing.entries
    );
    assert!(listing.unreadable_roots.is_empty());
    assert!(attacker_secret.exists(), "el directorio ajeno ni se toca");

    // Y tampoco se borra al vaciar: el mismo cuidado que trash_one aplica al
    // escribir se aplica aqui al borrar.
    let mut observer = Silent;
    let outcome = empty_trash_in(&[volume.root.clone()], &mut observer);
    assert!(outcome.removed.is_empty());
    assert!(outcome.removed_orphans.is_empty());
    assert!(attacker_secret.exists(), "vaciar tampoco sigue el enlace");
    assert_eq!(ok(fs::read(&attacker_secret), "read secret"), b"NO TOCAR".to_vec());
}

// ---------------------------------------------------------------------------
// delete_trash_entry: removing one entry without touching the rest.
// ---------------------------------------------------------------------------

#[test]
fn delete_trash_entry_borra_solo_el_elemento_pedido() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("delete-one");
    let keep_source = tree.write("queda.txt", b"1");
    let drop_source = tree.write("se-va.txt", b"2");
    if let Err(error) = trash_one(&keep_source, &TrashPolicy::default()) {
        panic!("trash_one should succeed, got {error:?}");
    }
    if let Err(error) = trash_one(&drop_source, &TrashPolicy::default()) {
        panic!("trash_one should succeed, got {error:?}");
    }
    let listing = list_trash_in(&[]);
    assert_eq!(listing.entries.len(), 2);
    let to_drop = listing
        .entries
        .iter()
        .find(|entry| match entry {
            TrashEntry::Item(item) => item.original_path == drop_source,
            _ => false,
        })
        .unwrap_or_else(|| panic!("expected to find the drop entry"));
    let mut observer = Silent;

    if let Err(error) = delete_trash_entry(to_drop, &mut observer) {
        panic!("delete_trash_entry should succeed, got {error:?}");
    }

    let after = list_trash_in(&[]);
    assert_eq!(after.entries.len(), 1, "solo desaparece el elegido");
    match &after.entries[0] {
        TrashEntry::Item(item) => assert_eq!(item.original_path, keep_source),
        other => panic!("expected the kept item, got {other:?}"),
    }
    assert!(entry_display_gone(to_drop), "el fichero y su registro desaparecen");
    assert_eq!(entry_names_in(&env.files()).len(), 1);
    assert_eq!(entry_names_in(&env.info()).len(), 1);
}

fn entry_display_gone(entry: &TrashEntry) -> bool {
    std::fs::symlink_metadata(entry.display_path()).is_err()
}

fn entry_names_in(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// empty_trash_in: batch semantics matching trash_batch's.
// ---------------------------------------------------------------------------

#[test]
fn empty_trash_in_borra_los_emparejados_y_los_huerfanos_de_ambos_lados() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("empty-all");
    let paired_source = tree.write("normal.txt", b"1");
    let missing_file_source = tree.write("perdido.txt", b"2");
    let paired = match trash_one(&paired_source, &TrashPolicy::default()) {
        Ok(item) => item,
        Err(error) => panic!("trash_one should succeed, got {error:?}"),
    };
    let missing_file = match trash_one(&missing_file_source, &TrashPolicy::default()) {
        Ok(item) => item,
        Err(error) => panic!("trash_one should succeed, got {error:?}"),
    };
    ok(fs::remove_file(&missing_file.trashed_path), "orphan the .trashinfo");
    let orphan_file = env.files().join("sin-info.bin");
    ok(fs::write(&orphan_file, b"3"), "orphan file with no .trashinfo");
    let mut observer = Recorder::new();

    let outcome = empty_trash_in(&[], &mut observer);

    assert_eq!(outcome.removed.len(), 1);
    assert_eq!(outcome.removed[0].original_path, paired_source);
    assert_eq!(outcome.removed_orphans.len(), 2, "el .trashinfo huerfano y el fichero huerfano");
    assert!(outcome.failed.is_empty());
    assert!(!outcome.cancelled);
    assert!(!paired.trashed_path.exists());
    assert!(!paired.info_path.exists());
    assert!(!missing_file.info_path.exists(), "el .trashinfo huerfano tambien se limpia");
    assert!(!orphan_file.exists());
    assert!(entry_names_in(&env.files()).is_empty());
    assert!(entry_names_in(&env.info()).is_empty());
    assert!(observer.done.contains(&paired_source));
}

#[test]
fn empty_trash_in_omitir_un_fallo_no_aborta_el_resto() {
    let env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("empty-skip");
    let first_source = tree.write("uno.txt", b"1");
    let second_source = tree.write("dos.txt", b"2");
    if let Err(error) = trash_one(&first_source, &TrashPolicy::default()) {
        panic!("trash_one should succeed, got {error:?}");
    }
    if let Err(error) = trash_one(&second_source, &TrashPolicy::default()) {
        panic!("trash_one should succeed, got {error:?}");
    }
    set_mode(&env.files(), 0o500); // files/ itself loses write: unlink fails inside it.
    let mut observer = Recorder::deciding(ErrorDecision::Skip);

    let outcome = empty_trash_in(&[], &mut observer);

    set_mode(&env.files(), 0o700);
    assert_eq!(outcome.failed.len(), 2, "ambos ficheros viven en el files/ bloqueado");
    assert!(outcome.removed.is_empty());
    assert!(!outcome.cancelled);
}

#[test]
fn empty_trash_in_es_cancelable_a_mitad() {
    let _env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("empty-cancel");
    for name in ["uno.txt", "dos.txt", "tres.txt"] {
        let source = tree.write(name, b"x");
        if let Err(error) = trash_one(&source, &TrashPolicy::default()) {
            panic!("trash_one should succeed, got {error:?}");
        }
    }
    let mut observer = Recorder::cancelling_at(1);

    let outcome = empty_trash_in(&[], &mut observer);

    assert!(outcome.cancelled);
    assert_eq!(outcome.removed.len(), 1, "solo el primero se llego a procesar");
    assert!(outcome.failed.is_empty(), "cancelar no es omitir");
    assert_eq!(observer.starts.len(), 2, "no se consulta mas alla del cancelado");
}

// ---------------------------------------------------------------------------
// list_trash: the real-mounts wrapper still finds the personal trash.
// ---------------------------------------------------------------------------

#[test]
fn list_trash_encuentra_lo_de_la_papelera_personal_sin_pedir_topdirs() {
    let _env = TrashEnv::on_home_device();
    let tree = TempTree::on_home_device("wrapper");
    let source = tree.write("x.txt", b"AAA");
    if let Err(error) = trash_one(&source, &TrashPolicy::default()) {
        panic!("trash_one should succeed, got {error:?}");
    }

    let listing = list_trash();

    assert!(
        listing.entries.iter().any(|entry| matches!(
            entry,
            TrashEntry::Item(item) if item.original_path == source
        )),
        "el elemento recien enviado aparece via el wrapper publico: {:?}",
        listing.entries
    );
}
