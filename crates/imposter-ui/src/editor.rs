//! The request editor's form state and its conversion to and from a [`Request`].
//!
//! Widgets edit these fields directly; [`Editor::to_request`] builds what is saved and sent.
//! Rows without a name are kept while editing but never saved, as in the Python app.

use imposter_app::core::ids::RequestId;
use imposter_app::core::model::{
    AuthKind, BodyType, FormItem, Options, ParamItem, Request, RequestAuth, Variables,
};
use imposter_app::core::settings::AppSettings;
use imposter_app::core::url;

/// The body types the editor offers. `form-data` loads as URL-encoded and `text` as plain
/// text: they are sent the same way.
pub const BODY_CHOICES: [(BodyType, &str); 6] = [
    (BodyType::None, "None"),
    (BodyType::Json, "JSON"),
    (BodyType::Plain, "Plain text"),
    (BodyType::Urlencoded, "URL-encoded"),
    (BodyType::Multipart, "Multipart"),
    (BodyType::Binary, "Binary file"),
];

pub const AUTH_CHOICES: [(Option<AuthKind>, &str); 4] = [
    (None, "None"),
    (Some(AuthKind::Basic), "Basic"),
    (Some(AuthKind::BearerToken), "Bearer"),
    (Some(AuthKind::Digest), "Digest"),
];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuthForm {
    pub kind: Option<AuthKind>,
    pub username: String,
    pub password: String,
    pub token: String,
}

impl AuthForm {
    fn load(auth: &RequestAuth) -> Self {
        let credentials = if auth.kind == Some(AuthKind::Basic) {
            auth.basic.as_ref()
        } else {
            auth.digest.as_ref()
        };
        Self {
            kind: auth.kind,
            username: credentials.map(|c| c.username.clone()).unwrap_or_default(),
            password: credentials.map(|c| c.password.clone()).unwrap_or_default(),
            token: auth
                .bearer_token
                .as_ref()
                .map(|b| b.token.clone())
                .unwrap_or_default(),
        }
    }

    fn read(&self) -> RequestAuth {
        match self.kind {
            None => RequestAuth::default(),
            Some(AuthKind::Basic) => RequestAuth::basic(&self.username, &self.password),
            Some(AuthKind::Digest) => RequestAuth::digest(&self.username, &self.password),
            Some(AuthKind::BearerToken) => RequestAuth::bearer(&self.token),
        }
    }
}

/// A setting that inherits the application default unless overridden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Override<T> {
    pub inherit: bool,
    pub value: T,
}

impl<T: Default> Default for Override<T> {
    fn default() -> Self {
        Self {
            inherit: true,
            value: T::default(),
        }
    }
}

impl<T: Clone> Override<T> {
    fn load(value: Option<&T>, empty: T) -> Self {
        match value {
            Some(value) => Self {
                inherit: false,
                value: value.clone(),
            },
            None => Self {
                inherit: true,
                value: empty,
            },
        }
    }

