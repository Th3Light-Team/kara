//! `LocalBackend::rename` never overwrites, even when the destination appears
//! between its existence check and the rename itself: the rename must be
//! atomic no-replace (RENAME_NOREPLACE, or link-then-unlink), not
//! check-then-rename. Two renames race onto one name; at most one may win and
//! no file may be lost.

use std::fs;
use std::io;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use kara_fs::LocalBackend;
use kara_vfs::{Backend, BackendErrorKind, RemotePath};

fn rp(text: &str) -> io::Result<RemotePath> {
    RemotePath::parse(text).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}

#[test]
fn two_renames_racing_onto_one_name_never_lose_a_file() -> io::Result<()> {
    let tmp = tempfile::tempdir()?;
    let backend = LocalBackend::with_root(tmp.path()).map_err(io::Error::from)?;
    let target = rp("/target")?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut rounds = 0_u32;
    while Instant::now() < deadline || rounds < 200 {
        rounds += 1;
        fs::write(tmp.path().join("a"), b"from a")?;
        fs::write(tmp.path().join("b"), b"from b")?;
        let barrier = Arc::new(Barrier::new(2));
        let racers: Vec<_> = ["/a", "/b"]
            .into_iter()
            .map(|name| {
                let backend = backend.clone();
                let barrier = Arc::clone(&barrier);
                let from = rp(name);
                let to = target.clone();
                thread::spawn(move || {
                    let from = from?;
                    barrier.wait();
                    Ok::<_, io::Error>(backend.rename(&from, &to))
                })
            })
            .collect();
        let mut wins = 0;
        for racer in racers {
            let outcome = racer
                .join()
                .map_err(|_| io::Error::other("a racer panicked"))??;
            match outcome {
                Ok(()) => wins += 1,
                Err(error) => assert_eq!(
                    error.kind,
                    BackendErrorKind::AlreadyExists,
                    "round {rounds}: the loser must see AlreadyExists: {error:?}"
                ),
            }
        }
        assert_eq!(wins, 1, "round {rounds}: exactly one rename may win");
        let mut contents: Vec<Vec<u8>> = Vec::new();
        for name in ["a", "b", "target"] {
            if let Ok(bytes) = fs::read(tmp.path().join(name)) {
                contents.push(bytes);
            }
        }
        contents.sort();
        assert_eq!(
            contents,
            vec![b"from a".to_vec(), b"from b".to_vec()],
            "round {rounds}: a rename overwrote a file"
        );
        for name in ["a", "b", "target"] {
            let _ = fs::remove_file(tmp.path().join(name));
        }
    }
    Ok(())
}
