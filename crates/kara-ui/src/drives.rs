//! The remote drives panel and its dialogs, as a second QML object.
//!
//! `Drives` is separate from `App` on purpose: the panel, the «Add drive»
//! dialog and the connection prompts share nothing with the folder view but a
//! window, and `App` is already the biggest file in the crate. It owns the
//! process-wide [`DriveRegistry`] (see [`registry`]), mirrors it into
//! properties QML binds to, and runs everything that can block (connecting,
//! the keyring, the connection test) on worker threads that report back with
//! `qt_thread().queue`, exactly like `App`'s listings.
//!
//! What the window says lives in `remote_present.rs`, what is validated lives
//! in `kara_remote::form`, and how drives reach `settings.conf` in
//! `remote_store.rs`; this file only moves values between them and Qt.
//!
//! **Browsing a connected drive is not wired yet.** Tabs, history and the
//! listing path hold `PathBuf`s; `docs/remote-ops-integration.md` makes
//! Location-aware tabs the prerequisite. Clicking a connected drive emits
//! `open_requested` with the drive's `kara+<scheme>://…` URI, and `App::navigate`
//! says so instead of failing silently. [`registry`] is what the ops path will
//! take its resolver from (`registry().resolver()` into `spawn_with`).

use core::pin::Pin;
use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use kara_remote::form::{FieldError, Form};
use kara_remote::keyring::KeyringSecretStore;
use kara_remote::objstore::{GcsFactory, S3Factory};
use kara_remote::sftp::SftpFactory;
use kara_remote::{
    ConnectError, ConnectOrRegistryError, ConnectionState, DriveRegistry, FallbackSecretStore,
    MemorySecretStore, Prompt, PromptAnswer, PromptHandler, SecretStore,
};
use kara_vfs::{Cancel, DriveId, Location, RemotePath};

