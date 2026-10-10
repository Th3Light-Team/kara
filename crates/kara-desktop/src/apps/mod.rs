//! Applications and the file types they open: what «Abrir con» lists, what
//! «usar siempre» changes, and how a chosen application is started.
//!
//! Reference convenience: `ground/spec/06-contexto-power.md`, «Abrir con».
//!
//! Everything here is read straight from the FreeDesktop files — `.desktop`
//! entries, `mimeapps.list`, shared-mime-info — which are what GIO (Nautilus)
//! and KService (Dolphin) read too. No desktop API is involved, so the answer
//! is the same on GNOME and on Plasma, and it matches what each of them shows
//! for the same file.

pub mod desktop_entry;
pub mod exec;
pub mod mime_tree;
pub mod mimeapps;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

pub use desktop_entry::AppInfo;
use desktop_entry::Parsed;
use mime_tree::MimeTree;
use mimeapps::MimeAppsList;

/// The XDG base directories, resolved once.
#[derive(Debug, Clone)]
pub struct BaseDirs {
    /// `$XDG_CONFIG_HOME`, then `$XDG_CONFIG_DIRS`.
    pub config: Vec<PathBuf>,
    /// `$XDG_DATA_HOME`, then `$XDG_DATA_DIRS`.
    pub data: Vec<PathBuf>,
}

impl BaseDirs {
    /// From the environment, with the spec's defaults for what is unset.
    #[must_use]
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let absolute = |var: &str| std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute());
        let list = |var: &str, default: &str| -> Vec<PathBuf> {
            let value = std::env::var(var).ok().filter(|v| !v.trim().is_empty());
            value
                .as_deref()
                .unwrap_or(default)
                .split(':')
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .collect()
        };

        let mut config = vec![absolute("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"))];
        config.extend(list("XDG_CONFIG_DIRS", "/etc/xdg"));
        let mut data = vec![absolute("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"))];
        data.extend(list("XDG_DATA_DIRS", "/usr/local/share:/usr/share"));
        Self { config, data }
    }

    /// The user's own `mimeapps.list`, the one Kara writes.
    #[must_use]
    pub fn user_mimeapps(&self) -> Option<PathBuf> {
        self.config.first().map(|dir| dir.join("mimeapps.list"))
    }
}

/// Every installed application and every association between types and
/// applications, as the desktop's files say right now.
#[derive(Debug, Clone, Default)]
pub struct Associations {
    /// By desktop file ID, after a user's file has shadowed the system one.
    apps: BTreeMap<String, AppInfo>,
    /// Data-directory order of the IDs, which breaks ties between
    /// associations the spec leaves unordered.
    order: Vec<String>,
    lists: Vec<MimeAppsList>,
    tree: MimeTree,
    desktops: Vec<String>,
}

impl Associations {
    /// Reads everything from disk. Takes a few tens of milliseconds on a
    /// desktop install: call it off the UI thread, or once.
    #[must_use]
    pub fn load(dirs: &BaseDirs, desktops: &[String], languages: &[String]) -> Self {
        let mut found: Vec<AppInfo> = Vec::new();
        let mut taken: HashSet<String> = HashSet::new();
        for dir in &dirs.data {
            let root = dir.join("applications");
            let mut entries = Vec::new();
            collect_desktop_files(&root, &root, &mut entries);
            entries.sort();
            for (id, path) in entries {
                // The first directory to name an ID owns it, even to delete it.
                if !taken.insert(id.clone()) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if let Parsed::App(app) = desktop_entry::parse(&text, &id, path, languages) {
                    found.push(*app);
                }
            }
        }

        let lists = mimeapps::search_paths(&dirs.config, &dirs.data, desktops)
            .iter()
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .map(|text| mimeapps::parse(&text))
            .collect();

        Self::from_parts(found, lists, MimeTree::load(&dirs.data), desktops)
    }

    /// Builds the set from already-read parts. `apps` in data-directory
    /// order, `lists` most important first.
    #[must_use]
    pub fn from_parts(apps: Vec<AppInfo>, lists: Vec<MimeAppsList>, tree: MimeTree, desktops: &[String]) -> Self {
        let order = apps.iter().map(|a| a.id.clone()).collect();
        let apps = apps
            .into_iter()
            .filter(installed)
            .map(|a| (a.id.clone(), a))
            .collect();
        Self {
            apps,
            order,
            lists,
            tree,
            desktops: desktops.to_vec(),
        }
    }

    #[must_use]
    pub fn app(&self, id: &str) -> Option<&AppInfo> {
        self.apps.get(id)
    }

