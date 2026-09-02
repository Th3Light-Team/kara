//! Criterio de aceptación de la conveniencia «Ordenar por» y sus tres hermanas
//! (sentido asc/desc, carpetas primero, clic en cabecera).
//!
//! Un test por cada caso borde del contrato, numerado `case_NN_...`.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kara_core::{
    Collation, ColumnId, DirectoryGrouping, EntryKind, FileEntry, MetadataBag, MetadataKey,
    MetadataValue, SortError, SortKey, SortOrder, SortSpec, available_keys, collation_key,
    compare_entries, compare_names, insertion_index, invert_permutation, remap_selection,
    sort_entries, sort_key_for_column, sort_permutation,
};

// ---------------------------------------------------------------- helpers ---

fn base(name: &str, kind: EntryKind) -> FileEntry {
    FileEntry {
        name: OsString::from(name),
        display: name.to_string(),
        kind,
        is_symlink: false,
        symlink_broken: false,
        is_hidden: name.starts_with('.'),
        size: None,
        modified: None,
        created: None,
        accessed: None,
        type_label: None,
        location: None,
        extra: MetadataBag::new(),
    }
}

fn file(name: &str) -> FileEntry {
    base(name, EntryKind::File)
}

fn dir(name: &str) -> FileEntry {
    base(name, EntryKind::Directory)
}

fn raw_file(bytes: &[u8]) -> FileEntry {
    let name = OsString::from_vec(bytes.to_vec());
    let display = name.to_string_lossy().into_owned();
    FileEntry {
        name,
        display,
        kind: EntryKind::File,
        is_symlink: false,
        symlink_broken: false,
        is_hidden: false,
        size: None,
        modified: None,
        created: None,
        accessed: None,
        type_label: None,
        location: None,
        extra: MetadataBag::new(),
    }
}

fn sized(name: &str, size: u64) -> FileEntry {
    let mut e = file(name);
    e.size = Some(size);
    e
}

fn sized_dir(name: &str, size: u64) -> FileEntry {
    let mut e = dir(name);
    e.size = Some(size);
    e
}

/// Instante relativo a la época UNIX; admite valores negativos (ext4 los permite).
fn at(secs: i64) -> SystemTime {
    if secs >= 0 {
        UNIX_EPOCH + Duration::from_secs(secs.unsigned_abs())
    } else {
        UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
    }
}

fn timed(name: &str, secs: i64) -> FileEntry {
    let mut e = file(name);
    e.modified = Some(at(secs));
    e
}

fn timed_dir(name: &str, secs: i64) -> FileEntry {
    let mut e = dir(name);
    e.modified = Some(at(secs));
    e
}

fn spec(key: SortKey, order: SortOrder, grouping: DirectoryGrouping) -> SortSpec {
    SortSpec {
        key,
        order,
        grouping,
        collation: Collation::default(),
    }
}

fn names(entries: &[FileEntry]) -> Vec<String> {
    entries.iter().map(|e| e.display.clone()).collect()
}

fn sorted_names(entries: &[FileEntry], spec: &SortSpec) -> Vec<String> {
    let mut v = entries.to_vec();
    sort_entries(&mut v, spec);
    names(&v)
}

fn name_asc(list: &[&str]) -> Vec<String> {
    let entries: Vec<FileEntry> = list.iter().map(|n| file(n)).collect();
    sorted_names(
        &entries,
        &spec(SortKey::Name, SortOrder::Ascending, DirectoryGrouping::Mixed),
    )
}

const INTRINSIC_KEYS: [SortKey; 9] = [
    SortKey::Name,
    SortKey::Extension,
    SortKey::Size,
    SortKey::Modified,
    SortKey::Created,
    SortKey::Accessed,
    SortKey::Kind,
    SortKey::Location,
    SortKey::Unsorted,
];

// ------------------------------------------------------------------ tests ---

/// Caso 1: dos entradas que comparan `Equal` conservan el orden de entrada, en
/// ascendente y en descendente.
#[test]
fn case_01_equal_entries_keep_input_order_in_both_directions() {
    let mut first = file("dup.txt");
    first.location = Some(PathBuf::from("/uno"));
    let mut second = file("dup.txt");
    second.location = Some(PathBuf::from("/dos"));

    let s = spec(SortKey::Name, SortOrder::Ascending, DirectoryGrouping::Mixed);
    assert_eq!(
        compare_entries(&first, &second, &s),
        core::cmp::Ordering::Equal,
        "mismo basename crudo debe comparar Equal"
    );

    let input = vec![first.clone(), file("aaa.txt"), second.clone()];

    let mut asc = input.clone();
    sort_entries(&mut asc, &s);
    assert_eq!(names(&asc), vec!["aaa.txt", "dup.txt", "dup.txt"]);
    assert_eq!(asc[1].location, Some(PathBuf::from("/uno")));
    assert_eq!(asc[2].location, Some(PathBuf::from("/dos")));

    let mut desc = input;
    sort_entries(
        &mut desc,
        &spec(SortKey::Name, SortOrder::Descending, DirectoryGrouping::Mixed),
    );
    assert_eq!(names(&desc), vec!["dup.txt", "dup.txt", "aaa.txt"]);
    assert_eq!(
        desc[0].location,
        Some(PathBuf::from("/uno")),
        "la estabilidad debe conservarse tambien en descendente"
    );
    assert_eq!(desc[1].location, Some(PathBuf::from("/dos")));
}

/// Caso 2: empate en el criterio principal (tres ficheros de 4096 bytes) → desempate
/// por nombre, nunca por el orden de llegada del scandir.
#[test]
fn case_02_equal_primary_values_are_broken_by_name() {
    let s = spec(SortKey::Size, SortOrder::Ascending, DirectoryGrouping::Mixed);

    let order_a = vec![
        sized("gamma", 4096),
        sized("alpha", 4096),
        sized("tiny", 10),
        sized("beta", 4096),
    ];
    let order_b = vec![
        sized("beta", 4096),
        sized("tiny", 10),
        sized("alpha", 4096),
        sized("gamma", 4096),
    ];

    let expected = vec!["tiny", "alpha", "beta", "gamma"];
    assert_eq!(sorted_names(&order_a, &s), expected);
    assert_eq!(
        sorted_names(&order_b, &s),
        expected,
        "el resultado no puede depender del orden de scandir"
    );
}

/// Caso 3: `sort(desc)` es exactamente `reverse(sort(asc))` sin empates totales, y el
/// desempate por nombre también se invierte.
#[test]
fn case_03_descending_is_reverse_of_ascending_and_tiebreak_follows_order() {
    let entries = vec![
        sized("delta", 40),
        sized("alpha", 10),
        sized("charlie", 30),
        sized("bravo", 20),
    ];
    let asc = sorted_names(
        &entries,
        &spec(SortKey::Size, SortOrder::Ascending, DirectoryGrouping::Mixed),
    );
    let desc = sorted_names(
        &entries,
        &spec(SortKey::Size, SortOrder::Descending, DirectoryGrouping::Mixed),
    );
    let mut reversed = asc.clone();
    reversed.reverse();
    assert_eq!(asc, vec!["alpha", "bravo", "charlie", "delta"]);
    assert_eq!(desc, reversed);

    // El desempate por nombre lleva el sentido aplicado.
    let tied = vec![sized("a", 10), sized("b", 10), sized("c", 5)];
    assert_eq!(
        sorted_names(
            &tied,
            &spec(SortKey::Size, SortOrder::Descending, DirectoryGrouping::Mixed)
        ),
        vec!["b", "a", "c"]
    );
}

