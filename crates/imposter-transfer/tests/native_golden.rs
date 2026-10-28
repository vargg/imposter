//! Golden compatibility with documents written and judged by the Python app itself
//! (`tests/fixtures/python/generate.py`).
#![cfg(test)]

use std::path::{Path, PathBuf};

use imposter_transfer::native::{decode, encode};
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
