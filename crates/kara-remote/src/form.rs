//! What the «Add drive» dialog shows and how it validates, without any UI.
//!
//! Each protocol is described by a [`DriveKind`]: its parameters ([`Field`]),
//! how each is edited ([`FieldKind`]), which ones depend on another
//! ([`When`]) and what the secret is called. The dialog is generic over these
//! descriptors, so a new protocol or parameter changes this file and nothing in
//! QML. [`Form`] holds the values being edited and turns them into a
//! [`DriveConfig`] plus an optional [`Secret`], or into per-field errors the
//! dialog shows inline.
//!
//! The descriptors cover **every** protocol Kara knows, whether or not its
//! adapter is compiled in; the UI offers only the schemes the registry has a
//! factory for ([`crate::DriveRegistry::schemes`]).
//!
//! User-facing strings are Spanish, like the rest of the window (no i18n yet).

use std::collections::BTreeMap;

use crate::config::{ConfigError, DriveConfig};
use crate::secrets::Secret;
use kara_vfs::DriveIdError;

/// A value a [`FieldKind::Choice`] accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// What is stored.
    pub value: &'static str,
    /// What the user reads.
    pub label: &'static str,
}

/// How a parameter is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Number { min: u32, max: u32 },
    /// A file path; the dialog offers a file chooser.
    Path,
    /// Stored as `true` / `false`.
    Toggle,
    Choice(&'static [Choice]),
}

/// When a field (or the secret) applies, in terms of another field's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Always,
    /// `key` has this value (its default counts when it is empty).
    Equals(&'static str, &'static str),
    /// `key` starts with this prefix (case-insensitive).
    StartsWith(&'static str, &'static str),
}

/// One parameter of a protocol.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    /// The `DriveConfig::params` key.
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
    pub required: bool,
    /// What the adapter assumes when the parameter is absent. Shown as the
    /// placeholder; also what `Equals` conditions compare against.
    pub default: &'static str,
    pub hint: &'static str,
    pub when: When,
    /// Rarely needed: the dialog folds it under «Opciones avanzadas».
    pub advanced: bool,
}

/// What the secret of a protocol is.
#[derive(Debug, Clone, Copy)]
pub struct SecretSpec {
    pub label: &'static str,
    pub hint: &'static str,
    pub when: When,
}

/// A protocol the dialog can register a drive for.
#[derive(Debug, Clone, Copy)]
pub struct DriveKind {
    pub scheme: &'static str,
    pub title: &'static str,
    pub fields: &'static [Field],
    pub secret: Option<SecretSpec>,
}

const fn field(key: &'static str, label: &'static str, kind: FieldKind) -> Field {
    Field {
        key,
        label,
        kind,
        required: false,
        default: "",
        hint: "",
        when: When::Always,
        advanced: false,
    }
}

const fn required(mut f: Field) -> Field {
    f.required = true;
    f
}

const fn hint(mut f: Field, hint: &'static str) -> Field {
    f.hint = hint;
    f
}

const fn default(mut f: Field, default: &'static str) -> Field {
    f.default = default;
    f
}

const fn when(mut f: Field, when: When) -> Field {
    f.when = when;
    f
}

const fn advanced(mut f: Field) -> Field {
    f.advanced = true;
    f
}

const PORT: FieldKind = FieldKind::Number { min: 1, max: 65535 };
const SECONDS: FieldKind = FieldKind::Number { min: 1, max: 86400 };

const S3_CREDENTIALS: &[Choice] = &[
    Choice { value: "key", label: "Clave de acceso" },
    Choice { value: "env", label: "Variables de entorno de AWS" },
    Choice { value: "anonymous", label: "Anónimo (bucket público)" },
];

const GCS_CREDENTIALS: &[Choice] = &[
    Choice { value: "file", label: "Archivo de clave de cuenta de servicio" },
    Choice { value: "adc", label: "Credenciales por defecto de la aplicación" },
];

const CONDITIONAL_PUT: &[Choice] = &[
    Choice { value: "etag", label: "Condicional (If-None-Match)" },
    Choice { value: "disabled", label: "Desactivado" },
];

