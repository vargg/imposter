//! OAuth 2.0 token requests (RFC 6749): a form POST to the token endpoint through the same
//! transport, TLS and proxy settings as the request it is for, and its JSON answer read as
//! a token or as the OAuth error it names. Which token to use, and keeping it, is the
//! app's (`imposter_app::oauth`).

use base64::Engine as _;
use imposter_core::model::{ClientAuth, OAuth2};
use imposter_core::prepare::{Prepared, PreparedAuth, PreparedBody};
use imposter_core::url::{encode_pairs, quote_plus};
use tokio_util::sync::CancellationToken;

use crate::{Policy, SendError};

/// The most of a token endpoint's answer kept: tokens are small, and a page of HTML in its
/// place should not be read whole.
const ANSWER_LIMIT: u64 = 256 * 1024;

/// What the token endpoint gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub access_token: String,
    /// `Bearer` when the server says nothing else.
    pub token_type: String,
    /// Seconds the token is valid for, when the server says.
    pub expires_in: Option<u64>,
    pub refresh_token: Option<String>,
}

/// What to ask the token endpoint for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant<'a> {
    /// A token for the client itself (RFC 6749 4.4).
    ClientCredentials,
    /// A new token for the refresh token an earlier answer gave (RFC 6749 6).
    Refresh(&'a str),
    /// The token for the code a sign-in gave (RFC 6749 4.1.3), with the PKCE verifier.
    AuthorizationCode {
        code: &'a str,
        redirect_uri: &'a str,
        verifier: &'a str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OAuthError {
    /// The endpoint answered with an OAuth error (`invalid_client`, `invalid_scope`…).
    #[error("{error}{}", if .description.is_empty() { String::new() } else { format!(": {}", .description) })]
    Refused { error: String, description: String },
    #[error("the token endpoint answered {0} without saying why")]
    Status(u16),
    #[error("the token endpoint's answer is not a token: {0}")]
    Invalid(String),
    #[error("{0}")]
    Transport(String),
    #[error("cancelled")]
    Cancelled,
    /// The browser came back with another sign-in's `state`: not this one's answer.
    #[error("the sign-in answer did not match this sign-in")]
    Mismatch,
    #[error("no sign-in within {} minutes", crate::sign_in::SIGN_IN_TIME.as_secs() / 60)]
    TimedOut,
}

/// The token request for `grant`, `oauth`'s values already resolved: the client's
/// credentials in a Basic header (each form-encoded first, as RFC 6749 2.3.1 says) or in
/// the form.
pub fn prepare(oauth: &OAuth2, grant: Grant<'_>) -> Prepared {
    let mut form: Vec<(&str, &str)> = Vec::new();
    match grant {
        Grant::ClientCredentials => form.push(("grant_type", "client_credentials")),
        Grant::Refresh(token) => {
            form.push(("grant_type", "refresh_token"));
            form.push(("refresh_token", token));
        }
        Grant::AuthorizationCode {
            code,
            redirect_uri,
            verifier,
        } => {
            form.push(("grant_type", "authorization_code"));
            form.push(("code", code));
            form.push(("redirect_uri", redirect_uri));
            form.push(("code_verifier", verifier));
        }
    }
    // A code was already issued for its scope.
    if !oauth.scope.is_empty() && !matches!(grant, Grant::AuthorizationCode { .. }) {
        form.push(("scope", &oauth.scope));
    }
    if !oauth.audience.is_empty() && grant == Grant::ClientCredentials {
        form.push(("audience", &oauth.audience));
    }
    let mut headers = vec![
        (
            "Content-Type".to_owned(),
            "application/x-www-form-urlencoded".to_owned(),
        ),
        ("Accept".to_owned(), "application/json".to_owned()),
    ];
    match oauth.client_auth {
        // A public client (an app with PKCE and no secret) only names itself.
        _ if oauth.client_secret.is_empty() => form.push(("client_id", &oauth.client_id)),
        ClientAuth::Basic => {
            let pair = format!(
                "{}:{}",
                quote_plus(&oauth.client_id),
                quote_plus(&oauth.client_secret)
            );
            let encoded = base64::engine::general_purpose::STANDARD.encode(pair);
            headers.push(("Authorization".to_owned(), format!("Basic {encoded}")));
        }
        ClientAuth::Body => {
            form.push(("client_id", &oauth.client_id));
            form.push(("client_secret", &oauth.client_secret));
        }
    }
    Prepared {
        method: "POST".to_owned(),
        url: oauth.token_url.clone(),
        headers,
        body: PreparedBody::Bytes(encode_pairs(form).into_bytes()),
        auth: PreparedAuth::None,
    }
}

/// Ask the token endpoint for a token, with `policy`'s timeout, TLS and proxy, but no
/// cookie jar: the endpoint is not the API.
pub async fn request(
    oauth: &OAuth2,
    grant: Grant<'_>,
    policy: &Policy,
    cancel: &CancellationToken,
) -> Result<Token, OAuthError> {
    let policy = Policy {
        jar: None,
        capture_limit: Some(ANSWER_LIMIT),
        ..policy.clone()
    };
    let response = match crate::send(&prepare(oauth, grant), &policy, cancel).await {
        Ok(response) => response,
        Err(SendError::Cancelled) => return Err(OAuthError::Cancelled),
        Err(SendError::Failed { message, .. }) => return Err(OAuthError::Transport(message)),
    };
    read_answer(response.status, &response.body)
}

/// A token endpoint's answer: a token on success, else the OAuth error it names.
pub fn read_answer(status: u16, body: &[u8]) -> Result<Token, OAuthError> {
    let json: Option<serde_json::Map<String, serde_json::Value>> =
        serde_json::from_slice(body).ok();
    let text = |json: &serde_json::Map<String, serde_json::Value>, key: &str| {
        json.get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    if !(200..300).contains(&status) {
        return Err(
            match json.as_ref().and_then(|j| text(j, "error").map(|e| (j, e))) {
                Some((json, error)) => OAuthError::Refused {
                    error,
                    description: text(json, "error_description").unwrap_or_default(),
                },
                None => OAuthError::Status(status),
            },
        );
    }
    let json = json.ok_or_else(|| OAuthError::Invalid("it is not a JSON object".to_owned()))?;
    let access_token = text(&json, "access_token")
        .filter(|token| !token.is_empty())
        .ok_or_else(|| OAuthError::Invalid("it has no access_token".to_owned()))?;
    // Some servers send the lifetime as a string.
    let expires_in = json.get("expires_in").and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
    });
    Ok(Token {
        access_token,
        token_type: text(&json, "token_type").unwrap_or_else(|| "Bearer".to_owned()),
        expires_in,
        refresh_token: text(&json, "refresh_token").filter(|t| !t.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oauth(client_auth: ClientAuth) -> OAuth2 {
        OAuth2 {
            token_url: "https://auth.test/token".into(),
            client_id: "my app".into(),
            client_secret: "s:cr&t".into(),
            scope: "read write".into(),
            audience: "https://api.test".into(),
            client_auth,
            ..OAuth2::default()
        }
    }

    fn body(prepared: &Prepared) -> String {
        match &prepared.body {
            PreparedBody::Bytes(bytes) => String::from_utf8(bytes.clone()).unwrap(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn client_credentials_go_in_a_form_encoded_basic_header_or_the_form() {
        let basic = prepare(&oauth(ClientAuth::Basic), Grant::ClientCredentials);
        assert_eq!(
            body(&basic),
            "grant_type=client_credentials&scope=read+write&audience=https%3A%2F%2Fapi.test"
        );
        let header = basic
            .headers
            .iter()
            .find(|(n, _)| n == "Authorization")
            .unwrap();
        // base64 of "my+app:s%3Acr%26t"
        assert_eq!(header.1, "Basic bXkrYXBwOnMlM0FjciUyNnQ=");

        let in_body = prepare(&oauth(ClientAuth::Body), Grant::Refresh("r-1"));
        assert_eq!(
            body(&in_body),
            "grant_type=refresh_token&refresh_token=r-1&scope=read+write&client_id=my+app&client_secret=s%3Acr%26t"
        );
        assert!(
            !in_body.headers.iter().any(|(n, _)| n == "Authorization"),
            "{:?}",
            in_body.headers
        );
    }

    #[test]
    fn a_code_is_exchanged_with_its_verifier_and_a_public_client_only_names_itself() {
        let public = OAuth2 {
            client_secret: String::new(),
            ..oauth(ClientAuth::Basic)
        };
        let exchange = prepare(
            &public,
            Grant::AuthorizationCode {
                code: "c-1",
                redirect_uri: "http://127.0.0.1:5000/callback",
                verifier: "v-1",
            },
        );
        assert_eq!(
            body(&exchange),
            "grant_type=authorization_code&code=c-1\
             &redirect_uri=http%3A%2F%2F127.0.0.1%3A5000%2Fcallback&code_verifier=v-1\
             &client_id=my+app"
        );
        assert!(
            !exchange.headers.iter().any(|(n, _)| n == "Authorization"),
            "{:?}",
            exchange.headers
        );
    }

    #[test]
    fn answers_are_tokens_or_the_oauth_error_they_name() {
        assert_eq!(
            read_answer(
                200,
                br#"{"access_token": "a-1", "token_type": "bearer", "expires_in": "3600", "refresh_token": "r-1"}"#
            ),
            Ok(Token {
                access_token: "a-1".into(),
                token_type: "bearer".into(),
                expires_in: Some(3600),
                refresh_token: Some("r-1".into()),
            })
        );
        assert_eq!(
            read_answer(200, br#"{"access_token": "a-1"}"#).map(|t| (t.token_type, t.expires_in)),
            Ok(("Bearer".to_owned(), None))
        );
        assert_eq!(
            read_answer(
                401,
                br#"{"error": "invalid_client", "error_description": "Unknown client"}"#
            )
            .unwrap_err()
            .to_string(),
            "invalid_client: Unknown client"
        );
        assert_eq!(read_answer(500, b"<html>"), Err(OAuthError::Status(500)));
        assert!(matches!(
            read_answer(200, br#"{"token": "x"}"#),
            Err(OAuthError::Invalid(_))
        ));
        assert!(matches!(
            read_answer(200, b"not json"),
            Err(OAuthError::Invalid(_))
        ));
    }
}
