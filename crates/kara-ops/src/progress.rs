//! Medida de progreso: velocidad suavizada, tiempo restante y fases.
//!
//! Conveniencias de referencia: `ground/spec/05-operaciones.md` — «Diálogo de
//! progreso de copia/movimiento» y «Velocidad, tiempo restante y gráfico».
//!
//! # El tiempo se recibe, no se lee
//!
//! Ningún método consulta el reloj: quien muestrea pasa el instante. Consultarlo
//! aquí haría el módulo imposible de probar sin dormir el hilo, y la aritmética
//! de una ETA es justo lo que hay que poder probar a fondo.

use std::time::Duration;

/// Fase de la operación. Antes de transferir hay que recorrer el árbol, y
/// durante ese rato **no hay barra determinista**: la spec lo pide explícito
/// para no fingir un porcentaje que aún no se conoce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Recorriendo el origen. «Calculando…»
    Measuring,
    Running,
    Paused,
    Done,
}

/// Qué enseñar como tiempo restante.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Eta {
    /// Aún no hay muestras suficientes para una estimación honesta. Evita el
    /// «99 horas» del arranque que la spec prohíbe.
    Unknown,
    /// La velocidad ha caído a cero. No se congela la última cifra buena.
    Stalled,
    /// En pausa: ni velocidad ni ETA tienen sentido.
    Paused,
    Remaining(Duration),
}

/// Cuánto pesa la muestra nueva frente al histórico.
///
/// Bajo a propósito: la spec pide cifras suavizadas, «no erráticas». Con 0,25 un
/// pico aislado mueve la estimación un cuarto y no la dispara.
const SMOOTHING: f64 = 0.25;

/// Muestras necesarias antes de arriesgar una ETA.
const MIN_SAMPLES: u32 = 3;

/// Segundos sin avance a partir de los cuales se declara atasco.
const STALL_AFTER: f64 = 2.0;

/// Contador de una operación en curso.
#[derive(Debug, Clone)]
pub struct Meter {
    phase: Phase,
    total_bytes: Option<u64>,
    total_items: u64,
    bytes_done: u64,
    items_done: u64,
    current: Option<String>,
    last_at: Option<f64>,
    last_bytes: u64,
    last_items: u64,
    bytes_per_second: Option<f64>,
    items_per_second: Option<f64>,
    samples: u32,
    idle_for: f64,
}

impl Default for Meter {
    fn default() -> Self {
        Self::measuring()
    }
}

impl Meter {
    /// Arranca en fase de medida: todavía no se sabe cuánto hay.
    #[must_use]
    pub fn measuring() -> Self {
        Self {
            phase: Phase::Measuring,
            total_bytes: None,
            total_items: 0,
            bytes_done: 0,
            items_done: 0,
            current: None,
            last_at: None,
            last_bytes: 0,
            last_items: 0,
            bytes_per_second: None,
            items_per_second: None,
            samples: 0,
            idle_for: 0.0,
        }
    }

    /// Termina la fase de medida con el total que se acaba de descubrir.
    ///
    /// `total_bytes` es `None` cuando la operación no recorre bytes —un
    /// `rename(2)` dentro del mismo volumen—, y entonces el progreso se cuenta
    /// solo por elementos.
    pub fn start(&mut self, total_bytes: Option<u64>, total_items: u64) {
        self.total_bytes = total_bytes;
        self.total_items = total_items;
        self.phase = Phase::Running;
    }

    /// Registra el avance observado en el instante `at` (segundos monótonos).
    pub fn sample(&mut self, at: f64, bytes_done: u64, items_done: u64) {
        self.bytes_done = bytes_done;
        self.items_done = items_done;

        let Some(previous) = self.last_at else {
            self.last_at = Some(at);
            self.last_bytes = bytes_done;
            self.last_items = items_done;
            return;
        };
        let elapsed = at - previous;
        if elapsed <= 0.0 {
            return;
        }

        let bytes_delta = bytes_done.saturating_sub(self.last_bytes);
        let items_delta = items_done.saturating_sub(self.last_items);

        if bytes_delta == 0 && items_delta == 0 {
            self.idle_for += elapsed;
        } else {
            self.idle_for = 0.0;
            self.samples = self.samples.saturating_add(1);
            blend(&mut self.bytes_per_second, bytes_delta as f64 / elapsed);
            blend(&mut self.items_per_second, items_delta as f64 / elapsed);
        }

        self.last_at = Some(at);
        self.last_bytes = bytes_done;
        self.last_items = items_done;
    }

