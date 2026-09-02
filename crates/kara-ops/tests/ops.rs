//! Pruebas de la capa de decisiones (`ground/spec/05-operaciones.md`).

use std::collections::BTreeSet;
use std::path::PathBuf;

use kara_ops::{
    BatchPolicy, ConflictKind, ConflictPolicy, Failure, FailureAction, FailureKind, Resolution,
    local_utc_offset_seconds, split_name, unique_name,
};

/// El foco arranca en la opcion menos destructiva: nunca en Reemplazar.
#[test]
fn the_default_resolution_is_never_destructive() {
    for kind in [
        ConflictKind::FileOverFile,
        ConflictKind::FileOverDirectory,
        ConflictKind::DirectoryOverFile,
        ConflictKind::DirectoryOverDirectory,
    ] {
        assert_ne!(kind.default_resolution(), Resolution::Replace, "{kind:?}");
    }
}

/// Combinar solo tiene sentido entre carpetas.
#[test]
fn merge_is_offered_only_between_directories() {
    assert!(ConflictKind::DirectoryOverDirectory.allows(&Resolution::Merge));
    for kind in [
        ConflictKind::FileOverFile,
        ConflictKind::FileOverDirectory,
        ConflictKind::DirectoryOverFile,
    ] {
        assert!(!kind.allows(&Resolution::Merge), "{kind:?}");
    }
}

/// «Aplicar a todos» se guarda por TIPO: aplicar a un choque carpeta-contra-
/// fichero lo elegido para dos ficheros seria la sorpresa destructiva a evitar.
#[test]
fn apply_to_all_is_scoped_per_conflict_kind() {
    let mut p = ConflictPolicy::new();
    p.apply_to_all(ConflictKind::FileOverFile, Resolution::Replace);
    assert_eq!(p.decide(ConflictKind::FileOverFile), Some(&Resolution::Replace));
    assert_eq!(p.decide(ConflictKind::FileOverDirectory), None, "este sigue preguntando");
}

/// La spec pide poder cambiar de opinion mientras queden conflictos.
#[test]
fn a_blanket_decision_can_be_changed_or_revoked() {
    let mut p = ConflictPolicy::new();
    p.apply_to_all(ConflictKind::FileOverFile, Resolution::Replace);
    p.apply_to_all(ConflictKind::FileOverFile, Resolution::Skip);
    assert_eq!(p.decide(ConflictKind::FileOverFile), Some(&Resolution::Skip));
    p.clear(ConflictKind::FileOverFile);
    assert_eq!(p.decide(ConflictKind::FileOverFile), None);
}

/// El resumen final debe decir cuantos se reemplazaron, omitieron y duplicaron.
#[test]
fn the_policy_counts_what_it_resolved() {
    let mut p = ConflictPolicy::new();
    p.record(&Resolution::Replace);
    p.record(&Resolution::Skip);
    p.record(&Resolution::Skip);
    p.record(&Resolution::KeepBoth);
    let c = p.counts();
    assert_eq!((c.replaced, c.skipped, c.kept_both, c.total()), (1, 2, 1, 4));
}

#[test]
fn a_free_name_is_returned_untouched() {
    assert_eq!(unique_name("informe.pdf", |_| false), "informe.pdf");
}

/// El patron de Windows: sufijo numerico ANTES de la extension.
#[test]
fn the_suffix_goes_before_the_extension_and_increments() {
    let taken: BTreeSet<&str> =
        ["informe.pdf", "informe (2).pdf", "informe (3).pdf"].into_iter().collect();
    assert_eq!(unique_name("informe.pdf", |n| taken.contains(n)), "informe (4).pdf");
}

/// Extensiones compuestas: el sufijo no puede partir el .tar.gz.
#[test]
fn composite_extensions_are_preserved() {
    assert_eq!(split_name("copia.tar.gz"), ("copia", Some("tar.gz")));
    assert_eq!(unique_name("copia.tar.gz", |n| n == "copia.tar.gz"), "copia (2).tar.gz");
}

#[test]
fn name_splitting_edge_cases() {
    assert_eq!(split_name("Makefile"), ("Makefile", None));
    assert_eq!(split_name(".bashrc"), (".bashrc", None), "el punto marca oculto");
    assert_eq!(split_name("file."), ("file.", None));
    assert_eq!(split_name("a.tar"), ("a", Some("tar")), "tar suelto no es compuesta");
}

