//! Pruebas del progreso, la ETA y la cola (`ground/spec/05-operaciones.md`).

use std::path::PathBuf;
use std::time::Duration;

use kara_ops::{Concurrency, Eta, JobState, Kind, Meter, Phase, Queue, humanize};

/// Antes de transferir se recorre el arbol, y ahi NO puede haber barra
/// determinista ni ETA: la spec prohibe el «99 horas» del arranque.
#[test]
fn while_measuring_there_is_no_bar_and_no_eta() {
    let m = Meter::measuring();
    assert_eq!(m.phase(), Phase::Measuring);
    assert_eq!(m.fraction(), None);
    assert_eq!(m.eta(), Eta::Unknown);
    assert_eq!(humanize(m.eta()), "Calculando…");
}

/// Hacen falta varias muestras antes de arriesgar una estimacion.
#[test]
fn the_eta_waits_for_enough_samples() {
    let mut m = Meter::measuring();
    m.start(Some(1000), 1);
    m.sample(0.0, 0, 0);
    m.sample(1.0, 100, 0);
    assert_eq!(m.eta(), Eta::Unknown, "una sola muestra no basta");
    m.sample(2.0, 200, 0);
    m.sample(3.0, 300, 0);
    m.sample(4.0, 400, 0);
    assert!(matches!(m.eta(), Eta::Remaining(_)));
}

#[test]
fn the_eta_is_roughly_right_at_a_steady_rate() {
    let mut m = Meter::measuring();
    m.start(Some(1000), 1);
    for s in 0u64..=5 {
        m.sample(s as f64, s * 100, 0);
    }
    // 500 de 1000 a 100 B/s -> unos 5 s.
    match m.eta() {
        Eta::Remaining(d) => assert!(d.as_secs_f64() > 3.0 && d.as_secs_f64() < 8.0, "{d:?}"),
        other => panic!("esperaba una estimacion, llego {other:?}"),
    }
    assert_eq!(m.fraction(), Some(0.5));
}

/// El suavizado no impide que una rafaga real suba la cifra —si de verdad
/// llegan 99 600 bytes en un segundo, la velocidad ES mayor—; lo que impide es
/// que la estimacion SALTE al valor bruto de una sola muestra.
#[test]
fn a_single_spike_moves_the_estimate_only_partway() {
    let mut m = Meter::measuring();
    m.start(Some(100_000), 1);
    for s in 0u64..=4 {
        m.sample(s as f64, s * 100, 0);
    }
    let antes = m.bytes_per_second().unwrap_or_default();
    let pico = 99_600.0;
    m.sample(5.0, 100_000, 0);
    let despues = m.bytes_per_second().unwrap_or_default();

    assert!(despues > antes, "una rafaga real sube la cifra");
    assert!(
        despues < pico / 3.0,
        "pero no salta al valor bruto: {antes} -> {despues} con un pico de {pico}"
    );
}

/// Si la velocidad cae a cero hay que decirlo, no congelar la ultima cifra.
#[test]
fn a_stall_is_reported_instead_of_freezing_the_number() {
    let mut m = Meter::measuring();
    m.start(Some(1000), 1);
    for s in 0u64..=4 {
        m.sample(s as f64, s * 100, 0);
    }
    m.sample(6.0, 400, 0);
    m.sample(8.0, 400, 0);
    assert_eq!(m.eta(), Eta::Stalled);
    assert_eq!(humanize(m.eta()), "Esperando…");
}

/// En pausa, velocidad y ETA se marcan como tal.
#[test]
fn pausing_marks_the_eta_as_paused() {
    let mut m = Meter::measuring();
    m.start(Some(1000), 1);
    for s in 0u64..=4 {
        m.sample(s as f64, s * 100, 0);
    }
    m.pause();
    assert_eq!(m.phase(), Phase::Paused);
    assert_eq!(m.eta(), Eta::Paused);
    assert_eq!(humanize(m.eta()), "En pausa");
    m.resume();
    assert_eq!(m.phase(), Phase::Running);
}

/// Con muchos ficheros minusculos los bytes enganan: se cuenta por elementos.
#[test]
fn without_a_byte_total_progress_counts_items() {
    let mut m = Meter::measuring();
    m.start(None, 200);
    for s in 0u64..=4 {
        m.sample(s as f64, 0, s * 10);
    }
    assert_eq!(m.fraction(), Some(40.0 / 200.0));
    assert!(m.items_per_second().unwrap_or_default() > 0.0);
    assert!(matches!(m.eta(), Eta::Remaining(_)));
}

