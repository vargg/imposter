//! Postman conversion against golden cases (`tests/fixtures/golden/postman.json`): the
//! document, warnings and consent each input decodes to, and the collection each document
//! encodes to.
#![cfg(test)]

use std::path::Path;

use imposter_transfer::native::{NativeDocument, TransferCollection, encode as encode_native};
use imposter_transfer::postman::{Decoder, companion, encode};
use serde_json::Value;

fn fixture() -> Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/golden/postman.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// Form bodies hold JSON arrays whose spacing is not part of the format: compare them parsed.
fn normalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let form = matches!(
                map.get("body_type").and_then(Value::as_str),
                Some("urlencoded" | "form-data" | "multipart")
            );
            if form && let Some(Value::String(body)) = map.get("body") {
                let bytes = imposter_core::base64_bytes::decode(body).unwrap();
                let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                map.insert("body".into(), parsed);
            }
            map.values_mut().for_each(normalize);
        }
        Value::Array(items) => items.iter_mut().for_each(normalize),
        _ => {}
    }
}

/// The document as exported, form bodies parsed.
fn document_value(document: &NativeDocument) -> Value {
    let mut value: Value = serde_json::from_slice(&encode_native(document)).unwrap();
    normalize(&mut value);
    value
}

#[test]
fn decoding_gives_the_expected_document_warnings_and_consent() {
    for case in fixture()["decode"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut decoder = Decoder::default();
        let result = decoder.decode(&case["input"]);
        let expected = &case["expected"];
        if expected.get("error").is_some() {
            assert!(
                result.is_err(),
                "{name}: expected a refusal, got {result:?}"
            );
            continue;
        }
        let document = result.unwrap_or_else(|error| panic!("{name}: {error}"));
        let mut wanted = expected["document"].clone();
        normalize(&mut wanted);
        assert_eq!(document_value(&document), wanted, "{name}: document");
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
fn companions_add_their_environments_and_globals() {
    for case in fixture()["companions"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut decoder = Decoder::default();
        let mut document = NativeDocument::new(TransferCollection {
            scripts: imposter_core::model::Scripts::default(),
            name: imposter_transfer::native::Name::new("C").unwrap(),
            description: String::new(),
            variables: imposter_core::model::Variables::new(),
            folders: Vec::new(),
            requests: Vec::new(),
            saved_values: imposter_core::model::SavedValues::default(),
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
fn encoding_gives_the_expected_collection_and_warnings() {
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

/// Imposter generates `$guid`, `$timestamp`, `$randomInt` and the rest of its own, so those
/// Postman dynamic variables stay as they are and need no consent; only the ones it cannot
/// generate are removed, with consent.
#[test]
fn generated_dynamic_variables_are_kept() {
    let input = serde_json::json!({
        "info": {"name": "C", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
        "item": [{"name": "R", "request": {
            "method": "GET",
            "url": "http://h/{{$guid}}?t={{ $timestamp }}",
            "header": [{"key": "X", "value": "{{$randomInt}}"}]
        }}]
    });
    let mut decoder = Decoder::default();
    let document = decoder.decode(&input).unwrap();
    let request = &document.collection.requests[0];
    assert!(
        request.url.starts_with("http://h/{{$guid}}"),
        "{}",
        request.url
    );
    assert_eq!(
        (
            request.queries[0].name.as_str(),
            request.queries[0].value.as_str()
        ),
        ("t", "{{ $timestamp }}")
    );
    assert_eq!(request.headers[0].value, "{{$randomInt}}");
    assert!(decoder.warnings.is_empty(), "{:?}", decoder.warnings);
    assert!(!decoder.requires_removal);

    let mut input = input.clone();
    input["item"][0]["request"]["header"][0]["value"] = "{{$randomFirstName}}".into();
    let mut decoder = Decoder::default();
    let document = decoder.decode(&input).unwrap();
    assert_eq!(document.collection.requests[0].headers[0].value, "");
    assert_eq!(
        decoder.warnings,
        ["C/R: Postman dynamic variables will be removed"]
    );
    assert!(decoder.requires_removal);
}

/// Postman's `apikey` auth comes in as API-key auth and goes back out the same way; a key
/// in a place Imposter cannot send is removed only with consent.
#[test]
fn api_keys_travel_both_ways() {
    use imposter_core::model::{ApiKeyIn, RequestAuth};
    let collection = |auth: Value| {
        serde_json::json!({
            "info": {"name": "C", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
            "item": [{"name": "R", "request": {"method": "GET", "url": "http://h/", "auth": auth}}]
        })
    };
    let mut decoder = Decoder::default();
    let document = decoder
        .decode(&collection(
            serde_json::json!({"type": "apikey", "apikey": [
                {"key": "key", "value": "api_key", "type": "string"},
                {"key": "value", "value": "{{key}}", "type": "string"},
                {"key": "in", "value": "query", "type": "string"},
            ]}),
        ))
        .unwrap();
    assert_eq!(
        document.collection.requests[0].auth,
        RequestAuth::api_key("api_key", "{{key}}", ApiKeyIn::Query)
    );
    assert!(!decoder.requires_removal);

    let (postman, warnings) = encode(&document).unwrap();
    assert_eq!(
        postman["item"][0]["request"]["auth"],
        serde_json::json!({"type": "apikey", "apikey": [
            {"key": "key", "value": "api_key", "type": "string"},
            {"key": "value", "value": "{{key}}", "type": "string"},
            {"key": "in", "value": "query", "type": "string"},
        ]})
    );
    assert!(warnings.is_empty(), "{warnings:?}");

    let mut decoder = Decoder::default();
    let document = decoder
        .decode(&collection(
            serde_json::json!({"type": "apikey", "apikey": [
                {"key": "key", "value": "X-Key"},
                {"key": "value", "value": "v"},
            ]}),
        ))
        .unwrap();
    assert_eq!(
        document.collection.requests[0].auth,
        RequestAuth::api_key("X-Key", "v", ApiKeyIn::Header),
        "Postman sends a key without a place as a header"
    );

    let mut decoder = Decoder::default();
    let document = decoder
        .decode(&collection(
            serde_json::json!({"type": "apikey", "apikey": [
                {"key": "key", "value": "k"},
                {"key": "in", "value": "cookie"},
            ]}),
        ))
        .unwrap();
    assert_eq!(document.collection.requests[0].auth, RequestAuth::default());
    assert!(decoder.requires_removal, "{:?}", decoder.warnings);
}

#[test]
fn oauth2_client_credentials_travel_both_ways_and_unsupported_grants_need_consent() {
    use imposter_core::model::{ClientAuth, OAuth2, RequestAuth};
    let collection = |auth: Value| {
        serde_json::json!({
            "info": {"name": "C", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
            "item": [{"name": "R", "request": {"method": "GET", "url": "http://h/", "auth": auth}}]
        })
    };
    let mut decoder = Decoder::default();
    let document = decoder
        .decode(&collection(
            serde_json::json!({"type": "oauth2", "oauth2": [
                {"key": "grant_type", "value": "client_credentials", "type": "string"},
                {"key": "accessTokenUrl", "value": "{{auth}}/token", "type": "string"},
                {"key": "clientId", "value": "app", "type": "string"},
                {"key": "clientSecret", "value": "{{secret}}", "type": "string"},
                {"key": "scope", "value": "read", "type": "string"},
                {"key": "client_authentication", "value": "body", "type": "string"},
                {"key": "addTokenTo", "value": "header", "type": "string"},
                {"key": "tokenName", "value": "Token 1", "type": "string"},
                {"key": "accessToken", "value": "an-old-token", "type": "string"},
            ]}),
        ))
        .unwrap();
    let expected = RequestAuth::oauth2(OAuth2 {
        token_url: "{{auth}}/token".into(),
        client_id: "app".into(),
        client_secret: "{{secret}}".into(),
        scope: "read".into(),
        client_auth: ClientAuth::Body,
        ..OAuth2::default()
    });
    assert_eq!(document.collection.requests[0].auth, expected);
    assert!(
        !decoder.requires_removal,
        "Postman's cached token and labels change nothing"
    );

    let (postman, warnings) = encode(&document).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut decoder = Decoder::default();
    let back = decoder
        .decode(&collection(postman["item"][0]["request"]["auth"].clone()))
        .unwrap();
    assert_eq!(back.collection.requests[0].auth, expected);

    let mut decoder = Decoder::default();
    let code = decoder
        .decode(&collection(
            serde_json::json!({"type": "oauth2", "oauth2": [
                {"key": "grant_type", "value": "implicit"},
                {"key": "authUrl", "value": "https://auth/authorize"},
            ]}),
        ))
        .unwrap();
    assert_eq!(code.collection.requests[0].auth, RequestAuth::default());
    assert!(
        decoder.requires_removal,
        "an unsupported grant needs consent"
    );
}

#[test]
fn oauth2_authorization_code_keeps_a_loopback_port_and_warns_about_another_redirect() {
    use imposter_core::model::{OAuth2, OAuthGrant, RequestAuth};
    let collection = |auth: Value| {
        serde_json::json!({
            "info": {"name": "C", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
            "item": [{"name": "R", "request": {"method": "GET", "url": "http://h/", "auth": auth}}]
        })
    };
    let code = |redirect: &str| {
        serde_json::json!({"type": "oauth2", "oauth2": [
            {"key": "grant_type", "value": "authorization_code_with_pkce"},
            {"key": "authUrl", "value": "https://auth/authorize"},
            {"key": "accessTokenUrl", "value": "https://auth/token"},
            {"key": "clientId", "value": "desktop"},
            {"key": "redirect_uri", "value": redirect},
            {"key": "challengeAlgorithm", "value": "S256"},
            {"key": "useBrowser", "value": true},
        ]})
    };
    let mut decoder = Decoder::default();
    let document = decoder
        .decode(&collection(code("http://127.0.0.1:53682/callback")))
        .unwrap();
    let expected = RequestAuth::oauth2(OAuth2 {
        grant: OAuthGrant::AuthorizationCode,
        auth_url: "https://auth/authorize".into(),
        token_url: "https://auth/token".into(),
        client_id: "desktop".into(),
        redirect_port: Some(53682),
        ..OAuth2::default()
    });
    assert_eq!(document.collection.requests[0].auth, expected);
    assert!(!decoder.requires_removal);
    assert_eq!(decoder.warnings, [] as [String; 0]);

    let (postman, warnings) = encode(&document).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let back = Decoder::default()
        .decode(&collection(postman["item"][0]["request"]["auth"].clone()))
        .unwrap();
    assert_eq!(back.collection.requests[0].auth, expected);

    let mut decoder = Decoder::default();
    let elsewhere = decoder
        .decode(&collection(code("https://oauth.pstmn.io/v1/callback")))
        .unwrap();
    assert_eq!(
        elsewhere.collection.requests[0]
            .auth
            .oauth2
            .as_ref()
            .unwrap()
            .redirect_port,
        None
    );
    assert!(
        !decoder.requires_removal,
        "sending is unchanged; signing in may need the server set up"
    );
    assert!(
        decoder.warnings[0].contains("the redirect URI https://oauth.pstmn.io/v1/callback"),
        "{:?}",
        decoder.warnings
    );
}
