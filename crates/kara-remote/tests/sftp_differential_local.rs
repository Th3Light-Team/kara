//! Differential test: the same seeded random operation sequences against
//! `SftpBackend` (over the in-process server, on a tempdir) and against
//! `LocalBackend` on another tempdir. Every step must give the same answer
//! (`Ok`, or the same error kind naming the same path) and the final trees
//! must be equal (names, kinds, contents, links).
//!
//! The generator is the one of `kara-fs/tests/differential_memory_local.rs`
//! (that file is hash-pinned, so it is copied, not shared).
//!
//! Reproduce with `KARA_DIFF_SEED=<seed>` (`KARA_DIFF_TRACE=1` prints each
//! op); widen with `KARA_DIFF_SEEDS=<count>` (default 40) and
//! `KARA_DIFF_OPS=<count>` (100).
//!
//! Not compared, as in the original: error sources, times, modes, `location`.

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use kara_core::{EntryKind, FileEntry};
use kara_fs::LocalBackend;
use kara_vfs::{Backend, BackendError, BackendErrorKind, Cancel, RemotePath};
use support::{ServerOptions, TestServer};

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

fn downward_target(rng: &mut Rng) -> String {
    let depth = rng.below(3);
    if depth == 0 {
        return String::from(".");
    }
    (0..depth)
        .map(|_| NAMES[usize::try_from(rng.below(NAMES.len() as u64)).unwrap_or(0)])
        .collect::<Vec<_>>()
        .join("/")
}

fn relative_target(link: &RemotePath, to: &RemotePath) -> String {
    let parent = link.parent().unwrap_or_else(RemotePath::root);
    let from: Vec<&str> = parent.segments().collect();
    let to_segments: Vec<&str> = to.segments().collect();
    let common = from.iter().zip(&to_segments).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend(&to_segments[common..]);
    if parts.is_empty() {
        String::from(".")
    } else {
        parts.join("/")
    }
}