/// Caso 4: orden natural/numérico — `archivo2` antes que `archivo10`.
#[test]
fn case_04_natural_numeric_order_beats_lexicographic() {
    assert_eq!(
        name_asc(&["archivo10", "archivo2", "archivo1"]),
        vec!["archivo1", "archivo2", "archivo10"]
    );

    let plain = Collation {
        case_sensitive: false,
        natural_numeric: false,
    };
    assert_eq!(
        compare_names("archivo10", "archivo2", &plain),
        core::cmp::Ordering::Less,
        "sin natural_numeric la comparacion vuelve a ser lexicografica"
    );
    assert_eq!(
        compare_names("archivo10", "archivo2", &Collation::default()),
        core::cmp::Ordering::Greater
    );
}

/// Caso 5: tramo numérico mayor que `u64` — ni desbordamiento ni pánico.
#[test]
fn case_05_numeric_run_larger_than_u64_does_not_overflow() {
    assert_eq!(
        name_asc(&[
            "f99999999999999999999999.txt",
            "f100.txt",
            "f9.txt",
            "f18446744073709551616.txt",
        ]),
        vec![
            "f9.txt",
            "f100.txt",
            "f18446744073709551616.txt",
            "f99999999999999999999999.txt",
        ]
    );
}

/// Caso 6: ceros a la izquierda con el mismo valor — menos ceros va primero, y nunca
/// comparan `Equal`.
#[test]
fn case_06_leading_zeros_tiebreak_is_deterministic() {
    assert_eq!(name_asc(&["a01", "a1"]), vec!["a1", "a01"]);
    assert_eq!(
        name_asc(&["b0010", "b010", "b10"]),
        vec!["b10", "b010", "b0010"]
    );
    assert_ne!(
        compare_names("a1", "a01", &Collation::default()),
        core::cmp::Ordering::Equal
    );
}

/// Caso 7: insensibilidad a mayúsculas por defecto, y comportamiento con
/// `case_sensitive = true`.
#[test]
fn case_07_case_insensitive_by_default_and_sensitive_on_demand() {
    assert_eq!(name_asc(&["b.txt", "A.txt"]), vec!["A.txt", "b.txt"]);
    assert_eq!(name_asc(&["B.txt", "a.txt"]), vec!["a.txt", "B.txt"]);

    let cs = Collation {
        case_sensitive: true,
        natural_numeric: true,
    };
    let cs_spec = SortSpec {
        key: SortKey::Name,
        order: SortOrder::Ascending,
        grouping: DirectoryGrouping::Mixed,
        collation: cs,
    };
    assert_eq!(
        sorted_names(&[file("b.txt"), file("A.txt")], &cs_spec),
        vec!["A.txt", "b.txt"]
    );
    assert_eq!(
        sorted_names(&[file("a.txt"), file("B.txt")], &cs_spec),
        vec!["B.txt", "a.txt"],
        "con case_sensitive se ordena por punto de codigo"
    );
}

/// Caso 8: `README` y `readme` conviven — empatan tras el plegado y desempatan por
/// punto de código.
#[test]
fn case_08_case_only_difference_never_compares_equal() {
    assert_eq!(name_asc(&["readme", "README"]), vec!["README", "readme"]);
    assert_eq!(
        compare_names("README", "readme", &Collation::default()),
        core::cmp::Ordering::Less
    );
    assert_ne!(
        compare_names("README", "readme", &Collation::default()),
        core::cmp::Ordering::Equal
    );
}

/// Caso 9: independencia del locale — nada de `strcoll`.
#[test]
fn case_09_collation_is_independent_of_locale() {
    // glibc con es_ES.UTF-8 ignora la puntuacion en el primer nivel: ordenaria
    // ["ab", "a-b", "ax", "_x"] de otro modo. Kara compara por punto de codigo.
    let sample = ["ab", "a-b", "ax", "_x"];
    let expected = vec!["_x", "a-b", "ab", "ax"];
    assert_eq!(name_asc(&sample), expected);

    let previous = std::env::var("LC_ALL").ok();
    for locale in ["C", "es_ES.UTF-8", "en_US.UTF-8"] {
        // SAFETY: test de un solo hilo logico; se restaura al final.
        unsafe { std::env::set_var("LC_ALL", locale) };
        assert_eq!(
            name_asc(&sample),
            expected,
            "la salida cambio con LC_ALL={locale}"
        );
    }
    match previous {
        Some(v) => unsafe { std::env::set_var("LC_ALL", v) },
        None => unsafe { std::env::remove_var("LC_ALL") },
    }
}

/// Caso 10: NFC y NFD del mismo nombre caen juntos; los acentos no se pliegan.
#[test]
fn case_10_nfc_and_nfd_normalize_together_without_accent_folding() {
    let nfc = "acci\u{f3}n";
    let nfd = "accio\u{301}n";
    assert_eq!(
        compare_names(nfc, nfd, &Collation::default()),
        core::cmp::Ordering::Equal,
        "NFC y NFD del mismo nombre deben comparar Equal"
    );
    assert_eq!(
        compare_names("accion", nfc, &Collation::default()),
        core::cmp::Ordering::Less,
        "no debe haber plegado de acentos: accion y accion acentuado son distintos"
    );

    let entries = vec![file(nfd), file("acceso"), file(nfc), file("adios")];
    let ordered = sorted_names(
        &entries,
        &spec(SortKey::Name, SortOrder::Ascending, DirectoryGrouping::Mixed),
    );
    assert_eq!(ordered[0], "acceso");
    assert_eq!(ordered[3], "adios");
    let middle: BTreeSet<&str> = [ordered[1].as_str(), ordered[2].as_str()].into_iter().collect();
    let expected: BTreeSet<&str> = [nfc, nfd].into_iter().collect();
    assert_eq!(middle, expected, "las dos formas deben quedar adyacentes");
}

/// Caso 11: nombres POSIX no UTF-8 cuya conversión lossy colisiona — nunca `Equal`,
/// y el orden es total y reproducible.
#[test]
fn case_11_non_utf8_names_are_totally_ordered_by_raw_bytes() {
    let a = raw_file(b"bad\xFF");
    let b = raw_file(b"bad\xFE");
    assert_eq!(a.display, b.display, "la conversion lossy debe colisionar");

    let s = spec(SortKey::Name, SortOrder::Ascending, DirectoryGrouping::Mixed);
    assert_ne!(
        compare_entries(&a, &b, &s),
        core::cmp::Ordering::Equal,
        "dos ficheros distintos nunca pueden comparar Equal"
    );

    let mut one = vec![a.clone(), b.clone()];
    let mut other = vec![b.clone(), a.clone()];
    sort_entries(&mut one, &s);
    sort_entries(&mut other, &s);
    assert_eq!(one, other, "el orden debe ser reproducible");
    assert_eq!(
        one[0].name,
        b.name,
        "el desempate final por name crudo es ascendente"
    );
}

/// Caso 12: carpetas primero al ordenar por tamaño y por fecha; cada grupo ordenado
/// por ese mismo criterio.
#[test]
fn case_12_directories_stay_grouped_when_sorting_by_size_or_date() {
    let entries = vec![
        sized_dir("z-dir", 5),
        sized("b-file", 1),
        sized_dir("a-dir", 100),
        sized("c-file", 50),
    ];
    assert_eq!(
        sorted_names(
            &entries,
            &spec(SortKey::Size, SortOrder::Ascending, DirectoryGrouping::First)
        ),
        vec!["z-dir", "a-dir", "b-file", "c-file"]
    );

    let dated = vec![
        timed_dir("z-dir", 500),
        timed("b-file", 100),
        timed_dir("a-dir", 900),
        timed("c-file", 800),
    ];
    assert_eq!(
        sorted_names(
            &dated,
            &spec(
                SortKey::Modified,
                SortOrder::Ascending,
                DirectoryGrouping::First
            )
        ),
        vec!["z-dir", "a-dir", "b-file", "c-file"]
    );
}

