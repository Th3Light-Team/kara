//! How the remote drives panel and its dialogs read, with no Qt in sight.
//!
//! `drives.rs` (the bridge) only moves these values into properties. Everything
//! that decides what the window says about a drive, a prompt or a form field
//! lives here so it can be tested.

use kara_remote::form::{Choice, FieldError, FieldKind, Form};
use kara_remote::{
    ConnectError, ConnectionState, DriveConfig, Prompt, PromptAnswer, RegistryError, Remember,
    Secret,
};
use kara_vfs::DriveId;

/// One row of the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveRow {
    /// `scheme:name`, what QML hands back to identify the drive.
    pub id: String,
    pub label: String,
    /// Protocol, group and where it points, for the line under the label.
    pub subtitle: String,
    /// `disconnected`, `connecting`, `ready`, `lost` or `failed`.
    pub state: &'static str,
    /// What the state means, for the tooltip.
    pub state_text: String,
}

/// The id QML uses for a drive.
#[must_use]
pub fn row_id(id: &DriveId) -> String {
    format!("{}:{}", id.scheme(), id.name())
}

/// The inverse of [`row_id`].
#[must_use]
pub fn parse_row_id(text: &str) -> Option<DriveId> {
    let (scheme, name) = text.split_once(':')?;
    DriveId::new(scheme, name).ok()
}

fn short_scheme(scheme: &str) -> String {
    match scheme {
        "sftp" => String::from("SFTP"),
        "s3" => String::from("S3"),
        "gcs" => String::from("GCS"),
        other => other.to_uppercase(),
    }
}

/// `host`, or `bucket` (plus its prefix): where the drive points.
fn target(config: &DriveConfig) -> String {
    if let Some(host) = config.param("host") {
        return match config.param("user") {
            Some(user) => format!("{user}@{host}"),
            None => host.to_owned(),
        };
    }
    match (config.param("bucket"), config.param("prefix")) {
        (Some(bucket), Some(prefix)) if !prefix.is_empty() => format!("{bucket}/{prefix}"),
        (Some(bucket), _) => bucket.to_owned(),
        _ => String::new(),
    }
}

#[must_use]
pub fn state_code(state: &ConnectionState) -> &'static str {
    match state {
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::Connecting => "connecting",
        ConnectionState::Ready => "ready",
        ConnectionState::Lost { .. } => "lost",
        ConnectionState::Failed { .. } => "failed",
    }
}

#[must_use]
pub fn state_text(state: &ConnectionState) -> String {
    match state {
        ConnectionState::Disconnected => String::from("Desconectada"),
        ConnectionState::Connecting => String::from("Conectando…"),
        ConnectionState::Ready => String::from("Conectada"),
        ConnectionState::Lost { reason } => format!("Se perdió la conexión: {reason}"),
        ConnectionState::Failed { reason } => format!("No se pudo conectar: {reason}"),
    }
}

/// The rows, grouped (ungrouped first) and then by label.
#[must_use]
pub fn rows(
    configs: &[DriveConfig],
    state_of: impl Fn(&DriveId) -> ConnectionState,
) -> Vec<DriveRow> {
    let mut sorted: Vec<&DriveConfig> = configs.iter().collect();
    sorted.sort_by_key(|c| {
        (
            c.group.clone().unwrap_or_default().to_lowercase(),
            c.label.to_lowercase(),
            row_id(&c.id),
        )
    });
    sorted
        .into_iter()
        .map(|config| {
            let state = state_of(&config.id);
            let mut parts = Vec::new();
            if let Some(group) = &config.group {
                parts.push(group.clone());
            }
            parts.push(short_scheme(config.id.scheme()));
            let target = target(config);
            if !target.is_empty() {
                parts.push(target);
            }
            DriveRow {
                id: row_id(&config.id),
                label: config.label.clone(),
                subtitle: parts.join(" · "),
                state: state_code(&state),
                state_text: state_text(&state),
            }
        })
        .collect()
}

/// What the context menu may offer for a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuState {
    pub connect: bool,
    pub disconnect: bool,
    pub edit: bool,
}

#[must_use]
pub fn menu_state(state: &str) -> MenuState {
    match state {
        "ready" => MenuState { connect: false, disconnect: true, edit: true },
        "connecting" => MenuState { connect: false, disconnect: false, edit: false },
        _ => MenuState { connect: true, disconnect: false, edit: true },
    }
}

