//! Árbol del panel de navegación: qué filas se ven y cuáles están desplegadas.
//!
//! Conveniencias de referencia: `ground/spec/01-navegacion.md`, «Panel de
//! navegación en árbol», «Expandir y colapsar nodos del árbol» y «Sincronizar el
//! árbol con la carpeta actual».
//!
//! # Decisiones de diseño
//!
//! - **Aquí no se lee ningún directorio.** La spec exige carga diferida: una
//!   rama no se lee hasta que se despliega. Este módulo lleva la cuenta de qué
//!   ramas están abiertas y cuáles tiene ya leídas; quien haga el I/O le entrega
//!   las hijas con [`Tree::set_children`]. Así el árbol entero se puede probar
//!   sin tocar el disco y sin fingir un sistema de ficheros.
//! - **El árbol se aplana a filas.** La vista es una lista, no una jerarquía de
//!   componentes anidados: con una lista plana el motor recicla los delegados y
//!   el coste no depende de cuántas ramas haya abiertas. La jerarquía sobrevive
//!   en [`Row::depth`].
//! - **Sin hijas leídas, la flecha se enseña igual.** No se puede saber si una
//!   carpeta tiene subcarpetas sin leerla, y leerlas todas para decidir qué
//!   flecha pintar es justo lo que la carga diferida evita. Se ofrece desplegar
//!   ([`Expandable::Unknown`]) y, si al abrirla no hay nada, la flecha
//!   desaparece. Es lo que hace el Explorador.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Las secciones en que se agrupa el panel.
///
/// Es un identificador, no un rótulo: cómo se lee cada sección lo decide la
/// vista, igual que con [`crate::breadcrumb::SegmentKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionId {
    /// Carpetas de uso frecuente: la personal y las de usuario.
    QuickAccess,
    /// La raíz y los volúmenes montados.
    ThisComputer,
}

/// Una carpeta del árbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub path: PathBuf,
    /// Cómo se llama. Para las raíces puede no ser el último componente de la
    /// ruta: la carpeta personal se enseña como «Inicio», no como `oliverv`.
    pub name: OsString,
}

impl Branch {
    /// Rama cuyo nombre es el último componente de su ruta.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let name = path
            .file_name()
            .map_or_else(|| OsString::from("/"), std::ffi::OsStr::to_os_string);
        Self { path, name }
    }
}

/// Una sección con sus raíces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub id: SectionId,
    pub roots: Vec<Branch>,
}

/// Si un nodo se puede desplegar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expandable {
    /// Se leyó y tiene subcarpetas.
    Yes,
    /// Se leyó y no tiene ninguna.
    No,
    /// Todavía no se ha leído.
    Unknown,
}

/// Qué es una fila del panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    /// Cabecera de sección. No navega a ningún sitio.
    Section(SectionId),
    /// Una carpeta.
    Folder {
        path: PathBuf,
        name: OsString,
        expandable: Expandable,
        expanded: bool,
    },
}

/// Una fila ya aplanada, lista para pintar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Cuánto se sangra. Las cabeceras de sección van a 0 y sus raíces a 1.
    pub depth: usize,
    pub kind: RowKind,
}

/// Tope de anidamiento al aplanar.
///
/// Un enlace simbólico que apunte a un ancestro suyo crea un ciclo, y el árbol
/// lo recorrería sin fin. Cortar a una profundidad que ningún árbol real alcanza
/// es más barato que resolver enlaces —que es I/O— solo para decidir si pintar
/// una fila.
const MAX_DEPTH: usize = 64;

/// El estado del panel de navegación.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    sections: Vec<Section>,
    expanded: BTreeSet<PathBuf>,
    /// Solo las ramas ya leídas. Que una ruta falte aquí significa «sin leer»,
    /// no «sin hijas»: son dos cosas distintas y la flecha depende de cuál.
    children: BTreeMap<PathBuf, Vec<Branch>>,
}

impl Tree {
    #[must_use]
    pub fn new(sections: Vec<Section>) -> Self {
        Self {
            sections,
            expanded: BTreeSet::new(),
            children: BTreeMap::new(),
        }
    }

