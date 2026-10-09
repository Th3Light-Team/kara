# Known test failures

State of `cargo test -p kara-vfs -p kara-fs --no-fail-fast` on the cloud
session (root user, `/tmp` on the same device as the tree): **666 pass, 21 fail**.
None of the 21 is a regression of the remote-backend work; 19 fail identically on
the commit before it. Run on a normal developer machine (non-root user, `/tmp`
on tmpfs) the first two groups should pass.

Re-check with `cargo test -p kara-vfs -p kara-fs --no-fail-fast`. Plain `cargo`
needs the 1.98 toolchain (`rustup default 1.98`).

## 1. Needs a second device for `/tmp` (9)

`crates/kara-fs/tests/trash_volume.rs`, all stop in `require_two_devices` because
`/tmp` and the tree share a device. CLAUDE.md: «`trash_volume` needs /tmp and the
tree on different devices; CI mounts a tmpfs.»

`cb_05_en_disco_la_papelera_de_volumen_registra_el_path_relativo_al_topdir`,
`cb_10_probe_trash_anticipa_el_volumen_sin_papelera_sin_crear_nada`,
`cb_10_un_volumen_sin_papelera_avisa_y_jamas_borra_el_fichero`,
`cb_11_la_papelera_de_volumen_se_crea_cuando_la_politica_lo_permite`,
`cb_11_sin_permiso_de_creacion_se_avisa_y_no_se_crea_nada`,
`cb_12_un_topdir_trash_que_es_symlink_se_rechaza_y_no_se_escribe_a_traves`,
`cb_12_un_topdir_trash_sin_bit_sticky_se_rechaza`,
`cb_14_ida_y_vuelta_completa_tambien_en_la_papelera_de_volumen`,
`cb_24_por_defecto_no_se_copia_ni_un_byte_entre_volumenes`.

Fix: mount a tmpfs on `/tmp` (CI does).

## 2. Permission denial does not apply to root (10)

These lock a directory with `set_mode(…, 0o500)` (or equivalent) and expect the
operation to fail. Root ignores the mode bits, so the operation succeeds and the
assertion fails (`result.is_err()`, or «expected PermissionDenied»).

- `trash_home.rs`: `cb_09_si_falla_el_rename_no_queda_ningun_trashinfo_huerfano`,
  `cb_15_si_el_padre_no_se_puede_crear_la_entrada_sigue_en_la_papelera`,
  `cb_19_cancelar_desde_on_error_detiene_el_lote_sin_perder_nada`,
  `cb_19_omitir_un_fallo_no_aborta_el_resto_del_lote`,
  `cb_19_reintentar_repite_el_mismo_elemento_hasta_que_funciona`,
  `cb_19_skip_all_no_vuelve_a_preguntar_por_los_siguientes_fallos`,
  `cb_21_sin_permisos_el_error_nombra_el_fichero_y_conserva_el_errno`
- `trash_listing.rs`: `empty_trash_in_omitir_un_fallo_no_aborta_el_resto`,
  `info_sin_permiso_de_lectura_se_reporta_en_unreadable_roots`
- `listing.rs`: `an_unreadable_directory_is_a_top_level_error`

Fix: run as a non-root user. (The newer `kara-fs` tests probe with
`permissions_enforced` and skip instead of failing; these older ones do not.)

## 3. Obsolete guards (2) — decision pending

Both are hash-pinned test files, so they were not edited.

- `kara-vfs/tests/source_rules.rs` · `cb_48_no_other_crate_depends_on_kara_vfs_yet`
  — written for step 1, when nothing depended on `kara-vfs`. `kara-fs` now does
  (step 2: `LocalBackend`). Intended change: allow `kara-fs` (later also
  `kara-ops`, `kara-remote`).
- `kara-fs/tests/local_backend_rules.rs` · `cb_30_kara_vfs_and_kara_core_are_unchanged`
  — pins the hashes of `kara-vfs` sources as they were at the end of step 1.
  `kara-vfs` was fixed after two independent reviews. Intended change: re-pin to
  the current hashes, or keep the guard for `kara-core` only.

Either edit changes a protected test and its recorded sha256 in
`.kara/progress/*.json`; it needs the project owner's explicit go-ahead.

## 4. Decisions where the code follows a pinned test (0 failing)

These pass today, but a review flagged the pinned behaviour as a defect. Changing
them means editing a protected test.

| Test | Pins | Review concern |
|---|---|---|
| `local_backend_write.rs` · `cb_17_the_new_file_gets_the_default_creation_mode` | A replaced file gets the default mode (dec_07). | Replacing a 0600 file (e.g. an SSH key) yields 0644. Proposal: keep the old mode and owner on replace. |
| `local_backend_read.rs` · `cb_07_list_entries_are_exactly_what_describe_returns` | List entries equal `describe()`: a symlink's size is the link's, and `FileEntry.location` is the local parent path. | A progress total built from `stat().size` of a symlink to a large file is wrong; `with_root` leaks the real root through `location`. |

## Not testable in this environment

fsync durability, the `RENAME_NOREPLACE` fallback on NFS/FUSE and some rename
races: see `remote-backends-testing.md`.