/// Nombres que exceden el limite: se recorta el CUERPO, nunca el sufijo ni la
/// extension.
#[test]
fn an_overlong_name_is_truncated_keeping_suffix_and_extension() {
    let largo = format!("{}.pdf", "a".repeat(300));
    let out = unique_name(&largo, |n| n == largo);
    assert!(out.len() <= 255, "cabe en NAME_MAX: {}", out.len());
    assert!(out.ends_with(" (2).pdf"), "conserva sufijo y extension");
}

/// Recortar no puede partir un caracter UTF-8 por la mitad.
#[test]
fn truncation_respects_character_boundaries() {
    let largo = format!("{}.txt", "ñ".repeat(200));
    let out = unique_name(&largo, |n| n == largo);
    assert!(out.len() <= 255);
    assert!(out.ends_with(" (2).txt"));
}

/// Omitir sigue con el resto; cancelar para. Un fallo no aborta el lote.
#[test]
fn skipping_records_the_path_and_keeps_going() {
    let mut p = BatchPolicy::new();
    p.record_success();
    p.record(
        Failure {
            path: PathBuf::from("/x/a.txt"),
            kind: FailureKind::PermissionDenied,
            reason: "permiso denegado".into(),
        },
        FailureAction::Skip,
    );
    p.record_success();
    let r = p.report();
    assert!(!p.is_cancelled());
    assert_eq!(r.succeeded, 2);
    assert_eq!(r.skipped, vec![PathBuf::from("/x/a.txt")]);
    assert!(!r.is_clean(), "hubo algo que contar");
}

#[test]
fn cancelling_stops_the_batch() {
    let mut p = BatchPolicy::new();
    p.record(
        Failure { path: "/x".into(), kind: FailureKind::Other, reason: "vaya".into() },
        FailureAction::Cancel,
    );
    assert!(p.is_cancelled());
    assert!(p.report().cancelled);
}

/// «Reintentar todos» no puede aplicarse a un medio desconectado: seria el bucle
/// infinito que la spec prohibe explicitamente.
#[test]
fn retry_all_does_not_apply_where_retrying_cannot_help() {
    let mut p = BatchPolicy::new();
    p.apply_to_all(FailureAction::Retry);
    let en_uso = Failure { path: "/a".into(), kind: FailureKind::InUse, reason: String::new() };
    let sin_medio =
        Failure { path: "/b".into(), kind: FailureKind::MediaGone, reason: String::new() };
    assert_eq!(p.decide(&en_uso), Some(FailureAction::Retry));
    assert_eq!(p.decide(&sin_medio), None, "vuelve a preguntar en vez de reintentar sin fin");
}

#[test]
fn skip_all_applies_to_every_kind() {
    let mut p = BatchPolicy::new();
    p.apply_to_all(FailureAction::Skip);
    for kind in [FailureKind::MediaGone, FailureKind::NoSpace, FailureKind::InUse] {
        let f = Failure { path: "/a".into(), kind, reason: String::new() };
        assert_eq!(p.decide(&f), Some(FailureAction::Skip));
    }
}

#[test]
fn some_failures_need_the_user_to_act_before_retrying() {
    assert!(FailureKind::NoSpace.needs_user_action_first());
    assert!(FailureKind::PermissionDenied.needs_user_action_first());
    assert!(!FailureKind::InUse.needs_user_action_first());
}

/// Cierra el sg_01 de la supervision: kara-fs no lee la zona horaria y espera
/// que esta capa le inyecte el desfase, o el .trashinfo se estampa en UTC.
#[test]
fn the_local_offset_matches_the_system() {
    let s = local_utc_offset_seconds();
    assert_eq!(s % 60, 0, "los desfases reales son multiplos de un minuto");
    assert!((-50400..=50400).contains(&s), "entre -14 y +14 horas: {s}");

    let z = String::from_utf8(
        std::process::Command::new("date").arg("+%z").output().expect("date").stdout,
    )
    .expect("utf8");
    let z = z.trim();
    let signo = if z.starts_with('-') { -1 } else { 1 };
    let h: i32 = z[1..3].parse().expect("horas");
    let m: i32 = z[3..5].parse().expect("minutos");
    assert_eq!(s, signo * (h * 3600 + m * 60), "debe coincidir con el sistema");
}
