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

pub mod backend;

pub use backend::{LocalBackend, LocalPathError};

pub mod clipboard;

pub mod listing;

pub mod icons;

pub mod mime;

pub mod places;

pub mod props;

pub mod settings;

pub mod thumbnails;

pub use thumbnails::{ThumbnailSize, Thumbnails};

pub mod uri;

pub use uri::file_uri;

pub use listing::{EntryError, Listing, describe, list_directory, read_hidden_file};

pub use mime::{MimeDatabase, MimeDescriptions};

pub use places::{Place, PlaceKind, quick_access, this_computer};

pub use settings::{
    LoadOutcome, LoadedSettings, Sections, Settings, SettingsError, default_path, load, parse,
    save, serialize,
};

pub mod transfer;

pub use transfer::{
    Transfer, TransferError, TransferOutcome, Transferred, copy_to, create_directory, move_to,
    rename, transfer_batch,
};
