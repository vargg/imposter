//! Behavior parity with the Python reference app. The fixtures are produced by running the
//! Python code itself (`tests/fixtures/python/generate.py`), not written by hand.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // reading fixture files is the test harness, not core code

use std::path::Path;

use imposter_core::base64_bytes;
use imposter_core::model::{AuthKind, BodyType, FormItem, Options, RequestTemplate, Variables};
use imposter_core::prepare::{PreparedBody, prepare};
use imposter_core::settings::AppSettings;
use imposter_core::variables::resolve_template;
use serde_json::Value;

fn fixture(name: &str) -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/parity")
        .join(name);
    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
}

fn template(case: &Value) -> RequestTemplate {
    serde_json::from_value(case["request"].clone()).unwrap()
}

#[test]
fn request_preparation_matches_python() {
    for case in fixture("prepare.json") {
        let name = case["name"].as_str().unwrap();
        let template = template(&case);
        let attach_cookies = case["settings"]["attach_cookies"].as_bool().unwrap_or(true);
        let expected = &case["expected"];
        let result = prepare(&template, attach_cookies);

        if let Some(error) = expected["error"].as_str() {
            assert_eq!(result.map(|_| ()).unwrap_err().to_string(), error, "{name}");
            continue;
        }
        let prepared = result.unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            prepared.url,
            expected["url"].as_str().unwrap(),
            "{name}: url"
        );
        let mut headers: Vec<(String, String)> =
            serde_json::from_value(expected["headers"].clone()).unwrap();
        // httpx adds Basic and Bearer `Authorization` after Python's prepare step, at the end.
        if matches!(
            template.auth.kind,
            Some(AuthKind::Basic | AuthKind::BearerToken)
        ) {
            let last = prepared.headers.last().unwrap();
            assert_eq!(last.0, "Authorization", "{name}");
            headers.push(last.clone());
        }
        assert_eq!(prepared.headers, headers, "{name}: headers");
        let python_body = base64_bytes::decode(expected["body"].as_str().unwrap()).unwrap();
        match prepared.body {
            PreparedBody::Bytes(bytes) => assert_eq!(bytes, python_body, "{name}: body"),
            // Python hands multipart and binary bodies to its transport unchanged.
            PreparedBody::Multipart(fields) => {
                let all = RequestTemplate {
                    body: python_body,
                    ..template.clone()
                }
                .multipart_fields()
                .unwrap();
                assert_eq!(
                    fields,
                    all.into_iter()
                        .filter(FormItem::is_sent)
                        .collect::<Vec<_>>(),
                    "{name}"
                );
            }
            PreparedBody::File(path) => assert_eq!(path.as_bytes(), python_body, "{name}: path"),
        }
    }
}

/// Form bodies are compared as JSON values: Python re-serializes them with its own spacing.
fn normalized(mut value: Value, body_type: BodyType) -> Value {
    if body_type.is_form() {
        let body = base64_bytes::decode(value["body"].as_str().unwrap()).unwrap();
        value["body"] = serde_json::from_slice(&body).unwrap();
    }
    value
}

#[test]
fn variable_resolution_matches_python() {
    for case in fixture("resolve.json") {
        let name = case["name"].as_str().unwrap();
        let template = template(&case);
        let values: Variables = serde_json::from_value(case["values"].clone()).unwrap();
        let result = resolve_template(&template, &values);

        if let Some(error) = case["expected"]["error"].as_str() {
            assert_eq!(result.unwrap_err().to_string(), error, "{name}");
            continue;
        }
        let resolved = result.unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            normalized(serde_json::to_value(&resolved).unwrap(), resolved.body_type),
            normalized(case["expected"]["request"].clone(), resolved.body_type),
            "{name}"
        );
    }
}

#[test]
fn settings_resolution_matches_python() {
    for case in fixture("settings.json") {
        let base: AppSettings = serde_json::from_value(case["base"].clone()).unwrap();
        let options: Options = serde_json::from_value(case["options"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(base.resolve(&options)).unwrap(),
            case["expected"],
            "{options:?}"
        );
    }
}

#[test]
fn python_template_dumps_round_trip_unchanged() {
    for file in ["prepare.json", "resolve.json"] {
        for case in fixture(file) {
            let value = case["request"].clone();
            let template: RequestTemplate = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(
                serde_json::to_value(&template).unwrap(),
                value,
                "{}",
                case["name"]
            );
        }
    }
}