const SFTP_FIELDS: &[Field] = &[
    hint(required(field("host", "Servidor", FieldKind::Text)), "nas.local o 192.168.1.20"),
    default(field("port", "Puerto", PORT), "22"),
    hint(field("user", "Usuario", FieldKind::Text), "Vacío: el usuario de esta sesión"),
    hint(
        field("key_file", "Archivo de clave privada", FieldKind::Path),
        "Opcional; sin él se usa el agente SSH y la contraseña",
    ),
    hint(field("root", "Carpeta inicial", FieldKind::Text), "Vacío: la raíz del servidor"),
    advanced(hint(
        field("known_hosts", "Archivo known_hosts", FieldKind::Path),
        "Vacío: ~/.ssh/known_hosts",
    )),
    advanced(hint(
        field("agent", "Agente SSH", FieldKind::Text),
        "«none» para no usarlo, o la ruta de un socket",
    )),
    advanced(default(field("timeout_s", "Tiempo de espera (s)", SECONDS), "30")),
    advanced(default(field("keepalive_s", "Latido (s)", SECONDS), "30")),
];

const S3_FIELDS: &[Field] = &[
    required(field("bucket", "Bucket", FieldKind::Text)),
    hint(
        field("endpoint", "Endpoint", FieldKind::Text),
        "Vacío: Amazon S3. MinIO, R2, B2…: https://…",
    ),
    default(field("region", "Región", FieldKind::Text), "us-east-1"),
    hint(field("prefix", "Prefijo", FieldKind::Text), "Carpeta del bucket que hace de raíz"),
    default(field("credentials", "Credenciales", FieldKind::Choice(S3_CREDENTIALS)), "key"),
    when(
        required(field("access_key_id", "Id de clave de acceso", FieldKind::Text)),
        When::Equals("credentials", "key"),
    ),
    when(
        hint(
            field("allow_http", "Permitir conexión sin cifrar", FieldKind::Toggle),
            "Las credenciales y los datos viajarán sin cifrar",
        ),
        When::StartsWith("endpoint", "http://"),
    ),
    advanced(hint(
        field("path_style", "Direcciones con el bucket en la ruta", FieldKind::Toggle),
        "Activado con endpoint propio; «false» para estilo virtual",
    )),
    advanced(default(
        field("conditional_put", "Escritura condicional", FieldKind::Choice(CONDITIONAL_PUT)),
        "etag",
    )),
    advanced(hint(
        field("copy_if_not_exists", "Copia sin sobrescribir", FieldKind::Text),
        "multipart, o «header: nombre: valor» (R2)",
    )),
    advanced(default(field("timeout_s", "Tiempo de espera (s)", SECONDS), "30")),
    advanced(default(
        field("part_size_mb", "Tamaño de parte (MB)", FieldKind::Number { min: 5, max: 5120 }),
        "8",
    )),
    advanced(default(
        field("upload_concurrency", "Partes en paralelo", FieldKind::Number { min: 1, max: 64 }),
        "4",
    )),
];

const GCS_FIELDS: &[Field] = &[
    required(field("bucket", "Bucket", FieldKind::Text)),
    hint(field("prefix", "Prefijo", FieldKind::Text), "Carpeta del bucket que hace de raíz"),
    default(field("credentials", "Credenciales", FieldKind::Choice(GCS_CREDENTIALS)), "file"),
    when(
        required(field("service_account_file", "Archivo de clave (JSON)", FieldKind::Path)),
        When::Equals("credentials", "file"),
    ),
    advanced(hint(field("endpoint", "Endpoint", FieldKind::Text), "Solo para emuladores")),
    when(
        advanced(field("allow_http", "Permitir conexión sin cifrar", FieldKind::Toggle)),
        When::StartsWith("endpoint", "http://"),
    ),
    advanced(default(field("timeout_s", "Tiempo de espera (s)", SECONDS), "30")),
    advanced(default(
        field("part_size_mb", "Tamaño de parte (MB)", FieldKind::Number { min: 5, max: 5120 }),
        "8",
    )),
    advanced(default(
        field("upload_concurrency", "Partes en paralelo", FieldKind::Number { min: 1, max: 64 }),
        "4",
    )),
];

static KINDS: [DriveKind; 3] = [
    DriveKind {
        scheme: "sftp",
        title: "SFTP (SSH)",
        fields: SFTP_FIELDS,
        secret: Some(SecretSpec {
            label: "Contraseña o frase de paso",
            hint: "Con archivo de clave es su frase de paso; vacío si no tiene",
            when: When::Always,
        }),
    },
    DriveKind {
        scheme: "s3",
        title: "Amazon S3 y compatibles",
        fields: S3_FIELDS,
        secret: Some(SecretSpec {
            label: "Clave secreta",
            hint: "Un token de sesión, si lo hay, va en una segunda línea",
            when: When::Equals("credentials", "key"),
        }),
    },
    DriveKind {
        scheme: "gcs",
        title: "Google Cloud Storage",
        fields: GCS_FIELDS,
        secret: None,
    },
];

