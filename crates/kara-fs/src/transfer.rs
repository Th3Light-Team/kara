//! Copiar, mover y renombrar.
//!
//! Conveniencias de referencia: `ground/spec/05-operaciones.md` — «Cortar,
//! copiar y pegar», «Resolución de conflictos», «Mantener ambos» y «Manejo de
//! errores»— y `02-seleccion.md`, «Renombrar inline».
//!
//! # Invariantes
//!
//! - **Mover dentro del mismo volumen es un `rename(2)`**, sin recorrer bytes.
//!   Solo entre volúmenes se copia y se borra, y entonces el borrado del origen
//!   ocurre **después** de que la copia haya terminado entera: si algo falla a
//!   mitad, el original sigue ahí.
//! - **Nunca se pisa nada sin que alguien lo haya pedido.** El defecto es
//!   [`ConflictPolicy::Fail`]; sobrescribir exige decirlo.
//! - **Un fallo no aborta el lote.** [`transfer_batch`] devuelve un resumen, no
//!   un `Result`.
//! - **El último componente nunca se sigue**: mover un enlace simbólico mueve el
//!   enlace, no su destino.

use std::ffi::OsStr;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use kara_core::unique_name;

pub use crate::trash::ConflictPolicy;

/// Qué falló y dónde.
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// El destino ya existe y la política era [`ConflictPolicy::Fail`].
    #[error("{0}: the destination already exists")]
    DestinationExists(PathBuf),
    /// Mover una carpeta dentro de sí misma dejaría un árbol inalcanzable.
    #[error("{0}: cannot move a directory into itself")]
    IntoItself(PathBuf),
    /// Un nombre vacío, con `/` o igual a `.` o `..` no nombra nada.
    #[error("{0:?}: not a usable file name")]
    InvalidName(std::ffi::OsString),
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> TransferError + '_ {
    move |source| TransferError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Lo que ocurrió con un elemento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transferred {
    pub source: PathBuf,
    pub destination: PathBuf,
    /// `None` en un `rename(2)`; `Some(n)` solo si hubo que copiar bytes.
    pub bytes_copied: Option<u64>,
    /// `true` si se conservaron ambos y el destino lleva sufijo.
    pub renamed_to_keep_both: bool,
}

/// Resumen de un lote. Nunca es un `Result`: un fallo no aborta el resto.
#[derive(Debug, Default)]
pub struct TransferOutcome {
    pub done: Vec<Transferred>,
    pub failed: Vec<(PathBuf, TransferError)>,
}

/// Comprueba que `name` puede ser el nombre de un fichero.
fn check_name(name: &OsStr) -> Result<(), TransferError> {
    let bytes = name.as_encoded_bytes();
    if bytes.is_empty() || bytes.contains(&b'/') || name == "." || name == ".." {
        return Err(TransferError::InvalidName(name.to_os_string()));
    }
    Ok(())
}

/// Resuelve el destino final aplicando la política de conflictos.
///
/// Devuelve la ruta y si hubo que renombrar. Con [`ConflictPolicy::Overwrite`]
/// se devuelve la ruta tal cual: pisar es cosa de quien mueve.
fn resolve_destination(
    candidate: PathBuf,
    policy: ConflictPolicy,
) -> Result<(PathBuf, bool), TransferError> {
    if std::fs::symlink_metadata(&candidate).is_err() {
        return Ok((candidate, false));
    }
    match policy {
        ConflictPolicy::Fail => Err(TransferError::DestinationExists(candidate)),
        ConflictPolicy::Overwrite => Ok((candidate, false)),
        ConflictPolicy::KeepBoth => {
            let parent = candidate.parent().unwrap_or(Path::new("."));
            let name = candidate
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let free = unique_name(&name, |candidate_name| {
                std::fs::symlink_metadata(parent.join(candidate_name)).is_ok()
            });
            Ok((parent.join(free), true))
        }
    }
}

