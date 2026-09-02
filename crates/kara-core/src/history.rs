//! Navigation history: the back/forward stack of a tab.
//!
//! Conveniencia de referencia: `ground/spec/01-navegacion.md`, «Atrás y Adelante».
//! Es lógica pura — no toca el disco y no sabe si una ruta existe; quien navega se
//! lo cuenta con [`History::invalidate`].
//!
//! # Decisiones de diseño
//!
//! - **La selección se guarda por nombre, no por índice.** Los índices no
//!   sobreviven a un re-listado: basta que alguien cree un fichero mientras estás
//!   fuera para que el índice 7 sea otra cosa al volver. La spec pide restaurar
//!   *la selección*, no la posición en una lista.
//! - **`scroll` es opaco para esta capa.** Guardamos el `f64` que nos dé la vista y
//!   se lo devolvemos tal cual; `kara-core` no sabe de píxeles ni de filas.
//! - **Las entradas inválidas se marcan, no se borran.** Quitarlas renumeraría la
//!   pila y rompería la relación entre atrás y adelante. `back`/`forward` las
//!   saltan, que es la lectura útil de «saltar a la más cercana disponible».

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Lo que hay que restaurar al volver a una ubicación ya visitada.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewState {
    /// Nombres seleccionados. Por nombre, no por índice: ver las notas del módulo.
    pub selection: BTreeSet<OsString>,
    /// Elemento con foco, que debe quedar visible al restaurar.
    pub focused: Option<OsString>,
    /// Posición de desplazamiento, opaca para esta capa.
    pub scroll: f64,
}

impl ViewState {
    /// The state a parent folder should be restored with after going up: the
    /// folder we came from selected and focused, so the user keeps their bearings.
    ///
    /// Spec 01-navegacion.md, «Subir un nivel»: "Al subir se deja
    /// seleccionada/resaltada y con scroll visible la carpeta de la que se venía".
    #[must_use]
    pub fn selecting(name: impl Into<OsString>) -> Self {
        let name = name.into();
        Self {
            selection: BTreeSet::from([name.clone()]),
            focused: Some(name),
            scroll: 0.0,
        }
    }
}

/// Una ubicación visitada, con el estado con que se dejó.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub path: PathBuf,
    pub state: ViewState,
    /// `false` cuando quien navega ha comprobado que la ruta ya no existe.
    pub valid: bool,
}

/// Pila de historial de una pestaña. El historial es independiente por pestaña,
/// así que cada una tiene la suya.
#[derive(Debug, Clone)]
pub struct History {
    entries: Vec<HistoryEntry>,
    cursor: usize,
}

impl History {
    /// Arranca el historial en `path`, que queda como ubicación actual.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            entries: vec![HistoryEntry {
                path: path.into(),
                state: ViewState::default(),
                valid: true,
            }],
            cursor: 0,
        }
    }

    /// La entrada actual. Nunca falla: la pila jamás queda vacía.
    #[must_use]
    pub fn current(&self) -> &HistoryEntry {
        &self.entries[self.cursor]
    }

    #[must_use]
    pub fn current_path(&self) -> &Path {
        &self.entries[self.cursor].path
    }

    /// Guarda el estado de la vista actual. Llámalo *antes* de navegar a otro
    /// sitio, o lo que había se pierde.
    pub fn set_state(&mut self, state: ViewState) {
        self.entries[self.cursor].state = state;
    }

    /// Navega a `path`, truncando la pila de «adelante».
    ///
    /// Volver a la ubicación en la que ya estás no apila nada ni pierde su estado:
    /// re-listar una carpeta no es un movimiento de historial.
    pub fn visit(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        if self.entries[self.cursor].path == path {
            return;
        }
        self.entries.truncate(self.cursor + 1);
        self.entries.push(HistoryEntry {
            path,
            state: ViewState::default(),
            valid: true,
        });
        self.cursor = self.entries.len() - 1;
    }

    /// Marca como no válidas todas las entradas de `path`. La actual también puede
    /// quedar marcada: quien navega decide entonces adónde ir.
    pub fn invalidate(&mut self, path: &Path) {
        for entry in &mut self.entries {
            if entry.path == path {
                entry.valid = false;
            }
        }
    }

    fn previous_valid(&self) -> Option<usize> {
        self.entries[..self.cursor]
            .iter()
            .rposition(|entry| entry.valid)
    }

    fn next_valid(&self) -> Option<usize> {
        self.entries[self.cursor + 1..]
            .iter()
            .position(|entry| entry.valid)
            .map(|offset| self.cursor + 1 + offset)
    }

    /// `true` si «Atrás» llevaría a alguna parte. Es lo que decide si el botón se
    /// muestra atenuado, así que cuenta entradas válidas, no posiciones.
    #[must_use]
    pub fn can_go_back(&self) -> bool {
        self.previous_valid().is_some()
    }

    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        self.next_valid().is_some()
    }

    /// Retrocede a la entrada válida más cercana, saltando las que ya no existen.
    /// Devuelve `None` —sin mover el cursor— si no queda ninguna.
    pub fn back(&mut self) -> Option<&HistoryEntry> {
        let target = self.previous_valid()?;
        self.cursor = target;
        Some(&self.entries[self.cursor])
    }

    /// Avanza a la entrada válida más cercana. Simétrica de [`History::back`].
    pub fn forward(&mut self) -> Option<&HistoryEntry> {
        let target = self.next_valid()?;
        self.cursor = target;
        Some(&self.entries[self.cursor])
    }

    /// Número de entradas, válidas o no.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Siempre `false`: la pila nace con una entrada y nunca se vacía. Existe
    /// porque clippy lo pide junto a `len`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Las entradas en orden de visita, para el desplegable de historial.
    #[must_use]
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }
}