use crate::remote_present::{
    self as present, DriveRow, PromptView, connect_error_text, form_view, menu_state, parse_row_id,
    prompt_answer, prompt_view, registry_error_text, rows,
};
use crate::remote_store;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        // ---- The list in the left panel, in parallel lists -----------------
        #[qproperty(QStringList, drive_ids)]
        #[qproperty(QStringList, drive_labels)]
        #[qproperty(QStringList, drive_subtitles)]
        /// `disconnected`, `connecting`, `ready`, `lost` or `failed`.
        #[qproperty(QStringList, drive_states)]
        #[qproperty(QStringList, drive_state_texts)]
        /// Bit 1 connect, 2 disconnect, 4 edit.
        #[qproperty(QList_i32, drive_flags)]
        #[qproperty(i32, drive_count)]
        /// Whether this build has any remote protocol to add a drive for.
        #[qproperty(bool, adapters_available)]
        /// «Las contraseñas no se guardarán», or empty when they will be.
        #[qproperty(QString, secrets_note)]
        /// A line about what just happened (connect failed, drive added...).
        #[qproperty(QString, notice)]
        // ---- The «Add drive» dialog ----------------------------------------
        #[qproperty(bool, dialog_open)]
        #[qproperty(bool, form_editing)]
        #[qproperty(QStringList, scheme_ids)]
        #[qproperty(QStringList, scheme_titles)]
        #[qproperty(i32, scheme_index)]
        #[qproperty(QString, form_name)]
        #[qproperty(QString, form_label)]
        #[qproperty(QString, form_group)]
        #[qproperty(QString, form_name_error)]
        #[qproperty(QString, form_label_error)]
        #[qproperty(QString, form_group_error)]
        #[qproperty(QString, form_secret_error)]
        #[qproperty(QString, form_general_error)]
        #[qproperty(bool, form_secret_visible)]
        #[qproperty(QString, form_secret_label)]
        #[qproperty(QString, form_secret_hint)]
        #[qproperty(bool, form_remember)]
        #[qproperty(QStringList, form_keys)]
        #[qproperty(QStringList, form_labels)]
        /// `text`, `number`, `path`, `toggle` or `choice`.
        #[qproperty(QStringList, form_kinds)]
        #[qproperty(QStringList, form_hints)]
        #[qproperty(QStringList, form_defaults)]
        /// `value=Label|value=Label` for a `choice`, else empty.
        #[qproperty(QStringList, form_choices)]
        #[qproperty(QStringList, form_values)]
        #[qproperty(QStringList, form_errors)]
        #[qproperty(QList_i32, form_visible)]
        #[qproperty(QList_i32, form_advanced)]
        #[qproperty(QList_i32, form_required)]
        /// Saving or testing is in progress.
        #[qproperty(bool, form_busy)]
        /// `running`, `ok`, `error`, or empty when no test has been run.
        #[qproperty(QString, test_state)]
        #[qproperty(QString, test_text)]
        // ---- A question in the middle of connecting ------------------------
        #[qproperty(bool, prompt_open)]
        /// `password`, `trust` or `changed`.
        #[qproperty(QString, prompt_kind)]
        #[qproperty(QString, prompt_title)]
        #[qproperty(QString, prompt_text)]
        #[qproperty(QString, prompt_detail)]
        // ---- Confirm removing a drive --------------------------------------
        #[qproperty(bool, remove_prompt)]
        #[qproperty(QString, remove_text)]
        type Drives = super::DrivesRust;

        /// A click on a connected drive: the `kara+<scheme>://…` URI of its root.
        #[qsignal]
        fn open_requested(self: Pin<&mut Drives>, uri: QString);

        /// Click on a row: opens a connected drive, connects any other.
        #[qinvokable]
        fn activate(self: Pin<&mut Drives>, id: &QString);

        #[qinvokable]
        fn connect_drive(self: Pin<&mut Drives>, id: &QString);

        #[qinvokable]
        fn disconnect_drive(self: Pin<&mut Drives>, id: &QString);

        /// Asks to forget a drive; nothing happens until `confirm_remove`.
        #[qinvokable]
        fn ask_remove(self: Pin<&mut Drives>, id: &QString);

        #[qinvokable]
        fn confirm_remove(self: Pin<&mut Drives>);

        #[qinvokable]
        fn cancel_remove(self: Pin<&mut Drives>);

        #[qinvokable]
        fn open_add(self: Pin<&mut Drives>);

        #[qinvokable]
        fn open_edit(self: Pin<&mut Drives>, id: &QString);

        #[qinvokable]
        fn select_scheme(self: Pin<&mut Drives>, index: i32);

        /// Sets `name`, `label`, `group`, `secret` or a parameter by its key.
        #[qinvokable]
        fn form_set(self: Pin<&mut Drives>, key: &QString, value: &QString);

        #[qinvokable]
        fn form_set_remember(self: Pin<&mut Drives>, remember: bool);

        /// Connects with what the form says, without saving anything.
        #[qinvokable]
        fn form_test(self: Pin<&mut Drives>);

        #[qinvokable]
        fn form_submit(self: Pin<&mut Drives>);

        #[qinvokable]
        fn close_dialog(self: Pin<&mut Drives>);

        /// Answers the open question. `remember`: for a host key 1 = write it
        /// down; for a password 1 = this drive, 2 = its group.
        #[qinvokable]
        fn answer_prompt(self: Pin<&mut Drives>, accept: bool, text: &QString, remember: i32);

        #[qinvokable]
        fn dismiss_notice(self: Pin<&mut Drives>);
    }

    impl cxx_qt::Threading for Drives {}
    impl cxx_qt::Initialize for Drives {}

    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        include!("cxx-qt-lib/qstringlist.h");
        include!("cxx-qt-lib/qlist.h");
        type QString = cxx_qt_lib::QString;
        type QStringList = cxx_qt_lib::QStringList;
        type QList_i32 = cxx_qt_lib::QList<i32>;
    }
}

/// The registry of this process: the keyring (falling back to memory) for
/// secrets, and one factory per protocol compiled in.
pub fn registry() -> Arc<DriveRegistry> {
    static REGISTRY: OnceLock<Arc<DriveRegistry>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| {
            let primary: Box<dyn SecretStore> = match KeyringSecretStore::open() {
                Ok(store) => Box::new(store),
                Err(_) => Box::new(MemorySecretStore::new()),
            };
            let registry = DriveRegistry::new(Arc::new(FallbackSecretStore::new(primary)));
            registry.register_factory(SftpFactory::new());
            registry.register_factory(S3Factory::new());
            registry.register_factory(GcsFactory::new());
            registry
        })
        .clone()
}

