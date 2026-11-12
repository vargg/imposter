//! OAuth 2.0 tokens for requests: one cached per what it is for (token endpoint, client,
//! scope, audience and grant, never the secret), used while it is good for at least
//! [`MARGIN`] more, refreshed when it has a refresh token, else fetched anew. Send reads the
//! stored cache (`store::oauth`) and keeps what it fetched; a run shares one cache of its
//! own, never stored.

use std::sync::{Arc, Mutex, PoisonError};

use imposter_core::model::{OAuth2, OAuthGrant};
use imposter_http::Policy;
use imposter_http::oauth::{self, Grant, OAuthError, Token};
use imposter_store::oauth::StoredToken;
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;

/// A token about to expire is not used: the request might reach the server after it did.
pub const MARGIN: std::time::Duration = std::time::Duration::from_secs(30);

/// The cache key of `oauth`'s token, its values resolved: what the token is for. The secret
/// is left out, so the key names nothing secret.
pub fn key(oauth: &OAuth2) -> String {
    let grant = match oauth.grant {
        OAuthGrant::ClientCredentials => "client_credentials",
    };
    let mut hash = Sha256::new();
    for part in [
        grant,
        &oauth.token_url,
        &oauth.client_id,
        &oauth.scope,
        &oauth.audience,
    ] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    hash.finalize()
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            use std::fmt::Write as _;
            // Writing to a String cannot fail.
            let _ = write!(text, "{byte:02x}");
            text
        })
}

/// Whether `token` may still be sent at `now` (milliseconds since the epoch). A token
/// without a lifetime is good until it is cleared.
pub fn good(token: &StoredToken, now: i64) -> bool {
    let margin = i64::try_from(MARGIN.as_millis()).unwrap_or(i64::MAX);
    token
        .expires_at
        .is_none_or(|at| at > now.saturating_add(margin))
}

/// The tokens one execution or run may use and add to.
#[derive(Clone, Default)]
pub struct SharedTokens(Arc<Mutex<Vec<StoredToken>>>);

impl SharedTokens {
    pub fn new(tokens: Vec<StoredToken>) -> Self {
        Self(Arc::new(Mutex::new(tokens)))
    }

    fn with<T>(&self, work: impl FnOnce(&mut Vec<StoredToken>) -> T) -> T {
        work(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub fn get(&self, key: &str) -> Option<StoredToken> {
        self.with(|tokens| tokens.iter().find(|t| t.key == key).cloned())
    }

    fn put(&self, token: StoredToken) {
        self.with(|tokens| {
            tokens.retain(|t| t.key != token.key);
            tokens.push(token);
        });
    }
}

/// The same cache, not an equal one, as with cookie jars.
impl PartialEq for SharedTokens {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for SharedTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Tokens stay out of debug output, which can reach logs.
        write!(
            f,
            "SharedTokens({} tokens)",
            self.with(|tokens| tokens.len())
        )
    }
}

/// A token to send, and whether it was fetched now (and so is new to keep).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obtained {
    pub token: StoredToken,
    pub fetched: bool,
}

fn stored(key: String, token: Token, now: i64) -> StoredToken {
    StoredToken {
        key,
        access: token.access_token,
        refresh: token.refresh_token,
        token_type: token.token_type,
        expires_at: token.expires_in.map(|seconds| {
            now.saturating_add(
                i64::try_from(seconds)
                    .unwrap_or(i64::MAX)
                    .saturating_mul(1000),
            )
        }),
    }
}

