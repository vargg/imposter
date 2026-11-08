//! `{{name}}` placeholders: finding, resolving and explaining them.
//!
//! Matching is the regex `\{\{(.*?)\}\}`: the first `}}` after `{{` closes a placeholder
//! unless a newline comes first, the name is trimmed, and values are inserted verbatim (never
//! resolved again). Pinned by `tests/fixtures/golden/resolve.json`.

use std::collections::BTreeSet;
use std::ops::Range;

use indexmap::IndexMap;

use crate::ids::{CollectionId, EnvironmentId};
use crate::model::{FormBodyError, Options, ParamItem, RequestTemplate, SavedValues, Variables};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder<'a> {
    /// Byte range of the whole `{{ … }}` in the text.
    pub range: Range<usize>,
    /// The trimmed name between the braces.
    pub name: &'a str,
}

/// Whitespace trimmed from a name: Unicode whitespace plus the ASCII separators 0x1C–0x1F.
fn is_name_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Every placeholder in `text`, left to right.
pub fn placeholders(text: &str) -> Vec<Placeholder<'_>> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find("{{") {
        let start = from + offset;
        let body = start + 2;
        let close = text[body..].find("}}").map(|end| body + end);
        match close {
            Some(end) if !text[body..end].contains('\n') => {
                found.push(Placeholder {
                    range: start..end + 2,
                    name: text[body..end].trim_matches(is_name_space),
                });
                from = end + 2;
            }
            // Like the regex, retry one character later.
            _ => from = start + 1,
        }
    }
    found
}

/// Replace placeholders whose name `lookup` knows; record the others in `missing`.
pub fn substitute<'v>(
    text: &str,
    lookup: impl Fn(&str) -> Option<&'v str>,
    missing: &mut BTreeSet<String>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for placeholder in placeholders(text) {
        out.push_str(&text[last..placeholder.range.start]);
        if let Some(value) = lookup(placeholder.name) {
            out.push_str(value);
        } else {
            missing.insert(placeholder.name.to_owned());
            out.push_str(&text[placeholder.range.clone()]);
        }
        last = placeholder.range.end;
    }
    out.push_str(&text[last..]);
    out
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("Unresolved variables: {}", .0.iter().cloned().collect::<Vec<_>>().join(", "))]
    Unresolved(BTreeSet<String>),
    #[error("The body is not valid UTF-8 text")]
    BodyNotUtf8,
    #[error(transparent)]
    FormBody(#[from] FormBodyError),
}

/// Every text of `template` that reaches the wire or configures the connection, in the
/// order they are resolved: URL, rows, auth, options, then the body (file paths too, never
/// file contents). Name and description are not visited. An error leaves the body
/// unvisited; everything before it was.
pub(crate) fn visit_texts(
    template: &mut RequestTemplate,
    visit: &mut dyn FnMut(&mut String),
) -> Result<(), ResolveError> {
    visit(&mut template.url);
    for items in [
        &mut template.headers,
        &mut template.queries,
        &mut template.cookies,
    ] {
        visit_items(items, visit);
    }
    for credentials in [template.auth.basic.as_mut(), template.auth.digest.as_mut()]
        .into_iter()
        .flatten()
    {
        visit(&mut credentials.username);
        visit(&mut credentials.password);
    }
    if let Some(bearer) = &mut template.auth.bearer_token {
        visit(&mut bearer.token);
    }
    visit_options(&mut template.options, visit);
    visit_body(template, visit)
}

fn visit_items(items: &mut [ParamItem], visit: &mut dyn FnMut(&mut String)) {
    for item in items {
        visit(&mut item.name);
        visit(&mut item.value);
    }
}

fn visit_options(options: &mut Options, visit: &mut dyn FnMut(&mut String)) {
    for field in [
        &mut options.proxy_url,
        &mut options.ca_bundle,
        &mut options.certificate_path,
        &mut options.key_file,
        &mut options.password,
    ]
    .into_iter()
    .flatten()
    {
        visit(field);
    }
}