type ThreadHandle = CxxQtThread<qobject::Drives>;
type PendingAnswer = Arc<Mutex<Option<Sender<PromptAnswer>>>>;

/// Everything `Drives` keeps besides its properties.
pub struct DrivesRust {
    drive_ids: QStringList,
    drive_labels: QStringList,
    drive_subtitles: QStringList,
    drive_states: QStringList,
    drive_state_texts: QStringList,
    drive_flags: cxx_qt_lib::QList<i32>,
    drive_count: i32,
    adapters_available: bool,
    secrets_note: QString,
    notice: QString,
    dialog_open: bool,
    form_editing: bool,
    scheme_ids: QStringList,
    scheme_titles: QStringList,
    scheme_index: i32,
    form_name: QString,
    form_label: QString,
    form_group: QString,
    form_name_error: QString,
    form_label_error: QString,
    form_group_error: QString,
    form_secret_error: QString,
    form_general_error: QString,
    form_secret_visible: bool,
    form_secret_label: QString,
    form_secret_hint: QString,
    form_remember: bool,
    form_keys: QStringList,
    form_labels: QStringList,
    form_kinds: QStringList,
    form_hints: QStringList,
    form_defaults: QStringList,
    form_choices: QStringList,
    form_values: QStringList,
    form_errors: QStringList,
    form_visible: cxx_qt_lib::QList<i32>,
    form_advanced: cxx_qt_lib::QList<i32>,
    form_required: cxx_qt_lib::QList<i32>,
    form_busy: bool,
    test_state: QString,
    test_text: QString,
    prompt_open: bool,
    prompt_kind: QString,
    prompt_title: QString,
    prompt_text: QString,
    prompt_detail: QString,
    remove_prompt: bool,
    remove_text: QString,

    /// The form being edited, while the dialog is open.
    form: Option<Form>,
    errors: Vec<FieldError>,
    /// Cancels the connection test in flight, if any.
    test_cancel: Option<Cancel>,
    /// Numbers the tests, so a late result of an abandoned one is dropped.
    test_generation: u64,
    /// The drive waiting for the removal answer.
    remove_target: Option<DriveId>,
    /// Where the worker waits for the open question's answer.
    pending: PendingAnswer,
    /// One question at a time, even with several drives connecting.
    gate: Arc<Mutex<()>>,
    settings_path: Option<PathBuf>,
}

impl Default for DrivesRust {
    fn default() -> Self {
        Self {
            drive_ids: QStringList::default(),
            drive_labels: QStringList::default(),
            drive_subtitles: QStringList::default(),
            drive_states: QStringList::default(),
            drive_state_texts: QStringList::default(),
            drive_flags: cxx_qt_lib::QList::default(),
            drive_count: 0,
            adapters_available: false,
            secrets_note: QString::default(),
            notice: QString::default(),
            dialog_open: false,
            form_editing: false,
            scheme_ids: QStringList::default(),
            scheme_titles: QStringList::default(),
            scheme_index: 0,
            form_name: QString::default(),
            form_label: QString::default(),
            form_group: QString::default(),
            form_name_error: QString::default(),
            form_label_error: QString::default(),
            form_group_error: QString::default(),
            form_secret_error: QString::default(),
            form_general_error: QString::default(),
            form_secret_visible: false,
            form_secret_label: QString::default(),
            form_secret_hint: QString::default(),
            form_remember: true,
            form_keys: QStringList::default(),
            form_labels: QStringList::default(),
            form_kinds: QStringList::default(),
            form_hints: QStringList::default(),
            form_defaults: QStringList::default(),
            form_choices: QStringList::default(),
            form_values: QStringList::default(),
            form_errors: QStringList::default(),
            form_visible: cxx_qt_lib::QList::default(),
            form_advanced: cxx_qt_lib::QList::default(),
            form_required: cxx_qt_lib::QList::default(),
            form_busy: false,
            test_state: QString::default(),
            test_text: QString::default(),
            prompt_open: false,
            prompt_kind: QString::default(),
            prompt_title: QString::default(),
            prompt_text: QString::default(),
            prompt_detail: QString::default(),
            remove_prompt: false,
            remove_text: QString::default(),
            form: None,
            errors: Vec::new(),
            test_cancel: None,
            test_generation: 0,
            remove_target: None,
            pending: Arc::new(Mutex::new(None)),
            gate: Arc::new(Mutex::new(())),
            settings_path: None,
        }
    }
}

