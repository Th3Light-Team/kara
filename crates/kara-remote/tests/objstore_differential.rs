//! Differential test: the same seeded random operation sequences against
//! `ObjectStoreBackend` (over the paged fault wrapper, with tiny pages and
//! tiny multipart parts so both paths are exercised) and against
//! `MemoryBackend::object_store_like()`, the contract's model of an object
//! store. Every step must give the same answer (`Ok`, or the same error kind
//! naming the same path) and the final trees must be equal (names, kinds,
//! contents, empty folders).
//!
//! The generator follows `kara-fs/tests/differential_memory_local.rs` (that
//! file is hash-pinned, so it is copied, not shared), without links: an
//! object store has none.
//!
//! Reproduce with `KARA_DIFF_SEED=<seed>` (`KARA_DIFF_TRACE=1` prints each
//! op); widen with `KARA_DIFF_SEEDS=<count>` (default 40) and
//! `KARA_DIFF_OPS=<count>` (100).
//!
//! Not compared: times (`MemoryBackend` gives a marker folder a time, the
//! object store gives folders none), `extra`, error sources.

mod objstore_support;

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::sync::atomic::Ordering;

use kara_core::{EntryKind, FileEntry};
use kara_vfs::memory::MemoryBackend;
use kara_vfs::{Backend, BackendError, BackendErrorKind, Cancel, RemotePath};
use objstore_support::{Faulty, backend_with_parts};

// ---------------------------------------------------------------------------
// Generator (splitmix64), as in kara-fs.

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

const NAMES: [&str; 4] = ["a", "b", "c d", ".h\u{fc}"];

fn random_path(rng: &mut Rng, allow_root: bool) -> RemotePath {
    if allow_root && rng.chance(4) {
        return RemotePath::root();
    }
    let depth = match rng.below(100) {
        0..=49 => 1,
        50..=84 => 2,
        _ => 3,
    };
    let mut path = RemotePath::root();
    for _ in 0..depth {
        let name = NAMES[usize::try_from(rng.below(NAMES.len() as u64)).unwrap_or(0)];
        path = path.join(name).unwrap_or(path);
    }
    path
}

fn random_content(rng: &mut Rng) -> Vec<u8> {
    let len = if rng.chance(15) { 0 } else { rng.below(40) };
    (0..len).map(|_| (rng.next() & 0xff) as u8).collect()
}

#[derive(Debug, Clone)]
enum Op {
    CreateDir(RemotePath),
    Write(RemotePath, Vec<u8>, bool),
    WriteThenAbort(RemotePath, Vec<u8>, bool),
    WriteThenDrop(RemotePath, Vec<u8>, bool),
    Rename(RemotePath, RemotePath),
    Remove(RemotePath),
    RemoveTree(RemotePath),
    /// Read then write, as kara-ops does across drives.
    Copy(RemotePath, RemotePath, bool),
    /// The service's own copy.
    CopyWithin(RemotePath, RemotePath),
    List(RemotePath),
    Stat(RemotePath),
    OpenRead(RemotePath, u64),
}