/// La spec pide «Aprox. 2 minutos», no «00:01:57».
#[test]
fn the_eta_is_rounded_for_humans() {
    assert_eq!(humanize(Eta::Remaining(Duration::from_secs(4))), "Unos segundos");
    assert_eq!(humanize(Eta::Remaining(Duration::from_secs(30))), "Menos de 1 min");
    assert_eq!(humanize(Eta::Remaining(Duration::from_secs(117))), "Aprox. 2 min");
    assert_eq!(humanize(Eta::Remaining(Duration::from_secs(7200))), "Aprox. 2 h");
}

// ------------------------------------------------------------------- cola

/// El defecto es encolar, no paralelizar: tres copias a la vez sobre el mismo
/// disco se estorban.
#[test]
fn the_queue_runs_one_at_a_time_by_default() {
    let mut q = Queue::new(Concurrency::default());
    let a = q.push(Kind::Copy, vec![PathBuf::from("/a")], Some("/d".into()));
    q.push(Kind::Copy, vec![PathBuf::from("/b")], Some("/d".into()));

    assert_eq!(q.next_runnable(), Some(a));
    q.start(a);
    assert_eq!(q.next_runnable(), None, "la segunda espera turno");
    q.finish(a);
    assert!(q.next_runnable().is_some());
}

#[test]
fn parallel_lets_several_run() {
    let mut q = Queue::new(Concurrency::Parallel(2));
    let a = q.push(Kind::Move, vec![PathBuf::from("/a")], None);
    let b = q.push(Kind::Move, vec![PathBuf::from("/b")], None);
    q.push(Kind::Move, vec![PathBuf::from("/c")], None);
    q.start(a);
    q.start(b);
    assert_eq!(q.active(), 2);
    assert_eq!(q.next_runnable(), None, "el limite son dos");
}

/// Cancelar una no puede afectar a las demas.
#[test]
fn cancelling_one_leaves_the_others_alone() {
    let mut q = Queue::new(Concurrency::Parallel(2));
    let a = q.push(Kind::Copy, vec![PathBuf::from("/a")], None);
    let b = q.push(Kind::Copy, vec![PathBuf::from("/b")], None);
    q.start(a);
    q.start(b);
    q.cancel(a);
    assert_eq!(q.get(a).map(|j| j.state), Some(JobState::Cancelled));
    assert_eq!(q.get(b).map(|j| j.state), Some(JobState::Active));
}

/// Se pausa cada operacion por separado.
#[test]
fn each_job_pauses_on_its_own() {
    let mut q = Queue::new(Concurrency::Parallel(2));
    let a = q.push(Kind::Copy, vec![PathBuf::from("/a")], None);
    let b = q.push(Kind::Copy, vec![PathBuf::from("/b")], None);
    q.start(a);
    q.start(b);
    q.pause(a);
    assert_eq!(q.get(a).map(|j| j.state), Some(JobState::Paused));
    assert_eq!(q.get(b).map(|j| j.state), Some(JobState::Active));
    assert_eq!(q.get(a).map(|j| j.meter.eta()), Some(Eta::Paused));
}

#[test]
fn the_queue_summarises_and_prunes() {
    let mut q = Queue::new(Concurrency::default());
    assert_eq!(q.summary(), "Sin operaciones");
    let a = q.push(Kind::Copy, vec![PathBuf::from("/x/informe.pdf")], None);
    assert_eq!(q.summary(), "Copiando informe.pdf");
    q.push(Kind::Move, vec![PathBuf::from("/x/b")], None);
    assert_eq!(q.summary(), "2 operaciones en curso");

    q.finish(a);
    q.prune();
    assert_eq!(q.jobs().len(), 1, "lo terminado se retira");
}

/// Un progreso global antes de conocer todos los totales seria inventado.
#[test]
fn no_combined_progress_while_something_is_still_measuring() {
    let mut q = Queue::new(Concurrency::Parallel(2));
    let a = q.push(Kind::Copy, vec![PathBuf::from("/a")], None);
    let b = q.push(Kind::Copy, vec![PathBuf::from("/b")], None);
    if let Some(m) = q.meter_mut(a) {
        m.start(Some(100), 1);
        m.sample(0.0, 50, 0);
    }
    assert_eq!(q.combined_fraction(), None, "b sigue midiendo");

    if let Some(m) = q.meter_mut(b) {
        m.start(Some(100), 1);
        m.sample(0.0, 100, 1);
    }
    assert_eq!(q.combined_fraction(), Some(0.75));
}