fn visit_body(
    template: &mut RequestTemplate,
    visit: &mut dyn FnMut(&mut String),
) -> Result<(), ResolveError> {
    use crate::model::BodyType;
    match template.body_type {
        BodyType::None => {}
        BodyType::FormData | BodyType::Urlencoded => {
            let mut fields = template.form_fields()?;
            visit_items(&mut fields, visit);
            template.set_form_fields(&fields);
        }
        BodyType::Multipart => {
            let mut fields = template.multipart_fields()?;
            for field in &mut fields {
                visit(&mut field.name);
                visit(&mut field.value);
                visit(&mut field.content_type);
                visit(&mut field.filename);
            }
            template.set_multipart_fields(&fields);
        }
        BodyType::Json | BodyType::Plain | BodyType::Text | BodyType::Binary => {
            let mut text =
                String::from_utf8(std::mem::take(&mut template.body)).map_err(|error| {
                    // Put the bytes back: a failed visit must not lose the body.
                    template.body = error.into_bytes();
                    ResolveError::BodyNotUtf8
                })?;
            visit(&mut text);
            template.body = text.into_bytes();
        }
    }
    Ok(())
}

/// Resolve every field that reaches the wire or configures the connection: URL, rows, auth,
/// options and the body (file paths too, never file contents). Name and description stay.
pub fn resolve_template(
    template: &RequestTemplate,
    values: &Variables,
) -> Result<RequestTemplate, ResolveError> {
    let mut resolved = template.clone();
    let mut missing = BTreeSet::new();
    visit_texts(&mut resolved, &mut |text| {
        if text.contains("{{") {
            *text = substitute(
                text,
                |name| values.get(name).map(String::as_str),
                &mut missing,
            );
        }
    })?;
    if missing.is_empty() {
        Ok(resolved)
    } else {
        Err(ResolveError::Unresolved(missing))
    }
}

/// Every variable name `template` uses where it reaches the wire, and in its checks'
/// expected values, sorted and once each. A body that cannot be read as text (not UTF-8, a
/// broken form) adds nothing, but never hides the names elsewhere. Names scripts read with
/// `vars.get` are not found: scripts are code, not templates.
pub fn used(template: &RequestTemplate) -> Vec<String> {
    let mut resolved = template.clone();
    let mut missing = BTreeSet::new();
    let _ = visit_texts(&mut resolved, &mut |text| {
        if text.contains("{{") {
            *text = substitute(text, |_| None, &mut missing);
        }
    });
    for check in &template.checks {
        missing.extend(
            placeholders(&check.expected)
                .into_iter()
                .map(|p| p.name.to_owned()),
        );
    }
    missing.into_iter().collect()
}

/// The active environment's identity and values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveEnvironment {
    /// `None` for an environment read from a file rather than the database: it has no
    /// stored identity, and nothing is written back to it.
    pub id: Option<EnvironmentId>,
    pub name: String,
    pub values: Variables,
    pub saved: SavedValues,
    /// Its secrets, by name without the `secret:` prefix.
    pub secrets: Variables,
}

/// Global and environment values captured before an execution's first await, so a later
/// environment switch cannot change a request that is already running.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VariableContext {
    pub globals: Variables,
    pub global_saved: SavedValues,
    /// Secrets that apply under every environment, below the environment's own.
    pub global_secrets: Variables,
    /// `None` when no environment is active (only globals apply).
    pub environment: Option<ActiveEnvironment>,
}

impl VariableContext {
    pub const GLOBAL_NAME: &'static str = "Global";

    pub fn environment_name(&self) -> &str {
        self.environment
            .as_ref()
            .map_or(Self::GLOBAL_NAME, |env| env.name.as_str())
    }

    pub fn environment_id(&self) -> Option<EnvironmentId> {
        self.environment.as_ref().and_then(|env| env.id)
    }
}

/// The collection a request belongs to, or its captured values when the collection is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionScope<'a> {
    Live {
        /// `None` for a collection read from a file rather than the database.
        id: Option<CollectionId>,
        name: &'a str,
        values: &'a Variables,
        saved: &'a SavedValues,
    },
    Deleted {
        captured: &'a Variables,
    },
}

