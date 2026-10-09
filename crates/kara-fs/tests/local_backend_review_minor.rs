//! Minor items from the LocalBackend review: a root that is a symlink is
//! described as the directory it leads to, `open_read` never opens anything
//! but a regular file, and the descriptor it hands out is blocking again.

mod local_backend_support;

use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{OpenOptionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use kara_core::EntryKind;
use kara_fs::LocalBackend;
use kara_vfs::{Backend, BackendErrorKind, RemotePath};
use local_backend_support::{assert_err, err_of, mkfifo, rooted, rp, tmp_tempdir};

fn set_mtime(path: &Path, secs: i64) -> io::Result<()> {
    let time = rustix::fs::Timespec {
        tv_sec: secs,
        tv_nsec: 0,
    };
    let times = rustix::fs::Timestamps {
        last_access: time,
        last_modification: time,
    };
    rustix::fs::utimensat(
        rustix::fs::CWD,
        path,
        &times,
        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
    )?;
    Ok(())
}

#[test]
fn the_root_given_as_a_symlink_is_described_as_its_directory() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let real = tmp.path().join("real");
    fs::create_dir(&real)?;
    let link = tmp.path().join("link");
    symlink(&real, &link)?;
    set_mtime(&real, 1_000_000_000)?;
    set_mtime(&link, 1_500_000_000)?;

    let entry = rooted(&link)?.stat(&RemotePath::root())?;

    assert_eq!(entry.kind, EntryKind::Directory);
    assert!(!entry.is_symlink, "the drive is the directory, not the link");
    assert!(!entry.symlink_broken);
    assert_eq!(entry.size, None);
    assert_eq!(entry.display, "/");
    let expected = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    assert_eq!(
        entry.modified,
        Some(expected),
        "times come from the directory"
    );
    Ok(())
}

#[test]
fn a_plain_root_still_reads_its_own_times() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let real = tmp.path().join("real");
    fs::create_dir(&real)?;
    set_mtime(&real, 1_234_567_890)?;
    let entry = rooted(&real)?.stat(&RemotePath::root())?;
    assert!(!entry.is_symlink);
    assert_eq!(
        entry.modified,
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_234_567_890))
    );
    Ok(())
}

#[test]
fn open_read_of_a_fifo_does_not_wake_a_blocked_writer() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let fifo = tmp.path().join("p");
    mkfifo(&fifo)?;
    let (tx, rx) = mpsc::channel();
    let writer_path = fifo.clone();
    std::thread::spawn(move || {
        // Blocks until somebody opens the read end.
        let opened = OpenOptions::new().write(true).open(&writer_path);
        let _ = tx.send(opened.is_ok());
    });
    std::thread::sleep(Duration::from_millis(150));

    let backend = rooted(tmp.path())?;
    let path = rp("/p")?;
    let error = err_of(backend.open_read(&path, 0), "open_read of a FIFO");
    assert_err(&error, BackendErrorKind::Unsupported, &path, "FIFO");

    assert!(
        rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "open_read opened the FIFO: the blocked writer woke up"
    );
    // Let the writer go.
    let _read_end = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits().cast_signed())
        .open(&fifo)?;
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).ok(), Some(true));
    Ok(())
}

#[test]
fn open_read_of_a_device_is_unsupported() -> io::Result<()> {
    let backend = LocalBackend::system();
    let path = rp("/dev/null")?;
    let error = err_of(backend.open_read(&path, 0), "open_read of /dev/null");
    assert_err(&error, BackendErrorKind::Unsupported, &path, "char device");
    Ok(())
}

/// The status flags of the descriptor of this process open on `target`.
fn flags_of_descriptor_on(target: &Path) -> io::Result<Option<u32>> {
    for item in fs::read_dir("/proc/self/fd")? {
        let fd = item?.file_name();
        let Ok(link) = fs::read_link(PathBuf::from("/proc/self/fd").join(&fd)) else {
            continue;
        };
        if link != target {
            continue;
        }
        let info = fs::read_to_string(PathBuf::from("/proc/self/fdinfo").join(&fd))?;
        for line in info.lines() {
            if let Some(octal) = line.strip_prefix("flags:") {
                let flags = u32::from_str_radix(octal.trim(), 8)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                return Ok(Some(flags));
            }
        }
    }
    Ok(None)
}

#[test]
fn the_reader_is_blocking_once_the_file_is_known_to_be_regular() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let file = tmp.path().join("unique-for-fdinfo");
    fs::write(&file, b"0123456789")?;
    let backend = rooted(tmp.path())?;
    let mut reader = backend.open_read(&rp("/unique-for-fdinfo")?, 3)?;

    let flags = flags_of_descriptor_on(&file)?;
    let nonblock = rustix::fs::OFlags::NONBLOCK.bits();
    match flags {
        Some(flags) => assert_eq!(flags & nonblock, 0, "O_NONBLOCK is still set: {flags:o}"),
        None => panic!("no descriptor of this process is open on {file:?}"),
    }
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest)?;
    assert_eq!(rest, b"3456789");
    Ok(())
}