/// See the kara-fs original: the link target is computed from where the
/// link's directory really is on disk (the reference).
fn place_link(op: Op, root: &Path) -> Op {
    let Op::SymlinkTo(link, to) = op else {
        return op;
    };
    let lexical = Op::Symlink(link.clone(), relative_target(&link, &to));
    let (Some(parent), Some(name)) = (link.parent(), link.file_name()) else {
        return lexical;
    };
    let mut local_parent = root.to_path_buf();
    local_parent.extend(parent.segments());
    let (Ok(real), Ok(real_root)) = (local_parent.canonicalize(), root.canonicalize()) else {
        return lexical;
    };
    let Ok(inside) = real.strip_prefix(&real_root) else {
        return lexical;
    };
    let mut real_link = RemotePath::root();
    for part in inside.iter() {
        match part.to_str().map(|part| real_link.join(part)) {
            Some(Ok(joined)) => real_link = joined,
            _ => return lexical,
        }
    }
    match real_link.join(name) {
        Ok(real_link) => Op::Symlink(link, relative_target(&real_link, &to)),
        Err(_) => lexical,
    }
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
    Copy(RemotePath, RemotePath, bool),
    List(RemotePath),
    Stat(RemotePath),
    OpenRead(RemotePath, u64),
    Symlink(RemotePath, String),
    SymlinkTo(RemotePath, RemotePath),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Links {
    None,
    Downward,
    AnywhereNoRename,
}

fn random_op(rng: &mut Rng, links: Links) -> Op {
    loop {
        let op = match rng.below(100) {
            0..=17 => Op::CreateDir(random_path(rng, true)),
            18..=39 => Op::Write(random_path(rng, true), random_content(rng), rng.chance(50)),
            40..=42 => Op::WriteThenAbort(random_path(rng, false), random_content(rng), rng.chance(50)),
            43..=44 => Op::WriteThenDrop(random_path(rng, false), random_content(rng), rng.chance(50)),
            45..=56 => {
                if links == Links::AnywhereNoRename {
                    continue;
                }
                Op::Rename(random_path(rng, true), random_path(rng, true))
            }
            57..=63 => Op::Remove(random_path(rng, true)),
            64..=67 => Op::RemoveTree(random_path(rng, true)),
            68..=74 => Op::Copy(random_path(rng, false), random_path(rng, false), rng.chance(50)),
            75..=81 => Op::List(random_path(rng, true)),
            82..=87 => Op::Stat(random_path(rng, true)),
            88..=93 => Op::OpenRead(random_path(rng, false), rng.below(12)),
            _ => {
                let link = random_path(rng, false);
                let target = match links {
                    Links::None => continue,
                    Links::Downward => downward_target(rng),
                    Links::AnywhereNoRename => return Op::SymlinkTo(link, random_path(rng, false)),
                };
                Op::Symlink(link, target)
            }
        };
        return op;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Seen {
    name: String,
    kind: EntryKind,
    size: Option<u64>,
    is_symlink: bool,
    symlink_broken: bool,
    is_hidden: bool,
}

impl Seen {
    fn of(entry: &FileEntry) -> Seen {
        Seen {
            name: entry.display.clone(),
            kind: entry.kind,
            size: entry.size,
            is_symlink: entry.is_symlink,
            symlink_broken: entry.symlink_broken,
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

/// Creates a link on a real filesystem, with LocalBackend's rule for errors.
fn make_link(root: &Path, link: &RemotePath, target: &str) -> Outcome {
    let mut at = root.to_path_buf();
    at.extend(link.segments());
    match std::os::unix::fs::symlink(target, &at) {
        Ok(()) => Outcome::Done,
        Err(error) => {
            let parent_missing = at.parent().is_some_and(|p| fs::metadata(p).is_err());
            let unresolvable =
                error.kind() == io::ErrorKind::NotADirectory || error.raw_os_error() == Some(40);
            let error = if unresolvable && parent_missing {
                io::Error::from(io::ErrorKind::NotFound)
            } else {
                error
            };
            failed(&BackendError::from_io(error, Some(link)))
        }
    }
}

fn apply(b: &dyn Backend, root: &Path, op: &Op) -> Outcome {
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
        Op::Symlink(link, target) => make_link(root, link, target),
        Op::SymlinkTo(..) => Outcome::Failed(BackendErrorKind::Other, None),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Shape {
    Dir,
    File(Vec<u8>),
    Link { broken: bool, kind: EntryKind },
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
            let shape = if entry.is_symlink {
                Shape::Link {
                    broken: entry.symlink_broken,
                    kind: entry.kind,
                }
            } else if entry.kind == EntryKind::Directory {
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

fn run_seed(server: &TestServer, known: &Path, seed: u64, ops: u64, links: Links) -> io::Result<Result<(), String>> {
    let local_dir = tempfile::tempdir()?;
    let local = LocalBackend::with_root(local_dir.path()).map_err(io::Error::from)?;
    let name = format!("s{seed}");
    let remote_root = server.root().join(&name);
    fs::create_dir(&remote_root)?;
    let root_param = format!("/{name}");
    let sftp = support::connect(server, known, &[("root", root_param.as_str())])?;
    let mut rng = Rng(seed);
    let mut log = Vec::new();
    let trace = std::env::var_os("KARA_DIFF_TRACE").is_some();
    for step in 0..ops {
        let op = place_link(random_op(&mut rng, links), local_dir.path());
        if trace {
            eprintln!("{step:3}: {op:?}");
        }
        let expected = apply(&local, local_dir.path(), &op);
        let got = apply(sftp.as_ref(), &remote_root, &op);
        log.push(format!("{step:3}: {op:?} -> {expected:?}"));
        if expected != got {
            return Ok(Err(format!(
                "seed {seed}, step {step}: {op:?}\n  local: {expected:?}\n  sftp:  {got:?}\nhistory:\n{}",
                log.join("\n")
            )));
        }
    }
    let reference = tree(&local).map_err(io::Error::other)?;
    let remote = tree(sftp.as_ref()).map_err(io::Error::other)?;
    if reference != remote {
        return Ok(Err(format!(
            "seed {seed}: final trees differ\n  local: {reference:?}\n  sftp:  {remote:?}\nhistory:\n{}",
            log.join("\n")
        )));
    }
    let leftovers = server.temporaries().len();
    if leftovers > 0 {
        return Ok(Err(format!("seed {seed}: {leftovers} temporaries left on the server")));
    }
    Ok(Ok(()))
}

fn run_all(options: ServerOptions, links: Links, first_seed: u64) -> io::Result<()> {
    let server = TestServer::start(options)?;
    let client = tempfile::tempdir()?;
    let known = support::trusting_known_hosts(&server, client.path())?;
    let ops = env_number("KARA_DIFF_OPS", 100);
    let seeds: Vec<u64> = match std::env::var("KARA_DIFF_SEED") {
        Ok(one) => vec![one.parse().map_err(io::Error::other)?],
        Err(_) => (first_seed..first_seed + env_number("KARA_DIFF_SEEDS", 40)).collect(),
    };
    for seed in seeds {
        if let Err(divergence) = run_seed(&server, &known, seed, ops, links)? {
            return Err(io::Error::other(divergence));
        }
    }
    Ok(())
}

fn without_posix_rename() -> ServerOptions {
    ServerOptions {
        posix_rename: false,
        fsync: false,
        ..ServerOptions::default()
    }
}

#[test]
fn sftp_and_local_agree_without_links() -> io::Result<()> {
    run_all(ServerOptions::default(), Links::None, 0)
}

#[test]
fn sftp_and_local_agree_with_links_that_point_down() -> io::Result<()> {
    run_all(ServerOptions::default(), Links::Downward, 1_000_000)
}

#[test]
fn sftp_and_local_agree_with_links_anywhere_when_nothing_moves() -> io::Result<()> {
    run_all(ServerOptions::default(), Links::AnywhereNoRename, 2_000_000)
}

#[test]
fn sftp_without_posix_rename_and_local_agree() -> io::Result<()> {
    run_all(without_posix_rename(), Links::Downward, 3_000_000)
}