/// Where a definition comes from. An id of `None` marks a scope read from a file, which
/// has no stored identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Global,
    Collection {
        id: Option<CollectionId>,
        name: String,
    },
    /// The collection values a History draft captured when its collection is gone.
    Captured,
    Environment {
        id: Option<EnvironmentId>,
        name: String,
    },
    /// A value given to one headless run (from a named process variable), above every scope.
    Input,
    /// A value an earlier request of the same run captured or set, above the inputs: a
    /// login's token replaces the environment's for the requests after it.
    Run,
    /// A secret, named `secret:<name>` in a template: the environment's (its id and name),
    /// or a global one (`None`). Stored apart from variables and never exported.
    Secret {
        environment: Option<(EnvironmentId, String)>,
    },
}

impl Source {
    pub const CAPTURED_NAME: &'static str = "Captured collection (deleted)";
    pub const INPUT_NAME: &'static str = "Input";
    pub const RUN_NAME: &'static str = "this run";

    pub fn label(&self) -> &str {
        match self {
            Self::Global => VariableContext::GLOBAL_NAME,
            Self::Captured => Self::CAPTURED_NAME,
            Self::Input => Self::INPUT_NAME,
            Self::Run => Self::RUN_NAME,
            Self::Collection { name, .. } | Self::Environment { name, .. } => name,
            Self::Secret { environment } => environment
                .as_ref()
                .map_or(VariableContext::GLOBAL_NAME, |(_, name)| name.as_str()),
        }
    }
}

/// What a template writes before a secret's name: `{{secret:token}}`.
pub const SECRET_PREFIX: &str = "secret:";

/// The placeholder name of the secret `name`: `secret:name`.
pub fn secret_key(name: &str) -> String {
    format!("{SECRET_PREFIX}{name}")
}

/// The secret a placeholder name refers to, when it refers to one.
pub fn secret_name(key: &str) -> Option<&str> {
    key.strip_prefix(SECRET_PREFIX).map(str::trim)
}

/// Where secrets are kept: the global ones, or an environment's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SecretScope {
    #[default]
    Global,
    Environment(EnvironmentId),
}

impl From<SecretScope> for Scope {
    /// The Variables screen page a secret is edited on.
    fn from(scope: SecretScope) -> Self {
        match scope {
            SecretScope::Global => Self::Global,
            SecretScope::Environment(id) => Self::Environment(id),
        }
    }
}

impl SecretScope {
    /// The secrets of a Variables screen page; collections have none.
    pub fn of_scope(scope: Scope) -> Option<Self> {
        match scope {
            Scope::Global => Some(Self::Global),
            Scope::Environment(id) => Some(Self::Environment(id)),
            Scope::Collection(_) => None,
        }
    }
}

/// A place variables are defined and edited: the globals, an environment or a collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scope {
    #[default]
    Global,
    Environment(EnvironmentId),
    Collection(CollectionId),
}

impl Scope {
    /// Where a definition lives; a deleted collection's captured values and scopes read
    /// from a file live nowhere in the database.
    pub fn of(source: &Source) -> Option<Self> {
        match source {
            Source::Global => Some(Self::Global),
            Source::Environment { id, .. } => id.map(Self::Environment),
            Source::Collection { id, .. } => id.map(Self::Collection),
            Source::Secret { environment } => Some(
                environment
                    .as_ref()
                    .map_or(Self::Global, |(id, _)| Self::Environment(*id)),
            ),
            Source::Captured | Source::Input | Source::Run => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub value: String,
    pub source: Source,
    /// Values saved for this name in the same scope, to swap the current one with.
    pub saved: Vec<String>,
}

/// Every definition of every name, highest precedence first; what execution uses and what
/// the inspector explains.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub definitions: IndexMap<String, Vec<Definition>>,
    /// The collection values in effect, captured into execution history.
    pub collection_values: Variables,
}

impl Snapshot {
    /// Precedence: global, then collection, then the active environment; secrets, named
    /// `secret:<name>`, global then the environment's.
    pub fn build(context: &VariableContext, collection: Option<CollectionScope<'_>>) -> Self {
        let mut snapshot = Self::default();
        let no_saved = SavedValues::default();
        let (collection_source, collection_values, collection_saved) = match collection {
            Some(CollectionScope::Live {
                id,
                name,
                values,
                saved,
            }) => (
                Source::Collection {
                    id,
                    name: name.to_owned(),
                },
                values.clone(),
                saved,
            ),
            Some(CollectionScope::Deleted { captured }) => {
                (Source::Captured, captured.clone(), &no_saved)
            }
            None => (
                Source::Collection {
                    id: None,
                    name: String::new(),
                },
                Variables::new(),
                &no_saved,
            ),
        };
        snapshot.add(&context.globals, &context.global_saved, &Source::Global);
        snapshot.add(&collection_values, collection_saved, &collection_source);
        if let Some(env) = &context.environment {
            snapshot.add(
                &env.values,
                &env.saved,
                &Source::Environment {
                    id: env.id,
                    name: env.name.clone(),
                },
            );
        }
        snapshot.add_secrets(
            &context.global_secrets,
            &Source::Secret { environment: None },
        );
        if let Some(env) = &context.environment
            && let Some(id) = env.id
        {
            snapshot.add_secrets(
                &env.secrets,
                &Source::Secret {
                    environment: Some((id, env.name.clone())),
                },
            );
        }
        snapshot.collection_values = collection_values;
        snapshot
    }

