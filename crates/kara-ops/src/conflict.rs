//! Resolución de conflictos de nombre y generación de nombres únicos.
//!
//! Conveniencias de referencia: `ground/spec/05-operaciones.md` — «Resolución de
//! conflictos», «Aplicar la misma acción a todos» y «Mantener ambos».
//!
//! Es la capa de **decisión**, no la de ejecución: aquí no se copia ni se mueve
//! nada. Quien toca el disco pregunta qué hacer y esta capa responde.

use std::collections::BTreeMap;

/// Qué choca contra qué. La spec separa estos casos a propósito: no es lo mismo
/// pisar un fichero con otro que pisar una carpeta con un fichero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConflictKind {
    /// Fichero entrante contra fichero existente: el caso corriente.
    FileOverFile,
    /// Entra un fichero donde hay una carpeta. **Nunca** se resuelve solo.
    FileOverDirectory,
    /// Entra una carpeta donde hay un fichero. Tampoco.
    DirectoryOverFile,
    /// Carpeta contra carpeta: aquí sí tiene sentido combinar.
    DirectoryOverDirectory,
}

/// Qué se decide hacer con un conflicto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Pisar lo que hay en el destino.
    Replace,
    /// Dejar el destino como está y seguir con el resto.
    Skip,
    /// Quedarse con los dos, renombrando el entrante.
    KeepBoth,
    /// Fusionar recursivamente. Solo entre carpetas.
    Merge,
    /// Nombre tecleado a mano en el diálogo, como ofrece Dolphin.
    RenameTo(String),
}

impl ConflictKind {
    /// La opción que debe llevar el foco al abrir el diálogo.
    ///
    /// Siempre **la menos destructiva**, que es lo que exige la spec: nunca
    /// `Replace`. Entre tipos distintos ni siquiera se ofrece conservar ambos por
    /// defecto, porque un fichero y una carpeta con el mismo nombre casi siempre
    /// significan que alguien se ha equivocado de destino.
    #[must_use]
    pub fn default_resolution(self) -> Resolution {
        match self {
            Self::FileOverFile | Self::DirectoryOverDirectory => Resolution::Skip,
            Self::FileOverDirectory | Self::DirectoryOverFile => Resolution::Skip,
        }
    }

    /// Acciones que tienen sentido para este tipo de choque.
    ///
    /// `Merge` solo aparece entre carpetas; ofrecerlo entre tipos distintos no
    /// significa nada. `Replace` sigue estando disponible entre tipos distintos,
    /// pero nunca es el defecto: la spec pide que no se sobrescriba una carpeta
    /// con un fichero «sin aviso explícito», no que se prohíba.
    #[must_use]
    pub fn offers(self) -> Vec<Resolution> {
        match self {
            Self::DirectoryOverDirectory => vec![
                Resolution::Skip,
                Resolution::Merge,
                Resolution::KeepBoth,
                Resolution::Replace,
            ],
            _ => vec![Resolution::Skip, Resolution::KeepBoth, Resolution::Replace],
        }
    }

    /// `true` si la acción es aplicable a este tipo de conflicto.
    #[must_use]
    pub fn allows(self, resolution: &Resolution) -> bool {
        match resolution {
            Resolution::RenameTo(_) => true,
            other => self.offers().contains(other),
        }
    }
}

/// Cuántos conflictos se resolvieron de cada manera, para el resumen final que
/// pide la spec.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResolutionCounts {
    pub replaced: usize,
    pub skipped: usize,
    pub kept_both: usize,
    pub merged: usize,
    pub renamed: usize,
}

impl ResolutionCounts {
    pub fn record(&mut self, resolution: &Resolution) {
        match resolution {
            Resolution::Replace => self.replaced += 1,
            Resolution::Skip => self.skipped += 1,
            Resolution::KeepBoth => self.kept_both += 1,
            Resolution::Merge => self.merged += 1,
            Resolution::RenameTo(_) => self.renamed += 1,
        }
    }

    #[must_use]
    pub fn total(&self) -> usize {
        self.replaced + self.skipped + self.kept_both + self.merged + self.renamed
    }
}

/// Decisiones «para todos los conflictos restantes» de una operación en curso.
///
/// No confundir con `kara_fs::ConflictPolicy`, que es otra cosa y vive una capa
/// más abajo: aquel es un ajuste fijo de qué hacer cuando el destino de una
/// restauración está ocupado, y este es el estado vivo de lo que el usuario ha
/// ido respondiendo en un lote. Compartían nombre por accidente.
///
/// Se guardan **por tipo de conflicto**, no una sola global: la spec sugiere
/// poder decidir distinto según el tipo, y aplicar a un choque carpeta-contra-
/// fichero lo que se eligió para dos ficheros sería justo la sorpresa
/// destructiva que se quiere evitar.
#[derive(Debug, Clone, Default)]
pub struct ConflictDecisions {
    blanket: BTreeMap<ConflictKind, Resolution>,
    counts: ResolutionCounts,
}

impl ConflictDecisions {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Qué hacer sin preguntar, o `None` si hay que abrir el diálogo.
    #[must_use]
    pub fn decide(&self, kind: ConflictKind) -> Option<&Resolution> {
        self.blanket.get(&kind)
    }

    /// Marca «hacer esto para todos los conflictos de este tipo».
    ///
    /// Se puede volver a llamar mientras queden conflictos: la spec pide
    /// explícitamente poder cambiar de opinión a mitad.
    pub fn apply_to_all(&mut self, kind: ConflictKind, resolution: Resolution) {
        self.blanket.insert(kind, resolution);
    }

    /// Vuelve a preguntar por este tipo.
    pub fn clear(&mut self, kind: ConflictKind) {
        self.blanket.remove(&kind);
    }

    /// Anota una resolución ya aplicada, para el resumen.
    pub fn record(&mut self, resolution: &Resolution) {
        self.counts.record(resolution);
    }

    #[must_use]
    pub fn counts(&self) -> ResolutionCounts {
        self.counts
    }
}