/// Every protocol the dialog knows, whether or not its adapter is compiled in.
#[must_use]
pub fn kinds() -> &'static [DriveKind] {
    &KINDS
}

/// The descriptor of `scheme`.
#[must_use]
pub fn kind(scheme: &str) -> Option<&'static DriveKind> {
    KINDS.iter().find(|kind| kind.scheme == scheme)
}

/// A problem with one input, for the dialog to show beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    /// A parameter key, or `name`, `label`, `group`, `secret`.
    pub field: String,
    pub message: String,
}

impl FieldError {
    fn new(field: &str, message: impl Into<String>) -> FieldError {
        FieldError {
            field: field.to_owned(),
            message: message.into(),
        }
    }
}

/// A validated form.
#[derive(Debug, Clone)]
pub struct Built {
    pub config: DriveConfig,
    /// What the user typed in the secret box, if it applies and is not empty.
    pub secret: Option<Secret>,
    /// Whether the user wants it kept.
    pub remember: bool,
}

/// The values being edited for one protocol.
#[derive(Debug, Clone)]
pub struct Form {
    kind: &'static DriveKind,
    /// Fixed when editing an existing drive: renaming would change its identity.
    editing: bool,
    pub name: String,
    pub label: String,
    pub group: String,
    pub secret: String,
    pub remember: bool,
    values: Vec<String>,
    /// Parameters no field describes (hand-edited settings): kept as they are.
    extra: BTreeMap<String, String>,
}

impl Form {
    /// A blank form for `scheme`.
    #[must_use]
    pub fn new(scheme: &str) -> Option<Form> {
        let kind = kind(scheme)?;
        Some(Form {
            kind,
            editing: false,
            name: String::new(),
            label: String::new(),
            group: String::new(),
            secret: String::new(),
            remember: true,
            values: vec![String::new(); kind.fields.len()],
            extra: BTreeMap::new(),
        })
    }

    /// A form filled from an existing drive. Its identity cannot be edited and
    /// its secret is left blank: blank means «keep the stored one».
    #[must_use]
    pub fn editing(config: &DriveConfig) -> Option<Form> {
        let mut form = Form::new(config.id.scheme())?;
        form.editing = true;
        form.name = config.id.name().to_owned();
        form.label.clone_from(&config.label);
        form.group = config.group.clone().unwrap_or_default();
        for (key, value) in &config.params {
            match form.kind.fields.iter().position(|f| f.key == key) {
                Some(index) => form.values[index].clone_from(value),
                None => {
                    form.extra.insert(key.clone(), value.clone());
                }
            }
        }
        Some(form)
    }

