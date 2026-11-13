//! Signing in for the OAuth 2.0 authorization code grant (RFC 6749 4.1, RFC 8252): a
//! PKCE pair (RFC 7636, always S256) and a random `state`, the authorization URL the
//! browser opens, and a one-shot listener on the loopback address that the browser is sent
//! back to with the code. Opening the browser and exchanging the code are the caller's.

use std::time::Duration;

use base64::Engine as _;
use imposter_core::model::OAuth2;
use imposter_core::url::{encode_pairs, parse_pairs};
use sha2::Digest as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::oauth::OAuthError;

/// How long a person has to sign in before the wait ends.
pub const SIGN_IN_TIME: Duration = Duration::from_mins(5);

/// The most of the browser's request read: its request line and headers.
const HEAD_LIMIT: usize = 16 * 1024;

/// What PKCE sends: the challenge with the authorization request, the verifier with the
/// code (RFC 7636 4.1–4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

fn random_text(bytes: usize) -> Result<String, OAuthError> {
    let mut buffer = vec![0_u8; bytes];
    // A guessable verifier or state defeats their purpose: no fallback.
    getrandom::fill(&mut buffer).map_err(|_| {
        OAuthError::Invalid("the system has no randomness to sign in with".to_owned())
    })?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buffer))
}

/// The S256 challenge of `verifier`: base64url of its SHA-256, without padding.
pub fn challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier))
}

/// A new verifier of 32 random bytes (43 characters) and its challenge.
pub fn pkce() -> Result<Pkce, OAuthError> {
    let verifier = random_text(32)?;
    Ok(Pkce {
        challenge: challenge(&verifier),
        verifier,
    })
}

/// A new `state`, which ties the browser's answer to this sign-in.
pub fn state() -> Result<String, OAuthError> {
    random_text(16)
}

/// The URL the browser opens: `oauth.auth_url` with the authorization request added after
/// any query it has.
pub fn authorization_url(oauth: &OAuth2, redirect_uri: &str, state: &str, pkce: &Pkce) -> String {
    let mut pairs = vec![
        ("response_type", "code"),
        ("client_id", oauth.client_id.as_str()),
        ("redirect_uri", redirect_uri),
    ];
    if !oauth.scope.is_empty() {
        pairs.push(("scope", &oauth.scope));
    }
    if !oauth.audience.is_empty() {
        pairs.push(("audience", &oauth.audience));
    }
    pairs.extend([
        ("state", state),
        ("code_challenge", pkce.challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]);
    let separator = if oauth.auth_url.contains('?') {
        '&'
    } else {
        '?'
    };
    format!("{}{separator}{}", oauth.auth_url, encode_pairs(pairs))
}

/// The loopback address the browser comes back to, listening before the browser opens.
pub struct Callback {
    listener: TcpListener,
    port: u16,
}

impl Callback {
    /// Listen on 127.0.0.1, on `port` or any free one.
    pub async fn bind(port: Option<u16>) -> Result<Self, OAuthError> {
        let listener = TcpListener::bind(("127.0.0.1", port.unwrap_or(0)))
            .await
            .map_err(|e| {
                OAuthError::Transport(match port {
                    Some(port) => format!("cannot listen for the sign-in on port {port}: {e}"),
                    None => format!("cannot listen for the sign-in: {e}"),
                })
            })?;
        let port = listener
            .local_addr()
            .map_err(|e| OAuthError::Transport(e.to_string()))?
            .port();
        Ok(Self { listener, port })
    }

    /// Where the authorization server sends the browser back.
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.port)
    }

    /// Wait for the browser to come back to `/callback` with this sign-in's `state`, and
    /// give its code. Other paths (a browser asks for `/favicon.ico`) are answered 404 and
    /// waited past. Ends with `Cancelled`, or `TimedOut` after `within`.
    pub async fn code(
        self,
        state: &str,
        cancel: &CancellationToken,
        within: Duration,
    ) -> Result<String, OAuthError> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let (stream, _) = tokio::select! {
                () = cancel.cancelled() => return Err(OAuthError::Cancelled),
                () = tokio::time::sleep_until(deadline) => return Err(OAuthError::TimedOut),
                accepted = self.listener.accept() => match accepted {
                    Ok(accepted) => accepted,
                    // A connection that failed to arrive: wait for the next.
                    Err(_) => continue,
                },
            };
            if let Some(answer) = answer(stream, state).await {
                return answer;
            }
        }
    }
}

/// Read one request and answer it: `Some` with the outcome for `/callback`, `None` for
/// anything else (answered 404) or a connection that sent nothing usable.
async fn answer(mut stream: TcpStream, state: &str) -> Option<Result<String, OAuthError>> {
    let mut head = Vec::new();
    let mut chunk = [0_u8; 2048];
    let read = tokio::time::timeout(Duration::from_secs(10), async {
        while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < HEAD_LIMIT {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => head.extend_from_slice(&chunk[..n]),
            }
        }
    })
    .await;
    if read.is_err() {
        return None;
    }
    let line = String::from_utf8_lossy(&head);
    let target = line.lines().next()?.split(' ').nth(1)?.to_owned();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    if path != "/callback" {
        reply(&mut stream, "404 Not Found", "Not found.").await;
        return None;
    }
    let pairs = parse_pairs(query);
    let get = |name: &str| {
        pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    let outcome = if let Some(error) = get("error") {
        Err(OAuthError::Refused {
            error,
            description: get("error_description").unwrap_or_default(),
        })
    } else if get("state").as_deref() != Some(state) {
        Err(OAuthError::Mismatch)
    } else {
        get("code")
            .filter(|code| !code.is_empty())
            .ok_or_else(|| OAuthError::Invalid("the sign-in gave no code".to_owned()))
    };
    match &outcome {
        Ok(_) => {
            reply(
                &mut stream,
                "200 OK",
                "Signed in. You can close this tab and go back to Imposter.",
            )
            .await;
        }
        Err(error) => {
            reply(
                &mut stream,
                "400 Bad Request",
                &format!("Sign-in failed: {error}. Go back to Imposter to try again."),
            )
            .await;
        }
    }
    Some(outcome)
}

/// A small page in plain text, so nothing the server sent can run in it.
async fn reply(stream: &mut TcpStream, status: &str, text: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{text}",
        text.len()
    );
    // The browser may have gone; the outcome is known either way.
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7636 appendix B.
    #[test]
    fn the_challenge_is_the_rfc_7636_example() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let pair = pkce().unwrap();
        assert_eq!(pair.verifier.len(), 43);
        assert_eq!(pair.challenge, challenge(&pair.verifier));
        assert_ne!(pkce().unwrap(), pair, "each sign-in has its own");
        assert_ne!(state().unwrap(), state().unwrap());
    }

    #[test]
    fn the_authorization_url_asks_for_a_code_with_pkce() {
        let oauth = OAuth2 {
            auth_url: "https://auth.test/authorize?tenant=a".into(),
            client_id: "my app".into(),
            scope: "openid profile".into(),
            ..OAuth2::default()
        };
        let pkce = Pkce {
            verifier: "v".into(),
            challenge: "c-1".into(),
        };
        assert_eq!(
            authorization_url(&oauth, "http://127.0.0.1:5000/callback", "s-1", &pkce),
            "https://auth.test/authorize?tenant=a&response_type=code&client_id=my+app\
             &redirect_uri=http%3A%2F%2F127.0.0.1%3A5000%2Fcallback&scope=openid+profile\
             &state=s-1&code_challenge=c-1&code_challenge_method=S256"
        );
    }
}
