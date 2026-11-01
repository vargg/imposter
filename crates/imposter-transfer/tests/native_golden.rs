//! Golden compatibility with documents written and judged by the Python app itself
//! (`tests/fixtures/python/generate.py`).
#![cfg(test)]

use std::path::{Path, PathBuf};

use imposter_transfer::native::{Format, Version, decode, encode};
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

#[test]
fn python_exports_round_trip_byte_for_byte() {
    for name in ["full.poster.json", "minimal.poster.json"] {
        let original = std::fs::read(fixtures().join("native").join(name)).unwrap();
        let document = decode(&original).unwrap_or_else(|error| panic!("{name}: {error}"));
        let encoded = encode(&document);
        assert_eq!(
            String::from_utf8(encoded).unwrap(),
            String::from_utf8(original).unwrap(),
            "{name}"
        );
    }
}

#[test]
fn acceptance_matches_python_and_accepted_documents_encode_alike() {
    let cases: Vec<Value> =
        serde_json::from_slice(&std::fs::read(fixtures().join("parity/decode.json")).unwrap())
            .unwrap();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let result = decode(case["document"].as_str().unwrap().as_bytes());
        assert_eq!(
            result.is_ok(),
            case["accepted"].as_bool().unwrap(),
            "{name}: {result:?}"
        );
        if let Ok(document) = result {
            let encoded: Value = serde_json::from_slice(&encode(&document)).unwrap();
            assert_eq!(encoded, case["result"], "{name}");
        }
    }
}

#[test]
fn version_2_documents_carry_checks_and_round_trip_byte_for_byte() {
    let original = std::fs::read(fixtures().join("native/checks.imposter.json")).unwrap();
    let document = decode(&original).unwrap();
    assert_eq!(
        (document.format, document.version),
        (Format::Imposter, Version::V2)
    );
    let checks = &document.requests()[0].checks;
    assert_eq!(checks.len(), 8);
    assert!(!checks[5].enabled);
    assert_eq!(checks[6].describe(), "$.email matches ^[^@]+@{{domain}}$");
    let captures = &document.requests()[0].captures;
    assert_eq!(captures.len(), 3);
    assert_eq!(captures[1].variable, "etag");
    assert_eq!(
        String::from_utf8(encode(&document)).unwrap(),
        String::from_utf8(original).unwrap()
    );
}

#[test]
fn markers_and_versions_must_agree_and_version_1_stays_python_shaped() {
    let minimal = std::fs::read_to_string(fixtures().join("native/minimal.poster.json")).unwrap();
    let with = |format: &str, version: &str| {
        minimal
            .replace("\"poster\"", &format!("\"{format}\""))
            .replace("\"version\": 1", &format!("\"version\": {version}"))
    };
    assert!(decode(with("poster", "1").as_bytes()).is_ok());
    assert!(decode(with("imposter", "2").as_bytes()).is_ok());
    for (format, version) in [("poster", "2"), ("imposter", "1"), ("imposter", "3")] {
        assert!(
            decode(with(format, version).as_bytes()).is_err(),
            "{format} {version}"
        );
    }

    let checks = std::fs::read_to_string(fixtures().join("native/checks.imposter.json")).unwrap();
    let as_version_1 = checks
        .replace("\"imposter\"", "\"poster\"")
        .replace("\"version\": 2", "\"version\": 1");
    let error = decode(as_version_1.as_bytes()).unwrap_err().to_string();
    assert_eq!(
        error,
        "Request \"Create user\": checks and captures need format version 2"
    );
    let python = std::fs::read(fixtures().join("native/full.poster.json")).unwrap();
    let document = decode(&python).unwrap();
    assert_eq!(
        (document.format, document.version),
        (Format::Poster, Version::V1),
        "a document keeps its version"
    );
}

#[test]
fn postman_exports_name_the_checks_and_captures_they_leave_out() {
    let original = std::fs::read(fixtures().join("native/checks.imposter.json")).unwrap();
    let document = decode(&original).unwrap();
    let (data, warnings) = imposter_transfer::postman::encode(&document).unwrap();
    assert!(!data.to_string().contains("$.email"));
    assert!(
        warnings.contains(
            &"Create user: checks are not exported to Postman, which runs tests as scripts"
                .to_owned()
        ),
        "{warnings:?}"
    );
    assert!(
        warnings.contains(
            &"Create user: captured variables are not exported to Postman, which sets them in scripts"
                .to_owned()
        ),
        "{warnings:?}"
    );
}

#[test]
fn saved_values_need_version_2() {
    let minimal = std::fs::read_to_string(fixtures().join("native/minimal.poster.json")).unwrap();
    let with_saved = minimal.replace(
        "\"globals\": {}",
        "\"globals\": {\"user\": \"1\"},\n  \"saved_globals\": {\"user\": [\"1\", \"2\"]}",
    );
    assert_eq!(
        decode(with_saved.as_bytes()).unwrap_err().to_string(),
        "Saved values need format version 2"
    );
    let version_2 = with_saved
        .replace("\"poster\"", "\"imposter\"")
        .replace("\"version\": 1", "\"version\": 2");
    let document = decode(version_2.as_bytes()).unwrap();
    assert_eq!(document.saved_globals.values("user"), ["1", "2"]);
}

#[test]
fn environment_colors_need_version_2_and_are_left_out_when_unset() {
    let minimal = std::fs::read_to_string(fixtures().join("native/minimal.poster.json")).unwrap();
    let colored = minimal.replace(
        "\"environments\": []",
        "\"environments\": [{\"name\": \"Prod\", \"color\": \"red\"}, {\"name\": \"Dev\"}]",
    );
    assert_eq!(
        decode(colored.as_bytes()).unwrap_err().to_string(),
        "Environment colors need format version 2"
    );
    let version_2 = colored
        .replace("\"poster\"", "\"imposter\"")
        .replace("\"version\": 1", "\"version\": 2");
    let document = decode(version_2.as_bytes()).unwrap();
    assert_eq!(
        document
            .environments
            .iter()
            .map(|e| e.color)
            .collect::<Vec<_>>(),
        [Some(imposter_core::model::EnvColor::Red), None]
    );
    let encoded = String::from_utf8(encode(&document)).unwrap();
    assert_eq!(encoded.matches("\"color\"").count(), 1, "{encoded}");
    assert!(
        decode(version_2.replace("\"red\"", "\"pink\"").as_bytes()).is_err(),
        "only known colors"
    );
}
