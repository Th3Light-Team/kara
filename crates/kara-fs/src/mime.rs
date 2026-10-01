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
use std::path::PathBuf;

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
    /// Patrones `*.algo`.
    suffixes: Vec<Glob>,
    /// Posiciones en `suffixes`, por la cola (`.algo`) en minúsculas.
    ///
    /// Sin esto cada consulta recorría los ~1 400 patrones, y listar una carpeta
    /// con 100 000 entradas hace varias consultas por entrada: un segundo
    /// entero solo en esto. Con el índice se prueba cada punto del nombre.
    suffix_index: HashMap<Vec<u8>, Vec<usize>>,
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

        // Cada patrón `*.algo` solo puede casar si la cola del nombre desde
        // algún punto es `.algo`, así que se prueban esas colas y no todos los
        // patrones. El resultado es el mismo que recorrerlos todos.
        let by_suffix = name
            .bytes()
            .enumerate()
            .filter(|(_, byte)| *byte == b'.')
            .filter_map(|(at, _)| {
                self.suffix_index
                    .get(&name.as_bytes()[at..].to_ascii_lowercase())
            })
            .flatten()
            .map(|index| &self.suffixes[*index])
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
    ///
    /// La comparación va por bytes y no por rebanada de `str` a propósito:
    /// cortar un `&str` por una posición contada desde el final cae dentro de un
    /// carácter en cuanto el nombre lleva un acento o una raya, y eso es un
    /// pánico. Comparar bytes da el mismo resultado —dos textos UTF-8 son
    /// iguales si y solo si sus bytes lo son— y no puede reventar.
    fn matches_suffix(&self, name: &str) -> bool {
        let suffix = self.pattern.as_bytes();
        let Some(suffix) = suffix.get(1..) else {
            return false;
        };
        let name = name.as_bytes();
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

    for (position, glob) in db.suffixes.iter().enumerate() {
        // `*` + `.algo`: la cola es el patrón sin el comodín.
        let tail = glob.pattern.as_bytes()[1..].to_ascii_lowercase();
        db.suffix_index.entry(tail).or_default().push(position);
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

/// La descripción legible de un tipo, de la base de FreeDesktop.
///
/// Cada tipo tiene su `/usr/share/mime/<medio>/<subtipo>.xml` con un `<comment>`
/// y sus traducciones. Es lo que hace que la columna «Tipo» diga «documento
/// JSON» y no «Archivo», y sale traducido sin que Kara traduzca nada.
///
/// Se recuerda lo ya leído: una carpeta con miles de ficheros tiene un puñado de
/// tipos, y abrir un XML por entrada sería absurdo.
#[derive(Debug, Clone)]
pub struct MimeDescriptions {
    languages: Vec<String>,
    cache: HashMap<String, Option<String>>,
}

impl MimeDescriptions {
    /// `languages` va en orden de preferencia y en la forma de `xml:lang`
    /// (`es`, `pt-BR`), no en la del `locale` (`pt_BR`).
    #[must_use]
    pub fn new(languages: Vec<String>) -> Self {
        Self {
            languages,
            cache: HashMap::new(),
        }
    }

    /// La descripción de un tipo, o `None` si el sistema no lo describe.
    pub fn of(&mut self, mime: &str) -> Option<&str> {
        if !self.cache.contains_key(mime) {
            let found = read_comment(mime, &self.languages);
            self.cache.insert(mime.to_string(), found);
        }
        self.cache.get(mime).and_then(Option::as_deref)
    }
}

/// Un tipo MIME solo puede llevar estos caracteres.
///
/// La ruta del XML se compone con el tipo, así que se comprueba antes de
/// tocarla: un tipo con `..` construiría una ruta fuera de la base de datos.
fn is_safe_mime(mime: &str) -> bool {
    let Some((media, subtype)) = mime.split_once('/') else {
        return false;
    };
    let ok = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+' | b'_'))
    };
    ok(media) && ok(subtype)
}

fn read_comment(mime: &str, languages: &[String]) -> Option<String> {
    if !is_safe_mime(mime) {
        return None;
    }

    let mut bases: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"));
        bases.push(data.join("mime"));
    }
    let dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    bases.extend(
        dirs.split(':')
            .filter(|d| !d.is_empty())
            .map(|d| PathBuf::from(d).join("mime")),
    );

    bases
        .iter()
        .map(|base| base.join(format!("{mime}.xml")))
        .find_map(|path| std::fs::read_to_string(path).ok())
        .and_then(|xml| parse_comment(&xml, languages))
}