/// Spanish text for a failed connection.
#[must_use]
pub fn connect_error_text(error: &ConnectError) -> String {
    match error {
        ConnectError::AuthRequired => {
            String::from("Falta la contraseña, la frase de paso o la clave secreta")
        }
        ConnectError::AuthFailed => {
            String::from("El servidor rechazó las credenciales")
        }
        ConnectError::Unreachable(detail) => format!("No se puede llegar al servidor ({detail})"),
        ConnectError::HostKeyRefused => String::from("Rechazaste la clave del servidor"),
        ConnectError::Cancelled => String::from("Cancelado"),
        ConnectError::Other(detail) => detail.clone(),
    }
}

#[must_use]
pub fn registry_error_text(error: &RegistryError) -> String {
    match error {
        RegistryError::UnsupportedScheme(scheme) => {
            format!("Esta versión de Kara no incluye el protocolo {scheme}")
        }
        RegistryError::AlreadyExists => String::from("Ya hay una unidad con ese identificador"),
        RegistryError::Unknown => String::from("Esa unidad ya no existe"),
        RegistryError::Busy => String::from("La unidad está conectando"),
    }
}

/// A question to the user in the middle of connecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptView {
    /// `password`, `trust` or `changed`.
    pub kind: &'static str,
    pub title: String,
    pub text: String,
    /// Fingerprints, in a monospaced block.
    pub detail: String,
}

#[must_use]
pub fn prompt_view(prompt: &Prompt) -> PromptView {
    match prompt {
        Prompt::Password { label, .. } => PromptView {
            kind: "password",
            title: format!("Contraseña de «{label}»"),
            text: format!("«{label}» pide una contraseña o la frase de paso de su clave."),
            detail: String::new(),
        },
        Prompt::TrustHostKey {
            host, fingerprint, ..
        } => PromptView {
            kind: "trust",
            title: String::from("Servidor desconocido"),
            text: format!(
                "Es la primera vez que te conectas a {host}. Comprueba que la huella es la de tu servidor antes de confiar en él."
            ),
            detail: fingerprint.clone(),
        },
        Prompt::HostKeyChanged {
            host,
            known,
            presented,
            ..
        } => PromptView {
            kind: "changed",
            title: String::from("La clave del servidor ha cambiado"),
            text: format!(
                "{host} presenta una clave distinta de la que Kara conocía. Puede que lo hayan reinstalado, o que alguien se esté haciendo pasar por él. No continúes si no lo esperabas."
            ),
            detail: format!("Conocida:    {known}\nPresentada:  {presented}"),
        },
    }
}

/// What the dialog's buttons mean. `remember` is 0 no, 1 yes (for a host key),
/// and for a password 1 this drive, 2 its group.
#[must_use]
pub fn prompt_answer(kind: &str, accept: bool, text: &str, remember: i32) -> PromptAnswer {
    if !accept {
        return PromptAnswer::Refuse;
    }
    match kind {
        "password" if !text.is_empty() => PromptAnswer::Secret {
            secret: Secret::new(text),
            remember: match remember {
                1 => Remember::ForDrive,
                2 => Remember::ForGroup,
                _ => Remember::No,
            },
        },
        "trust" => PromptAnswer::Trust {
            remember: remember == 1,
        },
        // Never written down: the user removes the old line from known_hosts.
        "changed" => PromptAnswer::Trust { remember: false },
        _ => PromptAnswer::Refuse,
    }
}

/// The «Add drive» form as parallel lists, one entry per parameter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormView {
    pub keys: Vec<String>,
    pub labels: Vec<String>,
    /// `text`, `number`, `path`, `toggle` or `choice`.
    pub kinds: Vec<&'static str>,
    pub hints: Vec<String>,
    /// What the adapter assumes, shown as the placeholder.
    pub defaults: Vec<String>,
    /// `value=Label|value=Label`, empty unless the kind is `choice`.
    pub choices: Vec<String>,
    pub values: Vec<String>,
    pub visible: Vec<bool>,
    pub advanced: Vec<bool>,
    pub required: Vec<bool>,
    pub errors: Vec<String>,
    pub name_error: String,
    pub label_error: String,
    pub group_error: String,
    pub secret_error: String,
    pub secret_visible: bool,
    pub secret_label: String,
    pub secret_hint: String,
    /// Errors on a parameter the form does not show (should not happen).
    pub general_error: String,
}

fn choice_text(choices: &[Choice]) -> String {
    choices
        .iter()
        .map(|c| format!("{}={}", c.value, c.label))
        .collect::<Vec<_>>()
        .join("|")
}

fn kind_code(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Text => "text",
        FieldKind::Number { .. } => "number",
        FieldKind::Path => "path",
        FieldKind::Toggle => "toggle",
        FieldKind::Choice(_) => "choice",
    }
}

