//! Postman conversion parity with the Python app (`tests/fixtures/parity/postman.json`, produced
//! by running the Python adapter itself).
#![cfg(test)]

use std::path::Path;

use poster_transfer::native::{NativeDocument, TransferCollection, encode as encode_native};
use poster_transfer::postman::{Decoder, companion, encode};
use serde_json::Value;

fn fixture() -> Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/parity/postman.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// Form bodies hold JSON arrays that Python and Rust space differently: compare them parsed.
fn normalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let form = matches!(
                map.get("body_type").and_then(Value::as_str),
                Some("urlencoded" | "form-data" | "multipart")
            );
            if form && let Some(Value::String(body)) = map.get("body") {
                let bytes = poster_core::base64_bytes::decode(body).unwrap();
                let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                map.insert("body".into(), parsed);
            }
            map.values_mut().for_each(normalize);
        }
        Value::Array(items) => items.iter_mut().for_each(normalize),
        _ => {}
    }
}

fn document_value(document: &NativeDocument) -> Value {
    let mut value: Value = serde_json::from_slice(&encode_native(document)).unwrap();
    normalize(&mut value);
    value
}

#[test]
fn decoding_matches_python_including_warnings_and_consent() {
    for case in fixture()["decode"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut decoder = Decoder::default();
        let result = decoder.decode(&case["input"]);
        let expected = &case["expected"];
        if expected.get("error").is_some() {
            assert!(
                result.is_err(),
                "{name}: Python rejected it, Rust accepted {result:?}"
            );
            continue;
        }
        let document = result.unwrap_or_else(|error| panic!("{name}: {error}"));
        let mut python = expected["document"].clone();
        normalize(&mut python);
        assert_eq!(document_value(&document), python, "{name}: document");
        let warnings: Vec<String> = serde_json::from_value(expected["warnings"].clone()).unwrap();
        assert_eq!(decoder.warnings, warnings, "{name}: warnings");
        assert_eq!(
            decoder.requires_removal,
            expected["requires_removal"].as_bool().unwrap(),
            "{name}: consent"
        );
    }
}

#[test]
fn companions_match_python() {
    for case in fixture()["companions"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut decoder = Decoder::default();
        let mut document = NativeDocument::new(TransferCollection {
            name: poster_transfer::native::Name::new("C").unwrap(),
            description: String::new(),
            variables: poster_core::model::Variables::new(),
            folders: Vec::new(),
            requests: Vec::new(),
        });
        let result = decoder.companion(&case["input"], &mut document);
        let expected = &case["expected"];
        if expected.get("error").is_some() {
            assert!(result.is_err(), "{name}");
            continue;
        }
        result.unwrap();
        assert_eq!(document_value(&document), expected["document"], "{name}");
        let warnings: Vec<String> = serde_json::from_value(expected["warnings"].clone()).unwrap();
        assert_eq!(decoder.warnings, warnings, "{name}: warnings");
    }
}

#[test]
fn encoding_matches_python() {
    let fixture = fixture();
    for case in fixture["encode"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let document: NativeDocument = serde_json::from_value(case["document"].clone()).unwrap();
        let (postman, warnings) = encode(&document).unwrap();
        assert_eq!(postman, case["postman"], "{name}: collection");
        let expected: Vec<String> = serde_json::from_value(case["warnings"].clone()).unwrap();
        assert_eq!(warnings, expected, "{name}: warnings");
    }
    let values = [("a".to_owned(), "1".to_owned())].into_iter().collect();
    assert_eq!(
        companion("Dev", &values, "environment"),
        fixture["companion_bytes"]
    );
}