    fn add_secrets(&mut self, secrets: &Variables, source: &Source) {
        let keyed: Variables = secrets
            .iter()
            .map(|(name, value)| (secret_key(name), value.clone()))
            .collect();
        self.add(&keyed, &SavedValues::default(), source);
    }

    /// Every value given for a `secret:` name, the shadowed ones too, whether stored, given
    /// to the run or captured by it: what a record must not keep.
    pub fn secret_values(&self) -> impl Iterator<Item = &str> {
        self.definitions
            .iter()
            .filter(|(name, _)| secret_name(name).is_some())
            .flat_map(|(_, definitions)| definitions)
            .map(|d| d.value.as_str())
    }

    /// `inputs` above every scope, as a headless run's named inputs are.
    #[must_use]
    pub fn with_inputs(self, inputs: &Variables) -> Self {
        self.with_layer(inputs, &Source::Input)
    }

    /// `values` above everything already here, from `source`.
    #[must_use]
    pub fn with_layer(mut self, values: &Variables, source: &Source) -> Self {
        self.add(values, &SavedValues::default(), source);
        self
    }

    fn add(&mut self, values: &Variables, saved: &SavedValues, source: &Source) {
        for (name, value) in values {
            let definitions = self.definitions.entry(name.clone()).or_default();
            definitions.insert(
                0,
                Definition {
                    value: value.clone(),
                    source: source.clone(),
                    saved: saved.values(name),
                },
            );
        }
    }

    /// The effective value of every name.
    pub fn values(&self) -> Variables {
        self.definitions
            .iter()
            .filter_map(|(name, definitions)| {
                Some((name.clone(), definitions.first()?.value.clone()))
            })
            .collect()
    }

