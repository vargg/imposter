//! Pins what hyper puts on the wire for ordered, duplicate headers (decision in AGENTS.md):
//! values of one name keep their order, names follow their first occurrence, names are
//! lowercase, and empty values are sent.
#![cfg(test)]

mod support;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use support::{OK_RESPONSE, http_once};

#[tokio::test]
async fn same_name_values_keep_order_and_names_follow_first_occurrence() {
    let (url, server) = http_once(OK_RESPONSE).await;
    let mut headers = HeaderMap::new();
    for (name, value) in [("X-A", "1"), ("X-B", "2"), ("X-A", "3"), ("X-Empty", "")] {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(
            poster_http::tls::client_config(&poster_http::tls::TlsSettings::default()).unwrap(),
        )
        .no_proxy()
        .build()
        .unwrap();

    client.get(&url).headers(headers).send().await.unwrap();

    let raw = String::from_utf8(server.await.unwrap()).unwrap();
    let ours: Vec<&str> = raw.lines().filter(|line| line.starts_with("x-")).collect();
    assert_eq!(ours, ["x-a: 1", "x-a: 3", "x-b: 2", "x-empty: "]);
}