#[must_use]
pub fn form_view(form: &Form, errors: &[FieldError]) -> FormView {
    let fields = form.kind().fields;
    let error_of = |key: &str| -> String {
        errors
            .iter()
            .find(|e| e.field == key)
            .map(|e| e.message.clone())
            .unwrap_or_default()
    };
    let mut view = FormView::default();
    for (index, field) in fields.iter().enumerate() {
        view.keys.push(field.key.to_owned());
        view.labels.push(field.label.to_owned());
        view.kinds.push(kind_code(field.kind));
        view.hints.push(field.hint.to_owned());
        view.defaults.push(field.default.to_owned());
        view.choices.push(match field.kind {
            FieldKind::Choice(choices) => choice_text(choices),
            _ => String::new(),
        });
        view.values.push(form.values()[index].clone());
        view.visible.push(form.field_visible(index));
        view.advanced.push(field.advanced);
        view.required.push(field.required);
        view.errors.push(error_of(field.key));
    }
    view.name_error = error_of("name");
    view.label_error = error_of("label");
    view.group_error = error_of("group");
    view.secret_error = error_of("secret");
    view.general_error = errors
        .iter()
        .filter(|e| {
            !["name", "label", "group", "secret"].contains(&e.field.as_str())
                && !fields.iter().any(|f| f.key == e.field)
        })
        .map(|e| e.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(spec) = form.kind().secret {
        view.secret_visible = form.secret_visible();
        view.secret_label = spec.label.to_owned();
        view.secret_hint = spec.hint.to_owned();
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use kara_remote::form::FieldError;

    fn config(scheme: &str, name: &str, label: &str, params: &[(&str, &str)]) -> DriveConfig {
        DriveConfig::new(
            scheme,
            name,
            label,
            params.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
        )
        .expect("valid")
    }

    #[test]
    fn row_ids_round_trip() {
        let id = DriveId::new("sftp", "work-nas").expect("id");
        assert_eq!(row_id(&id), "sftp:work-nas");
        assert_eq!(parse_row_id("sftp:work-nas"), Some(id));
        assert_eq!(parse_row_id("nonsense"), None);
        assert_eq!(parse_row_id("sftp:Bad Name"), None);
    }

    #[test]
    fn rows_say_where_each_drive_points() {
        let configs = vec![
            config("sftp", "nas", "NAS", &[("host", "nas.local"), ("user", "ana")]),
            config("s3", "datos", "Datos", &[("bucket", "datos"), ("prefix", "2026")]),
            config("gcs", "bk", "Backups", &[("bucket", "bk")]),
        ];
        let rows = rows(&configs, |_| ConnectionState::Disconnected);
        let by_label = |label: &str| rows.iter().find(|r| r.label == label).expect("row");
        assert_eq!(by_label("NAS").subtitle, "SFTP · ana@nas.local");
        assert_eq!(by_label("Datos").subtitle, "S3 · datos/2026");
        assert_eq!(by_label("Backups").subtitle, "GCS · bk");
    }

    #[test]
    fn rows_sort_ungrouped_first_then_by_group_and_label() {
        let grouped = |name: &str, label: &str, group: &str| {
            config("sftp", name, label, &[("host", "h")])
                .with_group(group)
                .expect("group")
        };
        let configs = vec![
            grouped("b", "beta", "Proxmox"),
            config("sftp", "z", "Zeta", &[("host", "h")]),
            grouped("a", "alfa", "Proxmox"),
            config("sftp", "m", "mu", &[("host", "h")]),
        ];
        let labels: Vec<_> = rows(&configs, |_| ConnectionState::Disconnected)
            .into_iter()
            .map(|r| r.label)
            .collect();
        assert_eq!(labels, ["mu", "Zeta", "alfa", "beta"]);
    }

    #[test]
    fn group_shows_in_the_subtitle() {
        let configs = vec![
            config("sftp", "a", "a", &[("host", "h")])
                .with_group("Proxmox")
                .expect("group"),
        ];
        let rows = rows(&configs, |_| ConnectionState::Disconnected);
        assert_eq!(rows[0].subtitle, "Proxmox · SFTP · h");
    }

    #[test]
    fn states_have_codes_and_readable_text() {
        let lost = ConnectionState::Lost { reason: String::from("timeout") };
        assert_eq!(state_code(&lost), "lost");
        assert!(state_text(&lost).contains("timeout"));
        assert_eq!(state_code(&ConnectionState::Ready), "ready");
        assert_eq!(state_text(&ConnectionState::Connecting), "Conectando…");
    }

    #[test]
    fn the_menu_follows_the_state() {
        assert!(menu_state("ready").disconnect);
        assert!(!menu_state("ready").connect);
        assert!(menu_state("lost").connect);
        assert!(menu_state("failed").connect);
        assert!(menu_state("disconnected").connect);
        let busy = menu_state("connecting");
        assert!(!busy.connect && !busy.disconnect && !busy.edit);
    }

    #[test]
    fn host_key_questions_do_not_read_alike() {
        let drive = DriveId::new("sftp", "n").expect("id");
        let first = prompt_view(&Prompt::TrustHostKey {
            drive: drive.clone(),
            host: String::from("h"),
            fingerprint: String::from("SHA256:abc"),
        });
        let changed = prompt_view(&Prompt::HostKeyChanged {
            drive,
            host: String::from("h"),
            known: String::from("SHA256:old"),
            presented: String::from("SHA256:new"),
        });
        assert_eq!(first.kind, "trust");
        assert_eq!(changed.kind, "changed");
        assert_eq!(first.detail, "SHA256:abc");
        assert!(changed.detail.contains("SHA256:old") && changed.detail.contains("SHA256:new"));
        assert!(changed.text.contains("se esté haciendo pasar"));
    }

    #[test]
    fn closing_a_prompt_is_a_refusal_never_a_trust() {
        for kind in ["password", "trust", "changed", "other"] {
            assert_eq!(prompt_answer(kind, false, "x", 1), PromptAnswer::Refuse);
        }
        assert_eq!(prompt_answer("other", true, "x", 1), PromptAnswer::Refuse);
        assert_eq!(prompt_answer("password", true, "", 1), PromptAnswer::Refuse);
    }

    #[test]
    fn accepting_maps_to_the_right_answer() {
        assert_eq!(
            prompt_answer("password", true, "pw", 1),
            PromptAnswer::Secret { secret: Secret::new("pw"), remember: Remember::ForDrive }
        );
        assert_eq!(
            prompt_answer("password", true, "pw", 2),
            PromptAnswer::Secret { secret: Secret::new("pw"), remember: Remember::ForGroup }
        );
        assert_eq!(
            prompt_answer("password", true, "pw", 0),
            PromptAnswer::Secret { secret: Secret::new("pw"), remember: Remember::No }
        );
        assert_eq!(prompt_answer("trust", true, "", 1), PromptAnswer::Trust { remember: true });
        assert_eq!(prompt_answer("trust", true, "", 0), PromptAnswer::Trust { remember: false });
    }

    #[test]
    fn a_changed_key_is_never_remembered() {
        assert_eq!(
            prompt_answer("changed", true, "", 1),
            PromptAnswer::Trust { remember: false }
        );
    }

    #[test]
    fn form_view_lines_up_with_the_descriptors() {
        let mut form = Form::new("s3").expect("s3");
        form.set_value(0, "datos");
        let errors = [
            FieldError { field: String::from("bucket"), message: String::from("mal") },
            FieldError { field: String::from("name"), message: String::from("nombre") },
            FieldError { field: String::from("zzz"), message: String::from("otro") },
        ];
        let view = form_view(&form, &errors);
        let n = form.kind().fields.len();
        for len in [
            view.keys.len(), view.labels.len(), view.kinds.len(), view.hints.len(),
            view.defaults.len(), view.choices.len(), view.values.len(), view.visible.len(),
            view.advanced.len(), view.required.len(), view.errors.len(),
        ] {
            assert_eq!(len, n);
        }
        assert_eq!(view.values[0], "datos");
        assert_eq!(view.errors[0], "mal");
        assert_eq!(view.name_error, "nombre");
        assert_eq!(view.general_error, "otro");
        let credentials = view.keys.iter().position(|k| k == "credentials").expect("credentials");
        assert_eq!(view.kinds[credentials], "choice");
        assert!(view.choices[credentials].starts_with("key=Clave de acceso|"));
        assert!(view.secret_visible);
        assert_eq!(view.secret_label, "Clave secreta");
    }

    #[test]
    fn form_view_hides_what_does_not_apply() {
        let mut form = Form::new("s3").expect("s3");
        let credentials = form.kind().fields.iter().position(|f| f.key == "credentials").expect("c");
        form.set_value(credentials, "anonymous");
        let view = form_view(&form, &[]);
        let key = view.keys.iter().position(|k| k == "access_key_id").expect("key");
        assert!(!view.visible[key]);
        assert!(!view.secret_visible);
    }

    #[test]
    fn errors_have_text() {
        assert!(connect_error_text(&ConnectError::AuthRequired).contains("Falta"));
        assert!(connect_error_text(&ConnectError::Unreachable(String::from("refused"))).contains("refused"));
        assert!(registry_error_text(&RegistryError::AlreadyExists).contains("Ya hay"));
    }
}
