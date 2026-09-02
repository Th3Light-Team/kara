//! Tipo MIME de un fichero a partir de su nombre, con la base compartida de
//! FreeDesktop (`shared-mime-info`).
//!
//! Kara declara que respeta las convenciones FreeDesktop, y esta es una de
//! ellas: el tipo de un fichero no se inventa con una tabla propia, sale de
//! `/usr/share/mime/globs2`, que es la misma que usan Dolphin, Nautilus y el
//! resto del escritorio. Así un tipo instalado por un paquete de terceros
//! aparece en Kara sin tocar Kara.
//!
//! # Decisiones de diseño
//!
//! - **No se mira el contenido.** La base trae además firmas mágicas por bytes;
//!   aplicarlas exige abrir cada fichero de la carpeta, y listar 100 000
//!   entradas no puede costar 100 000 `open`. Por el nombre se acierta en la
//!   práctica totalidad de los casos.
//! - **El troceo va aparte de la lectura.** [`parse_globs2`] es una función pura
//!   sobre texto, que es donde están las reglas que se pueden equivocar.

use std::cmp::Reverse;
use std::collections::HashMap;

/// Un patrón de la base con su tipo.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Glob {
    weight: u32,
    mime: String,
    pattern: String,
    /// El patrón distingue mayúsculas. `*.C` (C++) y `*.c` (C) son el ejemplo
    /// de por qué hace falta.
    case_sensitive: bool,
}

/// La base de tipos, ya troceada y repartida por forma de patrón.
#[derive(Debug, Clone, Default)]
pub struct MimeDatabase {
    /// Nombres completos: `Makefile`, `pom.xml`.
    literals: Vec<Glob>,
    /// Patrones `*.algo`, indexados por su extensión ya en minúsculas.
    suffixes: Vec<Glob>,
    /// Todo lo demás: `callgrind.out*`, `sconscript.*`.
    others: Vec<Glob>,
    /// Icono genérico por tipo, de `/usr/share/mime/generic-icons`.
    generic_icons: HashMap<String, String>,
}

impl MimeDatabase {
    /// Lee la base instalada en el sistema. Si no está, queda vacía y todo sale
    /// como desconocido, que es mejor que no arrancar.
    #[must_use]
    pub fn load() -> Self {
        let globs = std::fs::read_to_string("/usr/share/mime/globs2").unwrap_or_default();
        let mut db = parse_globs2(&globs);

        let generic = std::fs::read_to_string("/usr/share/mime/generic-icons").unwrap_or_default();
        db.generic_icons = parse_generic_icons(&generic);
        db
    }

    /// El tipo MIME de un nombre de fichero.
    ///
    /// Se sigue el orden de la spec de `shared-mime-info`: un nombre completo
    /// gana a cualquier comodín; entre comodines de extensión gana el más largo
    /// (`.tar.gz` antes que `.gz`); y a igualdad decide el peso.
    #[must_use]
    pub fn of(&self, name: &str) -> Option<&str> {
        if let Some(found) = best(self.literals.iter().filter(|g| g.matches_literal(name))) {
            return Some(found);
        }

        let by_suffix = self
            .suffixes
            .iter()
            .filter(|g| g.matches_suffix(name))
            // El patrón más largo primero; a igualdad, el de más peso; y en
            // último término el tipo alfabéticamente menor. Ese último criterio
            // no es capricho: `*.json` lo declaran igual `application/json` y
            // `application/schema+json`, la spec no dice cuál gana, y sin una
            // regla el resultado dependería del orden del fichero generado.
            // `xdg-mime` responde `application/json`, que es el que sale así.
            .max_by_key(|g| (g.pattern.len(), g.weight, Reverse(&g.mime)));
        if let Some(found) = by_suffix {
            return Some(&found.mime);
        }

        best(self.others.iter().filter(|g| g.matches_glob(name)))
    }