/// Caso 13: en descendente, las carpetas siguen arriba; solo se invierte el orden
/// dentro de cada grupo.
#[test]
fn case_13_descending_does_not_flip_the_directory_group() {
    let entries = vec![
        sized_dir("z-dir", 5),
        sized("b-file", 1),
        sized_dir("a-dir", 100),
        sized("c-file", 50),
    ];
    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Size,
                SortOrder::Descending,
                DirectoryGrouping::First
            )
        ),
        vec!["a-dir", "z-dir", "c-file", "b-file"]
    );
}

/// Caso 14: con el toggle desactivado y fecha descendente, el archivo más reciente
/// queda por delante de las carpetas.
#[test]
fn case_14_mixed_grouping_lets_a_recent_file_win() {
    let entries = vec![
        timed_dir("old-dir", 100),
        timed_dir("mid-dir", 200),
        timed("newest.txt", 300),
    ];
    let out = sorted_names(
        &entries,
        &spec(
            SortKey::Modified,
            SortOrder::Descending,
            DirectoryGrouping::Mixed,
        ),
    );
    assert_eq!(out, vec!["newest.txt", "mid-dir", "old-dir"]);
}

/// Caso 15: `DirectoryGrouping::Last` es el simétrico de `First`.
#[test]
fn case_15_directories_last_is_symmetric() {
    let entries = vec![
        sized_dir("z-dir", 5),
        sized("b-file", 1),
        sized_dir("a-dir", 100),
        sized("c-file", 50),
    ];
    assert_eq!(
        sorted_names(
            &entries,
            &spec(SortKey::Size, SortOrder::Ascending, DirectoryGrouping::Last)
        ),
        vec!["b-file", "c-file", "z-dir", "a-dir"]
    );
}

/// Caso 16: tamaño aún sin calcular (`None`) — siempre al final, en ambos sentidos.
#[test]
fn case_16_missing_size_always_sinks_to_the_bottom() {
    let entries = vec![
        file("a"),
        sized("b", 10),
        sized("c", 5),
        file("d"),
    ];
    assert_eq!(
        sorted_names(
            &entries,
            &spec(SortKey::Size, SortOrder::Ascending, DirectoryGrouping::Mixed)
        ),
        vec!["c", "b", "a", "d"]
    );
    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Size,
                SortOrder::Descending,
                DirectoryGrouping::Mixed
            )
        ),
        vec!["b", "c", "d", "a"],
        "los ausentes no pueden subir a lo alto al invertir el sentido"
    );
}

/// Caso 17: llega el tamaño calculado de una carpeta — se recoloca por búsqueda
/// binaria sin reordenar el listado entero.
#[test]
fn case_17_insertion_index_relocates_without_full_resort() {
    let s = spec(SortKey::Size, SortOrder::Ascending, DirectoryGrouping::Mixed);

    let mut small = vec![sized("a", 1), sized("b", 5), sized("c", 10), sized("d", 50)];
    let newcomer = sized("nuevo", 7);
    let idx = insertion_index(&small, &newcomer, &s);
    assert_eq!(idx, 2);
    small.insert(idx, newcomer);
    assert_eq!(names(&small), vec!["a", "b", "nuevo", "c", "d"]);

    let mut big: Vec<FileEntry> = (0..1000)
        .map(|i| sized(&format!("f{i:05}"), (i as u64) * 2))
        .collect();
    sort_entries(&mut big, &s);
    let entry = sized("recalculada", 999);
    let idx = insertion_index(&big, &entry, &s);
    assert_eq!(idx, 500);
    big.insert(idx, entry);
    let mut resorted = big.clone();
    sort_entries(&mut resorted, &s);
    assert_eq!(
        big, resorted,
        "insertar por insertion_index equivale a reordenar todo"
    );
}

/// Caso 18: fecha de creación no expuesta por el sistema de ficheros — el criterio
/// degenera limpiamente al desempate por nombre.
#[test]
fn case_18_created_all_none_degenerates_to_name() {
    let entries = vec![file("charlie"), file("alpha"), file("bravo")];
    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Created,
                SortOrder::Ascending,
                DirectoryGrouping::Mixed
            )
        ),
        vec!["alpha", "bravo", "charlie"]
    );
    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Created,
                SortOrder::Descending,
                DirectoryGrouping::Mixed
            )
        ),
        vec!["charlie", "bravo", "alpha"]
    );
}

/// Caso 19: instantes anteriores a `UNIX_EPOCH` — ni pánico ni orden invertido.
#[test]
fn case_19_timestamps_before_unix_epoch() {
    let entries = vec![timed("b", -1000), timed("a", 0), timed("c", 1000)];
    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Modified,
                SortOrder::Ascending,
                DirectoryGrouping::Mixed
            )
        ),
        vec!["b", "a", "c"]
    );
    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Modified,
                SortOrder::Descending,
                DirectoryGrouping::Mixed
            )
        ),
        vec!["c", "a", "b"]
    );
}

/// Caso 20: descendente por fecha deja lo más reciente en la posición 0.
#[test]
fn case_20_newest_first_when_sorting_by_date_descending() {
    let entries = vec![
        timed("viejo", 10),
        timed("reciente", 9_000_000),
        timed("medio", 5_000),
    ];
    let out = sorted_names(
        &entries,
        &spec(
            SortKey::Modified,
            SortOrder::Descending,
            DirectoryGrouping::Mixed,
        ),
    );
    assert_eq!(out[0], "reciente");
    assert_eq!(out, vec!["reciente", "medio", "viejo"]);
}

/// Caso 21: definición de extensión — sin extensión, punto final, dotfile y doble
/// extensión.
#[test]
fn case_21_extension_is_the_segment_after_the_last_non_initial_dot() {
    let entries = vec![
        file("Makefile"),
        file("file."),
        file(".bashrc"),
        file("factura.pdf.exe"),
        file("a.tar.gz"),
    ];
    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Extension,
                SortOrder::Ascending,
                DirectoryGrouping::Mixed
            )
        ),
        vec![
            ".bashrc",
            "file.",
            "Makefile",
            "factura.pdf.exe",
            "a.tar.gz"
        ],
        "la extension vacia es un valor, agrupa primero y desempata por nombre"
    );
}

/// Caso 22: extensiones con distinta caja agrupan juntas.
#[test]
fn case_22_extension_comparison_is_case_insensitive() {
    let entries = vec![file("a.TXT"), file("b.png"), file("c.txt")];
    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Extension,
                SortOrder::Ascending,
                DirectoryGrouping::Mixed
            )
        ),
        vec!["b.png", "a.TXT", "c.txt"]
    );
}

/// Caso 23: los ocultos no se agrupan aparte; `is_hidden` no participa en el orden.
#[test]
fn case_23_hidden_files_are_not_grouped_apart() {
    let entries = vec![file("apple"), file(".zshrc"), file("banana")];
    assert!(entries[1].is_hidden);
    assert_eq!(
        sorted_names(
            &entries,
            &spec(SortKey::Name, SortOrder::Ascending, DirectoryGrouping::Mixed)
        ),
        vec![".zshrc", "apple", "banana"]
    );
    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Name,
                SortOrder::Descending,
                DirectoryGrouping::Mixed
            )
        ),
        vec!["banana", "apple", ".zshrc"]
    );
}

/// Caso 24: se ordena siempre por el nombre real completo, extensión incluida;
/// ocultar extensiones no reordena la vista.
#[test]
fn case_24_name_sorting_always_uses_the_full_real_name() {
    let entries = vec![file("doc.zzz"), file("doc.aaa"), file("doc")];
    assert_eq!(
        sorted_names(
            &entries,
            &spec(SortKey::Name, SortOrder::Ascending, DirectoryGrouping::Mixed)
        ),
        vec!["doc", "doc.aaa", "doc.zzz"],
        "ordenar por el nombre base ignorando la extension seria incorrecto"
    );
}

