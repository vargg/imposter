//! Captures: values taken from a response into variables, so the next request can use them
//! (a login's token, a created item's id). Declarative, like checks: a variable name and a
//! selector, stored with the request.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::execution::{Outcome, Response};
use crate::select::{ResponseView, Selector};
use crate::variables::{Scope, SecretScope, Snapshot, Source, VariableContext, secret_key};

fn yes() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    #[serde(default = "yes")]
    pub enabled: bool,
    pub variable: String,
    pub selector: Selector,
    /// Store the value as a secret (`{{secret:variable}}`), never as a variable.
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
}

impl Capture {
    pub fn new(variable: impl Into<String>, selector: Selector) -> Self {
        Self {
            enabled: true,
            variable: variable.into(),
            selector,
            secret: false,
        }
    }
}

/// What a capture took from a response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Taken {
    /// The value, as text: strings as they are, anything else as JSON.
    Value(String),
    /// The selector named nothing (a missing header, a path without matches); the variable
    /// keeps its value.
    Missing,
    /// The response could not answer (a cut or non-JSON body, a bad path), and why.
    Unreadable(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Captured {
    pub variable: String,
    pub taken: Taken,
    /// The scope the value is written to, by name.
    pub target: String,
    /// Written as a secret: its value is never shown or kept.
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
}

/// A value to write: the current value of `name` in `scope`, or in the run only when
/// `scope` is `None` (nothing is stored). A secret goes to the secrets of `scope`, which
/// is then the globals or an environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    pub scope: Option<Scope>,
    pub name: String,
    pub value: String,
    pub secret: bool,
}

impl Write {
    /// The placeholder name the value answers to: `name`, or `secret:name`.
    pub fn key(&self) -> String {
        if self.secret {
            secret_key(&self.name)
        } else {
            self.name.clone()
        }
    }

    /// Where a secret is stored; `None` for a variable or a write kept in the run.
    pub fn secret_scope(&self) -> Option<SecretScope> {
        self.scope
            .filter(|_| self.secret)
            .and_then(SecretScope::of_scope)
    }
}

/// Where a run's captures and script variables go.
#[derive(Debug, Clone, Copy)]
pub enum Destination<'a> {
    /// Into the stored scopes the request's variables came from (Send in the app).
    Scopes(&'a Snapshot, &'a VariableContext),
    /// Into the run only: its variables were read from a file, and writing a value back
    /// is a separate edit.
    Run,
}

impl Destination<'_> {
    /// The words naming a write that stays in the run.
    pub const RUN_NAME: &'static str = "this run";

    /// Where a write of `name` goes, and its name for people.
    pub fn of(&self, name: &str) -> (Option<Scope>, String) {
        match self {
            Self::Scopes(snapshot, context) => target(snapshot, context, name),
            Self::Run => (None, Self::RUN_NAME.to_owned()),
        }
    }

    /// Where a secret `name` goes, and its name for people ("Dev secrets").
    pub fn of_secret(&self, name: &str) -> (Option<Scope>, String) {
        match self {
            Self::Scopes(snapshot, context) => {
                let (scope, label) = secret_target(snapshot, context, name);
                (Some(scope.into()), label)
            }
            Self::Run => (None, Self::RUN_NAME.to_owned()),
        }
    }
}

/// Where a secret `name` is stored: where it is defined now, else the active
/// environment's secrets, else the global ones. An environment read from a file has no
/// stored secrets, so its writes go to the global ones.
pub fn secret_target(
    snapshot: &Snapshot,
    context: &VariableContext,
    name: &str,
) -> (SecretScope, String) {
    let defined = snapshot
        .effective(&secret_key(name))
        .and_then(|definition| match &definition.source {
            Source::Secret { environment } => Some(environment.clone()),
            _ => None,
        });
    let environment = match defined {
        Some(environment) => environment,
        None => context
            .environment
            .as_ref()
            .and_then(|env| Some((env.id?, env.name.clone()))),
    };
    match environment {
        Some((id, name)) => (SecretScope::Environment(id), format!("{name} secrets")),
        None => (
            SecretScope::Global,
            format!("{} secrets", VariableContext::GLOBAL_NAME),
        ),
    }
}

