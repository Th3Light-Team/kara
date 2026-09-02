//! Política de errores de un lote: reintentar, omitir o cancelar.
//!
//! Conveniencia de referencia: `ground/spec/05-operaciones.md`, «Manejo de
//! errores con reintentar / omitir / cancelar».
//!
//! La regla que gobierna todo el módulo es la del proyecto: **un fallo no aborta
//! el lote**. Omitir sigue con el resto; cancelar para de forma ordenada; y al
//! final siempre hay un resumen de lo que no salió, porque una operación que
//! termina en silencio habiendo fallado a medias es peor que una que falla.

use std::path::PathBuf;

/// Qué hacer ante un elemento que ha fallado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAction {
    /// Volver a intentarlo con este elemento.
    Retry,
    /// Dejarlo y seguir con el resto.
    Skip,
    /// Parar el lote de forma ordenada.
    Cancel,
}

/// Categoría del fallo. Determina qué se ofrece: la spec pide que «sin espacio»
/// y «acceso denegado» no se traten como un error corriente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Permiso denegado. Puede tener arreglo elevando privilegios.
    PermissionDenied,
    /// El fichero está en uso por otra aplicación.
    InUse,
    /// No queda espacio en el destino.
    NoSpace,
    /// El medio se desconectó a mitad.
    MediaGone,
    /// Cualquier otro fallo de E/S.
    Other,
}

impl FailureKind {
    /// `true` si reintentar puede funcionar **sin que el usuario haga nada**.
    ///
    /// Con el medio desconectado no: reintentar en bucle es lo que cuelga una
    /// operación indefinidamente, que es justo lo que la spec prohíbe.
    #[must_use]
    pub fn retry_may_help(self) -> bool {
        matches!(self, Self::InUse | Self::Other)
    }

    /// `true` si tiene sentido ofrecer una acción previa al reintento —liberar
    /// espacio, autenticarse— en vez de un reintento a secas.
    #[must_use]
    pub fn needs_user_action_first(self) -> bool {
        matches!(self, Self::PermissionDenied | Self::NoSpace)
    }
}

/// Un elemento que no se pudo procesar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub path: PathBuf,
    pub kind: FailureKind,
    /// Motivo en lenguaje llano, para enseñárselo a una persona.
    pub reason: String,
}

/// Recuerda las decisiones «para todos» y acumula el resumen final.
#[derive(Debug, Clone, Default)]
pub struct BatchPolicy {
    blanket: Option<FailureAction>,
    cancelled: bool,
    failures: Vec<Failure>,
    skipped: Vec<PathBuf>,
    succeeded: usize,
}

impl BatchPolicy {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Qué hacer sin preguntar, o `None` si hay que abrir el diálogo.
    ///
    /// Una decisión «para todos» de reintentar **no** se aplica a fallos donde
    /// reintentar no puede ayudar: convertiría «Reintentar todos» en un bucle
    /// infinito sobre un medio desconectado.
    #[must_use]
    pub fn decide(&self, failure: &Failure) -> Option<FailureAction> {
        match self.blanket {
            Some(FailureAction::Retry) if !failure.kind.retry_may_help() => None,
            other => other,
        }
    }

    /// Marca «omitir todos» o «reintentar todos».
    pub fn apply_to_all(&mut self, action: FailureAction) {
        self.blanket = Some(action);
    }

    /// Vuelve a preguntar en cada fallo.
    pub fn clear(&mut self) {
        self.blanket = None;
    }

    /// Registra lo que se hizo con un fallo.
    pub fn record(&mut self, failure: Failure, action: FailureAction) {
        match action {
            FailureAction::Skip => {
                self.skipped.push(failure.path.clone());
                self.failures.push(failure);
            }
            FailureAction::Cancel => {
                self.cancelled = true;
                self.failures.push(failure);
            }
            FailureAction::Retry => {}
        }
    }

    pub fn record_success(&mut self) {
        self.succeeded += 1;
    }

    /// `true` si alguien pidió parar. Quien ejecuta debe dejar de coger
    /// elementos, sin deshacer lo ya hecho.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// El resumen que se enseña al terminar.
    #[must_use]
    pub fn report(&self) -> BatchReport {
        BatchReport {
            succeeded: self.succeeded,
            skipped: self.skipped.clone(),
            failures: self.failures.clone(),
            cancelled: self.cancelled,
        }
    }
}

/// Cómo acabó el lote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchReport {
    pub succeeded: usize,
    /// Rutas omitidas, para poder listarlas y copiarlas al portapapeles.
    pub skipped: Vec<PathBuf>,
    pub failures: Vec<Failure>,
    pub cancelled: bool,
}

impl BatchReport {
    /// `true` si todo salió bien y no hay nada que contarle al usuario.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.failures.is_empty() && !self.cancelled
    }
}
