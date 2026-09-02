//! Acceso al sistema de ficheros: listado y `stat`, copia, movimiento, enlaces,
//! permisos POSIX y papelera FreeDesktop (`.trashinfo`).
//!
//! Reglas de la capa (ver `CLAUDE.md`):
//! - Cada syscall devuelve `Result`; el error se propaga o se reporta, nunca se traga.
//! - Prohibido `unwrap()` / `expect()` en rutas que toquen ficheros del usuario.
//! - Eliminar va **siempre** a la papelera; el borrado permanente es una operación
//!   distinta y explícita.

#![forbid(unsafe_code)]

pub mod trash;

pub mod listing;

pub use listing::{EntryError, Listing, describe, list_directory, read_hidden_file};

pub mod transfer;

pub use transfer::{
    Transfer, TransferError, TransferOutcome, Transferred, copy_to, create_directory, move_to,
    rename, transfer_batch,
};