/// Where a captured `name` goes: the scope whose definition is in effect, so the value the
/// request saw is the one replaced. A name nobody defines, or one defined only by a deleted
/// collection's captured values, goes to the active environment, else to the globals. A
/// scope that is not stored (read from a file) gives `None`.
pub fn target(
    snapshot: &Snapshot,
    context: &VariableContext,
    name: &str,
) -> (Option<Scope>, String) {
    if let Some(definition) = snapshot.effective(name)
        && definition.source != Source::Captured
    {
        return (
            Scope::of(&definition.source),
            definition.source.label().to_owned(),
        );
    }
    match &context.environment {
        Some(environment) => (
            environment.id.map(Scope::Environment),
            environment.name.clone(),
        ),
        None => (Some(Scope::Global), VariableContext::GLOBAL_NAME.to_owned()),
    }
}

/// Take the enabled captures' values from `response`. Nothing is taken without a response
/// (a transport error or a cancel), and later captures of the same name win.
pub fn evaluate(
    captures: &[Capture],
    response: &Response,
    destination: Destination<'_>,
) -> (Vec<Captured>, Vec<Write>) {
    if response.outcome != Outcome::Response {
        return (Vec::new(), Vec::new());
    }
    let view = ResponseView::new(response);
    let mut results = Vec::new();
    let mut writes: Vec<Write> = Vec::new();
    for capture in captures {
        let name = capture.variable.trim();
        if !capture.enabled || name.is_empty() {
            continue;
        }
        let (scope, label) = if capture.secret {
            destination.of_secret(name)
        } else {
            destination.of(name)
        };
        let taken = match view.read(&capture.selector) {
            Ok(Some(Value::String(text))) => Taken::Value(text),
            Ok(Some(value)) => Taken::Value(value.to_string()),
            Ok(None) => Taken::Missing,
            Err(error) => Taken::Unreadable(error.to_string()),
        };
        if let Taken::Value(value) = &taken {
            writes.retain(|w| w.name != name || w.secret != capture.secret);
            writes.push(Write {
                scope,
                name: name.to_owned(),
                value: value.clone(),
                secret: capture.secret,
            });
        }
        results.push(Captured {
            variable: name.to_owned(),
            taken,
            target: label,
            secret: capture.secret,
        });
    }
    (results, writes)
}

