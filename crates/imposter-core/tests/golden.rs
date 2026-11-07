//! Request preparation, variable resolution and settings resolution against golden cases
//! (`tests/fixtures/golden`): the URL, headers and body each request goes out with.
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
        .join("../../tests/fixtures/golden")
        .join(name);
    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
}

fn template(case: &Value) -> RequestTemplate {
    serde_json::from_value(case["request"].clone()).unwrap()
}

#[test]
fn requests_are_prepared_as_the_golden_cases_say() {
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
        // The cases list the headers before authentication; Basic and Bearer `Authorization`
        // comes last.
        if matches!(
            template.auth.kind,
            Some(AuthKind::Basic | AuthKind::BearerToken)
        ) {
            let last = prepared.headers.last().unwrap();
            assert_eq!(last.0, "Authorization", "{name}");
            headers.push(last.clone());
        }
        assert_eq!(prepared.headers, headers, "{name}: headers");
        let golden_body = base64_bytes::decode(expected["body"].as_str().unwrap()).unwrap();
        match prepared.body {
            PreparedBody::Bytes(bytes) => assert_eq!(bytes, golden_body, "{name}: body"),
            // Multipart bodies go to the transport as fields; the case holds them encoded.
            PreparedBody::Multipart(fields) => {
                let all = RequestTemplate {
                    body: golden_body,
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
            PreparedBody::File(path) => assert_eq!(path.as_bytes(), golden_body, "{name}: path"),
        }
    }
}

/// Form bodies are compared as JSON values: their spacing is not part of the format.
fn normalized(mut value: Value, body_type: BodyType) -> Value {
    if body_type.is_form() {
        let body = base64_bytes::decode(value["body"].as_str().unwrap()).unwrap();
        value["body"] = serde_json::from_slice(&body).unwrap();
    }
    value
}

#[test]
fn variables_resolve_as_the_golden_cases_say() {
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
fn settings_resolve_as_the_golden_cases_say() {
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
fn stored_templates_round_trip_unchanged() {
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