/// `true` si ambas rutas viven en el mismo dispositivo.
fn same_device(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

/// Renombra en el sitio, sin cambiar de carpeta.
///
/// Es la operación de F2. La extensión no se protege aquí: quién la conserva al
/// editar solo el cuerpo es decisión de la vista, que sabe si las está ocultando.
pub fn rename(path: &Path, new_name: &OsStr, policy: ConflictPolicy) -> Result<PathBuf, TransferError> {
    check_name(new_name)?;
    let parent = path.parent().unwrap_or(Path::new("."));
    let (destination, _) = resolve_destination(parent.join(new_name), policy)?;
    if destination == path {
        return Ok(destination);
    }
    std::fs::rename(path, &destination).map_err(io(path))?;
    Ok(destination)
}

/// Crea una carpeta, resolviendo el conflicto si el nombre está ocupado.
pub fn create_directory(
    parent: &Path,
    name: &OsStr,
    policy: ConflictPolicy,
) -> Result<PathBuf, TransferError> {
    check_name(name)?;
    let (destination, _) = resolve_destination(parent.join(name), policy)?;
    std::fs::create_dir(&destination).map_err(io(&destination))?;
    Ok(destination)
}

/// Mueve `source` dentro de `dest_dir`.
pub fn move_to(
    source: &Path,
    dest_dir: &Path,
    policy: ConflictPolicy,
) -> Result<Transferred, TransferError> {
    let name = source
        .file_name()
        .ok_or_else(|| TransferError::InvalidName(source.as_os_str().to_os_string()))?;
    guard_into_itself(source, dest_dir)?;
    let (destination, renamed) = resolve_destination(dest_dir.join(name), policy)?;

    if same_device(source, dest_dir) {
        if policy == ConflictPolicy::Overwrite {
            remove_tree(&destination);
        }
        std::fs::rename(source, &destination).map_err(io(source))?;
        return Ok(Transferred {
            source: source.to_path_buf(),
            destination,
            bytes_copied: None,
            renamed_to_keep_both: renamed,
        });
    }

    // Entre volúmenes: copiar entero y solo entonces borrar el original.
    let bytes = copy_tree(source, &destination, policy)?;
    remove_tree(source);
    Ok(Transferred {
        source: source.to_path_buf(),
        destination,
        bytes_copied: Some(bytes),
        renamed_to_keep_both: renamed,
    })
}

/// Copia `source` dentro de `dest_dir`, dejando el original donde está.
pub fn copy_to(
    source: &Path,
    dest_dir: &Path,
    policy: ConflictPolicy,
) -> Result<Transferred, TransferError> {
    let name = source
        .file_name()
        .ok_or_else(|| TransferError::InvalidName(source.as_os_str().to_os_string()))?;
    guard_into_itself(source, dest_dir)?;
    let (destination, renamed) = resolve_destination(dest_dir.join(name), policy)?;
    let bytes = copy_tree(source, &destination, policy)?;
    Ok(Transferred {
        source: source.to_path_buf(),
        destination,
        bytes_copied: Some(bytes),
        renamed_to_keep_both: renamed,
    })
}

/// Copiar o mover una carpeta dentro de sí misma dejaría el árbol inalcanzable
/// o entraría en recursión infinita.
fn guard_into_itself(source: &Path, dest_dir: &Path) -> Result<(), TransferError> {
    let (Ok(s), Ok(d)) = (source.canonicalize(), dest_dir.canonicalize()) else {
        return Ok(());
    };
    if d == s || d.starts_with(&s) {
        return Err(TransferError::IntoItself(source.to_path_buf()));
    }
    Ok(())
}

/// Copia recursivamente. Los enlaces simbólicos se recrean como enlaces, no se
/// siguen: seguirlos duplicaría el destino y con un enlace a un ancestro no
/// terminaría nunca.
fn copy_tree(source: &Path, destination: &Path, policy: ConflictPolicy) -> Result<u64, TransferError> {
    let metadata = std::fs::symlink_metadata(source).map_err(io(source))?;

    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(source).map_err(io(source))?;
        if policy == ConflictPolicy::Overwrite {
            remove_tree(destination);
        }
        std::os::unix::fs::symlink(&target, destination).map_err(io(destination))?;
        return Ok(0);
    }

    if metadata.is_dir() {
        if std::fs::symlink_metadata(destination).is_err() {
            std::fs::create_dir(destination).map_err(io(destination))?;
        }
        let mut total = 0;
        for entry in std::fs::read_dir(source).map_err(io(source))? {
            let entry = entry.map_err(io(source))?;
            total += copy_tree(&entry.path(), &destination.join(entry.file_name()), policy)?;
        }
        return Ok(total);
    }

    if policy == ConflictPolicy::Overwrite {
        remove_tree(destination);
    }
    std::fs::copy(source, destination).map_err(io(source))
}

/// Borra una ruta, sea fichero, enlace o carpeta. Los fallos se ignoran a
/// propósito: se llama justo antes de escribir encima, y si el borrado no salió
/// el escritor fallará a continuación con un error mejor.
fn remove_tree(path: &Path) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

/// Qué hacer con cada elemento del lote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Move,
    Copy,
}

/// Aplica `operation` a cada ruta. Un fallo se anota y el lote continúa.
pub fn transfer_batch(
    sources: &[PathBuf],
    dest_dir: &Path,
    operation: Transfer,
    policy: ConflictPolicy,
) -> TransferOutcome {
    let mut outcome = TransferOutcome::default();
    for source in sources {
        let result = match operation {
            Transfer::Move => move_to(source, dest_dir, policy),
            Transfer::Copy => copy_to(source, dest_dir, policy),
        };
        match result {
            Ok(done) => outcome.done.push(done),
            Err(error) => outcome.failed.push((source.clone(), error)),
        }
    }
    outcome
}