/// Caso 25: enlaces simbólicos — `kara-fs` decide el `EntryKind` y la agrupación lo
/// respeta.
#[test]
fn case_25_symlinks_group_by_resolved_kind() {
    let mut link_dir = dir("bb-link-dir");
    link_dir.is_symlink = true;
    let mut broken = file("aa-broken");
    broken.is_symlink = true;
    broken.symlink_broken = true;

    let entries = vec![
        file("zz-real-file"),
        broken,
        dir("yy-real-dir"),
        link_dir,
    ];
    assert_eq!(
        sorted_names(
            &entries,
            &spec(SortKey::Name, SortOrder::Ascending, DirectoryGrouping::First)
        ),
        vec!["bb-link-dir", "yy-real-dir", "aa-broken", "zz-real-file"]
    );
}

/// Caso 26: clic en una columna que no es el criterio activo → fija criterio y
/// resetea a ascendente.
#[test]
fn case_26_header_click_on_another_column_resets_to_ascending() {
    let current = SortSpec {
        key: SortKey::Name,
        order: SortOrder::Descending,
        grouping: DirectoryGrouping::Last,
        collation: Collation {
            case_sensitive: true,
            natural_numeric: false,
        },
    };
    let next = current
        .on_header_click(&ColumnId(Cow::Borrowed("size")))
        .expect("la columna size es ordenable");
    assert_eq!(next.key, SortKey::Size);
    assert_eq!(next.order, SortOrder::Ascending);
    assert_eq!(next.grouping, DirectoryGrouping::Last);
    assert_eq!(next.collation, current.collation);
    assert_eq!(current.order, SortOrder::Descending, "no debe mutar");
}

/// Caso 27: primer, segundo y tercer clic sobre la misma cabecera →
/// ascendente, descendente, ascendente, sin cambiar de criterio.
#[test]
fn case_27_repeated_header_clicks_alternate_the_order() {
    let column = ColumnId(Cow::Borrowed("size"));
    let first = SortSpec::default()
        .on_header_click(&column)
        .expect("size ordenable");
    assert_eq!((first.key.clone(), first.order), (SortKey::Size, SortOrder::Ascending));

    let second = first.on_header_click(&column).expect("size ordenable");
    assert_eq!((second.key.clone(), second.order), (SortKey::Size, SortOrder::Descending));

    let third = second.on_header_click(&column).expect("size ordenable");
    assert_eq!((third.key.clone(), third.order), (SortKey::Size, SortOrder::Ascending));
}

/// Caso 28: columna añadida por el usuario, columna no ordenable y columna
/// desconocida.
#[test]
fn case_28_header_click_on_custom_and_unknown_columns() {
    assert_eq!(
        sort_key_for_column(&ColumnId(Cow::Borrowed("duration"))),
        Ok(SortKey::Metadata(MetadataKey::Duration))
    );
    assert_eq!(
        sort_key_for_column(&ColumnId(Cow::Borrowed("meta/mi-columna"))),
        Ok(SortKey::Metadata(MetadataKey::Custom(Cow::Borrowed(
            "mi-columna"
        ))))
    );
    assert_eq!(
        sort_key_for_column(&ColumnId(Cow::Borrowed("name"))),
        Ok(SortKey::Name)
    );
    assert_eq!(
        sort_key_for_column(&ColumnId(Cow::Borrowed("kind"))),
        Ok(SortKey::Kind)
    );
    assert_eq!(
        sort_key_for_column(&ColumnId(Cow::Borrowed("location"))),
        Ok(SortKey::Location)
    );
    assert_eq!(
        sort_key_for_column(&ColumnId(Cow::Borrowed("wobbly"))),
        Err(SortError::UnknownColumn("wobbly".to_string()))
    );
    assert_eq!(
        sort_key_for_column(&ColumnId(Cow::Borrowed("thumbnail"))),
        Err(SortError::UnsortableColumn("thumbnail".to_string()))
    );

    let current = SortSpec::default();
    let promoted = current
        .on_header_click(&ColumnId(Cow::Borrowed("duration")))
        .expect("duration ordenable");
    assert_eq!(
        promoted.key,
        SortKey::Metadata(MetadataKey::Duration)
    );
    assert_eq!(
        current.on_header_click(&ColumnId(Cow::Borrowed("wobbly"))),
        Err(SortError::UnknownColumn("wobbly".to_string())),
        "una cabecera desconocida deja el orden intacto, sin panico"
    );
}

/// Caso 29: cambiar el criterio conserva la selección.
#[test]
fn case_29_changing_the_key_preserves_the_selection() {
    let entries = vec![file("delta"), file("alpha"), file("charlie"), file("bravo")];
    let selection: BTreeSet<usize> = [0, 2].into_iter().collect();

    let s = spec(SortKey::Name, SortOrder::Ascending, DirectoryGrouping::Mixed);
    let perm = sort_permutation(&entries, &s);
    assert_eq!(perm, vec![1, 3, 2, 0]);

    let remapped = remap_selection(&selection, &perm).expect("permutacion valida");
    assert_eq!(remapped, [2, 3].into_iter().collect::<BTreeSet<usize>>());

    let mut sorted = entries.clone();
    sort_entries(&mut sorted, &s);
    let picked: BTreeSet<String> = remapped
        .iter()
        .filter_map(|i| sorted.get(*i).map(|e| e.display.clone()))
        .collect();
    let original: BTreeSet<String> = selection
        .iter()
        .filter_map(|i| entries.get(*i).map(|e| e.display.clone()))
        .collect();
    assert_eq!(picked, original, "deben ser exactamente las mismas entradas");
}

/// Caso 30: invertir el sentido mantiene el criterio y la selección.
#[test]
fn case_30_toggling_the_order_keeps_key_and_selection() {
    let current = spec(SortKey::Size, SortOrder::Ascending, DirectoryGrouping::First);
    let toggled = current.toggled_order();
    assert_eq!(toggled.key, current.key);
    assert_eq!(toggled.order, SortOrder::Descending);
    assert_eq!(toggled.grouping, current.grouping);
    assert_eq!(toggled.collation, current.collation);
    assert_eq!(toggled.toggled_order(), current);
    assert_eq!(current.with_order(SortOrder::Descending), toggled);

    let entries = vec![sized("a", 30), sized("b", 10), sized("c", 20)];
    let selection: BTreeSet<usize> = [0, 1].into_iter().collect();
    let perm = sort_permutation(&entries, &toggled);
    let remapped = remap_selection(&selection, &perm).expect("permutacion valida");
    assert_eq!(remapped.len(), selection.len());

    let mut sorted = entries.clone();
    sort_entries(&mut sorted, &toggled);
    let picked: BTreeSet<String> = remapped
        .iter()
        .filter_map(|i| sorted.get(*i).map(|e| e.display.clone()))
        .collect();
    assert_eq!(
        picked,
        ["a".to_string(), "b".to_string()]
            .into_iter()
            .collect::<BTreeSet<String>>()
    );
}

/// Caso 31: el menú «Ordenar por» solo ofrece los metadatos presentes.
#[test]
fn case_31_available_keys_depend_on_present_metadata() {
    let plain = vec![file("a.txt"), dir("carpeta")];
    assert_eq!(available_keys(&plain), INTRINSIC_KEYS.to_vec());
    assert_eq!(available_keys(&[]), INTRINSIC_KEYS.to_vec());

    let mut jpeg = file("foto.jpg");
    jpeg.extra
        .insert(MetadataKey::Dimensions, MetadataValue::Pair(1920, 1080));
    let mut song = file("tema.mp3");
    song.extra
        .insert(MetadataKey::Duration, MetadataValue::Unsigned(180));
    song.extra.insert(
        MetadataKey::Custom(Cow::Borrowed("bpm")),
        MetadataValue::Unsigned(128),
    );

    let rich = vec![file("a.txt"), jpeg, song];
    let expected = vec![
        SortKey::Name,
        SortKey::Extension,
        SortKey::Size,
        SortKey::Modified,
        SortKey::Created,
        SortKey::Accessed,
        SortKey::Kind,
        SortKey::Location,
        SortKey::Metadata(MetadataKey::Dimensions),
        SortKey::Metadata(MetadataKey::Duration),
        SortKey::Metadata(MetadataKey::Custom(Cow::Borrowed("bpm"))),
        SortKey::Unsorted,
    ];
    assert_eq!(available_keys(&rich), expected);
}

