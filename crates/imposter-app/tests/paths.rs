//! Relative file paths in requests are found in the collections folder: a binary body and
//! a multipart file reach the wire from there, whatever the working directory, and Copy as
//! curl shows the path joined. Absolute and empty paths stay as they are.
#![cfg(test)]

use std::path::Path;

use imposter_app::executor::{curl_command, execute, root_settings, root_template};
use imposter_core::model::{BodyType, FormItem, Request, RequestTemplate, Variables};
use imposter_core::settings::{AppSettings, HostIdentity};
use imposter_store::{Store, collections};
use imposter_testkit::{Server, response};
use tokio_util::sync::CancellationToken;

async fn saved(store: &Store, template: RequestTemplate) -> Request {
    store
        .call(move |db| {
            let api = collections::create_collection(db, "API", "", &Variables::new())?;
            collections::create_request(db, api.id, None, &template)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn a_relative_upload_is_read_from_the_collections_folder() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    let fixtures = dir.path().join("collections/fixtures");
    std::fs::create_dir_all(&fixtures).unwrap();
    std::fs::write(fixtures.join("data.bin"), b"binary-from-the-folder").unwrap();
    std::fs::write(fixtures.join("avatar.txt"), b"avatar-from-the-folder").unwrap();
    let server = Server::start(|_, _| response("200 OK", &[], b"")).await;

    let binary = saved(
        &store,
        RequestTemplate {
            name: "Upload".into(),
            method: "POST".into(),
            url: format!("{}/upload", server.url),
            body_type: BodyType::Binary,
            body: b"fixtures/data.bin".to_vec(),
            ..RequestTemplate::default()
        },
    )
    .await;
    let executed = execute(&store.handle(), binary.clone(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        executed.response.status, 200,
        "{:?}",
        executed.response.reason
    );
    assert_eq!(server.requests()[0].body, b"binary-from-the-folder");
    let curl = curl_command(&store.handle(), &binary).await.unwrap();
    let joined = fixtures.join("data.bin").display().to_string();
    assert!(curl.contains(&joined), "{curl}");

    let mut form = RequestTemplate {
        name: "Form".into(),
        method: "POST".into(),
        url: format!("{}/form", server.url),
        body_type: BodyType::Multipart,
        ..RequestTemplate::default()
    };
    form.set_multipart_fields(&[FormItem::file("avatar", "fixtures/avatar.txt")]);
    let form = store
        .call(move |db| {
            let api = collections::collections(db)?[0].id;
            collections::create_request(db, api, None, &form)
        })
        .await
        .unwrap();
    execute(&store.handle(), form, CancellationToken::new())
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&server.requests()[1].body).into_owned();
    assert!(body.contains("avatar-from-the-folder"), "{body}");
}

#[test]
fn only_relative_paths_are_joined() {
    let base = Path::new("work").join("api");
    let base = base.as_path();
    // Absolute on every OS, a drive letter included on Windows.
    let absolute = std::env::temp_dir().join("data.bin").display().to_string();
    let mut template = RequestTemplate {
        body_type: BodyType::Binary,
        body: absolute.clone().into_bytes(),
        ..RequestTemplate::default()
    };
    root_template(&mut template, base);
    assert_eq!(template.body, absolute.as_bytes());
    template.body = Vec::new();
    root_template(&mut template, base);
    assert!(template.body.is_empty(), "no path, nothing joined");

    let mut settings = AppSettings {
        ca_bundle: "certs/ca.pem".into(),
        certificate_path: String::new(),
        key_file: absolute.clone(),
        host_identities: vec![HostIdentity {
            host: "api.example.com".into(),
            port: 443,
            certificate_path: "certs/client.pem".into(),
            key_file: String::new(),
            password: String::new(),
        }],
        ..AppSettings::default()
    };
    root_settings(&mut settings, base);
    assert_eq!(
        settings.ca_bundle,
        base.join("certs/ca.pem").display().to_string()
    );
    assert_eq!(settings.certificate_path, "");
    assert_eq!(settings.key_file, absolute);
    assert_eq!(
        settings.host_identities[0].certificate_path,
        base.join("certs/client.pem").display().to_string()
    );
}
