//! Captures: values taken from a response into variables, so the next request can use them
//! (a login's token, a created item's id). Declarative, like checks: a variable name and a
//! selector, stored with the request.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::execution::{Outcome, Response};
use crate::select::{ResponseView, Selector};
use crate::variables::{Scope, Snapshot, Source, VariableContext};

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    #[serde(default = "yes")]
    pub enabled: bool,
    pub variable: String,
    pub selector: Selector,
}

impl Capture {
    pub fn new(variable: impl Into<String>, selector: Selector) -> Self {
        Self {
            enabled: true,
            variable: variable.into(),
            selector,
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
}

/// A value to write: the current value of `name` in `scope`, or in the run only when
/// `scope` is `None` (nothing is stored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    pub scope: Option<Scope>,
    pub name: String,
    pub value: String,
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
        let (scope, label) = destination.of(name);
        let taken = match view.read(&capture.selector) {
            Ok(Some(Value::String(text))) => Taken::Value(text),
            Ok(Some(value)) => Taken::Value(value.to_string()),
            Ok(None) => Taken::Missing,
            Err(error) => Taken::Unreadable(error.to_string()),
        };
        if let Taken::Value(value) = &taken {
            writes.retain(|w| w.name != name);
            writes.push(Write {
                scope,
                name: name.to_owned(),
                value: value.clone(),
            });
        }
        results.push(Captured {
            variable: name.to_owned(),
            taken,
            target: label,
        });
    }
    (results, writes)
}

/// The stored scopes `writes` touch, once each.
pub fn scopes(writes: &[Write]) -> Vec<Scope> {
    let mut scopes: Vec<Scope> = Vec::new();
    for scope in writes.iter().filter_map(|w| w.scope) {
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
}