/// Caso 32: la ordenación es composable por grupos («Agrupar por»).
#[test]
fn case_32_sorting_a_group_yields_the_same_subsequence() {
    let entries = vec![
        sized_dir("dir-z", 7),
        sized("file-b", 3),
        sized_dir("dir-a", 7),
        sized("file-a", 90),
        sized_dir("dir-m", 1),
        sized("file-c", 3),
    ];
    let s = spec(
        SortKey::Size,
        SortOrder::Descending,
        DirectoryGrouping::First,
    );

    let mut full = entries.clone();
    sort_entries(&mut full, &s);
    let split = full
        .iter()
        .position(|e| e.kind == EntryKind::File)
        .expect("hay archivos");

    let mut dirs: Vec<FileEntry> = entries
        .iter()
        .filter(|e| e.kind == EntryKind::Directory)
        .cloned()
        .collect();
    sort_entries(&mut dirs, &s);
    assert_eq!(dirs, full[..split].to_vec());

    let mut files: Vec<FileEntry> = entries
        .iter()
        .filter(|e| e.kind == EntryKind::File)
        .cloned()
        .collect();
    sort_entries(&mut files, &s);
    assert_eq!(files, full[split..].to_vec());

    // Un sub-slice contiguo del resultado ya ordenado no cambia al reordenarlo.
    let mut slice = full[1..4].to_vec();
    sort_entries(&mut slice, &s);
    assert_eq!(slice, full[1..4].to_vec());
}

/// Caso 33: `SortKey::Unsorted` conserva el orden de llegada del scandir.
#[test]
fn case_33_unsorted_key_keeps_scandir_order() {
    let entries = vec![
        sized("zzz", 1),
        sized_dir("aaa", 99),
        sized("mmm", 50),
    ];
    for order in [SortOrder::Ascending, SortOrder::Descending] {
        for grouping in [
            DirectoryGrouping::First,
            DirectoryGrouping::Last,
            DirectoryGrouping::Mixed,
        ] {
            let s = spec(SortKey::Unsorted, order, grouping);
            let mut copy = entries.clone();
            sort_entries(&mut copy, &s);
            assert_eq!(copy, entries, "Unsorted no puede mover nada");
            assert_eq!(sort_permutation(&entries, &s), vec![0, 1, 2]);
            assert_eq!(
                insertion_index(&entries, &file("nuevo"), &s),
                entries.len()
            );
            assert_eq!(
                compare_entries(&entries[0], &entries[1], &s),
                core::cmp::Ordering::Equal
            );
        }
    }
}

/// Caso 34: ordenar por ubicación en resultados de búsqueda; sin ubicación va al
/// final en ambos sentidos.
#[test]
fn case_34_location_sorting_puts_missing_paths_last() {
    let mut a = file("l-a");
    a.location = Some(PathBuf::from("/home/z"));
    let mut b = file("l-b");
    b.location = Some(PathBuf::from("/home/a"));
    let c = file("l-c");
    let mut d = file("l-d");
    d.location = Some(PathBuf::from("/tmp"));
    let entries = vec![a, b, c, d];

    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Location,
                SortOrder::Ascending,
                DirectoryGrouping::Mixed
            )
        ),
        vec!["l-b", "l-a", "l-d", "l-c"]
    );
    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Location,
                SortOrder::Descending,
                DirectoryGrouping::Mixed
            )
        ),
        vec!["l-d", "l-a", "l-b", "l-c"]
    );
}

/// Caso 35: ordenar por tipo usa la etiqueta legible con la collation activa; sin
/// etiqueta, al final.
#[test]
fn case_35_kind_sorting_uses_the_type_label() {
    let mut pdf = file("k-a");
    pdf.type_label = Some("Documento PDF".to_string());
    let mut txt = file("k-b");
    txt.type_label = Some("archivo de texto".to_string());
    let unknown = file("k-c");
    let entries = vec![pdf, txt, unknown];

    assert_eq!(
        sorted_names(
            &entries,
            &spec(SortKey::Kind, SortOrder::Ascending, DirectoryGrouping::Mixed)
        ),
        vec!["k-b", "k-a", "k-c"],
        "la etiqueta se compara insensible a mayusculas, y los None al final"
    );
    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Kind,
                SortOrder::Descending,
                DirectoryGrouping::Mixed
            )
        ),
        vec!["k-a", "k-b", "k-c"]
    );
}

/// Caso 36: listado vacío y listado de un solo elemento.
#[test]
fn case_36_empty_and_single_entry_lists() {
    let s = SortSpec::default();
    let mut empty: Vec<FileEntry> = Vec::new();
    sort_entries(&mut empty, &s);
    assert!(empty.is_empty());
    assert_eq!(sort_permutation(&empty, &s), Vec::<usize>::new());
    assert_eq!(insertion_index(&empty, &file("solo"), &s), 0);
    let no_indices: [usize; 0] = [];
    assert_eq!(invert_permutation(&no_indices), Ok(Vec::new()));

    let mut one = vec![file("solo")];
    sort_entries(&mut one, &s);
    assert_eq!(names(&one), vec!["solo"]);
    assert_eq!(sort_permutation(&one, &s), vec![0]);
    assert_eq!(invert_permutation(&[0]), Ok(vec![0]));
}

/// Caso 37: estado persistido corrupto — error tipado y caída al valor por defecto.
#[test]
fn case_37_corrupt_persisted_spec_falls_back_to_default() {
    assert_eq!(
        SortSpec::from_str("modified:desc:???"),
        Err(SortError::MalformedSpec("modified:desc:???".to_string()))
    );
    assert_eq!(
        SortSpec::from_str(""),
        Err(SortError::MalformedSpec(String::new()))
    );
    assert_eq!(
        SortSpec::from_str("name:sideways:mixed:ci,natural"),
        Err(SortError::MalformedSpec(
            "name:sideways:mixed:ci,natural".to_string()
        ))
    );
    assert_eq!(
        SortSpec::from_str("wobble:asc:dirs-first:ci,natural"),
        Err(SortError::UnknownSortKey("wobble".to_string()))
    );
    assert_eq!(
        SortSpec::from_str("modified:desc:???").unwrap_or_default(),
        SortSpec::default()
    );
}