/// The stored variable scopes `writes` touch, once each; secrets are not variables.
pub fn scopes(writes: &[Write]) -> Vec<Scope> {
    let mut scopes: Vec<Scope> = Vec::new();
    for scope in writes.iter().filter(|w| !w.secret).filter_map(|w| w.scope) {
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    scopes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{CollectionId, EnvironmentId};
    use crate::model::{SavedValues, Variables};
    use crate::variables::{ActiveEnvironment, CollectionScope};

    fn vars(pairs: &[(&str, &str)]) -> Variables {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn context(environment: bool) -> VariableContext {
        VariableContext {
            globals: vars(&[("region", "eu")]),
            environment: environment.then(|| ActiveEnvironment {
                id: Some(EnvironmentId(3)),
                name: "Dev".into(),
                values: vars(&[("token", "old")]),
                saved: SavedValues::default(),
                secrets: Variables::new(),
            }),
            ..VariableContext::default()
        }
    }

    fn snapshot(context: &VariableContext, collection: &Variables) -> Snapshot {
        let saved = SavedValues::default();
        Snapshot::build(
            context,
            Some(CollectionScope::Live {
                id: Some(CollectionId(9)),
                name: "API",
                values: collection,
                saved: &saved,
            }),
        )
    }

    fn response() -> Response {
        Response {
            status: 200,
            headers: vec![("x-request-id".into(), "r-1".into())],
            body: br#"{"token":"new","user":{"id":7},"tags":["a"]}"#.to_vec(),
            ..Response::default()
        }
    }

    #[test]
    fn values_go_to_the_scope_whose_definition_is_in_effect() {
        let active = context(true);
        let collection = vars(&[("user_id", "1"), ("region", "us")]);
        let built = snapshot(&active, &collection);
        let (snapshot, context) = (&built, &active);
        assert_eq!(
            target(snapshot, context, "token"),
            (Some(Scope::Environment(EnvironmentId(3))), "Dev".into())
        );
        assert_eq!(
            target(snapshot, context, "user_id"),
            (Some(Scope::Collection(CollectionId(9))), "API".into())
        );
        assert_eq!(
            target(snapshot, context, "region"),
            (Some(Scope::Collection(CollectionId(9))), "API".into()),
            "the collection overrides the globals"
        );
        assert_eq!(
            target(snapshot, context, "new_name"),
            (Some(Scope::Environment(EnvironmentId(3))), "Dev".into()),
            "new names go to the active environment"
        );
        let global = self::context(false);
        assert_eq!(
            target(
                &self::snapshot(&global, &Variables::new()),
                &global,
                "new_name"
            ),
            (Some(Scope::Global), "Global".into())
        );
        let deleted = Snapshot::build(
            &global,
            Some(CollectionScope::Deleted {
                captured: &vars(&[("gone", "x")]),
            }),
        );
        assert_eq!(
            target(&deleted, &global, "gone"),
            (Some(Scope::Global), "Global".into()),
            "a deleted collection holds nothing"
        );
    }

    #[test]
    fn captures_take_text_and_json_and_report_what_they_could_not() {
        let context = context(true);
        let snapshot = snapshot(&context, &Variables::new());
        let captures = [
            Capture::new("token", Selector::Json("$.token".into())),
            Capture::new(" user_id ", Selector::Json("$.user.id".into())),
            Capture::new("user", Selector::Json("$.user".into())),
            Capture::new("request", Selector::Header("X-Request-Id".into())),
            Capture::new("missing", Selector::Json("$.nope".into())),
            Capture::new("broken", Selector::Json("nope".into())),
            Capture::new("", Selector::Status),
            Capture {
                enabled: false,
                ..Capture::new("off", Selector::Status)
            },
        ];
        let (results, writes) = evaluate(
            &captures,
            &response(),
            Destination::Scopes(&snapshot, &context),
        );
        let taken: Vec<(&str, &Taken)> = results
            .iter()
            .map(|r| (r.variable.as_str(), &r.taken))
            .collect();
        assert_eq!(taken[0], ("token", &Taken::Value("new".into())));
        assert_eq!(taken[1], ("user_id", &Taken::Value("7".into())));
        assert_eq!(taken[2], ("user", &Taken::Value(r#"{"id":7}"#.into())));
        assert_eq!(taken[3], ("request", &Taken::Value("r-1".into())));
        assert_eq!(taken[4], ("missing", &Taken::Missing));
        assert!(
            matches!(taken[5], ("broken", Taken::Unreadable(r)) if r.starts_with("Invalid JSONPath"))
        );
        assert_eq!(
            results.len(),
            6,
            "unnamed and disabled captures are skipped"
        );
        assert!(results.iter().all(|r| r.target == "Dev"));
        let written: Vec<(&str, &str)> = writes
            .iter()
            .map(|w| (w.name.as_str(), w.value.as_str()))
            .collect();
        assert_eq!(
            written,
            [
                ("token", "new"),
                ("user_id", "7"),
                ("user", r#"{"id":7}"#),
                ("request", "r-1")
            ]
        );
        assert_eq!(scopes(&writes), [Scope::Environment(EnvironmentId(3))]);
    }

    #[test]
    fn the_last_capture_of_a_name_wins_and_failures_take_nothing() {
        let context = context(false);
        let snapshot = snapshot(&context, &Variables::new());
        let captures = [
            Capture::new("id", Selector::Status),
            Capture::new("id", Selector::Json("$.user.id".into())),
        ];
        let (_, writes) = evaluate(
            &captures,
            &response(),
            Destination::Scopes(&snapshot, &context),
        );
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].value, "7");

        let failed = Response::failure(Outcome::TransportError, "Connect", "refused");
        assert_eq!(
            evaluate(&captures, &failed, Destination::Scopes(&snapshot, &context)),
            (Vec::new(), Vec::new())
        );
    }

    #[test]
    fn a_run_from_a_file_keeps_its_writes() {
        let mut context = context(true);
        if let Some(environment) = &mut context.environment {
            environment.id = None;
        }
        let collection = vars(&[("user_id", "1")]);
        let saved = SavedValues::default();
        let snapshot = Snapshot::build(
            &context,
            Some(CollectionScope::Live {
                id: None,
                name: "API",
                values: &collection,
                saved: &saved,
            }),
        );
        assert_eq!(
            target(&snapshot, &context, "user_id"),
            (None, "API".into()),
            "a collection read from a file is not stored"
        );
        assert_eq!(
            target(&snapshot, &context, "new_name"),
            (None, "Dev".into())
        );
        let captures = [
            Capture::new("token", Selector::Json("$.token".into())),
            Capture::new("region", Selector::Json("$.tags[0]".into())),
        ];
        let (results, writes) = evaluate(&captures, &response(), Destination::Run);
        assert!(results.iter().all(|r| r.target == "this run"));
        assert!(writes.iter().all(|w| w.scope.is_none()));
        assert_eq!(writes.len(), 2);
        assert_eq!(scopes(&writes), [], "nothing to store");
    }

    #[test]
    fn captures_store_compactly() {
        let capture = Capture::new("token", Selector::Json("$.token".into()));
        assert_eq!(
            serde_json::to_string(&capture).unwrap(),
            r#"{"enabled":true,"variable":"token","selector":{"json":"$.token"}}"#
        );
        assert!(
            serde_json::from_str::<Capture>(r#"{"variable":"a","selector":"status","x":1}"#)
                .is_err()
        );
    }

    #[test]
    fn secret_captures_go_to_the_secrets_that_define_them_and_never_to_variables() {
        let mut active = context(true);
        active.global_secrets = vars(&[("api_key", "global-key")]);
        if let Some(env) = &mut active.environment {
            env.secrets = vars(&[("token", "dev-token")]);
        }
        let built = snapshot(&active, &Variables::new());
        let dev = Scope::Environment(EnvironmentId(3));
        assert_eq!(
            secret_target(&built, &active, "token"),
            (
                SecretScope::Environment(EnvironmentId(3)),
                "Dev secrets".into()
            )
        );
        assert_eq!(
            secret_target(&built, &active, "api_key"),
            (SecretScope::Global, "Global secrets".into()),
            "defined globally, replaced there"
        );
        assert_eq!(
            secret_target(&built, &active, "fresh").0,
            SecretScope::Environment(EnvironmentId(3)),
            "a new secret goes to the active environment"
        );
        let global = context(false);
        assert_eq!(
            secret_target(&snapshot(&global, &Variables::new()), &global, "fresh"),
            (SecretScope::Global, "Global secrets".into())
        );

        let mut capture = Capture::new("token", Selector::Json("$.token".into()));
        capture.secret = true;
        let plain = Capture::new("token", Selector::Json("$.user.id".into()));
        let (results, writes) = evaluate(
            &[plain, capture],
            &response(),
            Destination::Scopes(&built, &active),
        );
        assert_eq!(
            writes.len(),
            2,
            "a secret and a variable of one name are apart"
        );
        let secret = writes.iter().find(|w| w.secret).unwrap();
        assert_eq!(
            (secret.key().as_str(), secret.value.as_str()),
            ("secret:token", "new")
        );
        assert_eq!(
            secret.secret_scope(),
            Some(SecretScope::Environment(EnvironmentId(3)))
        );
        assert_eq!(results[1].target, "Dev secrets");
        assert!(results[1].secret);
        assert_eq!(
            scopes(&writes),
            [dev],
            "only the variable is a variable write"
        );

        let (_, run) = evaluate(
            &[Capture {
                secret: true,
                ..Capture::new("token", Selector::Json("$.token".into()))
            }],
            &response(),
            Destination::Run,
        );
        assert_eq!(run[0].scope, None);
        assert_eq!(run[0].secret_scope(), None, "kept in the run");
        assert_eq!(
            serde_json::to_string(&Capture {
                secret: true,
                ..Capture::new("t", Selector::Status)
            })
            .unwrap(),
            r#"{"enabled":true,"variable":"t","selector":"status","secret":true}"#
        );
    }
}