    #[must_use]
    pub fn kind(&self) -> &'static DriveKind {
        self.kind
    }

    #[must_use]
    pub fn is_editing(&self) -> bool {
        self.editing
    }

    #[must_use]
    pub fn values(&self) -> &[String] {
        &self.values
    }

    /// Sets the value of the field at `index`. `false` if there is none.
    pub fn set_value(&mut self, index: usize, value: &str) -> bool {
        match self.values.get_mut(index) {
            Some(slot) => {
                value.clone_into(slot);
                true
            }
            None => false,
        }
    }

    /// The value in force for `key`: what was typed, else the default.
    fn effective(&self, key: &str) -> &str {
        match self.kind.fields.iter().position(|f| f.key == key) {
            Some(index) => {
                let typed = self.values[index].trim();
                if typed.is_empty() {
                    self.kind.fields[index].default
                } else {
                    typed
                }
            }
            None => "",
        }
    }

    fn applies(&self, condition: When) -> bool {
        match condition {
            When::Always => true,
            When::Equals(key, value) => self.effective(key) == value,
            When::StartsWith(key, prefix) => self
                .effective(key)
                .to_ascii_lowercase()
                .starts_with(prefix),
        }
    }

    /// Whether the field at `index` is shown (and counts).
    #[must_use]
    pub fn field_visible(&self, index: usize) -> bool {
        self.kind
            .fields
            .get(index)
            .is_some_and(|field| self.applies(field.when))
    }

    /// Whether the secret box is shown.
    #[must_use]
    pub fn secret_visible(&self) -> bool {
        self.kind.secret.is_some_and(|spec| self.applies(spec.when))
    }

    /// Checks everything and builds the config, or lists every problem.
    pub fn build(&self) -> Result<Built, Vec<FieldError>> {
        let mut errors = Vec::new();
        let label = self.label.trim();
        if label.is_empty() {
            errors.push(FieldError::new("label", "Escribe un nombre para la unidad"));
        }
        let name = if self.name.trim().is_empty() {
            slug(label)
        } else {
            self.name.trim().to_owned()
        };
        if name.is_empty() && !label.is_empty() {
            errors.push(FieldError::new(
                "name",
                "No se pudo derivar un identificador del nombre; escribe uno (letras, números y guiones)",
            ));
        }

        let mut params: Vec<(String, String)> = Vec::new();
        for (index, field) in self.kind.fields.iter().enumerate() {
            if !self.applies(field.when) {
                continue;
            }
            let raw = self.values[index].trim();
            if raw.is_empty() {
                if field.required {
                    errors.push(FieldError::new(field.key, "Obligatorio"));
                }
                continue;
            }
            match check_value(field, raw) {
                Ok(()) => params.push((field.key.to_owned(), raw.to_owned())),
                Err(message) => errors.push(FieldError::new(field.key, message)),
            }
        }
        // An `http://` endpoint is only allowed when the user said so.
        if let Some(index) = self.kind.fields.iter().position(|f| f.key == "allow_http")
            && self.field_visible(index)
            && self.effective("allow_http") != "true"
        {
            errors.push(FieldError::new(
                "allow_http",
                "Un endpoint http:// necesita permitir la conexión sin cifrar",
            ));
        }
        // Parameters no field describes survive an edit untouched.
        for (key, value) in &self.extra {
            params.push((key.clone(), value.clone()));
        }

        let group = self.group.trim();
        let config = if errors.is_empty() {
            match DriveConfig::new(self.kind.scheme, &name, label, params).and_then(|config| {
                if group.is_empty() {
                    Ok(config)
                } else {
                    config.with_group(group)
                }
            }) {
                Ok(config) => Some(config),
                Err(error) => {
                    errors.push(config_error(&error));
                    None
                }
            }
        } else {
            None
        };

        match config {
            Some(config) if errors.is_empty() => Ok(Built {
                config,
                secret: (self.secret_visible() && !self.secret.is_empty())
                    .then(|| Secret::new(self.secret.clone())),
                remember: self.remember,
            }),
            _ => Err(errors),
        }
    }
}

fn check_value(field: &Field, raw: &str) -> Result<(), String> {
    match field.kind {
        FieldKind::Text | FieldKind::Path => Ok(()),
        FieldKind::Number { min, max } => match raw.parse::<u32>() {
            Ok(n) if (min..=max).contains(&n) => Ok(()),
            Ok(_) => Err(format!("Debe estar entre {min} y {max}")),
            Err(_) => Err(String::from("Debe ser un número")),
        },
        FieldKind::Toggle => match raw {
            "true" | "false" => Ok(()),
            _ => Err(String::from("Debe ser sí o no")),
        },
        FieldKind::Choice(choices) => {
            if choices.iter().any(|choice| choice.value == raw) {
                Ok(())
            } else {
                Err(String::from("Elige una de las opciones"))
            }
        }
    }
}

fn config_error(error: &ConfigError) -> FieldError {
    match error {
        ConfigError::Id(DriveIdError::InvalidName { .. }) => FieldError::new(
            "name",
            "Solo letras minúsculas, números y guiones; empieza y acaba con letra o número (máx. 63)",
        ),
        ConfigError::Id(DriveIdError::InvalidScheme { .. }) => {
            FieldError::new("name", "Protocolo no válido")
        }
        ConfigError::BadLabel => {
            FieldError::new("label", "El nombre no puede ir vacío ni tener saltos de línea")
        }
        ConfigError::BadGroup => {
            FieldError::new("group", "Hasta 64 caracteres, sin «:» ni saltos de línea")
        }
        ConfigError::LooksLikeSecret { key } | ConfigError::BadParamKey { key } => {
            FieldError::new(key, "Parámetro no permitido")
        }
        ConfigError::BadParamValue { key } => {
            FieldError::new(key, "No puede tener saltos de línea")
        }
    }
}

/// A drive name from a label: lowercase ASCII letters and digits, single
/// hyphens between them, at most 63 characters. Empty if nothing usable is left
/// (a label made only of symbols or of non-Latin letters).
#[must_use]
pub fn slug(label: &str) -> String {
    let mut out = String::new();
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let cut: String = out.trim_matches('-').chars().take(63).collect();
    cut.trim_matches('-').to_owned()
}
