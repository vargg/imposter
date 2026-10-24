//! `{{name}}` placeholders: finding, resolving and explaining them.
//!
//! Matching follows the Python app's regex `\{\{(.*?)\}\}` exactly: the first `}}` after `{{`
//! closes a placeholder unless a newline comes first, the name is trimmed, and values are
//! inserted verbatim (never resolved again). Parity is pinned by
//! `tests/fixtures/parity/resolve.json`.

use std::collections::BTreeSet;
use std::ops::Range;

use indexmap::IndexMap;

use crate::ids::{CollectionId, EnvironmentId};
use crate::model::{FormBodyError, Options, ParamItem, RequestAuth, RequestTemplate, Variables};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder<'a> {
    /// Byte range of the whole `{{ … }}` in the text.
    pub range: Range<usize>,
    /// The trimmed name between the braces.
    pub name: &'a str,
}

/// Python's `str.strip()` whitespace: Unicode whitespace plus the ASCII separators 0x1C–0x1F.
fn is_python_space(c: char) -> bool {
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
                    name: text[body..end].trim_matches(is_python_space),
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

struct Resolver<'a> {
    values: &'a Variables,
    missing: BTreeSet<String>,
}

impl Resolver<'_> {
    fn text(&mut self, text: &mut String) {
        if text.contains("{{") {
            *text = substitute(
                text,
                |name| self.values.get(name).map(String::as_str),
                &mut self.missing,
            );
        }
    }

    fn optional(&mut self, text: &mut Option<String>) {
        if let Some(text) = text {
            self.text(text);
        }
    }

    fn items(&mut self, items: &mut [ParamItem]) {
        for item in items {
            self.text(&mut item.name);
            self.text(&mut item.value);
        }
    }

    fn auth(&mut self, auth: &mut RequestAuth) {
        for credentials in [auth.basic.as_mut(), auth.digest.as_mut()]
            .into_iter()
            .flatten()
        {
            self.text(&mut credentials.username);
            self.text(&mut credentials.password);
        }
        if let Some(bearer) = &mut auth.bearer_token {
            self.text(&mut bearer.token);
        }
    }

    fn options(&mut self, options: &mut Options) {
        for field in [
            &mut options.proxy_url,
            &mut options.ca_bundle,
            &mut options.certificate_path,
            &mut options.key_file,
            &mut options.password,
        ] {
            self.optional(field);
        }
    }

    fn body(&mut self, template: &mut RequestTemplate) -> Result<(), ResolveError> {
        use crate::model::BodyType;
        match template.body_type {
            BodyType::None => {}
            BodyType::FormData | BodyType::Urlencoded => {
                let mut fields = template.form_fields()?;
                self.items(&mut fields);
                template.set_form_fields(&fields);
            }
            BodyType::Multipart => {
                let mut fields = template.multipart_fields()?;
                for field in &mut fields {
                    self.text(&mut field.name);
                    self.text(&mut field.value);
                    self.text(&mut field.content_type);
                    self.text(&mut field.filename);
                }
                template.set_multipart_fields(&fields);
            }
            BodyType::Json | BodyType::Plain | BodyType::Text | BodyType::Binary => {
                let mut text = String::from_utf8(std::mem::take(&mut template.body))
                    .map_err(|_| ResolveError::BodyNotUtf8)?;
                self.text(&mut text);
                template.body = text.into_bytes();
            }
        }
        Ok(())
    }
}

/// Resolve every field that reaches the wire or configures the connection: URL, rows, auth,
/// options and the body (file paths too, never file contents). Name and description stay.
pub fn resolve_template(
    template: &RequestTemplate,
    values: &Variables,
) -> Result<RequestTemplate, ResolveError> {
    let mut resolved = template.clone();
    let mut walk = Resolver {
        values,
        missing: BTreeSet::new(),
    };
    walk.text(&mut resolved.url);
    walk.items(&mut resolved.headers);
    walk.items(&mut resolved.queries);
    walk.items(&mut resolved.cookies);
    walk.auth(&mut resolved.auth);
    walk.options(&mut resolved.options);
    walk.body(&mut resolved)?;
    if walk.missing.is_empty() {
        Ok(resolved)
    } else {
        Err(ResolveError::Unresolved(walk.missing))
    }
}

/// The active environment's identity and values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveEnvironment {
    pub id: EnvironmentId,
    pub name: String,
    pub values: Variables,
}

/// Global and environment values captured before an execution's first await, so a later
/// environment switch cannot change a request that is already running.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VariableContext {
    pub globals: Variables,
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
        self.environment.as_ref().map(|env| env.id)
    }
}

/// The collection a request belongs to, or its captured values when the collection is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionScope<'a> {
    Live {
        id: CollectionId,
        name: &'a str,
        values: &'a Variables,
    },
    Deleted {
        captured: &'a Variables,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Global,
    Collection {
        id: Option<CollectionId>,
        name: String,
    },
    Environment {
        id: EnvironmentId,
        name: String,
    },
}

impl Source {
    pub fn label(&self) -> &str {
        match self {
            Self::Global => VariableContext::GLOBAL_NAME,
            Self::Collection { name, .. } | Self::Environment { name, .. } => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub value: String,
    pub source: Source,
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
    /// Precedence: global, then collection, then the active environment.
    pub fn build(context: &VariableContext, collection: Option<CollectionScope<'_>>) -> Self {
        let mut snapshot = Self::default();
        let (collection_source, collection_values) = match collection {
            Some(CollectionScope::Live { id, name, values }) => (
                Source::Collection {
                    id: Some(id),
                    name: name.to_owned(),
                },
                values.clone(),
            ),
            Some(CollectionScope::Deleted { captured }) => (
                Source::Collection {
                    id: None,
                    name: "Captured collection (deleted)".to_owned(),
                },
                captured.clone(),
            ),
            None => (
                Source::Collection {
                    id: None,
                    name: String::new(),
                },
                Variables::new(),
            ),
        };
        snapshot.add(&context.globals, &Source::Global);
        snapshot.add(&collection_values, &collection_source);
        if let Some(env) = &context.environment {
            snapshot.add(
                &env.values,
                &Source::Environment {
                    id: env.id,
                    name: env.name.clone(),
                },
            );
        }
        snapshot.collection_values = collection_values;
        snapshot
    }

    fn add(&mut self, values: &Variables, source: &Source) {
        for (name, value) in values {
            let definitions = self.definitions.entry(name.clone()).or_default();
            definitions.insert(
                0,
                Definition {
                    value: value.clone(),
                    source: source.clone(),
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
    fn placeholder_matching_follows_the_python_regex() {
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
    fn snapshot_precedence_and_provenance() {
        let context = VariableContext {
            globals: vars(&[("a", "global"), ("g", "only global"), ("e", "")]),
            environment: Some(ActiveEnvironment {
                id: EnvironmentId(7),
                name: "Dev".into(),
                values: vars(&[("a", "env"), ("e", "")]),
            }),
        };
        let collection = vars(&[("a", "collection"), ("c", "only collection")]);
        let snapshot = Snapshot::build(
            &context,
            Some(CollectionScope::Live {
                id: CollectionId(3),
                name: "API",
                values: &collection,
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
        assert_eq!(
            definition.source,
            Source::Collection {
                id: None,
                name: "Captured collection (deleted)".into()
            }
        );
    }
}
