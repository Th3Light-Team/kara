//! MIME aliases and inheritance, from shared-mime-info's `aliases` and
//! `subclasses` files.
//!
//! «Open with» needs both: a `.desktop` file may name a type by an alias
//! (`application/x-pdf`), and a text editor registered for `text/plain` is a
//! fine way to open `text/x-python`, which descends from it.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

/// Aliases and parents of every known type.
#[derive(Debug, Clone, Default)]
pub struct MimeTree {
    aliases: HashMap<String, String>,
    parents: HashMap<String, Vec<String>>,
}

impl MimeTree {
    /// Reads `mime/aliases` and `mime/subclasses` from every data directory.
    #[must_use]
    pub fn load(data_dirs: &[PathBuf]) -> Self {
        let mut tree = Self::default();
        // Lowest precedence first, so a user's own files win.
        for dir in data_dirs.iter().rev() {
            let mime = dir.join("mime");
            if let Ok(text) = std::fs::read_to_string(mime.join("aliases")) {
                tree.add_aliases(&text);
            }
            if let Ok(text) = std::fs::read_to_string(mime.join("subclasses")) {
                tree.add_subclasses(&text);
            }
        }
        tree
    }

    /// Adds the lines of an `aliases` file: `alias canonical`.
    pub fn add_aliases(&mut self, text: &str) {
        for (alias, canonical) in pairs(text) {
            self.aliases.insert(alias.to_ascii_lowercase(), canonical.to_ascii_lowercase());
        }
    }

    /// Adds the lines of a `subclasses` file: `child parent`.
    pub fn add_subclasses(&mut self, text: &str) {
        for (child, parent) in pairs(text) {
            let parents = self.parents.entry(child.to_ascii_lowercase()).or_default();
            let parent = parent.to_ascii_lowercase();
            if !parents.contains(&parent) {
                parents.push(parent);
            }
        }
    }

    /// The canonical name of a type; MIME types compare case-insensitively.
    #[must_use]
    pub fn canonical(&self, mime: &str) -> String {
        let lower = mime.to_ascii_lowercase();
        self.aliases.get(&lower).cloned().unwrap_or(lower)
    }

    /// Every ancestor of `mime`, nearest first, without `mime` itself.
    ///
    /// Besides the declared parents, shared-mime-info makes every `text/*` a
    /// `text/plain` and every other type that is not a directory or a special
    /// file an `application/octet-stream`.
    #[must_use]
    pub fn ancestors(&self, mime: &str) -> Vec<String> {
        let start = self.canonical(mime);
        let mut seen: HashSet<String> = HashSet::from([start.clone()]);
        let mut order = Vec::new();
        let mut queue = VecDeque::from([start]);

        while let Some(current) = queue.pop_front() {
            let mut next: Vec<String> = self
                .parents
                .get(&current)
                .map(|ps| ps.iter().map(|p| self.canonical(p)).collect())
                .unwrap_or_default();
            if current.starts_with("text/") && current != "text/plain" {
                next.push("text/plain".to_string());
            }
            if !current.starts_with("inode/") && current != "application/octet-stream" {
                next.push("application/octet-stream".to_string());
            }
            for parent in next {
                if seen.insert(parent.clone()) {
                    order.push(parent.clone());
                    queue.push_back(parent);
                }
            }
        }

        // The catch-all goes last whatever path reached it first: an editor
        // for `text/plain` is a better guess for a script than a hex viewer.
        if let Some(index) = order.iter().position(|m| m == "application/octet-stream") {
            let generic = order.remove(index);
            order.push(generic);
        }
        order
    }
}

fn pairs(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let mut parts = line.split_whitespace();
        Some((parts.next()?, parts.next()?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> MimeTree {
        let mut tree = MimeTree::default();
        tree.add_aliases("application/x-pdf application/pdf\n");
        tree.add_subclasses(
            "text/x-python application/x-executable\ntext/x-python text/plain\napplication/x-shellscript text/plain\n",
        );
        tree
    }

    #[test]
    fn an_alias_resolves_to_its_canonical_name() {
        assert_eq!(tree().canonical("application/X-PDF"), "application/pdf");
        assert_eq!(tree().canonical("image/png"), "image/png");
    }

    #[test]
    fn a_script_descends_from_plain_text_before_the_generic_binary() {
        let ancestors = tree().ancestors("text/x-python");
        assert_eq!(ancestors.first().map(String::as_str), Some("application/x-executable"));
        assert!(ancestors.contains(&"text/plain".to_string()));
        assert_eq!(ancestors.last().map(String::as_str), Some("application/octet-stream"));
    }

    #[test]
    fn every_text_type_is_plain_text_even_undeclared() {
        assert_eq!(tree().ancestors("text/x-unknown"), ["text/plain", "application/octet-stream"]);
    }

    #[test]
    fn directories_have_no_generic_ancestor() {
        assert!(tree().ancestors("inode/directory").is_empty());
    }

    #[test]
    fn a_cycle_in_the_data_does_not_hang() {
        let mut tree = MimeTree::default();
        tree.add_subclasses("a/b a/c\na/c a/b\n");
        assert_eq!(tree.ancestors("a/b"), ["a/c", "application/octet-stream"]);
    }
}
