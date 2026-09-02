//! Generación de nombres únicos y partición cuerpo/extensión.
//!
//! Vive en `kara-core` porque es lógica pura de cadenas y la necesitan dos capas
//! distintas: `kara-fs` al resolver un «Conservar ambos» durante una copia, y
//! `kara-ops` al decidirlo. Tenerla en `kara-ops` obligaba a `kara-fs` a
//! duplicarla, porque la regla de capas es `ops -> fs` y no al revés.
//!
//! No confundir [`split_name`] con [`crate::filter::base_and_extension`]:
//! aquel conoce las extensiones compuestas (`a.tar.gz` es cuerpo `a` más
//! extensión `tar.gz`, para que el sufijo no las parta) y este no, porque al
//! ocultar extensiones en la vista lo que se enseña de `a.tar.gz` es `a.tar`,
//! igual que hace Windows. Son dos preguntas distintas sobre el mismo nombre.

/// Extensiones compuestas que se tratan como una sola.
///
/// Es una lista explícita y no una regla general porque no la hay: `a.tar.gz`
/// tiene extensión compuesta y `informe.v2.pdf` no, y nada en el nombre los
/// distingue. La spec nombra `.tar.gz`; el resto son sus parientes directos.
const COMPOSITE_EXTENSIONS: &[&str] = &[
    "tar.gz", "tar.bz2", "tar.xz", "tar.zst", "tar.lz", "tar.lzma", "tar.z",
];

/// Parte un nombre en cuerpo y extensión, respetando las compuestas.
///
/// Un nombre que empieza por punto y no tiene más puntos (`.bashrc`) no tiene
/// extensión: ese punto marca «oculto».
#[must_use]
pub fn split_name(name: &str) -> (&str, Option<&str>) {
    let lower = name.to_ascii_lowercase();
    for composite in COMPOSITE_EXTENSIONS {
        let suffix = format!(".{composite}");
        if lower.ends_with(&suffix) && lower.len() > suffix.len() {
            let cut = name.len() - suffix.len();
            return (&name[..cut], Some(&name[cut + 1..]));
        }
    }
    match name.rfind('.') {
        Some(0) | None => (name, None),
        Some(dot) if dot + 1 == name.len() => (name, None),
        Some(dot) => (&name[..dot], Some(&name[dot + 1..])),
    }
}

/// Límite de longitud de un nombre en la mayoría de sistemas de ficheros Linux.
const NAME_MAX: usize = 255;

/// Genera el nombre del «Conservar ambos»: `informe (2).pdf`, `informe (3).pdf`…
///
/// `exists` responde si un nombre ya está ocupado en el destino; se recibe como
/// función porque mirarlo es I/O y esta capa no lo hace.
///
/// Se incrementa hasta encontrar uno libre, y si el resultado excede el límite
/// del sistema se recorta el **cuerpo**, conservando el sufijo y la extensión:
/// perder el `(2)` o el `.pdf` sería peor que perder unas letras del nombre.
///
/// # Un matiz de la spec que conviene conocer
///
/// El sufijo se añade siempre al nombre original, así que `informe (2).pdf`
/// genera `informe (2) (2).pdf`. Es la lectura literal de la spec, que dice
/// «añadir un sufijo numérico» e «incrementar hasta encontrar uno libre», sin
/// mencionar que haya que interpretar un sufijo previo. Windows sí lo
/// interpretaría y produciría `informe (3).pdf`; queda anotado como decisión
/// pendiente en vez de inventarla aquí.
#[must_use]
pub fn unique_name(name: &str, exists: impl Fn(&str) -> bool) -> String {
    if !exists(name) {
        return name.to_string();
    }
    let (stem, extension) = split_name(name);
    for n in 2..usize::MAX {
        let candidate = compose(stem, &format!(" ({n})"), extension);
        if !exists(&candidate) {
            return candidate;
        }
    }
    unreachable!("el bucle encuentra un hueco mucho antes de agotar usize")
}

/// Monta `cuerpo` + `sufijo` + `.extensión` recortando el cuerpo si hace falta.
fn compose(stem: &str, suffix: &str, extension: Option<&str>) -> String {
    let tail_len = suffix.len() + extension.map_or(0, |e| e.len() + 1);
    let room = NAME_MAX.saturating_sub(tail_len);

    let mut body = stem;
    if body.len() > room {
        // Recorta en un límite de carácter: cortar un UTF-8 por la mitad
        // produciría un nombre inválido.
        let mut cut = room;
        while cut > 0 && !body.is_char_boundary(cut) {
            cut -= 1;
        }
        body = &body[..cut];
    }

    match extension {
        Some(ext) => format!("{body}{suffix}.{ext}"),
        None => format!("{body}{suffix}"),
    }
}
