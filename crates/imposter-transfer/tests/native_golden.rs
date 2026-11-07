//! The native format against golden documents: exact bytes on the way out, and which
//! documents are accepted on the way in.
#![cfg(test)]

use std::path::{Path, PathBuf};

use imposter_transfer::native::{Format, Version, decode, encode};
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

#[test]
fn exports_round_trip_byte_for_byte() {
    for name in ["full.imposter.json", "minimal.imposter.json"] {
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
fn documents_are_accepted_or_refused_as_listed_and_encode_alike() {
    let cases: Vec<Value> = serde_json::from_slice(
        &std::fs::read(fixtures().join("native/decode-cases.json")).unwrap(),
    )
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
fn version_3_documents_carry_scripts_and_round_trip_byte_for_byte() {
    let original = std::fs::read(fixtures().join("native/scripts.imposter.json")).unwrap();
    let mut document = decode(&original).unwrap();
    assert_eq!(
        (document.format, document.version),
        (Format::Imposter, Version::V3)
    );
    let scripts = &document.requests()[0].scripts;
    assert!(scripts.before.contains("hmac_sha256"));
    assert!(scripts.after.contains("upload_id"));
    assert_eq!(
        String::from_utf8(encode(&document)).unwrap(),
        String::from_utf8(original.clone()).unwrap()
    );

    // Scripts need version 3: a version 2 document with them is refused.
    let as_version_2 = String::from_utf8(original)
        .unwrap()
        .replace("\"version\": 3", "\"version\": 2");
    assert_eq!(
        decode(as_version_2.as_bytes()).unwrap_err().to_string(),
        "Request \"Signed upload\": scripts need format version 3"
    );

    // New exports take the lowest version that holds them.
    document.fit_version();
    assert_eq!(document.version, Version::V3);
    document.collection.requests[0].scripts = imposter_core::model::Scripts::default();
    document.fit_version();
    assert_eq!(document.version, Version::V2, "readable by Imposter 0.4");
}

#[test]
fn the_marker_and_a_known_version_are_required() {
    let minimal = std::fs::read_to_string(fixtures().join("native/minimal.imposter.json")).unwrap();
    let with = |format: &str, version: &str| {
        minimal
            .replace("\"imposter\"", &format!("\"{format}\""))
            .replace("\"version\": 2", &format!("\"version\": {version}"))
    };
    assert!(decode(with("imposter", "2").as_bytes()).is_ok());
    assert!(decode(with("imposter", "3").as_bytes()).is_ok());
    for (format, version) in [("poster", "1"), ("imposter", "1"), ("imposter", "4")] {
        assert!(
            decode(with(format, version).as_bytes()).is_err(),
            "{format} {version}"
        );
    }
    let unmarked = minimal.replace("  \"format\": \"imposter\",\n", "");
    assert!(
        decode(unmarked.as_bytes()).is_err(),
        "the marker is required"
    );

    let full = std::fs::read(fixtures().join("native/full.imposter.json")).unwrap();
    let document = decode(&full).unwrap();
    assert_eq!(
        (document.format, document.version),
        (Format::Imposter, Version::V2),
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
fn postman_exports_name_the_scripts_they_leave_out() {
    let original = std::fs::read(fixtures().join("native/scripts.imposter.json")).unwrap();
    let document = decode(&original).unwrap();
    let (data, warnings) = imposter_transfer::postman::encode(&document).unwrap();
    assert!(!data.to_string().contains("hmac_sha256"));
    assert!(
        warnings.contains(
            &"Signed upload: scripts are not exported to Postman, whose scripts are JavaScript"
                .to_owned()
        ),
        "{warnings:?}"
    );
}

#[test]
fn saved_values_travel_with_their_scope() {
    let minimal = std::fs::read_to_string(fixtures().join("native/minimal.imposter.json")).unwrap();
    let with_saved = minimal.replace(
        "\"globals\": {}",
        "\"globals\": {\"user\": \"1\"},\n  \"saved_globals\": {\"user\": [\"1\", \"2\"]}",
    );
    let document = decode(with_saved.as_bytes()).unwrap();
    assert_eq!(document.saved_globals.values("user"), ["1", "2"]);
}

#[test]
fn environment_colors_are_known_names_left_out_when_unset() {
    let minimal = std::fs::read_to_string(fixtures().join("native/minimal.imposter.json")).unwrap();
    let colored = minimal.replace(
        "\"environments\": []",
        "\"environments\": [{\"name\": \"Prod\", \"color\": \"red\"}, {\"name\": \"Dev\"}]",
    );
    let document = decode(colored.as_bytes()).unwrap();
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
        decode(colored.replace("\"red\"", "\"pink\"").as_bytes()).is_err(),
        "only known colors"
    );
}

#[test]
fn collection_and_folder_scripts_need_version_3_and_round_trip() {
    let original = std::fs::read(fixtures().join("native/folder-scripts.imposter.json")).unwrap();
    let mut document = decode(&original).unwrap();
    assert!(document.collection.scripts.before.contains("hmac_sha256"));
    assert!(
        document.collection.folders[0]
            .scripts
            .before
            .contains("X-Folder")
    );
    assert_eq!(document.scripted_count(), 2);
    assert_eq!(
        String::from_utf8(encode(&document)).unwrap(),
        String::from_utf8(original.clone()).unwrap()
    );
    let as_version_2 = String::from_utf8(original)
        .unwrap()
        .replace("\"version\": 3", "\"version\": 2");
    assert_eq!(
        decode(as_version_2.as_bytes()).unwrap_err().to_string(),
        "Collection \"Signed API\": scripts need format version 3"
    );
    document.collection.scripts = imposter_core::model::Scripts::default();
    document.fit_version();
    assert_eq!(
        document.version,
        Version::V3,
        "the folder's scripts still need it"
    );
    document.collection.folders[0].scripts = imposter_core::model::Scripts::default();
    document.fit_version();
    assert_eq!(document.version, Version::V2);

    let original = std::fs::read(fixtures().join("native/folder-scripts.imposter.json")).unwrap();
    let (_, warnings) = imposter_transfer::postman::encode(&decode(&original).unwrap()).unwrap();
    for owner in ["Collection \"Signed API\"", "Folder \"Users\""] {
        assert!(
            warnings.contains(&format!(
                "{owner}: scripts are not exported to Postman, whose scripts are JavaScript"
            )),
            "{warnings:?}"
        );
    }
}