/// The token to send for `oauth` (values resolved): the cached one while good, else a
/// refreshed one, else a new one. A refresh that fails falls back to a new token, which the
/// client credentials grant can always ask for. What is fetched joins `tokens`.
pub async fn token(
    oauth: &OAuth2,
    tokens: &SharedTokens,
    policy: &Policy,
    cancel: &CancellationToken,
    now: i64,
) -> Result<Obtained, OAuthError> {
    let key = key(oauth);
    let cached = tokens.get(&key);
    if let Some(token) = cached.as_ref().filter(|t| good(t, now)) {
        return Ok(Obtained {
            token: token.clone(),
            fetched: false,
        });
    }
    let refreshed = match cached.and_then(|t| t.refresh) {
        Some(refresh) => {
            match oauth::request(oauth, Grant::Refresh(&refresh), policy, cancel).await {
                Ok(token) => Some(token),
                Err(OAuthError::Cancelled) => return Err(OAuthError::Cancelled),
                Err(error) => {
                    tracing::info!(%error, "refreshing an OAuth token failed; asking for a new one");
                    None
                }
            }
        }
        None => None,
    };
    let token = match refreshed {
        Some(token) => token,
        None => oauth::request(oauth, Grant::ClientCredentials, policy, cancel).await?,
    };
    let token = stored(key, token, now);
    tokens.put(token.clone());
    Ok(Obtained {
        token,
        fetched: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_names_what_the_token_is_for_and_not_the_secret() {
        let oauth = OAuth2 {
            token_url: "https://auth.test/token".into(),
            client_id: "app".into(),
            client_secret: "one".into(),
            ..OAuth2::default()
        };
        let other_secret = OAuth2 {
            client_secret: "two".into(),
            ..oauth.clone()
        };
        let other_scope = OAuth2 {
            scope: "admin".into(),
            ..oauth.clone()
        };
        assert_eq!(key(&oauth), key(&other_secret));
        assert_ne!(key(&oauth), key(&other_scope));
        assert_eq!(key(&oauth).len(), 64);
    }

    #[test]
    fn a_token_is_good_until_thirty_seconds_before_it_expires() {
        let token = |expires_at| StoredToken {
            key: "k".into(),
            access: "a".into(),
            refresh: None,
            token_type: "Bearer".into(),
            expires_at,
        };
        assert!(good(&token(Some(100_000)), 69_999));
        assert!(!good(&token(Some(100_000)), 70_000));
        assert!(good(&token(None), i64::MAX));
    }

    #[test]
    fn shared_tokens_replace_by_key_and_hide_values_from_debug() {
        let tokens = SharedTokens::default();
        let token = |access: &str| StoredToken {
            key: "k".into(),
            access: access.into(),
            refresh: None,
            token_type: "Bearer".into(),
            expires_at: None,
        };
        tokens.put(token("first"));
        tokens.put(token("second"));
        assert_eq!(tokens.get("k").map(|t| t.access), Some("second".into()));
        assert_eq!(format!("{tokens:?}"), "SharedTokens(1 tokens)");
        assert_eq!(tokens.clone(), tokens);
        assert_ne!(tokens, SharedTokens::default());
    }
}

/// Where the open request's token stands, for the Auth tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    /// None cached for what the request asks for.
    Missing,
    /// Good to send; until when (milliseconds since the epoch), if the server said.
    Good { expires_at: Option<i64> },
    /// Too old to send; the next send refreshes it when it can, else gets a new one.
    Expired { refreshable: bool },
}

/// The OAuth 2.0 settings `request` would use now, resolved as Send resolves them (before
/// any script), with the inputs they came from. `None` when OAuth 2.0 is not in effect.
async fn resolved(
    store: &imposter_store::StoreHandle,
    request: &imposter_core::model::Request,
) -> Result<Option<(OAuth2, crate::executor::Inputs, Policy)>, crate::executor::ExecuteError> {
    let inputs = crate::executor::inputs(store, request).await?;
    let (resolved, settings, _) = crate::executor::resolve_with(
        request,
        &inputs,
        &mut crate::executor::SystemEntropy::default(),
    )?;
    let Ok(Some(imposter_core::model::Auth::OAuth2(oauth))) = resolved.template.auth.active()
    else {
        return Ok(None);
    };
    Ok(Some((
        oauth.clone(),
        inputs,
        Policy::from_settings(&settings),
    )))
}

/// Where `request`'s token stands at `now` (milliseconds since the epoch).
pub async fn state(
    store: &imposter_store::StoreHandle,
    request: &imposter_core::model::Request,
    now: i64,
) -> Result<TokenState, crate::executor::ExecuteError> {
    let Some((oauth, inputs, _)) = resolved(store, request).await? else {
        return Ok(TokenState::Missing);
    };
    let cached = inputs.tokens.and_then(|tokens| tokens.get(&key(&oauth)));
    Ok(match cached {
        None => TokenState::Missing,
        Some(token) if good(&token, now) => TokenState::Good {
            expires_at: token.expires_at,
        },
        Some(token) => TokenState::Expired {
            refreshable: token.refresh.is_some(),
        },
    })
}

/// Ask for a new token for `request` now and keep it, as Send would; the cached one, if
/// any, is replaced. Scripts do not run: the token is for the settings as they stand.
pub async fn fetch(
    store: &imposter_store::StoreHandle,
    request: &imposter_core::model::Request,
    cancel: &CancellationToken,
    now: i64,
) -> Result<TokenState, crate::executor::ExecuteError> {
    use crate::executor::ExecuteError;
    let Some((oauth, inputs, policy)) = resolved(store, request).await? else {
        return Ok(TokenState::Missing);
    };
    let token = oauth::request(&oauth, Grant::ClientCredentials, &policy, cancel)
        .await
        .map_err(|error| ExecuteError::Token(format!("Token request failed: {error}")))?;
    let token = stored(key(&oauth), token, now);
    let expires_at = token.expires_at;
    let environment = inputs.context.environment_id();
    store
        .call(move |db| imposter_store::oauth::put(db, environment, &token))
        .await?;
    Ok(TokenState::Good { expires_at })
}

/// Forget `request`'s cached token; the next send gets a new one.
pub async fn clear(
    store: &imposter_store::StoreHandle,
    request: &imposter_core::model::Request,
) -> Result<(), crate::executor::ExecuteError> {
    let Some((oauth, inputs, _)) = resolved(store, request).await? else {
        return Ok(());
    };
    let (key, environment) = (key(&oauth), inputs.context.environment_id());
    store
        .call(move |db| imposter_store::oauth::remove(db, environment, &key))
        .await?;
    Ok(())
}