    /// Sustituye las secciones conservando qué estaba desplegado y lo ya leído.
    ///
    /// Es lo que ocurre al montar o desmontar un volumen: cambian las raíces, no
    /// lo que el usuario tenía abierto.
    pub fn set_sections(&mut self, sections: Vec<Section>) {
        self.sections = sections;
    }

    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    #[must_use]
    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    /// Si una rama se puede desplegar, hasta donde se sabe.
    #[must_use]
    pub fn expandable(&self, path: &Path) -> Expandable {
        match self.children.get(path) {
            None => Expandable::Unknown,
            Some(children) if children.is_empty() => Expandable::No,
            Some(_) => Expandable::Yes,
        }
    }

    /// Marca una rama como desplegada. Devuelve `true` si hay que leerla.
    pub fn expand(&mut self, path: &Path) -> bool {
        self.expanded.insert(path.to_path_buf());
        !self.children.contains_key(path)
    }

    pub fn collapse(&mut self, path: &Path) {
        self.expanded.remove(path);
    }

    /// Alterna una rama. Devuelve `true` si tras alternarla hay que leerla.
    pub fn toggle(&mut self, path: &Path) -> bool {
        if self.is_expanded(path) {
            self.collapse(path);
            false
        } else {
            self.expand(path)
        }
    }

    /// Entrega las subcarpetas ya leídas de una rama.
    pub fn set_children(&mut self, path: &Path, children: Vec<Branch>) {
        self.children.insert(path.to_path_buf(), children);
    }

    /// Olvida lo leído de una rama para que se relea al volver a mirarla.
    ///
    /// Se usa al refrescar y cuando la vigilancia avisa de que esa carpeta
    /// cambió. No la colapsa: el usuario la dejó abierta.
    pub fn forget(&mut self, path: &Path) {
        self.children.remove(path);
    }

    /// Qué ancestros hay que desplegar para que `target` se vea.
    ///
    /// Van de fuera hacia dentro, empezando por la raíz **más profunda** que
    /// contenga a `target`: la carpeta personal antes que `/`, porque abrir el
    /// árbol por la rama de `/home/ana` deja al usuario donde vive y abrirlo por
    /// `/` le hace bajar cuatro niveles para ver lo mismo.
    ///
    /// `target` no se incluye: se enseña, pero desplegarlo no hace falta.
    #[must_use]
    pub fn path_to_reveal(&self, target: &Path) -> Vec<PathBuf> {
        let Some(root) = self.deepest_root_containing(target) else {
            return Vec::new();
        };

        let mut chain = Vec::new();
        let mut current = target;
        // Se sube hasta la raíz recogiendo ancestros, y luego se le da la vuelta:
        // desplegar de dentro hacia fuera dejaría ramas abiertas colgando de
        // padres cerrados.
        while let Some(parent) = current.parent() {
            if current == root {
                break;
            }
            chain.push(parent.to_path_buf());
            if parent == root {
                break;
            }
            current = parent;
        }
        chain.reverse();
        chain
    }

    /// La raíz más específica que contiene a `target`, si alguna.
    fn deepest_root_containing(&self, target: &Path) -> Option<&Path> {
        self.sections
            .iter()
            .flat_map(|section| section.roots.iter())
            .map(|branch| branch.path.as_path())
            .filter(|root| target.starts_with(root))
            .max_by_key(|root| root.components().count())
    }

    /// Aplana el árbol a la lista de filas que se ven.
    #[must_use]
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for section in &self.sections {
            rows.push(Row {
                depth: 0,
                kind: RowKind::Section(section.id),
            });
            for root in &section.roots {
                self.push_branch(root, 1, &mut rows);
            }
        }
        rows
    }

    fn push_branch(&self, branch: &Branch, depth: usize, rows: &mut Vec<Row>) {
        if depth > MAX_DEPTH {
            return;
        }
        let expanded = self.is_expanded(&branch.path);
        rows.push(Row {
            depth,
            kind: RowKind::Folder {
                path: branch.path.clone(),
                name: branch.name.clone(),
                expandable: self.expandable(&branch.path),
                expanded,
            },
        });

        if !expanded {
            return;
        }
        if let Some(children) = self.children.get(&branch.path) {
            for child in children {
                self.push_branch(child, depth + 1, rows);
            }
        }
    }
}