    /// The canonical name of a MIME type.
    #[must_use]
    pub fn canonical(&self, mime: &str) -> String {
        self.tree.canonical(mime)
    }

    /// The application a double click would use for `mime`.
    ///
    /// The walk is GIO's (`g_app_info_get_default_for_type`), so Kara and
    /// Nautilus agree: for the type and then each ancestor in turn, the first
    /// installed `[Default Applications]` entry across the lists, else the
    /// most preferred association of that same type; only when the type has
    /// neither does the parent get a say.
    #[must_use]
    pub fn default_for(&self, mime: &str) -> Option<&AppInfo> {
        for kind in &self.lineage(mime) {
            for list in &self.lists {
                let Some(ids) = self.lookup(&list.defaults, kind) else {
                    continue;
                };
                if let Some(app) = ids.iter().find_map(|id| self.apps.get(id)) {
                    return Some(app);
                }
            }
            if let Some(app) = self.associated_exactly(kind).into_iter().next() {
                return Some(app);
            }
        }
        None
    }

    /// What «Abrir con» offers for `mime`: the default first, then the
    /// applications associated with the type itself (the user's recent picks
    /// first, as GIO orders them), then those that open it through an
    /// ancestor — the editor for `text/plain` under a Python script.
    #[must_use]
    pub fn apps_for(&self, mime: &str) -> Vec<&AppInfo> {
        let mut out: Vec<&AppInfo> = Vec::new();
        let mut push = |app: &AppInfo| {
            if !out.iter().any(|a| a.id == app.id)
                && let Some(app) = self.apps.get(&app.id)
            {
                out.push(app);
            }
        };
        if let Some(app) = self.default_for(mime) {
            push(app);
        }
        for kind in self.lineage(mime) {
            for app in self.associated_exactly(&kind) {
                push(app);
            }
        }
        out
    }

    /// What several selected files have in common: the applications that can
    /// open every one of the types, in the order of the first type. One
    /// application opening files it does not understand is worse than a
    /// shorter list.
    #[must_use]
    pub fn apps_for_all(&self, mimes: &[String]) -> Vec<&AppInfo> {
        let Some((first, rest)) = mimes.split_first() else {
            return Vec::new();
        };
        let others: Vec<HashSet<&str>> = rest
            .iter()
            .map(|m| self.apps_for(m).into_iter().map(|a| a.id.as_str()).collect())
            .collect();
        self.apps_for(first)
            .into_iter()
            .filter(|app| others.iter().all(|set| set.contains(app.id.as_str())))
            .collect()
    }

    /// Every application a menu would show, by name: what «Elegir otra
    /// aplicación» browses.
    #[must_use]
    pub fn all_apps(&self) -> Vec<&AppInfo> {
        let mut apps: Vec<&AppInfo> = self
            .apps
            .values()
            .filter(|a| !a.no_display && a.shown_in(&self.desktops))
            .collect();
        apps.sort_by_cached_key(|a| a.name.to_lowercase());
        apps
    }

    /// The type and its ancestors, canonical, nearest first.
    fn lineage(&self, mime: &str) -> Vec<String> {
        let mut lineage = vec![self.tree.canonical(mime)];
        lineage.extend(self.tree.ancestors(mime));
        lineage
    }

    fn lookup<'a>(&self, map: &'a BTreeMap<String, Vec<String>>, mime: &str) -> Option<&'a Vec<String>> {
        map.get(mime).or_else(|| {
            map.iter()
                .find(|(key, _)| self.tree.canonical(key) == mime)
                .map(|(_, ids)| ids)
        })
    }

    /// The applications associated with exactly `mime` (canonical), not
    /// through an ancestor: added associations from the most important list
    /// down, each list's removals applying to what comes after it, and then
    /// the `.desktop` files that declare the type.
    fn associated_exactly(&self, mime: &str) -> Vec<&AppInfo> {
        let mut out: Vec<&AppInfo> = Vec::new();
        let mut removed: HashSet<&str> = HashSet::new();
        for list in &self.lists {
            if let Some(ids) = self.lookup(&list.added, mime) {
                for id in ids {
                    if removed.contains(id.as_str()) || out.iter().any(|a| &a.id == id) {
                        continue;
                    }
                    if let Some(app) = self.apps.get(id) {
                        out.push(app);
                    }
                }
            }
            if let Some(ids) = self.lookup(&list.removed, mime) {
                removed.extend(ids.iter().map(String::as_str));
            }
        }
        for id in &self.order {
            let Some(app) = self.apps.get(id) else {
                continue;
            };
            if removed.contains(id.as_str()) || out.iter().any(|a| a.id == app.id) {
                continue;
            }
            if app.mime_types.iter().any(|m| self.tree.canonical(m) == mime) {
                out.push(app);
            }
        }
        out
    }
}