fn strings<I: IntoIterator<Item = String>>(values: I) -> QStringList {
    values.into_iter().map(|s| QString::from(&s)).collect()
}

fn ints<I: IntoIterator<Item = i32>>(values: I) -> cxx_qt_lib::QList<i32> {
    let mut list = cxx_qt_lib::QList::<i32>::default();
    for value in values {
        list.append(value);
    }
    list
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Asks the user through the dialog, from the worker that is connecting.
struct UiPrompts {
    thread: ThreadHandle,
    pending: PendingAnswer,
    gate: Arc<Mutex<()>>,
}

impl PromptHandler for UiPrompts {
    fn ask(&self, prompt: &Prompt) -> PromptAnswer {
        let _one_at_a_time = lock(&self.gate);
        let (sender, receiver) = channel();
        *lock(&self.pending) = Some(sender);
        let view = prompt_view(prompt);
        if self.thread.queue(move |drives| drives.show_prompt(&view)).is_err() {
            *lock(&self.pending) = None;
            return PromptAnswer::Refuse;
        }
        // A closed dialog answers Refuse; a vanished window drops the sender.
        receiver.recv().unwrap_or(PromptAnswer::Refuse)
    }
}

fn row_flags(state: &str) -> i32 {
    let menu = menu_state(state);
    i32::from(menu.connect) | (i32::from(menu.disconnect) << 1) | (i32::from(menu.edit) << 2)
}

impl cxx_qt::Initialize for qobject::Drives {
    fn initialize(mut self: Pin<&mut Self>) {
        let registry = registry();
        let path = kara_fs::settings::default_path().ok();
        self.as_mut().rust_mut().get_mut().settings_path.clone_from(&path);
        let mut problems = Vec::new();
        if let Some(path) = &path {
            let (configs, notes) = remote_store::load_configs(path);
            problems.extend(notes);
            // Already there when a second window asks: not a problem.
            for (id, error) in registry.add_all(configs) {
                if error != kara_remote::RegistryError::AlreadyExists {
                    problems.push(format!("{}: {}", present::row_id(&id), registry_error_text(&error)));
                }
            }
        }
        if !problems.is_empty() {
            self.as_mut().set_notice(QString::from(&problems.join("\n")));
        }
        let thread = self.qt_thread();
        registry.subscribe(move |_, _| {
            let _ = thread.queue(|drives| drives.refresh());
        });
        self.as_mut().refresh();
    }
}

impl qobject::Drives {
    /// Mirrors the registry into the list properties.
    fn refresh(mut self: Pin<&mut Self>) {
        let registry = registry();
        let configs = registry.configs();
        let list: Vec<DriveRow> = rows(&configs, |id| {
            registry.state(id).unwrap_or(ConnectionState::Disconnected)
        });
        self.as_mut().set_drive_count(i32::try_from(list.len()).unwrap_or(i32::MAX));
        self.as_mut().set_drive_ids(strings(list.iter().map(|r| r.id.clone())));
        self.as_mut().set_drive_labels(strings(list.iter().map(|r| r.label.clone())));
        self.as_mut().set_drive_subtitles(strings(list.iter().map(|r| r.subtitle.clone())));
        self.as_mut().set_drive_states(strings(list.iter().map(|r| r.state.to_owned())));
        self.as_mut().set_drive_state_texts(strings(list.iter().map(|r| r.state_text.clone())));
        self.as_mut().set_drive_flags(ints(list.iter().map(|r| row_flags(r.state))));
        let any = kara_remote::form::kinds()
            .iter()
            .any(|kind| registry.schemes().iter().any(|s| s == kind.scheme));
        self.as_mut().set_adapters_available(any);
        let note = if registry.secrets_are_persistent() {
            QString::default()
        } else {
            QString::from("Las contraseñas no se guardarán: no hay un llavero disponible.")
        };
        self.as_mut().set_secrets_note(note);
    }

    fn notify(mut self: Pin<&mut Self>, text: &str) {
        self.as_mut().set_notice(QString::from(text));
    }

    fn id_of(id: &QString) -> Option<DriveId> {
        parse_row_id(&id.to_string())
    }

    fn label_of(id: &DriveId) -> String {
        registry()
            .config(id)
            .map_or_else(|| present::row_id(id), |config| config.label)
    }

    fn prompts(&self) -> UiPrompts {
        UiPrompts {
            thread: self.qt_thread(),
            pending: self.rust().pending.clone(),
            gate: self.rust().gate.clone(),
        }
    }

    fn show_prompt(mut self: Pin<&mut Self>, view: &PromptView) {
        self.as_mut().set_prompt_kind(QString::from(view.kind));
        self.as_mut().set_prompt_title(QString::from(&view.title));
        self.as_mut().set_prompt_text(QString::from(&view.text));
        self.as_mut().set_prompt_detail(QString::from(&view.detail));
        self.as_mut().set_prompt_open(true);
    }

    /// Connects on a worker. `then_open` navigates into the drive afterwards.
    fn start_connect(self: Pin<&mut Self>, id: DriveId, then_open: bool) {
        let prompts = self.prompts();
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let result = registry().connect(&id, &prompts, &Cancel::new());
            let _ = thread.queue(move |drives| drives.finish_connect(&id, then_open, result));
        });
    }

    fn finish_connect(
        mut self: Pin<&mut Self>,
        id: &DriveId,
        then_open: bool,
        result: Result<(), ConnectOrRegistryError>,
    ) {
        self.as_mut().refresh();
        match result {
            Ok(()) => {
                if then_open {
                    self.as_mut().open_root(id);
                }
            }
            Err(ConnectOrRegistryError::Connect(ConnectError::Cancelled)) => {}
            Err(error) => {
                let reason = match &error {
                    ConnectOrRegistryError::Connect(error) => connect_error_text(error),
                    ConnectOrRegistryError::Registry(error) => registry_error_text(error),
                };
                let text = format!("No se pudo conectar con «{}»: {reason}", Self::label_of(id));
                self.as_mut().notify(&text);
            }
        }
    }

    fn open_root(mut self: Pin<&mut Self>, id: &DriveId) {
        let location = Location::Remote {
            drive: id.clone(),
            path: RemotePath::root(),
        };
        if let Ok(uri) = location.to_uri() {
            self.as_mut().open_requested(QString::from(&uri));
        }
    }

    fn activate(mut self: Pin<&mut Self>, id: &QString) {
        let Some(id) = Self::id_of(id) else { return };
        match registry().state(&id) {
            Some(ConnectionState::Ready) => self.as_mut().open_root(&id),
            Some(ConnectionState::Connecting) | None => {}
            Some(_) => self.start_connect(id, true),
        }
    }

    fn connect_drive(self: Pin<&mut Self>, id: &QString) {
        let Some(id) = Self::id_of(id) else { return };
        if matches!(
            registry().state(&id),
            Some(ConnectionState::Connecting | ConnectionState::Ready) | None
        ) {
            return;
        }
        self.start_connect(id, false);
    }

    fn disconnect_drive(mut self: Pin<&mut Self>, id: &QString) {
        let Some(id) = Self::id_of(id) else { return };
        if let Err(error) = registry().disconnect(&id) {
            self.as_mut().notify(&registry_error_text(&error));
        }
    }

    fn ask_remove(mut self: Pin<&mut Self>, id: &QString) {
        let Some(id) = Self::id_of(id) else { return };
        let text = format!(
            "¿Quitar «{}» de la lista? Se olvidan su configuración y la contraseña guardada; no se borra nada en el servidor.",
            Self::label_of(&id)
        );
        self.as_mut().rust_mut().get_mut().remove_target = Some(id);
        self.as_mut().set_remove_text(QString::from(&text));
        self.as_mut().set_remove_prompt(true);
    }

    fn cancel_remove(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().get_mut().remove_target = None;
        self.as_mut().set_remove_prompt(false);
    }

    fn confirm_remove(mut self: Pin<&mut Self>) {
        let target = self.as_mut().rust_mut().get_mut().remove_target.take();
        self.as_mut().set_remove_prompt(false);
        let Some(id) = target else { return };
        let thread = self.qt_thread();
        // Forgetting the secret may talk to the keyring.
        std::thread::spawn(move || {
            let label = registry()
                .config(&id)
                .map_or_else(|| present::row_id(&id), |config| config.label);
            let result = registry().remove(&id);
            let _ = thread.queue(move |drives| drives.finish_remove(&id, &label, result));
        });
    }

    fn finish_remove(
        mut self: Pin<&mut Self>,
        id: &DriveId,
        label: &str,
        result: Result<(), kara_remote::RegistryError>,
    ) {
        self.as_mut().refresh();
        if let Err(error) = result {
            self.as_mut().notify(&registry_error_text(&error));
            return;
        }
        let saved = match &self.rust().settings_path {
            Some(path) => remote_store::forget_config(path, id).err(),
            None => None,
        };
        let text = match saved {
            Some(error) => format!("«{label}» se quitó, pero no se pudo actualizar los ajustes: {error}"),
            None => format!("«{label}» se quitó de la lista"),
        };
        self.as_mut().notify(&text);
    }

    // ---- The dialog --------------------------------------------------------

    fn available_kinds() -> Vec<&'static kara_remote::form::DriveKind> {
        let schemes = registry().schemes();
        kara_remote::form::kinds()
            .iter()
            .filter(|kind| schemes.iter().any(|s| s == kind.scheme))
            .collect()
    }

    fn open_dialog(mut self: Pin<&mut Self>, form: Form) {
        let kinds = Self::available_kinds();
        let index = kinds
            .iter()
            .position(|kind| kind.scheme == form.kind().scheme)
            .unwrap_or(0);
        self.as_mut().set_scheme_ids(strings(kinds.iter().map(|k| k.scheme.to_owned())));
        self.as_mut().set_scheme_titles(strings(kinds.iter().map(|k| k.title.to_owned())));
        self.as_mut().set_scheme_index(i32::try_from(index).unwrap_or(0));
        self.as_mut().set_form_editing(form.is_editing());
        {
            let state = self.as_mut().rust_mut().get_mut();
            state.form = Some(form);
            state.errors.clear();
        }
        self.as_mut().set_test_state(QString::default());
        self.as_mut().set_test_text(QString::default());
        self.as_mut().set_form_busy(false);
        self.as_mut().publish_form();
        self.as_mut().set_dialog_open(true);
    }

    fn open_add(mut self: Pin<&mut Self>) {
        let Some(kind) = Self::available_kinds().first().copied() else {
            self.as_mut()
                .notify("Esta versión de Kara no incluye ningún protocolo de unidades remotas");
            return;
        };
        if let Some(form) = Form::new(kind.scheme) {
            self.open_dialog(form);
        }
    }

    fn open_edit(mut self: Pin<&mut Self>, id: &QString) {
        let Some(id) = Self::id_of(id) else { return };
        let Some(config) = registry().config(&id) else { return };
        match Form::editing(&config) {
            Some(form) => self.open_dialog(form),
            None => self
                .as_mut()
                .notify("Esta versión de Kara no sabe editar esa unidad"),
        }
    }

    fn select_scheme(mut self: Pin<&mut Self>, index: i32) {
        let Some(old) = self.rust().form.clone() else { return };
        if old.is_editing() {
            return;
        }
        let kinds = Self::available_kinds();
        let Some(kind) = usize::try_from(index).ok().and_then(|i| kinds.get(i).copied()) else {
            return;
        };
        let Some(mut form) = Form::new(kind.scheme) else { return };
        // What is common to every protocol is kept across the switch.
        form.name = old.name;
        form.label = old.label;
        form.group = old.group;
        form.secret = old.secret;
        form.remember = old.remember;
        self.as_mut().set_scheme_index(index);
        {
            let state = self.as_mut().rust_mut().get_mut();
            state.form = Some(form);
            state.errors.clear();
        }
        self.as_mut().cancel_test();
        self.as_mut().publish_form();
    }

    fn form_set(mut self: Pin<&mut Self>, key: &QString, value: &QString) {
        let key = key.to_string();
        let value = value.to_string();
        {
            let state = self.as_mut().rust_mut().get_mut();
            let Some(form) = state.form.as_mut() else { return };
            match key.as_str() {
                "name" => form.name = value,
                "label" => form.label = value,
                "group" => form.group = value,
                "secret" => form.secret = value,
                other => {
                    let Some(index) = form.kind().fields.iter().position(|f| f.key == other) else {
                        return;
                    };
                    form.set_value(index, &value);
                }
            }
            // Editing a box clears its complaint; the next submit re-checks.
            state.errors.retain(|e| e.field != key);
        }
        // What was tested is no longer what the form says.
        if *self.test_state() != QString::default() && !*self.form_busy() {
            self.as_mut().set_test_state(QString::default());
            self.as_mut().set_test_text(QString::default());
        }
        self.as_mut().publish_form();
    }

    fn form_set_remember(mut self: Pin<&mut Self>, remember: bool) {
        if let Some(form) = self.as_mut().rust_mut().get_mut().form.as_mut() {
            form.remember = remember;
        }
        self.as_mut().set_form_remember(remember);
    }

    /// Pushes the form and its errors into the properties.
    fn publish_form(mut self: Pin<&mut Self>) {
        let Some(form) = self.rust().form.clone() else { return };
        let view = form_view(&form, &self.rust().errors);
        self.as_mut().set_form_name(QString::from(&form.name));
        self.as_mut().set_form_label(QString::from(&form.label));
        self.as_mut().set_form_group(QString::from(&form.group));
        self.as_mut().set_form_remember(form.remember);
        self.as_mut().set_form_keys(strings(view.keys));
        self.as_mut().set_form_labels(strings(view.labels));
        self.as_mut().set_form_kinds(strings(view.kinds.iter().map(|k| (*k).to_owned())));
        self.as_mut().set_form_hints(strings(view.hints));
        self.as_mut().set_form_defaults(strings(view.defaults));
        self.as_mut().set_form_choices(strings(view.choices));
        self.as_mut().set_form_values(strings(view.values));
        self.as_mut().set_form_errors(strings(view.errors));
        self.as_mut().set_form_visible(ints(view.visible.iter().map(|b| i32::from(*b))));
        self.as_mut().set_form_advanced(ints(view.advanced.iter().map(|b| i32::from(*b))));
        self.as_mut().set_form_required(ints(view.required.iter().map(|b| i32::from(*b))));
        self.as_mut().set_form_name_error(QString::from(&view.name_error));
        self.as_mut().set_form_label_error(QString::from(&view.label_error));
        self.as_mut().set_form_group_error(QString::from(&view.group_error));
        self.as_mut().set_form_secret_error(QString::from(&view.secret_error));
        self.as_mut().set_form_general_error(QString::from(&view.general_error));
        self.as_mut().set_form_secret_visible(view.secret_visible);
        self.as_mut().set_form_secret_label(QString::from(&view.secret_label));
        self.as_mut().set_form_secret_hint(QString::from(&view.secret_hint));
    }

    fn cancel_test(mut self: Pin<&mut Self>) {
        let state = self.as_mut().rust_mut().get_mut();
        if let Some(cancel) = state.test_cancel.take() {
            cancel.cancel();
        }
        state.test_generation += 1;
        self.as_mut().set_test_state(QString::default());
        self.as_mut().set_test_text(QString::default());
        self.as_mut().set_form_busy(false);
    }

    fn form_test(mut self: Pin<&mut Self>) {
        if *self.form_busy() {
            return;
        }
        let Some(form) = self.rust().form.clone() else { return };
        let built = match form.build() {
            Ok(built) => built,
            Err(errors) => {
                self.as_mut().rust_mut().get_mut().errors = errors;
                self.as_mut().publish_form();
                return;
            }
        };
        let cancel = Cancel::new();
        let generation = {
            let state = self.as_mut().rust_mut().get_mut();
            state.errors.clear();
            state.test_generation += 1;
            state.test_cancel = Some(cancel.clone());
            state.test_generation
        };
        self.as_mut().publish_form();
        self.as_mut().set_form_busy(true);
        self.as_mut().set_test_state(QString::from("running"));
        self.as_mut().set_test_text(QString::from("Conectando…"));
        let prompts = self.prompts();
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let registry = registry();
            // A blank secret in the edit form means «the one already stored».
            let secret = built
                .secret
                .clone()
                .or_else(|| registry.stored_secret(&built.config));
            let result = registry.test_connection(&built.config, secret.as_ref(), &prompts, &cancel);
            let _ = thread.queue(move |drives| drives.finish_test(generation, result));
        });
    }

    fn finish_test(mut self: Pin<&mut Self>, generation: u64, result: Result<(), ConnectError>) {
        if self.rust().test_generation != generation {
            return;
        }
        self.as_mut().rust_mut().get_mut().test_cancel = None;
        self.as_mut().set_form_busy(false);
        match result {
            Ok(()) => {
                self.as_mut().set_test_state(QString::from("ok"));
                self.as_mut().set_test_text(QString::from("La conexión funciona"));
            }
            Err(ConnectError::Cancelled) => {
                self.as_mut().set_test_state(QString::default());
                self.as_mut().set_test_text(QString::default());
            }
            Err(error) => {
                self.as_mut().set_test_state(QString::from("error"));
                self.as_mut().set_test_text(QString::from(&connect_error_text(&error)));
            }
        }
    }

    fn form_submit(mut self: Pin<&mut Self>) {
        if *self.form_busy() {
            return;
        }
        let Some(form) = self.rust().form.clone() else { return };
        let built = match form.build() {
            Ok(built) => built,
            Err(errors) => {
                self.as_mut().rust_mut().get_mut().errors = errors;
                self.as_mut().publish_form();
                return;
            }
        };
        self.as_mut().rust_mut().get_mut().errors.clear();
        self.as_mut().publish_form();
        self.as_mut().set_form_busy(true);
        let editing = form.is_editing();
        let thread = self.qt_thread();
        // `remember_secret` may talk to the keyring.
        std::thread::spawn(move || {
            let registry = registry();
            let config = built.config.clone();
            let outcome = if editing {
                registry.update(config.clone()).map_err(|error| FieldError {
                    field: String::from("general"),
                    message: registry_error_text(&error),
                })
            } else {
                registry.add(config.clone()).map_err(|error| FieldError {
                    field: String::from(match error {
                        kara_remote::RegistryError::AlreadyExists => "name",
                        _ => "general",
                    }),
                    message: registry_error_text(&error),
                })
            };
            if outcome.is_ok()
                && built.remember
                && let Some(secret) = &built.secret
            {
                // The fallback store never fails; a keyring that does only
                // costs persistence, which `secrets_note` then reports.
                let _ = registry.remember_secret(&config.id, secret);
            }
            let _ = thread.queue(move |drives| drives.finish_submit(&config, editing, outcome));
        });
    }

    fn finish_submit(
        mut self: Pin<&mut Self>,
        config: &kara_remote::DriveConfig,
        editing: bool,
        outcome: Result<(), FieldError>,
    ) {
        self.as_mut().set_form_busy(false);
        if let Err(error) = outcome {
            self.as_mut().rust_mut().get_mut().errors = vec![error];
            self.as_mut().publish_form();
            return;
        }
        let saved = match &self.rust().settings_path {
            Some(path) => remote_store::save_config(path, config).err().map(|e| e.to_string()),
            None => Some(String::from("no hay dónde guardar los ajustes")),
        };
        let verb = if editing { "actualizó" } else { "añadió" };
        let text = match saved {
            Some(reason) => format!(
                "«{}» se {verb}, pero no se pudo guardar y desaparecerá al cerrar Kara: {reason}",
                config.label
            ),
            None => format!("«{}» se {verb}", config.label),
        };
        self.as_mut().notify(&text);
        self.as_mut().refresh();
        self.close_dialog();
    }

    fn close_dialog(mut self: Pin<&mut Self>) {
        self.as_mut().cancel_test();
        // A question asked by the test dies with it.
        self.as_mut().answer_prompt(false, &QString::default(), 0);
        {
            let state = self.as_mut().rust_mut().get_mut();
            state.form = None;
            state.errors.clear();
        }
        self.as_mut().set_dialog_open(false);
    }

    fn answer_prompt(mut self: Pin<&mut Self>, accept: bool, text: &QString, remember: i32) {
        let kind = self.prompt_kind().to_string();
        let sender = lock(&self.rust().pending).take();
        self.as_mut().set_prompt_open(false);
        self.as_mut().set_prompt_detail(QString::default());
        if let Some(sender) = sender {
            let _ = sender.send(prompt_answer(&kind, accept, &text.to_string(), remember));
        }
    }

    fn dismiss_notice(mut self: Pin<&mut Self>) {
        self.as_mut().set_notice(QString::default());
    }
}
