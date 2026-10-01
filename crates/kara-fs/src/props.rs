//! What the Properties dialog shows about one path.
//!
//! Reference convenience: `ground/spec/05-operaciones.md`, «Propiedades» and
//! «Gestión de permisos». This is the read side: Kara shows the POSIX owner,
//! group and mode but does not change them yet.
//!
//! The folder's recursive size is **not** here: it is a walk of the whole tree
//! and belongs to `kara-index`, which can report progress and be cancelled.

use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Facts about one path, taken without following a final symlink.
#[derive(Debug, Clone)]
pub struct Properties {
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// Where a symlink points, as written in the link.
    pub link_target: Option<PathBuf>,
    /// Logical size in bytes. For a folder this is the folder entry itself,
    /// not its contents.
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    /// The permission bits, `0o755` style, without the file-type bits.
    pub mode: u32,
    pub owner: Owner,
    pub group: Owner,
}

/// A user or group: the name if the system knows it, the number always.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub id: u32,
    pub name: Option<String>,
}

impl Owner {
    /// «ana» or, when the system has no name for it, the bare number.
    #[must_use]
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.id.to_string())
    }
}

/// Reads the properties of `path`.
pub fn properties(path: &Path) -> io::Result<Properties> {
    let meta = std::fs::symlink_metadata(path)?;
    let is_symlink = meta.file_type().is_symlink();
    // The label of a link says where it points; whether that is a folder is a
    // different question and the link itself is what is being inspected.
    let link_target = if is_symlink {
        std::fs::read_link(path).ok()
    } else {
        None
    };

    Ok(Properties {
        path: path.to_path_buf(),
        is_dir: meta.is_dir(),
        is_symlink,
        link_target,
        size: meta.len(),
        modified: meta.modified().ok(),
        created: meta.created().ok(),
        accessed: meta.accessed().ok(),
        mode: meta.permissions().mode() & 0o7777,
        owner: Owner {
            id: meta.uid(),
            name: name_of(Path::new("/etc/passwd"), meta.uid()),
        },
        group: Owner {
            id: meta.gid(),
            name: name_of(Path::new("/etc/group"), meta.gid()),
        },
    })
}

/// The name for `id` in a `passwd`- or `group`-style file, if it has one.
fn name_of(file: &Path, id: u32) -> Option<String> {
    parse_names(&std::fs::read_to_string(file).ok()?, id)
}

/// `name:x:id:…` lines. A line that does not fit is skipped, not an error: the
/// file is the system's and may carry things this does not understand.
#[must_use]
pub fn parse_names(text: &str, id: u32) -> Option<String> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let _password = fields.next()?;
            let found: u32 = fields.next()?.parse().ok()?;
            (found == id && !name.is_empty()).then(|| name.to_string())
        })
}

/// `rwxr-xr-x` for the nine permission bits of `mode`.
#[must_use]
pub fn mode_string(mode: u32) -> String {
    let bit = |mask: u32, ch: char| if mode & mask != 0 { ch } else { '-' };
    let mut out = String::with_capacity(9);
    for (shift, special, special_on, special_off) in [(6, 0o4000, 's', 'S'), (3, 0o2000, 's', 'S'), (0, 0o1000, 't', 'T')] {
        out.push(bit(0o4 << shift, 'r'));
        out.push(bit(0o2 << shift, 'w'));
        let executable = mode & (0o1 << shift) != 0;
        out.push(match (mode & special != 0, executable) {
            (true, true) => special_on,
            (true, false) => special_off,
            (false, true) => 'x',
            (false, false) => '-',
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nine_bits_read_like_ls() {
        assert_eq!(mode_string(0o644), "rw-r--r--");
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_string(0o000), "---------");
        assert_eq!(mode_string(0o4755), "rwsr-xr-x");
        assert_eq!(mode_string(0o1777), "rwxrwxrwt");
        // Setuid without execute is the capital S that `ls` prints.
        assert_eq!(mode_string(0o4644), "rwSr--r--");
    }

    #[test]
    fn names_come_from_the_third_field() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\n# comment\nana:x:1000:1000::/home/ana:/bin/zsh\nbroken line\n";
        assert_eq!(parse_names(passwd, 1000).as_deref(), Some("ana"));
        assert_eq!(parse_names(passwd, 0).as_deref(), Some("root"));
        assert_eq!(parse_names(passwd, 4242), None);
    }

    #[test]
    fn a_file_reports_its_size_and_mode() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "hello")?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640))?;

        let found = properties(&file)?;
        assert_eq!(found.size, 5);
        assert_eq!(found.mode, 0o640);
        assert!(!found.is_dir && !found.is_symlink);
        Ok(())
    }

    #[test]
    fn a_symlink_is_described_as_the_link_not_its_target() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("real");
        std::fs::create_dir(&target)?;
        let link = dir.path().join("link");
        std::os::unix::fs::symlink("real", &link)?;

        let found = properties(&link)?;
        assert!(found.is_symlink);
        assert!(!found.is_dir);
        assert_eq!(found.link_target.as_deref(), Some(Path::new("real")));
        Ok(())
    }

    #[test]
    fn a_missing_path_is_an_error() {
        assert!(properties(Path::new("/no/such/kara/path")).is_err());
    }
}