    pub fn effective(&self, name: &str) -> Option<&Definition> {
        self.definitions.get(name)?.first()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<&str> {
        placeholders(text).into_iter().map(|p| p.name).collect()
    }

    #[test]
    fn placeholder_matching_follows_the_regex() {
        assert_eq!(names("{{a}} {{ b }} {{}}"), ["a", "b", ""]);
        assert_eq!(names("{{{a}}}"), ["{a"]);
        assert_eq!(names("{{{{a}}}}"), ["{{a"]);
        assert_eq!(names("{{a\n}} {{b}}"), ["b"]);
        assert_eq!(names("{{a"), Vec::<&str>::new());
        assert_eq!(names("{{\u{1f}x\u{a0}}}"), ["x"]);
        assert_eq!(placeholders("ab{{c}}d")[0].range, 2..7);
    }

    #[test]
    fn substitution_inserts_values_verbatim_and_collects_missing() {
        let values: Variables = [("a".into(), "{{b}}".into())].into_iter().collect();
        let mut missing = BTreeSet::new();
        let out = substitute(
            "x{{a}}y{{c}}",
            |n| values.get(n).map(String::as_str),
            &mut missing,
        );
        assert_eq!(out, "x{{b}}y{{c}}");
        assert_eq!(missing, BTreeSet::from(["c".to_owned()]));
    }

    #[test]
    fn invalid_utf8_text_bodies_are_reported() {
        let template = RequestTemplate {
            body: vec![0xff],
            ..RequestTemplate::default()
        };
        assert!(matches!(
            resolve_template(&template, &Variables::new()),
            Err(ResolveError::BodyNotUtf8)
        ));
    }

    fn vars(pairs: &[(&str, &str)]) -> Variables {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn definitions_carry_the_values_saved_in_their_own_scope() {
        let saved = |name: &str, values: &[&str]| {
            let mut saved = SavedValues::default();
            saved.set(name, values.iter().map(|v| (*v).to_owned()));
            saved
        };
        let context = VariableContext {
            globals: vars(&[("id", "1")]),
            global_saved: saved("id", &["1", "2"]),
            environment: Some(ActiveEnvironment {
                id: Some(EnvironmentId(1)),
                name: "Dev".into(),
                values: vars(&[("id", "42")]),
                saved: saved("id", &["42", "77"]),
                secrets: Variables::new(),
            }),
            global_secrets: Variables::new(),
        };
        let collection = vars(&[("id", "c"), ("other", "o")]);
        let snapshot = Snapshot::build(
            &context,
            Some(CollectionScope::Live {
                id: Some(CollectionId(3)),
                name: "API",
                values: &collection,
                saved: &saved("id", &["c", "d"]),
            }),
        );
        let lists: Vec<_> = snapshot.definitions["id"]
            .iter()
            .map(|d| (d.source.label().to_owned(), d.saved.clone()))
            .collect();
        assert_eq!(
            lists,
            [
                ("Dev".to_owned(), vec!["42".to_owned(), "77".to_owned()]),
                ("API".to_owned(), vec!["c".to_owned(), "d".to_owned()]),
                ("Global".to_owned(), vec!["1".to_owned(), "2".to_owned()]),
            ]
        );
        assert_eq!(
            snapshot.effective("other").unwrap().saved,
            Vec::<String>::new()
        );
        assert_eq!(
            Scope::of(&snapshot.effective("id").unwrap().source),
            Some(Scope::Environment(EnvironmentId(1)))
        );
        assert_eq!(Scope::of(&Source::Global), Some(Scope::Global));
        assert_eq!(
            Scope::of(&Source::Captured),
            None,
            "captured values of a deleted collection cannot be edited"
        );
        assert_eq!(
            Scope::of(&Source::Environment {
                id: None,
                name: "Dev".into()
            }),
            None,
            "an environment read from a file is not stored"
        );
    }

    #[test]
    fn snapshot_precedence_and_provenance() {
        let context = VariableContext {
            globals: vars(&[("a", "global"), ("g", "only global"), ("e", "")]),
            environment: Some(ActiveEnvironment {
                id: Some(EnvironmentId(7)),
                name: "Dev".into(),
                values: vars(&[("a", "env"), ("e", "")]),
                saved: SavedValues::default(),
                secrets: Variables::new(),
            }),
            ..VariableContext::default()
        };
        let collection = vars(&[("a", "collection"), ("c", "only collection")]);
        let snapshot = Snapshot::build(
            &context,
            Some(CollectionScope::Live {
                id: Some(CollectionId(3)),
                name: "API",
                values: &collection,
                saved: &SavedValues::default(),
            }),
        );

        assert_eq!(
            snapshot.values(),
            vars(&[
                ("a", "env"),
                ("g", "only global"),
                ("e", ""),
                ("c", "only collection")
            ])
        );
        let sources: Vec<_> = snapshot.definitions["a"]
            .iter()
            .map(|d| d.source.label())
            .collect();
        assert_eq!(sources, ["Dev", "API", "Global"]);
        assert_eq!(snapshot.collection_values, collection);
    }

    #[test]
    fn used_names_cover_every_field_that_reaches_the_wire_and_checks() {
        use crate::check::{Check, Op};
        use crate::model::{BodyType, ParamItem, RequestAuth};
        use crate::select::Selector;
        let template = RequestTemplate {
            url: "{{base}}/users/{{ id }}".into(),
            headers: vec![ParamItem::new("X-Trace", "{{trace}}")],
            queries: vec![ParamItem::new("q", "{{$uuid}}")],
            auth: RequestAuth::bearer("{{token}}"),
            body: b"{\"n\": {{n}}}".to_vec(),
            body_type: BodyType::Json,
            checks: vec![Check::new(Selector::Status, Op::Equals, "{{expected}}")],
            ..RequestTemplate::default()
        };
        assert_eq!(
            used(&template),
            ["$uuid", "base", "expected", "id", "n", "token", "trace"]
        );
        let broken = RequestTemplate {
            url: "{{base}}".into(),
            body: vec![0xff, b'{', b'{'],
            body_type: BodyType::Json,
            ..RequestTemplate::default()
        };
        assert_eq!(
            used(&broken),
            ["base"],
            "an unreadable body hides nothing else"
        );
    }

    #[test]
    fn inputs_override_every_scope() {
        let context = VariableContext {
            globals: vars(&[("token", "global"), ("host", "h")]),
            ..VariableContext::default()
        };
        let snapshot =
            Snapshot::build(&context, None).with_inputs(&vars(&[("token", "from input")]));
        assert_eq!(
            snapshot.values(),
            vars(&[("token", "from input"), ("host", "h")])
        );
        let token = snapshot.effective("token").unwrap();
        assert_eq!(token.source, Source::Input);
        assert_eq!(token.source.label(), "Input");
        assert_eq!(Scope::of(&token.source), None, "an input is not stored");
        assert_eq!(snapshot.definitions["token"].len(), 2);
    }

    #[test]
    fn deleted_collection_uses_captured_values() {
        let captured = vars(&[("c", "captured")]);
        let snapshot = Snapshot::build(
            &VariableContext::default(),
            Some(CollectionScope::Deleted {
                captured: &captured,
            }),
        );
        let definition = snapshot.effective("c").unwrap();
        assert_eq!(definition.value, "captured");
        assert_eq!(definition.source, Source::Captured);
        assert_eq!(definition.source.label(), "Captured collection (deleted)");
    }

    #[test]
    fn secrets_resolve_by_their_prefix_the_environment_first() {
        let context = VariableContext {
            globals: [("token".into(), "a variable".into())]
                .into_iter()
                .collect(),
            global_secrets: [
                ("token".into(), "global-secret".into()),
                ("key".into(), "global-key".into()),
            ]
            .into_iter()
            .collect(),
            environment: Some(ActiveEnvironment {
                id: Some(EnvironmentId(2)),
                name: "Dev".into(),
                values: Variables::new(),
                saved: SavedValues::default(),
                secrets: [("token".into(), "dev-secret".into())]
                    .into_iter()
                    .collect(),
            }),
            ..VariableContext::default()
        };
        let snapshot = Snapshot::build(&context, None);
        let token = snapshot.effective("secret:token").unwrap();
        assert_eq!(token.value, "dev-secret");
        assert_eq!(
            token.source,
            Source::Secret {
                environment: Some((EnvironmentId(2), "Dev".into()))
            }
        );
        assert_eq!(token.source.label(), "Dev");
        assert_eq!(
            Scope::of(&token.source),
            Some(Scope::Environment(EnvironmentId(2)))
        );
        assert_eq!(snapshot.effective("token").unwrap().value, "a variable");
        let mut secrets: Vec<&str> = snapshot.secret_values().collect();
        secrets.sort_unstable();
        assert_eq!(secrets, ["dev-secret", "global-key", "global-secret"]);

        let template = RequestTemplate {
            url: "https://x/{{ secret:token }}/{{secret:key}}/{{token}}".into(),
            ..RequestTemplate::default()
        };
        let resolved = resolve_template(&template, &snapshot.values()).unwrap();
        assert_eq!(resolved.url, "https://x/dev-secret/global-key/a variable");
        let missing = RequestTemplate {
            url: "{{secret:nope}}".into(),
            ..RequestTemplate::default()
        };
        assert!(matches!(
            resolve_template(&missing, &snapshot.values()),
            Err(ResolveError::Unresolved(names)) if names.contains("secret:nope")
        ));
        assert_eq!(secret_name("secret: token "), Some("token"));
        assert_eq!(secret_name("token"), None);
        assert_eq!(
            SecretScope::of_scope(Scope::Collection(CollectionId(1))),
            None
        );

        // An environment read from a file has no stored secrets to add.
        let mut file = context.clone();
        if let Some(env) = &mut file.environment {
            env.id = None;
        }
        assert_eq!(
            Snapshot::build(&file, None)
                .effective("secret:token")
                .unwrap()
                .value,
            "global-secret"
        );
    }
}