    /// Nombre del elemento en curso, para el «copiando <fichero>».
    pub fn set_current(&mut self, name: impl Into<String>) {
        self.current = Some(name.into());
    }

    #[must_use]
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    pub fn pause(&mut self) {
        self.phase = Phase::Paused;
        // Se olvida el instante para que el rato en pausa no cuente como atasco
        // ni hunda la velocidad al reanudar.
        self.last_at = None;
    }

    pub fn resume(&mut self) {
        self.phase = Phase::Running;
        self.idle_for = 0.0;
    }

    pub fn finish(&mut self) {
        self.phase = Phase::Done;
    }

    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    #[must_use]
    pub fn bytes_done(&self) -> u64 {
        self.bytes_done
    }

    #[must_use]
    pub fn items_done(&self) -> u64 {
        self.items_done
    }

    /// Fracción completada de 0 a 1, o `None` mientras no se conozca el total.
    ///
    /// Con muchos ficheros minúsculos los bytes engañan, así que si no hay total
    /// de bytes se cuenta por elementos, como pide la spec.
    #[must_use]
    pub fn fraction(&self) -> Option<f64> {
        if self.phase == Phase::Measuring {
            return None;
        }
        match self.total_bytes {
            Some(total) if total > 0 => {
                Some((self.bytes_done as f64 / total as f64).min(1.0))
            }
            _ if self.total_items > 0 => {
                Some((self.items_done as f64 / self.total_items as f64).min(1.0))
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn bytes_per_second(&self) -> Option<f64> {
        self.bytes_per_second
    }

    /// Elementos por segundo, que es lo único informativo cuando se copian miles
    /// de ficheros diminutos y los MB/s se desploman.
    #[must_use]
    pub fn items_per_second(&self) -> Option<f64> {
        self.items_per_second
    }

    /// Cuánto queda.
    #[must_use]
    pub fn eta(&self) -> Eta {
        match self.phase {
            Phase::Paused => return Eta::Paused,
            Phase::Measuring | Phase::Done => return Eta::Unknown,
            Phase::Running => {}
        }
        if self.idle_for >= STALL_AFTER {
            return Eta::Stalled;
        }
        if self.samples < MIN_SAMPLES {
            return Eta::Unknown;
        }

        let (left, rate) = match self.total_bytes {
            Some(total) => (
                total.saturating_sub(self.bytes_done) as f64,
                self.bytes_per_second,
            ),
            None => (
                self.total_items.saturating_sub(self.items_done) as f64,
                self.items_per_second,
            ),
        };
        match rate {
            Some(rate) if rate > 0.0 => Eta::Remaining(Duration::from_secs_f64(left / rate)),
            _ => Eta::Stalled,
        }
    }
}

fn blend(current: &mut Option<f64>, sample: f64) {
    *current = Some(match *current {
        Some(previous) => previous * (1.0 - SMOOTHING) + sample * SMOOTHING,
        None => sample,
    });
}

/// Redondea una duración a algo que una persona lee de un vistazo.
///
/// La spec lo pide literalmente: «Aprox. 2 minutos restantes», no «00:01:57».
/// Una cifra al segundo cambia en cada tick y da sensación de inestabilidad
/// aunque la estimación sea buena.
#[must_use]
pub fn humanize(eta: Eta) -> String {
    let remaining = match eta {
        Eta::Unknown => return "Calculando…".to_string(),
        Eta::Stalled => return "Esperando…".to_string(),
        Eta::Paused => return "En pausa".to_string(),
        Eta::Remaining(d) => d.as_secs(),
    };
    match remaining {
        0..=9 => "Unos segundos".to_string(),
        10..=59 => "Menos de 1 min".to_string(),
        60..=5399 => format!("Aprox. {} min", (remaining as f64 / 60.0).round().max(1.0) as u64),
        _ => format!("Aprox. {} h", (remaining as f64 / 3600.0).round().max(1.0) as u64),
    }
}
