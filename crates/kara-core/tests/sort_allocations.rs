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