/// Caso 38: round-trip exacto de la vista persistida por carpeta.
#[test]
fn case_38_sort_spec_round_trips_for_every_combination() {
    assert_eq!(
        SortSpec {
            key: SortKey::Modified,
            order: SortOrder::Descending,
            grouping: DirectoryGrouping::First,
            collation: Collation::default(),
        }
        .to_string(),
        "modified:desc:dirs-first:ci,natural"
    );

    let mut keys: Vec<SortKey> = INTRINSIC_KEYS.to_vec();
    for meta in [
        MetadataKey::Dimensions,
        MetadataKey::Duration,
        MetadataKey::Album,
        MetadataKey::Artist,
        MetadataKey::Tags,
        MetadataKey::Rating,
        MetadataKey::Custom(Cow::Borrowed("bpm")),
    ] {
        keys.push(SortKey::Metadata(meta));
    }

    let mut checked = 0usize;
    for key in &keys {
        for order in [SortOrder::Ascending, SortOrder::Descending] {
            for grouping in [
                DirectoryGrouping::First,
                DirectoryGrouping::Last,
                DirectoryGrouping::Mixed,
            ] {
                for case_sensitive in [false, true] {
                    for natural_numeric in [false, true] {
                        let original = SortSpec {
                            key: key.clone(),
                            order,
                            grouping,
                            collation: Collation {
                                case_sensitive,
                                natural_numeric,
                            },
                        };
                        let text = original.to_string();
                        assert_eq!(
                            SortSpec::from_str(&text),
                            Ok(original.clone()),
                            "round-trip fallido para {text}"
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert_eq!(checked, keys.len() * 2 * 3 * 2 * 2);

    // El sentido de una carpeta no altera el de otra.
    let carpeta_a = SortSpec::default();
    let carpeta_b = carpeta_a.toggled_order();
    assert_ne!(carpeta_a.order, carpeta_b.order);
    assert_eq!(
        SortSpec::from_str(&carpeta_a.to_string()),
        Ok(SortSpec::default())
    );
}

/// Caso 39: type-ahead y renombrado secuencial recorren el orden visible, no el de
/// scandir.
#[test]
fn case_39_type_ahead_walks_the_visible_order() {
    let entries = vec![file("zeta"), file("alfa"), file("mike"), file("bravo")];
    let s = spec(SortKey::Name, SortOrder::Ascending, DirectoryGrouping::Mixed);
    let perm = sort_permutation(&entries, &s);

    let visible: Vec<&str> = perm
        .iter()
        .filter_map(|i| entries.get(*i).map(|e| e.display.as_str()))
        .collect();
    assert_eq!(visible, vec!["alfa", "bravo", "mike", "zeta"]);
    assert_ne!(visible, names(&entries));

    // El siguiente tras "bravo" en el orden visible es "mike", no "mike" por scandir.
    let pos = visible
        .iter()
        .position(|n| *n == "bravo")
        .expect("bravo esta en el listado");
    assert_eq!(visible.get(pos + 1).copied(), Some("mike"));

    let inverse = invert_permutation(&perm).expect("permutacion valida");
    assert_eq!(inverse.len(), entries.len());
    for (new_idx, old_idx) in perm.iter().enumerate() {
        assert_eq!(inverse[*old_idx], new_idx);
    }
    assert_eq!(
        invert_permutation(&[0, 0, 1]),
        Err(SortError::InvalidPermutation)
    );
    assert_eq!(
        invert_permutation(&[0, 5]),
        Err(SortError::InvalidPermutation)
    );
    assert_eq!(
        remap_selection(&BTreeSet::new(), &[0, 0]),
        Err(SortError::InvalidPermutation)
    );
    assert_eq!(
        remap_selection(&[9].into_iter().collect(), &[0, 1]),
        Err(SortError::SelectionOutOfRange { index: 9, len: 2 })
    );
}

/// Caso 40: refresco e `inotify` — la ordenación es idempotente y consistente con
/// `insertion_index`.
#[test]
fn case_40_sorting_is_idempotent_and_consistent_with_insertion() {
    let entries = vec![
        sized_dir("dir-b", 4),
        sized("file-z", 9),
        sized_dir("dir-a", 4),
        sized("file-a", 1),
    ];
    let s = spec(SortKey::Size, SortOrder::Ascending, DirectoryGrouping::First);

    let mut once = entries.clone();
    sort_entries(&mut once, &s);
    let mut twice = once.clone();
    sort_entries(&mut twice, &s);
    assert_eq!(once, twice, "sort(sort(x)) == sort(x)");

    let arrival = sized("file-m", 5);
    let idx = insertion_index(&once, &arrival, &s);
    let mut inserted = once.clone();
    inserted.insert(idx, arrival.clone());

    let mut full = once;
    full.push(arrival);
    sort_entries(&mut full, &s);
    assert_eq!(inserted, full);
}

/// Caso 41: 100 000 entradas por nombre en un solo hilo.
///
/// Se ejecuta con `cargo test -p kara-core --release -- --ignored`.
#[test]
#[ignore = "presupuesto de latencia; requiere --release"]
fn case_41_one_hundred_thousand_entries_sort_within_budget() {
    let mut entries: Vec<FileEntry> = (0..100_000u32)
        .map(|i| {
            let n = i.wrapping_mul(2_654_435_761) % 100_000;
            sized(&format!("archivo{n}-{i}.txt"), u64::from(n))
        })
        .collect();
    let s = SortSpec::default();

    let start = std::time::Instant::now();
    sort_entries(&mut entries, &s);
    let elapsed = start.elapsed();

    assert_eq!(entries.len(), 100_000);
    for pair in entries.windows(2) {
        assert_ne!(
            compare_entries(&pair[0], &pair[1], &s),
            core::cmp::Ordering::Greater,
            "el resultado debe quedar ordenado"
        );
    }
    assert!(
        elapsed <= Duration::from_millis(150),
        "ordenar 100 000 entradas tardo {elapsed:?}, presupuesto 150 ms"
    );
}

/// Caso 42: el comparador es un orden total — antisimétrico, transitivo y reflexivo —
/// y `Equal` implica el mismo basename crudo (salvo con `Unsorted`).
#[test]
fn case_42_compare_entries_is_a_total_order() {
    let mut jpeg = file("foto.jpg");
    jpeg.extra
        .insert(MetadataKey::Dimensions, MetadataValue::Pair(4, 3));
    let mut jpeg2 = file("otra.jpeg");
    jpeg2
        .extra
        .insert(MetadataKey::Dimensions, MetadataValue::Pair(6, 2));
    let mut jpeg3 = file("mini.jpeg");
    jpeg3
        .extra
        .insert(MetadataKey::Dimensions, MetadataValue::Pair(2, 2));
    let mut mixed = file("raro.bin");
    mixed
        .extra
        .insert(MetadataKey::Dimensions, MetadataValue::Text("x".into()));
    let mut labelled = file("etiqueta");
    labelled.type_label = Some("Zeta".to_string());

    let universe = vec![
        file("a"),
        file("A"),
        file("a1"),
        file("a01"),
        dir("a"),
        dir("zzz"),
        sized("s", 0),
        sized("s2", u64::MAX),
        timed("t", -5),
        timed("t2", 5),
        raw_file(b"bad\xFF"),
        raw_file(b"bad\xFE"),
        jpeg,
        jpeg2,
        jpeg3,
        mixed,
        labelled,
        file(""),
    ];

    let specs = vec![
        spec(SortKey::Name, SortOrder::Ascending, DirectoryGrouping::First),
        spec(
            SortKey::Name,
            SortOrder::Descending,
            DirectoryGrouping::Mixed,
        ),
        spec(SortKey::Size, SortOrder::Descending, DirectoryGrouping::Last),
        spec(
            SortKey::Modified,
            SortOrder::Ascending,
            DirectoryGrouping::First,
        ),
        spec(
            SortKey::Extension,
            SortOrder::Descending,
            DirectoryGrouping::First,
        ),
        spec(SortKey::Kind, SortOrder::Ascending, DirectoryGrouping::Mixed),
        spec(
            SortKey::Metadata(MetadataKey::Dimensions),
            SortOrder::Ascending,
            DirectoryGrouping::Mixed,
        ),
        spec(
            SortKey::Unsorted,
            SortOrder::Descending,
            DirectoryGrouping::First,
        ),
    ];

    for s in &specs {
        for a in &universe {
            assert_eq!(
                compare_entries(a, a, s),
                core::cmp::Ordering::Equal,
                "reflexividad"
            );
            for b in &universe {
                assert_eq!(
                    compare_entries(a, b, s),
                    compare_entries(b, a, s).reverse(),
                    "antisimetria rota"
                );
                if s.key != SortKey::Unsorted
                    && compare_entries(a, b, s) == core::cmp::Ordering::Equal
                {
                    assert_eq!(a.name, b.name, "Equal solo con el mismo basename crudo");
                }
                for c in &universe {
                    let ab = compare_entries(a, b, s);
                    let bc = compare_entries(b, c, s);
                    if ab == bc && ab != core::cmp::Ordering::Equal {
                        assert_eq!(compare_entries(a, c, s), ab, "transitividad rota");
                    }
                    if ab == core::cmp::Ordering::Equal && bc == core::cmp::Ordering::Equal {
                        assert_eq!(
                            compare_entries(a, c, s),
                            core::cmp::Ordering::Equal,
                            "transitividad de la igualdad rota"
                        );
                    }
                }
            }
        }
    }
}

// ------------------------------------------- plegado de caja (regresión) ---
//
// `compare_names` es un orden TOTAL: desempata por los puntos de código originales,
// así que jamás devuelve `Equal` para dos cadenas distintas. Comparar "żółw" con
// "żółw" en mayúsculas esperando `Equal` no probaría nada.
//
// La forma que sí discrimina —y la que usan los casos 43 a 48— es comparar una
// palabra en minúscula con una MAYÚSCULA que la tiene como prefijo:
//
//   - con plegado correcto: la minúscula es prefijo de la otra y sale `Less`;
//   - sin plegar: la minúscula empieza por un punto de código MAYOR que su capital
//     (`ż` U+017C > `Ż` U+017B) y sale `Greater`.
//
// Resultados opuestos: el test distingue de verdad una implementación de la otra.

/// Comprueba el par discriminante en los dos sentidos: `lower < UPPER_PREFIJADA`.
fn folds_case(lower: &str, upper_with_lower_as_prefix: &str) {
    let collation = Collation::default();
    assert_eq!(
        compare_names(lower, upper_with_lower_as_prefix, &collation),
        core::cmp::Ordering::Less,
        "{lower:?} deberia ir antes que {upper_with_lower_as_prefix:?}: \
         tras plegar la caja el primero es prefijo del segundo (sin plegar saldria Greater)"
    );
    assert_eq!(
        compare_names(upper_with_lower_as_prefix, lower, &collation),
        core::cmp::Ordering::Greater,
        "la comparacion debe ser antisimetrica para {lower:?} y {upper_with_lower_as_prefix:?}"
    );
}

/// Caso 43: plegado de caja en Latin Extended-A — polaco y checo.
///
/// Regresión del bloqueante: la tabla de plegado escrita a mano solo llegaba al
/// Suplemento Latin-1, así que `ż`, `ć`, `ł`, `ą`, `č` y `š` pasaban **sin plegar** y
/// la insensibilidad a mayúsculas —que la spec enuncia sin condiciones— no funcionaba
/// en polaco ni en checo.
#[test]
fn case_43_latin_extended_a_folds_for_polish_and_czech() {
    // żółw / ŻÓŁWY — U+017C vs U+017B.
    folds_case("\u{17C}\u{F3}\u{142}w", "\u{17B}\u{D3}\u{141}WY");
    // ćma / ĆMAS — U+0107 vs U+0106.
    folds_case("\u{107}ma", "\u{106}MAS");
    // łąka / ŁĄKAS — U+0142 vs U+0141 y U+0105 vs U+0104.
    folds_case("\u{142}\u{105}ka", "\u{141}\u{104}KAS");
    // čas / ČASY — U+010D vs U+010C.
    folds_case("\u{10D}as", "\u{10C}ASY");
    // šum / ŠUMY — U+0161 vs U+0160.
    folds_case("\u{161}um", "\u{160}UMY");

    // Y en una lista ordenada de verdad: [čaj, żubr, ŻÓŁW]. Sin plegar, ŻÓŁW
    // adelantaria a żubr porque Ż (U+017B) < ż (U+017C).
    assert_eq!(
        name_asc(&["\u{17C}ubr", "\u{17B}\u{D3}\u{141}W", "\u{10D}aj"]),
        vec!["\u{10D}aj", "\u{17C}ubr", "\u{17B}\u{D3}\u{141}W"]
    );
}

/// Caso 44: plegado de caja en maltés (Latin Extended-A) y en Latin Extended-B.
///
/// El mismo hueco del caso 43, en los dos bloques que la tabla a mano ni rozaba.
#[test]
fn case_44_latin_extended_b_and_maltese_fold() {
    // ħabib / ĦABIBI — U+0127 vs U+0126 (maltés).
    folds_case("\u{127}abib", "\u{126}ABIBI");
    // ħġieġ / ĦĠIEĠA — U+0127/U+0126 y U+0121/U+0120.
    folds_case("\u{127}\u{121}ie\u{121}", "\u{126}\u{120}IE\u{120}A");
    // știre / ȘTIRI — U+0219 vs U+0218, Latin Extended-B (rumano).
    folds_case("\u{219}tire", "\u{218}TIRI");
    // ǧala / ǦALAS — U+01E7 vs U+01E6, Latin Extended-B.
    folds_case("\u{1E7}ala", "\u{1E6}ALAS");
}

/// Caso 45: griego — sigma final y sigma medial pliegan ambas con la capital.
///
/// Sin el plegado de la sigma final, `"ΟΔΟΣ"` y `"οδος"` (el mismo nombre con la misma
/// escritura insensible a mayúsculas) se separarían en el listado. También cubre las
/// capitales acentuadas (`Ά` U+0386), que la tabla a mano dejaba fuera de su rango
/// U+0391..=U+03AB.
#[test]
fn case_45_greek_sigma_and_accented_capitals_fold() {
    // οδοσ / ΟΔΟΣΑ — sigma medial U+03C3 vs U+03A3.
    folds_case(
        "\u{3BF}\u{3B4}\u{3BF}\u{3C3}",
        "\u{39F}\u{394}\u{39F}\u{3A3}\u{391}",
    );
    // άλφα / ΆΛΦΑΣ — capital acentuada U+0386 -> U+03AC.
    folds_case(
        "\u{3AC}\u{3BB}\u{3C6}\u{3B1}",
        "\u{386}\u{39B}\u{3A6}\u{391}\u{3A3}",
    );

    // Sigma final: "οδος.txt" vs "ΟΔΟΣ.PDF". Plegando, las cuatro primeras letras
    // empatan y decide la extension (txt > pdf) -> Greater. Sin plegar la sigma final,
    // ς (U+03C2) < σ (U+03C3) decidiria antes y saldria Less.
    assert_eq!(
        compare_names(
            "\u{3BF}\u{3B4}\u{3BF}\u{3C2}.txt",
            "\u{39F}\u{394}\u{39F}\u{3A3}.PDF",
            &Collation::default()
        ),
        core::cmp::Ordering::Greater,
        "la sigma final debe plegar con la capital: decide la extension, no la sigma"
    );

    // Las dos sigmas minusculas caen en el mismo punto del orden: [ΟΔΟΣΑ, οδοςβ,
    // ΟΔΟΣΓ]. Sin plegar la sigma final, οδοςβ se adelantaria a las tres.
    assert_eq!(
        name_asc(&[
            "\u{39F}\u{394}\u{39F}\u{3A3}\u{391}",
            "\u{3BF}\u{3B4}\u{3BF}\u{3C2}\u{3B2}",
            "\u{39F}\u{394}\u{39F}\u{3A3}\u{393}",
        ]),
        vec![
            "\u{39F}\u{394}\u{39F}\u{3A3}\u{391}",
            "\u{3BF}\u{3B4}\u{3BF}\u{3C2}\u{3B2}",
            "\u{39F}\u{394}\u{39F}\u{3A3}\u{393}",
        ]
    );
}

/// Caso 46: el signo micro (U+00B5) pliega a mu griega minúscula (U+03BC).
///
/// Es plegado, no mapeo: `to_lowercase` deja el signo micro intacto, así que sin su
/// arm propio `µs.log` y `ΜS.LOG` se separarían.
#[test]
fn case_46_micro_sign_folds_to_greek_small_mu() {
    let collation = Collation::default();

    // µb vs ΜA: plegando ambos a μ decide la segunda letra (b > a) -> Greater.
    // Sin plegar, µ (U+00B5) < Μ (U+039C) decidiria antes -> Less.
    assert_eq!(
        compare_names("\u{B5}b", "\u{39C}A", &collation),
        core::cmp::Ordering::Greater,
        "el signo micro debe plegar con la mu capital"
    );
    // µsb vs μsa: mismo razonamiento contra la mu minuscula (U+03BC).
    assert_eq!(
        compare_names("\u{B5}sb", "\u{3BC}sa", &collation),
        core::cmp::Ordering::Greater,
        "el signo micro debe plegar con la mu minuscula"
    );
    // Plegan igual, pero el orden es total: desempatan por punto de codigo.
    assert_eq!(
        compare_names("\u{B5}s.log", "\u{3BC}s.log", &collation),
        core::cmp::Ordering::Less,
        "empatan tras plegar y desempata el punto de codigo (U+00B5 < U+03BC)"
    );
}

/// Caso 47: sin regresión — lo que la tabla a mano ya cubría sigue plegando.
///
/// ASCII, Suplemento Latin-1 y cirílico base.
#[test]
fn case_47_ascii_latin1_and_cyrillic_still_fold() {
    // ASCII.
    folds_case("readme", "READMES");
    // Latin-1: á/Á (U+00E1/U+00C1), ñ/Ñ (U+00F1/U+00D1), ö/Ö (U+00F6/U+00D6),
    // ø/Ø (U+00F8/U+00D8) y þ/Þ (U+00FE/U+00DE), los bordes de los dos rangos.
    folds_case("\u{E1}ngel", "\u{C1}NGELES");
    folds_case("\u{F1}u", "\u{D1}US");
    folds_case("\u{F6}l", "\u{D6}LS");
    folds_case("\u{F8}re", "\u{D8}RED");
    folds_case("\u{FE}or", "\u{DE}ORN");
    // Cirilico: мир / МИРОВ (U+043C vs U+041C) y ёж / ЁЖИ (U+0451 vs U+0401).
    folds_case("\u{43C}\u{438}\u{440}", "\u{41C}\u{418}\u{420}\u{41E}\u{412}");
    folds_case("\u{451}\u{436}", "\u{401}\u{416}\u{418}");
}

/// Caso 48: hueco conocido y deliberado — los mapeos que expanden a varios caracteres
/// se dejan **sin plegar**.
///
/// `İ` (U+0130) minúsculiza a `i` + U+0307: plegarlo exigiría un átomo
/// multi-carácter y una asignación por nombre, lo que rompe el presupuesto de 100 000
/// entradas. Este test fija el comportamiento para que cambiarlo sea una decisión
/// deliberada y no un accidente.
#[test]
fn case_48_multi_char_lowercase_expansions_stay_unfolded() {
    let collation = Collation::default();

    // İx vs iy: sin plegar decide la primera letra, İ (U+0130) > i (U+0069).
    // Si İ plegase a i, decidiria la segunda y saldria Less.
    assert_eq!(
        compare_names("\u{130}x", "iy", &collation),
        core::cmp::Ordering::Greater,
        "U+0130 se deja sin plegar a proposito: su minuscula expande a dos caracteres"
    );
    // El contraste: la I ASCII si pliega, y por eso decide la segunda letra.
    assert_eq!(
        compare_names("Ix", "iy", &collation),
        core::cmp::Ordering::Less,
        "un plegado dependiente del locale turco mandaria I a ı (U+0131) y saldria Greater"
    );
    // La ı sin punto (U+0131) tampoco pliega con la i ASCII: no es su mapeo por
    // defecto, solo el turco.
    assert_eq!(
        compare_names("\u{131}x", "iy", &collation),
        core::cmp::Ordering::Greater,
        "U+0131 y U+0069 son caracteres distintos fuera del locale turco"
    );
}

/// Invariante de [`collation_key`]: comparar claves precomputadas equivale a
/// [`compare_names`].
#[test]
fn collation_key_ordering_matches_compare_names() {
    let sample = [
        "a",
        "A",
        "a1",
        "a01",
        "a2",
        "a10",
        "archivo",
        "ARCHIVO",
        ".bashrc",
        "",
        "acci\u{f3}n",
        "accio\u{301}n",
        "accion",
        "z",
        "1",
        "01",
        // Plegado de caja mas alla de Latin-1: la clave precomputada debe seguir al
        // comparador tambien aqui.
        "\u{17C}\u{F3}\u{142}w",
        "\u{17B}\u{D3}\u{141}W",
        "\u{10D}as",
        "\u{10C}AS",
        "\u{3BF}\u{3B4}\u{3BF}\u{3C2}",
        "\u{3BF}\u{3B4}\u{3BF}\u{3C3}",
        "\u{39F}\u{394}\u{39F}\u{3A3}",
        "\u{B5}s",
        "\u{3BC}s",
        "\u{130}stanbul",
        "istanbul",
    ];
    for collation in [
        Collation::default(),
        Collation {
            case_sensitive: true,
            natural_numeric: true,
        },
        Collation {
            case_sensitive: false,
            natural_numeric: false,
        },
    ] {
        for a in sample {
            for b in sample {
                assert_eq!(
                    collation_key(a, &collation).cmp(&collation_key(b, &collation)),
                    compare_names(a, b, &collation),
                    "clave y comparador difieren para {a:?} vs {b:?} con {collation:?}"
                );
            }
        }
    }
}

/// `with_key` resetea el sentido al cambiar de criterio y lo conserva si es el mismo.
#[test]
fn with_key_resets_the_order_only_when_the_key_changes() {
    let current = spec(
        SortKey::Name,
        SortOrder::Descending,
        DirectoryGrouping::Mixed,
    );
    let changed = current.with_key(SortKey::Size);
    assert_eq!(changed.key, SortKey::Size);
    assert_eq!(changed.order, SortOrder::Ascending);
    assert_eq!(changed.grouping, DirectoryGrouping::Mixed);

    let same = current.with_key(SortKey::Name);
    assert_eq!(same, current);
    assert_eq!(SortOrder::Ascending.toggled(), SortOrder::Descending);
    assert_eq!(SortOrder::Descending.toggled(), SortOrder::Ascending);
    assert_eq!(DirectoryGrouping::default(), DirectoryGrouping::First);
    assert_eq!(
        Collation::default(),
        Collation {
            case_sensitive: false,
            natural_numeric: true
        }
    );
    assert_eq!(SortSpec::default().key, SortKey::Name);
    assert_eq!(SortSpec::default().order, SortOrder::Ascending);
}

/// Las dimensiones ordenan por área y luego por ancho.
#[test]
fn dimensions_sort_by_area_then_width() {
    let make = |name: &str, w: u32, h: u32| {
        let mut e = file(name);
        e.extra
            .insert(MetadataKey::Dimensions, MetadataValue::Pair(w, h));
        e
    };
    let entries = vec![
        make("ancha", 6, 2),
        make("mini", 2, 2),
        make("cuadrada", 4, 3),
        file("sin-metadato"),
    ];
    assert_eq!(
        sorted_names(
            &entries,
            &spec(
                SortKey::Metadata(MetadataKey::Dimensions),
                SortOrder::Ascending,
                DirectoryGrouping::Mixed
            )
        ),
        vec!["mini", "cuadrada", "ancha", "sin-metadato"]
    );
}