/// Whether an entry can be started: it has something to run, and its
/// `TryExec`, if any, exists.
fn installed(app: &AppInfo) -> bool {
    if app.exec.is_none() && !app.dbus_activatable {
        return false;
    }
    app.try_exec.as_deref().is_none_or(|program| find_program(program).is_some())
}

/// Looks a program up the way a shell would: an absolute path as is, a bare
/// name through `$PATH`.
#[must_use]
pub fn find_program(program: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let executable = |path: &Path| {
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        let path = PathBuf::from(program);
        return executable(&path).then_some(path);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| executable(candidate))
}

/// Finds every `.desktop` file under `root`, with its desktop file ID:
/// the path relative to `applications/`, `/` turned into `-`.
fn collect_desktop_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // Symlinked entries are common (alternatives, snaps); follow them
        // for files but not for directories, which could loop.
        if kind.is_dir() {
            collect_desktop_files(root, &path, out);
            continue;
        }
        if path.extension().is_none_or(|e| e != "desktop") {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let id = relative.to_string_lossy().replace('/', "-");
        out.push((id, path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, mimes: &[&str]) -> AppInfo {
        AppInfo {
            id: id.to_string(),
            name: id.trim_end_matches(".desktop").to_string(),
            generic_name: None,
            icon: None,
            exec: Some(format!("{} %U", id.trim_end_matches(".desktop"))),
            try_exec: None,
            path: None,
            terminal: false,
            no_display: false,
            dbus_activatable: false,
            only_show_in: Vec::new(),
            not_show_in: Vec::new(),
            mime_types: mimes.iter().map(|m| (*m).to_string()).collect(),
            source: PathBuf::from(format!("/usr/share/applications/{id}")),
        }
    }

    fn ids(apps: &[&AppInfo]) -> Vec<String> {
        apps.iter().map(|a| a.id.clone()).collect()
    }

    fn tree() -> MimeTree {
        let mut tree = MimeTree::default();
        tree.add_aliases("application/x-pdf application/pdf\n");
        tree.add_subclasses("text/x-python text/plain\n");
        tree
    }

    #[test]
    fn the_users_default_beats_the_distributions() {
        let user = mimeapps::parse("[Default Applications]\ntext/plain=gedit.desktop\n");
        let system = mimeapps::parse("[Default Applications]\ntext/plain=org.gnome.TextEditor.desktop\n");
        let set = Associations::from_parts(
            vec![app("org.gnome.TextEditor.desktop", &["text/plain"]), app("gedit.desktop", &["text/plain"])],
            vec![user, system],
            tree(),
            &[],
        );
        assert_eq!(set.default_for("text/plain").map(|a| a.id.as_str()), Some("gedit.desktop"));
    }

    #[test]
    fn a_default_that_is_not_installed_is_skipped() {
        let user = mimeapps::parse("[Default Applications]\ntext/plain=gone.desktop;org.gnome.TextEditor.desktop;\n");
        let set = Associations::from_parts(vec![app("org.gnome.TextEditor.desktop", &["text/plain"])], vec![user], tree(), &[]);
        assert_eq!(set.default_for("text/plain").map(|a| a.id.as_str()), Some("org.gnome.TextEditor.desktop"));
    }

    #[test]
    fn a_script_opens_with_the_plain_text_default_when_nothing_claims_it() {
        let system = mimeapps::parse("[Default Applications]\ntext/plain=org.gnome.TextEditor.desktop\n");
        let set = Associations::from_parts(
            vec![app("org.gnome.TextEditor.desktop", &["text/plain"]), app("idle.desktop", &["text/x-shellscript"])],
            vec![system],
            tree(),
            &[],
        );
        assert_eq!(set.default_for("text/x-python").map(|a| a.id.as_str()), Some("org.gnome.TextEditor.desktop"));
    }

    #[test]
    fn an_application_for_the_type_itself_beats_the_parents_default() {
        // GIO's order: the type's own associations are consulted before its
        // parent's default.
        let system = mimeapps::parse("[Default Applications]\ntext/plain=org.gnome.TextEditor.desktop\n");
        let set = Associations::from_parts(
            vec![app("org.gnome.TextEditor.desktop", &["text/plain"]), app("idle.desktop", &["text/x-python"])],
            vec![system],
            tree(),
            &[],
        );
        assert_eq!(set.default_for("text/x-python").map(|a| a.id.as_str()), Some("idle.desktop"));
        assert_eq!(ids(&set.apps_for("text/x-python")), ["idle.desktop", "org.gnome.TextEditor.desktop"]);
    }

    #[test]
    fn removed_associations_hide_what_the_desktop_file_declares() {
        let user = mimeapps::parse("[Removed Associations]\nimage/png=gimp.desktop;\n");
        let set = Associations::from_parts(
            vec![app("eog.desktop", &["image/png"]), app("gimp.desktop", &["image/png"])],
            vec![user],
            tree(),
            &[],
        );
        assert_eq!(ids(&set.apps_for("image/png")), ["eog.desktop"]);
    }

    #[test]
    fn recently_used_apps_come_right_after_the_default() {
        let user = mimeapps::parse(
            "[Default Applications]\nimage/png=eog.desktop\n[Added Associations]\nimage/png=krita.desktop;gimp.desktop;\n",
        );
        let set = Associations::from_parts(
            vec![app("eog.desktop", &["image/png"]), app("gimp.desktop", &["image/png"]), app("krita.desktop", &["image/png"])],
            vec![user],
            tree(),
            &[],
        );
        assert_eq!(ids(&set.apps_for("image/png")), ["eog.desktop", "krita.desktop", "gimp.desktop"]);
    }

    #[test]
    fn aliases_meet_their_canonical_type() {
        let set = Associations::from_parts(vec![app("evince.desktop", &["application/x-pdf"])], Vec::new(), tree(), &[]);
        assert_eq!(ids(&set.apps_for("application/pdf")), ["evince.desktop"]);
    }

    #[test]
    fn mixed_selections_offer_only_what_opens_every_type() {
        let set = Associations::from_parts(
            vec![
                app("eog.desktop", &["image/png"]),
                app("gimp.desktop", &["image/png", "image/jpeg"]),
                app("shotwell.desktop", &["image/jpeg"]),
            ],
            Vec::new(),
            tree(),
            &[],
        );
        assert_eq!(ids(&set.apps_for_all(&["image/png".into(), "image/jpeg".into()])), ["gimp.desktop"]);
        assert!(set.apps_for_all(&[]).is_empty());
    }

    #[test]
    fn hidden_from_menus_does_not_mean_unable_to_open() {
        let mut viewer = app("viewer.desktop", &["image/png"]);
        viewer.no_display = true;
        let set = Associations::from_parts(vec![viewer, app("eog.desktop", &[])], Vec::new(), tree(), &[]);
        assert_eq!(ids(&set.apps_for("image/png")), ["viewer.desktop"]);
        assert_eq!(ids(&set.all_apps()), ["eog.desktop"]);
    }

    #[test]
    fn an_entry_whose_try_exec_is_missing_is_not_installed() {
        let mut ghost = app("ghost.desktop", &["text/plain"]);
        ghost.try_exec = Some("kara-no-such-program".into());
        let set = Associations::from_parts(vec![ghost], Vec::new(), tree(), &[]);
        assert!(set.apps_for("text/plain").is_empty());
    }

    #[test]
    fn a_user_desktop_file_shadows_and_can_delete_the_system_one() -> std::io::Result<()> {
        let root = tempfile::tempdir()?;
        let user = root.path().join("home/applications");
        let system = root.path().join("usr/applications");
        std::fs::create_dir_all(&user)?;
        std::fs::create_dir_all(system.join("kde"))?;
        std::fs::write(
            system.join("a.desktop"),
            "[Desktop Entry]\nType=Application\nName=System A\nExec=a %U\nMimeType=text/plain;\n",
        )?;
        std::fs::write(user.join("a.desktop"), "[Desktop Entry]\nType=Application\nName=Mine\nExec=a %U\nMimeType=text/plain;\n")?;
        std::fs::write(system.join("b.desktop"), "[Desktop Entry]\nType=Application\nName=B\nExec=b\nMimeType=text/plain;\n")?;
        std::fs::write(user.join("b.desktop"), "[Desktop Entry]\nType=Application\nName=B\nHidden=true\n")?;
        std::fs::write(system.join("kde/c.desktop"), "[Desktop Entry]\nType=Application\nName=C\nExec=c\n")?;

        let dirs = BaseDirs {
            config: vec![root.path().join("config")],
            data: vec![root.path().join("home"), root.path().join("usr")],
        };
        let set = Associations::load(&dirs, &[], &[]);
        assert_eq!(set.app("a.desktop").map(|a| a.name.as_str()), Some("Mine"));
        assert!(set.app("b.desktop").is_none());
        assert!(set.app("kde-c.desktop").is_some());
        Ok(())
    }
}
