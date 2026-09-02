//! Type-ahead: saltar dentro de la carpeta escribiendo el principio del nombre.
//!
//! Conveniencia de referencia: `ground/spec/04-busqueda.md`, «Type-ahead find».
//!
//! No confundir con la caja de búsqueda: type-ahead **solo salta dentro de lo
//! listado en la carpeta actual** y nunca recurre en subcarpetas.
//!
//! # El reinicio del buffer no se mide aquí
//!
//! La spec pide que el buffer caduque tras ~1 s sin teclear. Medir ese tiempo es
//! ambiental, así que la caducidad la decide quien recibe las pulsaciones
//! llamando a [`TypeAhead::clear`]. Esta capa se queda con la parte que sí es
//! dominio: qué elemento seleccionar dado un buffer.

use crate::entry::FileEntry;
use crate::sort::fold_for_match;

/// Buffer de prefijo del type-ahead.
#[derive(Debug, Clone, Default)]
pub struct TypeAhead {
    buffer: String,
}

impl TypeAhead {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Lo tecleado hasta ahora, tal cual.
    #[must_use]
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Vacía el buffer: lo llama Esc y también la caducidad por inactividad.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Añade un carácter y devuelve a qué índice hay que saltar.
    ///
    /// `current` es el elemento seleccionado ahora, y solo importa en el modo de
    /// ciclo. Devuelve `None` si nada coincide, en cuyo caso la selección se
    /// queda donde estaba: la spec no pide deshacer nada por una tecla de más.
    ///
    /// # Los dos modos
    ///
    /// - **Afinar**: teclear `i`, `n`, `f` busca el primero que empiece por
    ///   `inf`.
    /// - **Ciclar**: pulsar la misma letra repetidamente recorre los elementos
    ///   que empiezan por ella, que es lo que hacen Windows y Dolphin. Se
    ///   detecta porque todo el buffer es el mismo carácter; entonces el prefijo
    ///   efectivo es ese carácter una sola vez.
    ///
    /// Ambos modos comparan sin distinción de mayúsculas.
    pub fn push(&mut self, ch: char, entries: &[FileEntry], current: Option<usize>) -> Option<usize> {
        self.buffer.push(ch);

        let cycling = self.buffer.chars().count() > 1
            && self.buffer.chars().all(|c| c == self.buffer.chars().next().unwrap_or(c));

        let needle = if cycling {
            fold_for_match(&ch.to_string())
        } else {
            fold_for_match(&self.buffer)
        };

        let starts = |entry: &FileEntry| fold_for_match(&entry.display).starts_with(&needle);

        if cycling {
            // Desde el siguiente al actual, dando la vuelta.
            let start = current.map_or(0, |i| i + 1);
            let len = entries.len();
            (0..len)
                .map(|offset| (start + offset) % len)
                .find(|&i| starts(&entries[i]))
        } else {
            entries.iter().position(starts)
        }
    }
}
