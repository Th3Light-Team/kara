//! Cola de operaciones: varias transferencias vistas y gobernadas juntas.
//!
//! Conveniencia de referencia: `ground/spec/05-operaciones.md`, «Operaciones
//! concurrentes agrupadas y cola de transferencia».
//!
//! # Encolar en vez de paralelizar
//!
//! El defecto es [`Concurrency::Serial`]. Lanzar tres copias a la vez sobre el
//! mismo disco no las hace ir más rápido: se estorban y las tres tardan más. La
//! spec señala que las colas explícitas de Total Commander y Directory Opus son
//! «muy valoradas por usuarios avanzados», así que ese es el comportamiento por
//! defecto y el paralelismo se pide a mano.
//!
//! Aquí no se ejecuta nada: esto ordena, cuenta y decide qué toca. Quien mueve
//! bytes pregunta [`Queue::next_runnable`] y va informando con
//! [`Queue::meter_mut`].

use std::path::PathBuf;

use crate::progress::{Eta, Meter, Phase};

/// Cómo se reparten las operaciones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Concurrency {
    /// Una detrás de otra. El defecto, y la razón está en las notas del módulo.
    #[default]
    Serial,
    /// Hasta `n` a la vez.
    Parallel(usize),
}

/// Qué hace una operación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Copy,
    Move,
    Trash,
    Delete,
}

impl Kind {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Copy => "Copiando",
            Self::Move => "Moviendo",
            Self::Trash => "Enviando a la papelera",
            Self::Delete => "Eliminando",
        }
    }
}

/// En qué estado está una operación de la cola.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// Esperando turno.
    Queued,
    Active,
    Paused,
    Done,
    /// Cancelada por quien la lanzó. Cancelar una **no** toca a las demás.
    Cancelled,
}

/// Identificador estable de una operación dentro de la cola.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(pub u64);

/// Una transferencia encolada.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: JobId,
    pub kind: Kind,
    pub sources: Vec<PathBuf>,
    pub destination: Option<PathBuf>,
    pub state: JobState,
    pub meter: Meter,
}

impl Job {
    /// Lo que se pinta como título de la fila: «Copiando 3 elementos».
    #[must_use]
    pub fn title(&self) -> String {
        match self.sources.len() {
            1 => format!(
                "{} {}",
                self.kind.label(),
                self.sources[0]
                    .file_name()
                    .map_or_else(|| self.sources[0].to_string_lossy(), |n| n.to_string_lossy())
            ),
            n => format!("{} {n} elementos", self.kind.label()),
        }
    }
}

/// La cola completa.
#[derive(Debug, Default)]
pub struct Queue {
    jobs: Vec<Job>,
    next_id: u64,
    concurrency: Concurrency,
}

impl Queue {
    #[must_use]
    pub fn new(concurrency: Concurrency) -> Self {
        Self {
            jobs: Vec::new(),
            next_id: 0,
            concurrency,
        }
    }

    /// Encola una operación y devuelve su identificador.
    pub fn push(&mut self, kind: Kind, sources: Vec<PathBuf>, destination: Option<PathBuf>) -> JobId {
        let id = JobId(self.next_id);
        self.next_id += 1;
        self.jobs.push(Job {
            id,
            kind,
            sources,
            destination,
            state: JobState::Queued,
            meter: Meter::measuring(),
        });
        id
    }

    #[must_use]
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    #[must_use]
    pub fn get(&self, id: JobId) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    /// El contador de una operación, para ir informando del avance.
    pub fn meter_mut(&mut self, id: JobId) -> Option<&mut Meter> {
        self.jobs.iter_mut().find(|j| j.id == id).map(|j| &mut j.meter)
    }

    #[must_use]
    pub fn active(&self) -> usize {
        self.jobs.iter().filter(|j| j.state == JobState::Active).count()
    }

