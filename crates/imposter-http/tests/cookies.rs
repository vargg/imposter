//! The cookie jar on the wire: what each hop's response sets is kept, and the jar's cookies
//! for each hop's URL arrive in its `Cookie` header, checked in the bytes received.
#![cfg(test)]

use std::time::Duration;

use imposter_core::execution::SetCookie;
use imposter_core::prepare::{Prepared, PreparedAuth, PreparedBody};
use imposter_http::cookies::{Jar, SharedJar};
use imposter_http::tls::TlsSettings;
use imposter_http::{Policy, send};
use imposter_testkit::{Server, response};
use tokio_util::sync::CancellationToken;

fn policy(jar: Option<SharedJar>, send_jar: bool) -> Policy {
    Policy {
        timeout: Duration::from_secs(5),
        follow_redirects: true,
        proxy: None,
        tls: TlsSettings {
            verify: true,
            ..TlsSettings::default()
        },
        identity_routing: None,
        destinations: None,
        capture_limit: None,
        jar,
        send_jar,
    }
}

fn get(url: &str, headers: &[(&str, &str)]) -> Prepared {
    Prepared {
        method: "GET".to_owned(),
        url: url.to_owned(),
        headers: headers
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect(),
        body: PreparedBody::Bytes(Vec::new()),
        auth: PreparedAuth::None,
    }
}

async fn ok(prepared: &Prepared, policy: &Policy) -> imposter_core::execution::Response {
    send(prepared, policy, &CancellationToken::new())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_cookie_set_on_a_redirect_reaches_the_target_and_is_listed_per_hop() {
    let server = Server::start(|index, _| match index {
        0 => response(
            "302 Found",
            &[("Location", "/home"), ("Set-Cookie", "session=abc; Path=/")],
            b"",
        ),
        _ => response("200 OK", &[("Set-Cookie", "seen=1")], b"home"),
    })
    .await;
    let jar = SharedJar::new(Jar::default());

    let answered = ok(
        &get(&format!("{}/login", server.url), &[]),
        &policy(Some(jar.clone()), true),
    )
    .await;

    let [login, home] = server.requests().try_into().unwrap();
    assert_eq!(login.header("cookie"), None);
    assert_eq!(home.header("cookie"), Some("session=abc"));
    assert_eq!(
        answered.set_cookies,
        [
            SetCookie {
                url: format!("{}/login", server.url),
                header: "session=abc; Path=/".into(),
            },
            SetCookie {
                url: format!("{}/home", server.url),
                header: "seen=1".into(),
            },
        ]
    );
    assert_eq!(jar.snapshot().len(), 2);
}

#[tokio::test]
async fn the_requests_own_cookies_win_and_the_jar_adds_the_rest() {
    let server = Server::start(|index, _| match index {
        0 => response("302 Found", &[("Location", "/next")], b""),
        _ => response("200 OK", &[], b""),
    })
    .await;
    let mut stored = Jar::default();
    stored.receive(&server.url, "session=from-jar");
    stored.receive(&server.url, "theme=dark");
    let jar = SharedJar::new(stored);

    ok(
        &get(&server.url, &[("Cookie", "session=mine")]),
        &policy(Some(jar), true),
    )
    .await;

    let [first, next] = server.requests().try_into().unwrap();
    assert_eq!(first.header("cookie"), Some("session=mine; theme=dark"));
    assert_eq!(
        next.header("cookie"),
        Some("session=from-jar; theme=dark"),
        "the request's own cookie was for the first location; the jar's go to the next"
    );
}

#[tokio::test]
async fn a_jar_that_does_not_send_still_keeps_and_no_jar_keeps_nothing() {
    let server = Server::start(|_, _| response("200 OK", &[("Set-Cookie", "new=1")], b"")).await;
    let mut stored = Jar::default();
    stored.receive(&server.url, "old=1");
    let jar = SharedJar::new(stored);

    let kept = ok(&get(&server.url, &[]), &policy(Some(jar.clone()), false)).await;
    let none = ok(&get(&server.url, &[]), &policy(None, true)).await;

    let [first, second] = server.requests().try_into().unwrap();
    assert_eq!(first.header("cookie"), None);
    assert_eq!(second.header("cookie"), None);
    assert_eq!(kept.set_cookies.len(), 1);
    assert_eq!(jar.snapshot().len(), 2);
    assert_eq!(none.set_cookies, []);
}

#[tokio::test]
async fn cookies_stay_with_their_host_and_loopback_counts_as_secure() {
    let target = Server::start(|_, _| response("200 OK", &[], b"")).await;
    let location = format!("http://localhost:{}/landing", target.port);
    let origin = Server::start(move |_, _| {
        response(
            "302 Found",
            &[("Location", &location), ("Set-Cookie", "origin=1")],
            b"",
        )
    })
    .await;
    let mut stored = Jar::default();
    stored.receive(
        &format!("https://localhost:{}/", target.port),
        "secure=1; Secure",
    );
    stored.receive(&format!("http://localhost:{}/", target.port), "plain=1");
    let jar = SharedJar::new(stored);

    ok(&get(&origin.url, &[]), &policy(Some(jar), true)).await;

    let landed = target.requests().remove(0);
    assert_eq!(
        landed.header("cookie"),
        Some("plain=1; secure=1"),
        "127.0.0.1's cookie is not localhost's; localhost is secure, as browsers treat it \
         (plain http elsewhere is not: the jar's unit tests)"
    );
}
