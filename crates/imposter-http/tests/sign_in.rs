//! The sign-in listener on the loopback address, driven as a browser would: it gives the
//! code of its own sign-in, refuses another's `state`, reports the server's error, waits
//! past other paths, and stops on cancel or when time is up.
#![cfg(test)]

use std::time::Duration;

use imposter_http::oauth::OAuthError;
use imposter_http::sign_in::Callback;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

/// What a browser gets for `target` on the callback's port.
async fn visit(redirect_uri: &str, target: &str) -> String {
    let address = redirect_uri
        .trim_start_matches("http://")
        .trim_end_matches("/callback");
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(format!("GET {target} HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.unwrap();
    answer
}

async fn outcome(target: &'static str) -> (Result<String, OAuthError>, String, String) {
    let callback = Callback::bind(None).await.unwrap();
    let uri = callback.redirect_uri();
    let browser = {
        let uri = uri.clone();
        tokio::spawn(async move {
            let icon = visit(&uri, "/favicon.ico").await;
            let page = visit(&uri, target).await;
            (icon, page)
        })
    };
    let result = callback
        .code("s-1", &CancellationToken::new(), Duration::from_secs(10))
        .await;
    let (icon, page) = browser.await.unwrap();
    (result, icon, page)
}

#[tokio::test]
async fn the_code_of_this_sign_in_is_given_and_the_browser_told() {
    let (result, icon, page) = outcome("/callback?code=c%2B1&state=s-1").await;
    assert_eq!(result, Ok("c+1".to_owned()));
    assert!(icon.starts_with("HTTP/1.1 404"), "{icon}");
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(
        page.contains("text/plain"),
        "nothing in the page runs: {page}"
    );
    assert!(page.ends_with("Signed in. You can close this tab and go back to Imposter."));
}

#[tokio::test]
async fn another_sign_ins_state_is_refused() {
    let (result, _, page) = outcome("/callback?code=c-1&state=other").await;
    assert_eq!(result, Err(OAuthError::Mismatch));
    assert!(page.starts_with("HTTP/1.1 400"), "{page}");
}

#[tokio::test]
async fn the_servers_error_is_reported() {
    let (result, _, page) =
        outcome("/callback?error=access_denied&error_description=User+said+no&state=s-1").await;
    assert_eq!(
        result.unwrap_err().to_string(),
        "access_denied: User said no"
    );
    assert!(
        page.contains("Sign-in failed: access_denied: User said no"),
        "{page}"
    );
}

#[tokio::test]
async fn the_wait_ends_on_cancel_or_when_time_is_up() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let callback = Callback::bind(None).await.unwrap();
    assert_eq!(
        callback.code("s", &cancel, Duration::from_secs(60)).await,
        Err(OAuthError::Cancelled)
    );
    let callback = Callback::bind(None).await.unwrap();
    assert_eq!(
        callback
            .code("s", &CancellationToken::new(), Duration::from_millis(1))
            .await,
        Err(OAuthError::TimedOut)
    );
}

#[tokio::test]
async fn a_chosen_port_is_used_and_a_taken_one_named() {
    let taken = Callback::bind(None).await.unwrap();
    let port: u16 = taken
        .redirect_uri()
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches("/callback")
        .parse()
        .unwrap();
    let error = Callback::bind(Some(port)).await.err().unwrap();
    assert!(
        error
            .to_string()
            .starts_with(&format!("cannot listen for the sign-in on port {port}: ")),
        "{error}"
    );
    assert!(error.to_string().contains("os error"), "{error}");
}