    /// Cuántas quedan por terminar, encoladas o en curso.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| matches!(j.state, JobState::Queued | JobState::Active | JobState::Paused))
            .count()
    }

    /// La siguiente que puede arrancar según la concurrencia, o `None` si no
    /// toca todavía. Marcarla activa es responsabilidad de quien la ejecuta.
    #[must_use]
    pub fn next_runnable(&self) -> Option<JobId> {
        let limit = match self.concurrency {
            Concurrency::Serial => 1,
            Concurrency::Parallel(n) => n.max(1),
        };
        if self.active() >= limit {
            return None;
        }
        self.jobs
            .iter()
            .find(|j| j.state == JobState::Queued)
            .map(|j| j.id)
    }

    /// Marca una operación como arrancada.
    pub fn start(&mut self, id: JobId) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            job.state = JobState::Active;
        }
    }

    /// Pausa **solo** esa operación: la spec pide poder pausar cada una por
    /// separado cuando hay varias.
    pub fn pause(&mut self, id: JobId) {
        if let Some(job) = self
            .jobs
            .iter_mut()
            .find(|j| j.id == id && j.state == JobState::Active)
        {
            job.state = JobState::Paused;
            job.meter.pause();
        }
    }

    pub fn resume(&mut self, id: JobId) {
        if let Some(job) = self
            .jobs
            .iter_mut()
            .find(|j| j.id == id && j.state == JobState::Paused)
        {
            job.state = JobState::Active;
            job.meter.resume();
        }
    }

    /// Cancela una. Las demás siguen: cancelar una no puede afectar al resto.
    pub fn cancel(&mut self, id: JobId) {
        if let Some(job) = self
            .jobs
            .iter_mut()
            .find(|j| j.id == id && j.state != JobState::Done)
        {
            job.state = JobState::Cancelled;
        }
    }

    pub fn finish(&mut self, id: JobId) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            job.state = JobState::Done;
            job.meter.finish();
        }
    }

    /// Quita lo ya terminado o cancelado, para que la ventana no crezca sin fin.
    pub fn prune(&mut self) {
        self.jobs
            .retain(|j| !matches!(j.state, JobState::Done | JobState::Cancelled));
    }

    /// Resumen combinado que pide la spec: «2 operaciones en curso».
    #[must_use]
    pub fn summary(&self) -> String {
        match self.pending() {
            0 => "Sin operaciones".to_string(),
            1 => self
                .jobs
                .iter()
                .find(|j| j.state != JobState::Done && j.state != JobState::Cancelled)
                .map_or_else(|| "1 operación".to_string(), Job::title),
            n => format!("{n} operaciones en curso"),
        }
    }

    /// Progreso combinado, promediando el de cada operación pendiente.
    ///
    /// `None` mientras alguna siga midiendo: presentar un porcentaje global
    /// antes de conocer todos los totales sería inventárselo.
    #[must_use]
    pub fn combined_fraction(&self) -> Option<f64> {
        let pending: Vec<&Job> = self
            .jobs
            .iter()
            .filter(|j| matches!(j.state, JobState::Queued | JobState::Active | JobState::Paused))
            .collect();
        if pending.is_empty() {
            return None;
        }
        if pending.iter().any(|j| j.meter.phase() == Phase::Measuring) {
            return None;
        }
        let sum: f64 = pending.iter().filter_map(|j| j.meter.fraction()).sum();
        Some(sum / pending.len() as f64)
    }

    /// La ETA de lo que queda: la mayor de las pendientes en serie, ya que se
    /// ejecutan una detrás de otra.
    #[must_use]
    pub fn combined_eta(&self) -> Eta {
        let mut worst: Option<std::time::Duration> = None;
        for job in &self.jobs {
            match job.meter.eta() {
                Eta::Remaining(d) => worst = Some(worst.map_or(d, |w: std::time::Duration| w.max(d))),
                Eta::Unknown => return Eta::Unknown,
                _ => {}
            }
        }
        worst.map_or(Eta::Unknown, Eta::Remaining)
    }
}