    /// El icono genérico de un tipo, si la base declara uno.
    ///
    /// Es lo que hace que un `.tar.xz` salga como paquete y no como un texto
    /// cualquiera: la familia se declara en la base, no se deduce del nombre.
    #[must_use]
    pub fn generic_icon(&self, mime: &str) -> Option<&str> {
        self.generic_icons.get(mime).map(String::as_str)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.literals.is_empty() && self.suffixes.is_empty() && self.others.is_empty()
    }
}

fn best<'a>(candidates: impl Iterator<Item = &'a Glob>) -> Option<&'a str> {
    candidates
        .max_by_key(|g| (g.weight, Reverse(&g.mime)))
        .map(|g| g.mime.as_str())
}

impl Glob {
    fn matches_literal(&self, name: &str) -> bool {
        if self.case_sensitive {
            self.pattern == name
        } else {
            self.pattern.eq_ignore_ascii_case(name)
        }
    }

    /// El patrón es `*.algo`; se compara la cola del nombre.
    fn matches_suffix(&self, name: &str) -> bool {
        let suffix = &self.pattern[1..];
        if name.len() <= suffix.len() {
            return false;
        }
        let tail = &name[name.len() - suffix.len()..];
        if self.case_sensitive {
            tail == suffix
        } else {
            tail.eq_ignore_ascii_case(suffix)
        }
    }

    fn matches_glob(&self, name: &str) -> bool {
        if self.case_sensitive {
            glob_matches(&self.pattern, name)
        } else {
            glob_matches(
                &self.pattern.to_ascii_lowercase(),
                &name.to_ascii_lowercase(),
            )
        }
    }
}

/// Trocea `globs2`: `peso:tipo:patrón[:banderas]`.
///
/// Las líneas que no encajan se saltan en vez de abortar: la base la genera
/// `update-mime-database` y una versión futura puede añadir campos.
#[must_use]
pub fn parse_globs2(text: &str) -> MimeDatabase {
    let mut db = MimeDatabase::default();

    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split(':');
        let (Some(weight), Some(mime), Some(pattern)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Ok(weight) = weight.trim().parse::<u32>() else {
            continue;
        };
        // Las banderas van sueltas y separadas por comas; solo importa `cs`.
        let case_sensitive = fields.any(|flags| flags.split(',').any(|flag| flag == "cs"));

        let glob = Glob {
            weight,
            mime: mime.to_string(),
            pattern: pattern.to_string(),
            case_sensitive,
        };

        // El patrón `*.algo` es el caso masivo y se compara por la cola, que es
        // mucho más barato que recorrer un comodín general.
        let simple_suffix = pattern.starts_with("*.")
            && !pattern[2..].contains(['*', '?', '[']);
        if simple_suffix {
            db.suffixes.push(glob);
        } else if pattern.contains(['*', '?', '[']) {
            db.others.push(glob);
        } else {
            db.literals.push(glob);
        }
    }

    db
}

/// Trocea `generic-icons`: `tipo:icono`.
#[must_use]
pub fn parse_generic_icons(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once(':'))
        .map(|(mime, icon)| (mime.to_string(), icon.to_string()))
        .collect()
}

/// Comodines de shell: `*` cualquier cosa, `?` un carácter.
///
/// No se admiten clases `[...]`: en la base instalada no hay ninguna, y un
/// motor de clases para cero casos es código que nadie ejerce.
#[must_use]
pub fn glob_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();

    // Recorrido iterativo con retroceso: al fallar tras un `*`, se vuelve a la
    // última estrella y se le come un carácter más. Evita la recursión, que con
    // un patrón lleno de estrellas se dispara.
    let (mut p, mut n) = (0, 0);
    let (mut star, mut retry) = (None, 0);

    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            retry = n;
            p += 1;
        } else if let Some(last_star) = star {
            p = last_star + 1;
            retry += 1;
            n = retry;
        } else {
            return false;
        }
    }

    pattern[p..].iter().all(|c| *c == '*')
}