fn random_op(rng: &mut Rng) -> Op {
    match rng.below(100) {
        0..=17 => Op::CreateDir(random_path(rng, true)),
        18..=37 => Op::Write(random_path(rng, true), random_content(rng), rng.chance(50)),
        38..=40 => Op::WriteThenAbort(random_path(rng, false), random_content(rng), rng.chance(50)),
        41..=42 => Op::WriteThenDrop(random_path(rng, false), random_content(rng), rng.chance(50)),
        43..=54 => Op::Rename(random_path(rng, true), random_path(rng, true)),
        55..=61 => Op::Remove(random_path(rng, true)),
        62..=65 => Op::RemoveTree(random_path(rng, true)),
        66..=71 => Op::Copy(random_path(rng, false), random_path(rng, false), rng.chance(50)),
        72..=77 => Op::CopyWithin(random_path(rng, true), random_path(rng, true)),
        78..=85 => Op::List(random_path(rng, true)),
        86..=92 => Op::Stat(random_path(rng, true)),
        _ => Op::OpenRead(random_path(rng, false), rng.below(12)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Seen {
    name: String,
    kind: EntryKind,
    size: Option<u64>,
    is_symlink: bool,
    is_hidden: bool,
}

impl Seen {
    fn of(entry: &FileEntry) -> Seen {
        Seen {
            name: entry.display.clone(),
            kind: entry.kind,
            size: entry.size,
            is_symlink: entry.is_symlink,
            is_hidden: entry.is_hidden,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Done,
    Failed(BackendErrorKind, Option<RemotePath>),
    Listed(Vec<Seen>, usize),
    Statted(Seen),
    Read(Vec<u8>),
}

fn failed(error: &BackendError) -> Outcome {
    Outcome::Failed(error.kind, error.path.clone())
}

fn io_failed(error: io::Error, path: &RemotePath) -> Outcome {
    failed(&BackendError::from_io(error, Some(path)))
}

fn write(b: &dyn Backend, path: &RemotePath, content: &[u8], replace: bool, end: u8) -> Outcome {
    let hint = Some(content.len() as u64);
    let mut session = match b.begin_write(path, hint, replace) {
        Ok(session) => session,
        Err(error) => return failed(&error),
    };
    if let Err(error) = session.write_all(content) {
        return io_failed(error, path);
    }
    let end = match end {
        0 => session.finish(),
        1 => session.abort(),
        _ => {
            drop(session);
            Ok(())
        }
    };
    match end {
        Ok(()) => Outcome::Done,
        Err(error) => failed(&error),
    }
}

fn read_all(b: &dyn Backend, path: &RemotePath, from: u64) -> Result<Vec<u8>, Outcome> {
    let mut reader = b.open_read(path, from).map_err(|e| failed(&e))?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out).map_err(|e| io_failed(e, path))?;
    Ok(out)
}

fn apply(b: &dyn Backend, op: &Op) -> Outcome {
    let done = |r: Result<(), BackendError>| match r {
        Ok(()) => Outcome::Done,
        Err(error) => failed(&error),
    };
    match op {
        Op::CreateDir(path) => done(b.create_dir(path)),
        Op::Write(path, content, replace) => write(b, path, content, *replace, 0),
        Op::WriteThenAbort(path, content, replace) => write(b, path, content, *replace, 1),
        Op::WriteThenDrop(path, content, replace) => write(b, path, content, *replace, 2),
        Op::Rename(from, to) => done(b.rename(from, to)),
        Op::Remove(path) => done(b.remove(path)),
        Op::RemoveTree(path) => done(b.remove_tree(path, &Cancel::new())),
        Op::Copy(from, to, replace) => match read_all(b, from, 0) {
            Ok(bytes) => write(b, to, &bytes, *replace, 0),
            Err(outcome) => outcome,
        },
        Op::CopyWithin(from, to) => done(b.copy_within(from, to)),
        Op::List(path) => match b.list(path, &Cancel::new()) {
            Ok(listing) => {
                let mut seen: Vec<Seen> = listing.entries.iter().map(Seen::of).collect();
                seen.sort();
                Outcome::Listed(seen, listing.errors.len())
            }
            Err(error) => failed(&error),
        },
        Op::Stat(path) => match b.stat(path) {
            Ok(entry) => Outcome::Statted(Seen::of(&entry)),
            Err(error) => failed(&error),
        },
        Op::OpenRead(path, from) => match read_all(b, path, *from) {
            Ok(bytes) => Outcome::Read(bytes),
            Err(outcome) => outcome,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Shape {
    Dir,
    File(Vec<u8>),
}

fn tree(b: &dyn Backend) -> Result<BTreeMap<String, Shape>, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![RemotePath::root()];
    while let Some(dir) = stack.pop() {
        let listing = b.list(&dir, &Cancel::new()).map_err(|e| format!("list({dir}): {e:?}"))?;
        if !listing.errors.is_empty() {
            return Err(format!("list({dir}) errors: {:?}", listing.errors));
        }
        for entry in listing.entries {
            let path = dir.join(&entry.display).map_err(|e| format!("{dir} + {:?}: {e}", entry.display))?;
            let shape = if entry.kind == EntryKind::Directory {
                stack.push(path.clone());
                Shape::Dir
            } else {
                Shape::File(read_all(b, &path, 0).map_err(|o| format!("read {path}: {o:?}"))?)
            };
            out.insert(path.to_string(), shape);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The driver.

fn env_number(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn run_seed(seed: u64, ops: u64, conditional: bool) -> io::Result<Result<(), String>> {
    let model = MemoryBackend::object_store_like();
    let store = Faulty::new();
    store.page_size.store(3, Ordering::SeqCst);
    store.conditional_put.store(conditional, Ordering::SeqCst);
    store.copy_create.store(conditional, Ordering::SeqCst);
    // Parts of 16 bytes: contents above that go through multipart uploads.
    let object = backend_with_parts(&store, 16, 2)?;
    let mut rng = Rng(seed);
    let mut log = Vec::new();
    let trace = std::env::var_os("KARA_DIFF_TRACE").is_some();
    for step in 0..ops {
        let op = random_op(&mut rng);
        if trace {
            eprintln!("{step:3}: {op:?}");
        }
        let expected = apply(&model, &op);
        let got = apply(&object, &op);
        log.push(format!("{step:3}: {op:?} -> {expected:?}"));
        if expected != got {
            return Ok(Err(format!(
                "seed {seed}, step {step}: {op:?}\n  memory: {expected:?}\n  object: {got:?}\nhistory:\n{}",
                log.join("\n")
            )));
        }
    }
    let reference = tree(&model).map_err(io::Error::other)?;
    let remote = tree(&object).map_err(io::Error::other)?;
    if reference != remote {
        return Ok(Err(format!(
            "seed {seed}: final trees differ\n  memory: {reference:?}\n  object: {remote:?}\nhistory:\n{}",
            log.join("\n")
        )));
    }
    if store.open_uploads() != 0 {
        return Ok(Err(format!("seed {seed}: {} multipart uploads left open", store.open_uploads())));
    }
    Ok(Ok(()))
}

fn run_all(first_seed: u64, conditional: bool) -> io::Result<()> {
    let ops = env_number("KARA_DIFF_OPS", 100);
    let seeds: Vec<u64> = match std::env::var("KARA_DIFF_SEED") {
        Ok(one) => vec![one.parse().map_err(io::Error::other)?],
        Err(_) => (first_seed..first_seed + env_number("KARA_DIFF_SEEDS", 40)).collect(),
    };
    for seed in seeds {
        if let Err(divergence) = run_seed(seed, ops, conditional)? {
            return Err(io::Error::other(divergence));
        }
    }
    Ok(())
}

#[test]
fn object_store_and_memory_model_agree() -> io::Result<()> {
    run_all(0, true)
}

#[test]
fn object_store_without_conditional_requests_and_memory_model_agree() -> io::Result<()> {
    run_all(5_000_000, false)
}
