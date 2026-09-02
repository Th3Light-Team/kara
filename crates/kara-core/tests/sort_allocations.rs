//! El comparador no puede asignar memoria: es lo que hace viable ordenar 100 000
//! entradas sin bloquear la UI.
//!
//! Se mide con un asignador global contador. Una implementación correcta precomputa
//! la clave de collation una vez por entrada (`decorate-sort-undecorate`), así que el
//! número de asignaciones crece con `n`, no con las `n·log n` comparaciones.

use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::OsString;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use kara_core::{
    Collation, DirectoryGrouping, EntryKind, FileEntry, MetadataBag, SortKey, SortOrder, SortSpec,
    compare_entries, sort_entries,
};

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

// SAFETY: delega integramente en el asignador del sistema y solo lleva la cuenta.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, AtomicOrdering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, AtomicOrdering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// El contador es global al proceso y el arnés de test corre en paralelo: dos medidas
/// simultáneas se contaminarían. Toda medida se toma con este cerrojo cogido, y se coge
/// **antes** de construir el corpus para que el test que espera no asigne mientras el
/// otro mide.
static MEASUREMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn entry(name: String, size: u64) -> FileEntry {
    FileEntry {
        name: OsString::from(&name),
        display: name,
        kind: EntryKind::File,
        is_symlink: false,
        symlink_broken: false,
        is_hidden: false,
        size: Some(size),
        modified: None,
        created: None,
        accessed: None,
        type_label: None,
        location: None,
        extra: MetadataBag::new(),
    }
}

fn corpus(n: u32) -> Vec<FileEntry> {
    (0..n)
        .map(|i| {
            let scattered = i.wrapping_mul(2_654_435_761) % n;
            entry(
                format!("archivo{scattered}-{i}.txt"),
                u64::from(scattered),
            )
        })
        .collect()
}

/// El comparador no asigna: con 10 000 entradas, `sort_entries` debe quedarse muy por
/// debajo de una asignación por comparación (~133 000 comparaciones).
#[test]
fn comparator_does_not_allocate_per_comparison() {
    const N: u32 = 10_000;
    let _measuring = MEASUREMENT.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = corpus(N);
    let spec = SortSpec {
        key: SortKey::Name,
        order: SortOrder::Ascending,
        grouping: DirectoryGrouping::First,
        collation: Collation::default(),
    };

    // Calienta cualquier estructura perezosa antes de medir.
    let _ = compare_entries(&entries[0], &entries[1], &spec);

    let before = ALLOCATIONS.load(AtomicOrdering::Relaxed);
    sort_entries(&mut entries, &spec);
    let after = ALLOCATIONS.load(AtomicOrdering::Relaxed);
    let used = after - before;

    let budget = 4 * N as usize;
    assert!(
        used <= budget,
        "sort_entries hizo {used} asignaciones para {N} entradas; \
         el presupuesto es {budget} (claves precomputadas, comparador sin asignar)"
    );

    // Y el resultado sigue siendo correcto.
    for pair in entries.windows(2) {
        assert_ne!(
            compare_entries(&pair[0], &pair[1], &spec),
            core::cmp::Ordering::Greater
        );
    }
}

/// El plegado de caja tampoco asigna, y ese es el motivo de que los mapeos que
/// expanden a varios caracteres (U+0130 `İ`) se dejen sin plegar: un átomo
/// multi-carácter costaría una asignación por nombre.
///
/// Con nombres en Latin Extended-A/B, griego y cirílico el presupuesto es el mismo que
/// con ASCII: dos asignaciones por clave (la cadena NFC y el vector de átomos).
#[test]
fn case_folding_outside_ascii_does_not_allocate_per_comparison() {
    const N: u32 = 10_000;
    // Cinco palabras que ejercitan los cuatro bloques: żółw, ĆMA, ČAS, ΟΔΟΣ (sigma
    // final), МИР y İSTANBUL.
    const WORDS: [&str; 6] = [
        "\u{17C}\u{F3}\u{142}w",
        "\u{106}MA",
        "\u{10D}as",
        "\u{3BF}\u{3B4}\u{3BF}\u{3C2}",
        "\u{41C}\u{418}\u{420}",
        "\u{130}STANBUL",
    ];
    let _measuring = MEASUREMENT.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries: Vec<FileEntry> = (0..N)
        .map(|i| {
            let scattered = i.wrapping_mul(2_654_435_761) % N;
            let word = WORDS[(scattered as usize) % WORDS.len()];
            entry(format!("{word}{scattered}.txt"), u64::from(scattered))
        })
        .collect();
    let spec = SortSpec::default();

    let _ = compare_entries(&entries[0], &entries[1], &spec);

    let before = ALLOCATIONS.load(AtomicOrdering::Relaxed);
    sort_entries(&mut entries, &spec);
    let after = ALLOCATIONS.load(AtomicOrdering::Relaxed);
    let used = after - before;

    let budget = 4 * N as usize;
    assert!(
        used <= budget,
        "ordenar {N} nombres no ASCII hizo {used} asignaciones; el presupuesto es \
         {budget}: el plegado de caja debe seguir siendo 1:1 y sin asignar"
    );
}

/// Presupuesto de latencia: 100 000 entradas por nombre en un solo hilo.
///
/// `cargo test -p kara-core --release -- --ignored`
#[test]
#[ignore = "presupuesto de latencia; requiere --release"]
fn sorting_one_hundred_thousand_entries_is_fast_enough() {
    let mut entries = corpus(100_000);
    let spec = SortSpec::default();

    let start = std::time::Instant::now();
    sort_entries(&mut entries, &spec);
    let elapsed = start.elapsed();

    assert_eq!(entries.len(), 100_000);
    assert!(
        elapsed <= std::time::Duration::from_millis(150),
        "ordenar 100 000 entradas tardo {elapsed:?}, presupuesto 150 ms"
    );
}