    fn read(&self) -> Option<T> {
        (!self.inherit).then(|| self.value.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionField {
    Timeout,
    FollowRedirects,
    AttachCookies,
    DownloadLimit,
    ProxyUrl,
    VerifySsl,
    CaBundle,
    CertificatePath,
    KeyFile,
    Password,
}

impl OptionField {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Timeout => "Timeout (seconds)",
            Self::FollowRedirects => "Follow redirects",
            Self::AttachCookies => "Attach cookies",
            Self::DownloadLimit => "Response limit (KiB; 0 unlimited)",
            Self::ProxyUrl => "Proxy URL",
            Self::VerifySsl => "Verify TLS",
            Self::CaBundle => "CA bundle",
            Self::CertificatePath => "Client certificate",
            Self::KeyFile => "Private key",
            Self::Password => "Key password",
        }
    }
}

/// Groups as the settings tab shows them.
pub const OPTION_GROUPS: [(&str, &[OptionField]); 4] = [
    (
        "Network and redirects",
        &[OptionField::Timeout, OptionField::FollowRedirects],
    ),
    (
        "Cookies and response capture",
        &[OptionField::AttachCookies, OptionField::DownloadLimit],
    ),
    ("Proxy", &[OptionField::ProxyUrl]),
    (
        "TLS and client certificates",
        &[
            OptionField::VerifySsl,
            OptionField::CaBundle,
            OptionField::CertificatePath,
            OptionField::KeyFile,
            OptionField::Password,
        ],
    ),
];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OptionsForm {
    pub follow_redirects: Override<bool>,
    pub verify_ssl: Override<bool>,
    pub attach_cookies: Override<bool>,
    /// Text fields keep what the user typed; numbers are validated when read.
    pub timeout: Override<String>,
    pub download_limit: Override<String>,
    pub proxy_url: Override<String>,
    pub ca_bundle: Override<String>,
    pub certificate_path: Override<String>,
    pub key_file: Override<String>,
    pub password: Override<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Fix the highlighted request settings before continuing")]
pub struct InvalidOptions(pub Vec<(OptionField, &'static str)>);

impl OptionsForm {
    fn load(options: &Options) -> Self {
        let text = |value: &Option<String>| Override::load(value.as_ref(), String::new());
        Self {
            follow_redirects: Override::load(options.follow_redirects.as_ref(), true),
            verify_ssl: Override::load(options.verify_ssl.as_ref(), true),
            attach_cookies: Override::load(options.attach_cookies.as_ref(), true),
            timeout: Override::load(options.timeout.map(format_number).as_ref(), String::new()),
            download_limit: Override::load(
                options.download_limit.map(|d| d.to_string()).as_ref(),
                String::new(),
            ),
            proxy_url: text(&options.proxy_url),
            ca_bundle: text(&options.ca_bundle),
            certificate_path: text(&options.certificate_path),
            key_file: text(&options.key_file),
            password: text(&options.password),
        }
    }

    pub fn read(&self) -> Result<Options, InvalidOptions> {
        let mut errors = Vec::new();
        let timeout = self
            .timeout
            .read()
            .and_then(|text| match text.trim().parse::<f64>() {
                Ok(seconds) if seconds > 0.0 && seconds.is_finite() => Some(seconds),
                _ => {
                    errors.push((OptionField::Timeout, "Enter a positive number of seconds."));
                    None
                }
            });
        let download_limit = self.download_limit.read().and_then(|text| {
            text.trim().parse::<u64>().map_or_else(
                |_| {
                    errors.push((
                        OptionField::DownloadLimit,
                        "Enter a whole number of KiB, zero or greater.",
                    ));
                    None
                },
                Some,
            )
        });
        if !errors.is_empty() {
            return Err(InvalidOptions(errors));
        }
        Ok(Options {
            follow_redirects: self.follow_redirects.read(),
            verify_ssl: self.verify_ssl.read(),
            attach_cookies: self.attach_cookies.read(),
            proxy_url: self.proxy_url.read(),
            timeout,
            ca_bundle: self.ca_bundle.read(),
            certificate_path: self.certificate_path.read(),
            key_file: self.key_file.read(),
            password: self.password.read(),
            download_limit,
        })
    }

    pub fn is_inherited(&self, field: OptionField) -> bool {
        match field {
            OptionField::Timeout => self.timeout.inherit,
            OptionField::FollowRedirects => self.follow_redirects.inherit,
            OptionField::AttachCookies => self.attach_cookies.inherit,
            OptionField::DownloadLimit => self.download_limit.inherit,
            OptionField::ProxyUrl => self.proxy_url.inherit,
            OptionField::VerifySsl => self.verify_ssl.inherit,
            OptionField::CaBundle => self.ca_bundle.inherit,
            OptionField::CertificatePath => self.certificate_path.inherit,
            OptionField::KeyFile => self.key_file.inherit,
            OptionField::Password => self.password.inherit,
        }
    }
}

/// `30` rather than `30.0`, as Python's `str()` of a whole float would not, but readable.
fn format_number(value: f64) -> String {
    let text = value.to_string();
    text.strip_suffix(".0").map_or(text.clone(), str::to_owned)
}

/// How an inherited setting reads in its hint: "Application default · 30 seconds".
pub fn default_hint(field: OptionField, defaults: &AppSettings) -> String {
    let on_off = |value: bool| if value { "On" } else { "Off" }.to_owned();
    let text = |value: &str| {
        if value.is_empty() {
            "Not configured".to_owned()
        } else {
            value.to_owned()
        }
    };
    let shown = match field {
        OptionField::Timeout => format!("{} seconds", format_number(defaults.timeout)),
        OptionField::FollowRedirects => on_off(defaults.follow_redirects),
        OptionField::AttachCookies => on_off(defaults.attach_cookies),
        OptionField::VerifySsl => on_off(defaults.verify_ssl),
        OptionField::DownloadLimit if defaults.download_limit == 0 => "Unlimited".to_owned(),
        OptionField::DownloadLimit => format!("{} KiB", defaults.download_limit),
        OptionField::ProxyUrl => text(&defaults.proxy_url),
        OptionField::CaBundle => text(&defaults.ca_bundle),
        OptionField::CertificatePath => text(&defaults.certificate_path),
        OptionField::KeyFile => text(&defaults.key_file),
        OptionField::Password if defaults.password.is_empty() => "Not configured".to_owned(),
        OptionField::Password => "••••••••".to_owned(),
    };
    format!("Application default · {shown}")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Editor {
    request: Request,
    pub description: String,
    pub method: String,
    pub url: String,
    pub queries: Vec<ParamItem>,
    pub headers: Vec<ParamItem>,
    pub cookies: Vec<ParamItem>,
    pub body_type: BodyType,
    pub body_text: String,
    pub form: Vec<ParamItem>,
    pub multipart: Vec<FormItem>,
    pub binary_path: String,
    pub auth: AuthForm,
    pub options: OptionsForm,
}

fn named<T: Clone>(rows: &[T], name: impl Fn(&T) -> &str) -> Vec<T> {
    rows.iter()
        .filter(|row| !name(row).is_empty())
        .cloned()
        .collect()
}

impl Editor {
    pub fn load(request: &Request) -> Self {
        let template = &request.template;
        let body_type = match template.body_type {
            BodyType::FormData => BodyType::Urlencoded,
            BodyType::Text => BodyType::Plain,
            other => other,
        };
        let text = || String::from_utf8(template.body.clone()).unwrap_or_default();
        Self {
            request: request.clone(),
            description: template.description.clone(),
            method: template.method.clone(),
            url: template.url.clone(),
            queries: template.queries.clone(),
            headers: template.headers.clone(),
            cookies: template.cookies.clone(),
            body_type,
            body_text: if matches!(body_type, BodyType::Json | BodyType::Plain | BodyType::None) {
                text()
            } else {
                String::new()
            },
            form: if body_type == BodyType::Urlencoded {
                template.form_fields().unwrap_or_default()
            } else {
                Vec::new()
            },
            multipart: if body_type == BodyType::Multipart {
                template.multipart_fields().unwrap_or_default()
            } else {
                Vec::new()
            },
            binary_path: if body_type == BodyType::Binary {
                text()
            } else {
                String::new()
            },
            auth: AuthForm::load(&template.auth),
            options: OptionsForm::load(&template.options),
        }
    }

    pub fn id(&self) -> Option<RequestId> {
        self.request.id
    }

    pub fn is_draft(&self) -> bool {
        self.request.id.is_none()
    }

    pub fn name(&self) -> &str {
        &self.request.template.name
    }

    pub fn identity(&self) -> &Request {
        &self.request
    }

    /// Attach a newly saved identity without touching the fields being edited.
    pub fn adopt_identity(&mut self, saved: &Request) {
        self.request.id = saved.id;
        self.request.collection_id = saved.collection_id;
        self.request.folder_id = saved.folder_id;
        self.request.template.name.clone_from(&saved.template.name);
    }

    /// Apply a tree rename or move made elsewhere.
    pub fn adopt_metadata(
        &mut self,
        name: &str,
        folder: Option<imposter_app::core::ids::FolderId>,
    ) {
        name.clone_into(&mut self.request.template.name);
        self.request.folder_id = folder;
    }

    pub fn collection_variables_snapshot(&self) -> &Variables {
        &self.request.collection_variables_snapshot
    }

    /// The request as it would be saved and sent.
    pub fn to_request(&self) -> Result<Request, InvalidOptions> {
        let mut request = self.request.clone();
        let template = &mut request.template;
        template.description.clone_from(&self.description);
        template.method.clone_from(&self.method);
        template.url.clone_from(&self.url);
        template.queries = named(&self.queries, |r| &r.name);
        template.headers = named(&self.headers, |r| &r.name);
        template.cookies = named(&self.cookies, |r| &r.name);
        template.auth = self.auth.read();
        template.options = self.options.read()?;
        template.body_type = self.body_type;
        match self.body_type {
            BodyType::None => template.body = Vec::new(),
            BodyType::Json | BodyType::Plain | BodyType::Text => {
                template.body = self.body_text.clone().into_bytes();
            }
            BodyType::FormData | BodyType::Urlencoded => {
                template.set_form_fields(&named(&self.form, |r| &r.name));
            }
            BodyType::Multipart => {
                template.set_multipart_fields(&named(&self.multipart, |r| &r.name));
            }
            BodyType::Binary => template.body = self.binary_path.clone().into_bytes(),
        }
        Ok(request)
    }

    /// The URL was edited: the query in it becomes the enabled parameter rows; disabled rows
    /// stay after them.
    pub fn url_changed(&mut self) {
        let parts = url::split(&self.url);
        let from_url: Vec<(String, String)> = url::parse_pairs(&parts.query);
        let active: Vec<(String, String)> = self
            .queries
            .iter()
            .filter(|r| r.is_enabled && !r.name.is_empty())
            .map(|r| (r.name.clone(), r.value.clone()))
            .collect();
        if from_url != active {
            let disabled: Vec<ParamItem> = self
                .queries
                .iter()
                .filter(|r| !r.is_enabled && !r.name.is_empty())
                .cloned()
                .collect();
            self.queries = from_url
                .into_iter()
                .map(|(name, value)| ParamItem::new(name, value))
                .collect();
            self.queries.extend(disabled);
        }
    }

    /// The parameter rows were edited: the URL's query becomes the enabled, named rows.
    pub fn params_changed(&mut self) {
        let mut parts = url::split(&self.url);
        parts.query = url::encode_pairs(
            self.queries
                .iter()
                .filter(|r| r.is_sent())
                .map(|r| (r.name.as_str(), r.value.as_str())),
        );
        self.url = url::join_with_fragment(&parts);
    }
}

#[cfg(test)]
mod tests {
    use imposter_app::core::model::{FormKind, RequestTemplate};

    use super::*;

    fn request(template: RequestTemplate) -> Request {
        Request {
            id: Some(RequestId(1)),
            template,
            ..Request::default()
        }
    }

    #[test]
    fn loading_then_reading_keeps_a_request_unchanged() {
        let mut template = RequestTemplate {
            name: "r".into(),
            method: "PATCH".into(),
            url: "https://h/x?a=1".into(),
            queries: vec![
                ParamItem::new("a", "1"),
                ParamItem {
                    is_enabled: false,
                    ..ParamItem::new("off", "x")
                },
            ],
            headers: vec![ParamItem::new("A", "1"), ParamItem::new("A", "")],
            cookies: vec![ParamItem::new("c", "1")],
            auth: RequestAuth::digest("u", "p"),
            body: b"{}".to_vec(),
            body_type: BodyType::Json,
            ..RequestTemplate::default()
        };
        template.options.timeout = Some(2.5);
        template.options.proxy_url = Some(String::new());
        let original = request(template);

        assert_eq!(Editor::load(&original).to_request().unwrap(), original);
    }

    #[test]
    fn legacy_body_types_load_as_their_editor_equivalents() {
        let mut form = RequestTemplate {
            body_type: BodyType::FormData,
            ..RequestTemplate::default()
        };
        form.set_form_fields(&[ParamItem::new("k", "v")]);
        let editor = Editor::load(&request(form));
        assert_eq!(editor.body_type, BodyType::Urlencoded);
        assert_eq!(editor.form, [ParamItem::new("k", "v")]);
        let text = Editor::load(&request(RequestTemplate {
            body_type: BodyType::Text,
            body: b"t".to_vec(),
            ..RequestTemplate::default()
        }));
        assert_eq!(
            (text.body_type, text.body_text.as_str()),
            (BodyType::Plain, "t")
        );
    }

    #[test]
    fn unnamed_rows_are_not_saved_and_none_drops_the_body() {
        let mut editor = Editor::load(&request(RequestTemplate::default()));
        editor.headers.push(ParamItem::new("", "orphan"));
        editor.body_type = BodyType::Multipart;
        editor.multipart = vec![FormItem::text("", "x"), FormItem::file("f", "/p")];
        let read = editor.to_request().unwrap();
        assert_eq!(read.template.headers, []);
        assert_eq!(
            read.template.multipart_fields().unwrap(),
            [FormItem::file("f", "/p")]
        );
        assert_eq!(
            read.template.multipart_fields().unwrap()[0].kind,
            FormKind::File
        );

        editor.body_text = "kept in the editor".into();
        editor.body_type = BodyType::None;
        assert_eq!(editor.to_request().unwrap().template.body, b"");
        editor.body_type = BodyType::Binary;
        editor.binary_path = "/data.bin".into();
        assert_eq!(editor.to_request().unwrap().template.body, b"/data.bin");
    }

    #[test]
    fn url_and_parameters_stay_in_sync() {
        let mut editor = Editor::load(&request(RequestTemplate {
            url: "http://h/p".into(),
            queries: vec![ParamItem {
                is_enabled: false,
                ..ParamItem::new("off", "1")
            }],
            ..RequestTemplate::default()
        }));

        editor.url = "http://h/p?a=1&b=x+y#top".into();
        editor.url_changed();
        assert_eq!(
            editor.queries,
            [
                ParamItem::new("a", "1"),
                ParamItem::new("b", "x y"),
                ParamItem {
                    is_enabled: false,
                    ..ParamItem::new("off", "1")
                }
            ]
        );

        editor.queries[0].value = "2 3".into();
        editor.queries.push(ParamItem::new("", "unnamed"));
        editor.params_changed();
        assert_eq!(editor.url, "http://h/p?a=2+3&b=x+y#top");
    }

    #[test]
    fn options_validate_numbers_and_keep_explicit_empty_overrides() {
        let mut editor = Editor::load(&request(RequestTemplate::default()));
        editor.options.timeout = Override {
            inherit: false,
            value: "0".into(),
        };
        editor.options.download_limit = Override {
            inherit: false,
            value: "-1".into(),
        };
        let InvalidOptions(errors) = editor.to_request().unwrap_err();
        assert_eq!(
            errors.iter().map(|(f, _)| *f).collect::<Vec<_>>(),
            [OptionField::Timeout, OptionField::DownloadLimit]
        );

        editor.options.timeout.value = " 1.5 ".into();
        editor.options.download_limit.value = "0".into();
        editor.options.ca_bundle = Override {
            inherit: false,
            value: String::new(),
        };
        let options = editor.to_request().unwrap().template.options;
        assert_eq!(
            (options.timeout, options.download_limit, options.ca_bundle),
            (Some(1.5), Some(0), Some(String::new()))
        );
        assert_eq!(options.proxy_url, None);
        assert!(!editor.options.is_inherited(OptionField::CaBundle));
    }

    #[test]
    fn auth_reads_only_the_selected_scheme() {
        let mut editor = Editor::load(&request(RequestTemplate {
            auth: RequestAuth::basic("u", "p"),
            ..RequestTemplate::default()
        }));
        assert_eq!(
            (editor.auth.username.as_str(), editor.auth.password.as_str()),
            ("u", "p")
        );
        editor.auth.kind = Some(AuthKind::BearerToken);
        editor.auth.token = "t".into();
        assert_eq!(
            editor.to_request().unwrap().template.auth,
            RequestAuth::bearer("t")
        );
        editor.auth.kind = None;
        assert_eq!(
            editor.to_request().unwrap().template.auth,
            RequestAuth::default()
        );
    }

    #[test]
    fn hints_describe_the_inherited_defaults() {
        let defaults = AppSettings {
            password: "secret".into(),
            download_limit: 0,
            ..AppSettings::default()
        };
        assert_eq!(
            default_hint(OptionField::Timeout, &defaults),
            "Application default · 30 seconds"
        );
        assert_eq!(
            default_hint(OptionField::DownloadLimit, &defaults),
            "Application default · Unlimited"
        );
        assert_eq!(
            default_hint(OptionField::Password, &defaults),
            "Application default · ••••••••"
        );
        assert_eq!(
            default_hint(OptionField::ProxyUrl, &defaults),
            "Application default · Not configured"
        );
        assert_eq!(
            default_hint(OptionField::VerifySsl, &defaults),
            "Application default · On"
        );
    }

    #[test]
    fn identity_and_metadata_are_adopted_without_touching_fields() {
        let mut editor = Editor::load(&Request::default());
        assert!(editor.is_draft());
        editor.url = "http://typed".into();
        let saved = Request {
            id: Some(RequestId(5)),
            template: RequestTemplate {
                name: "Saved".into(),
                ..RequestTemplate::default()
            },
            ..Request::default()
        };
        editor.adopt_identity(&saved);
        assert_eq!(
            (editor.id(), editor.name(), editor.url.as_str()),
            (Some(RequestId(5)), "Saved", "http://typed")
        );
        editor.adopt_metadata("Renamed", Some(imposter_app::core::ids::FolderId(2)));
        assert_eq!(
            editor.to_request().unwrap().folder_id,
            Some(imposter_app::core::ids::FolderId(2))
        );
        assert_eq!(editor.name(), "Renamed");
    }
}