/// Saca el `<comment>` de un XML de tipo, en el primer idioma disponible.
///
/// Se rasca el texto en vez de montar un analizador de XML: el fichero lo genera
/// `update-mime-database` con una forma fija, y lo único que interesa es una
/// etiqueta. Sin traducción utilizable queda el comentario sin idioma, que es el
/// inglés original.
#[must_use]
pub fn parse_comment(xml: &str, languages: &[String]) -> Option<String> {
    let mut untranslated: Option<String> = None;
    let mut translations: HashMap<String, String> = HashMap::new();

    let mut rest = xml;
    while let Some(start) = rest.find("<comment") {
        rest = &rest[start + "<comment".len()..];
        let Some(attributes_end) = rest.find('>') else {
            break;
        };
        let attributes = &rest[..attributes_end];
        rest = &rest[attributes_end + 1..];

        let Some(close) = rest.find("</comment>") else {
            break;
        };
        let text = unescape_xml(&rest[..close]);
        rest = &rest[close + "</comment>".len()..];

        match attribute(attributes, "xml:lang") {
            Some(lang) => {
                translations.insert(lang.to_string(), text);
            }
            None => {
                if untranslated.is_none() {
                    untranslated = Some(text);
                }
            }
        }
    }

    languages
        .iter()
        .find_map(|lang| translations.get(lang).cloned())
        .or(untranslated)
}

/// El valor de un atributo entrecomillado con comillas dobles.
fn attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let start = attributes.find(&format!("{name}=\""))? + name.len() + 2;
    let rest = &attributes[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Deshace las cinco entidades que XML define. La base no usa más.
fn unescape_xml(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        // La del ampersand va la última: al revés, `&amp;lt;` acabaría en `<`.
        .replace("&amp;", "&")
}

#[cfg(test)]
mod index_tests {
    use super::*;

    /// What a lookup did before the index: every suffix pattern, in turn.
    fn brute_force<'a>(db: &'a MimeDatabase, name: &str) -> Option<&'a str> {
        db.suffixes
            .iter()
            .filter(|g| g.matches_suffix(name))
            .max_by_key(|g| (g.pattern.len(), g.weight, Reverse(&g.mime)))
            .map(|g| g.mime.as_str())
    }

    #[test]
    fn the_suffix_index_answers_exactly_what_scanning_every_pattern_did() {
        let sample = "50:application/gzip:*.gz\n\
                      50:application/x-compressed-tar:*.tar.gz\n\
                      50:text/x-csrc:*.c\n\
                      50:text/x-c++src:*.C:cs\n\
                      50:application/json:*.json\n\
                      50:application/schema+json:*.json\n\
                      50:image/jpeg:*.jpg\n";
        let db = parse_globs2(sample);
        for name in [
            "a.tar.gz", "A.TAR.GZ", "x.gz", "x.c", "x.C", "x.cc", "noext", ".gz", "a..gz", "photo.JPG",
            "data.json", "archive.tar.gz.gz", "ñandú.jpg", "dot.", "..", "a.b.c",
        ] {
            assert_eq!(
                db.of(name).filter(|_| brute_force(&db, name).is_some()),
                brute_force(&db, name),
                "{name}"
            );
        }
    }
}
