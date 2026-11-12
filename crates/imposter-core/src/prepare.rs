//! Turn a resolved template into exactly what goes on the wire, validating before sending.

use base64::Engine as _;

use crate::model::{
    ApiKeyIn, Auth, BodyType, Credentials, FormBodyError, FormItem, MissingCredentials, ParamItem,
    RequestTemplate, is_method_token,
};
use crate::url::{self, UrlParts};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedBody {
    Bytes(Vec<u8>),
    /// Enabled, named fields; the transport streams them and sets the multipart content type.
    Multipart(Vec<FormItem>),
    /// A file streamed as the whole body.
    File(String),
}

/// Authentication left for the transport: Digest needs the server's challenge first.
/// Basic, Bearer and an API key are already in the headers or the URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedAuth {
    None,
    Digest(Credentials),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub method: String,
    pub url: String,
    /// In order, duplicates and empty values kept.
    pub headers: Vec<(String, String)>,
    pub body: PreparedBody,
    pub auth: PreparedAuth,
}

#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error("Enter an absolute HTTP or HTTPS URL")]
    InvalidUrl,
    #[error("Request method must be a nonempty HTTP token")]
    InvalidMethod,
    #[error("Use either Authentication or an Authorization header")]
    AuthConflict,
    /// The place named: "header" or "query parameter".
    #[error("Use either the API key or a {0} of the same name")]
    ApiKeyConflict(&'static str),
    #[error("Use either Cookies or a Cookie header")]
    CookieConflict,
    /// OAuth 2.0 is sent as the token it gets, which the executor puts in place first.
    #[error("OAuth 2.0 needs a token: send the request, or get one in the Auth tab")]
    NoToken,
    #[error(transparent)]
    Credentials(#[from] MissingCredentials),
    #[error(transparent)]
    FormBody(#[from] FormBodyError),
    #[error("The binary body path is not valid UTF-8")]
    BinaryPath,
}

fn sent(items: &[ParamItem]) -> impl Iterator<Item = &ParamItem> {
    items.iter().filter(|item| item.is_sent())
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(header, _)| header.eq_ignore_ascii_case(name))
}

/// The request URL: scheme, host and path from the URL field; the query from the enabled
/// rows, or from the URL itself when there are no rows at all. The fragment is never sent.
pub fn request_url(template: &RequestTemplate) -> Result<String, PrepareError> {
    let parts = url::split(&template.url);
    if !matches!(parts.scheme.as_str(), "http" | "https") || parts.netloc.is_empty() {
        return Err(PrepareError::InvalidUrl);
    }
    let query = if template.queries.is_empty() {
        let pairs = url::parse_pairs(&parts.query);
        url::encode_pairs(
            pairs
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )
    } else {
        url::encode_pairs(
            sent(&template.queries).map(|item| (item.name.as_str(), item.value.as_str())),
        )
    };
    Ok(url::join(&UrlParts {
        query,
        fragment: String::new(),
        ..parts
    }))
}

/// `url` with `name=value` added after its query, which is left as it was.
fn with_pair(url: &str, name: &str, value: &str) -> String {
    let parts = url::split(url);
    let pair = url::encode_pairs([(name, value)]);
    let query = if parts.query.is_empty() {
        pair
    } else {
        format!("{}&{pair}", parts.query)
    };
    url::join(&UrlParts { query, ..parts })
}

pub fn prepare(template: &RequestTemplate, attach_cookies: bool) -> Result<Prepared, PrepareError> {
    let mut url = request_url(template)?;
    let mut headers: Vec<(String, String)> = sent(&template.headers)
        .map(|item| (item.name.clone(), item.value.clone()))
        .collect();
    // An API key names its own header or parameter; only that one can conflict.
    if let Ok(Some(Auth::ApiKey(key))) = template.auth.active() {
        let taken = match key.location {
            ApiKeyIn::Header => has_header(&headers, &key.name),
            ApiKeyIn::Query => url::parse_pairs(&url::split(&url).query)
                .iter()
                .any(|(name, _)| *name == key.name),
        };
        if taken {
            return Err(PrepareError::ApiKeyConflict(match key.location {
                ApiKeyIn::Header => "header",
                ApiKeyIn::Query => "query parameter",
            }));
        }
    } else if template.auth.kind.is_some() && has_header(&headers, "authorization") {
        return Err(PrepareError::AuthConflict);
    }
    if attach_cookies && !template.cookies.is_empty() {
        if has_header(&headers, "cookie") {
            return Err(PrepareError::CookieConflict);
        }
        let cookie = sent(&template.cookies)
            .map(|item| format!("{}={}", item.name, item.value))
            .collect::<Vec<_>>()
            .join("; ");
        // No empty `Cookie:` header when every cookie is disabled.
        if !cookie.is_empty() {
            headers.push(("Cookie".to_owned(), cookie));
        }
    }

    let (body, content_type) = match template.body_type {
        BodyType::None => (PreparedBody::Bytes(Vec::new()), None),
        BodyType::FormData | BodyType::Urlencoded => {
            let fields = template.form_fields()?;
            let encoded =
                url::encode_pairs(sent(&fields).map(|f| (f.name.as_str(), f.value.as_str())));
            (
                PreparedBody::Bytes(encoded.into_bytes()),
                Some("application/x-www-form-urlencoded"),
            )
        }
        BodyType::Json => (
            PreparedBody::Bytes(template.body.clone()),
            Some("application/json"),
        ),
        BodyType::Plain | BodyType::Text => (
            PreparedBody::Bytes(template.body.clone()),
            Some("text/plain"),
        ),
        BodyType::Multipart => {
            let fields = template
                .multipart_fields()?
                .into_iter()
                .filter(FormItem::is_sent)
                .collect();
            (PreparedBody::Multipart(fields), None)
        }
        BodyType::Binary => {
            let path =
                String::from_utf8(template.body.clone()).map_err(|_| PrepareError::BinaryPath)?;
            (PreparedBody::File(path), None)
        }
    };
    if let Some(content_type) = content_type
        && !has_header(&headers, "content-type")
    {
        headers.push(("Content-Type".to_owned(), content_type.to_owned()));
    }

    let auth = match template.auth.active()? {
        None => PreparedAuth::None,
        Some(Auth::Basic(credentials)) => {
            let token = base64::engine::general_purpose::STANDARD
                .encode(format!("{}:{}", credentials.username, credentials.password));
            headers.push(("Authorization".to_owned(), format!("Basic {token}")));
            PreparedAuth::None
        }
        Some(Auth::Bearer(bearer)) => {
            headers.push((
                "Authorization".to_owned(),
                format!("Bearer {}", bearer.token),
            ));
            PreparedAuth::None
        }
        Some(Auth::Digest(credentials)) => PreparedAuth::Digest(credentials.clone()),
        Some(Auth::ApiKey(key)) => {
            match key.location {
                ApiKeyIn::Header => headers.push((key.name.clone(), key.value.clone())),
                ApiKeyIn::Query => url = with_pair(&url, &key.name, &key.value),
            }
            PreparedAuth::None
        }
        Some(Auth::OAuth2(_)) => return Err(PrepareError::NoToken),
    };
    if !is_method_token(&template.method) {
        return Err(PrepareError::InvalidMethod);
    }
    Ok(Prepared {
        method: template.method.clone(),
        url,
        headers,
        body,
        auth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AuthKind, RequestAuth};

    fn get(url: &str) -> RequestTemplate {
        RequestTemplate {
            url: url.to_owned(),
            body_type: BodyType::None,
            ..RequestTemplate::default()
        }
    }

    #[test]
    fn basic_and_bearer_go_last_digest_waits_for_a_challenge() {
        let basic = RequestTemplate {
            auth: RequestAuth::basic("Aladdin", "open sesame"),
            ..get("http://h")
        };
        let prepared = prepare(&basic, true).unwrap();
        assert_eq!(
            prepared.headers.last().unwrap().1,
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );

        let bearer = RequestTemplate {
            auth: RequestAuth::bearer("t"),
            ..get("http://h")
        };
        assert_eq!(
            prepare(&bearer, true).unwrap().headers,
            [("Authorization".into(), "Bearer t".into())]
        );

        let digest = RequestTemplate {
            auth: RequestAuth::digest("u", "p"),
            ..get("http://h")
        };
        let prepared = prepare(&digest, true).unwrap();
        assert_eq!(prepared.headers, Vec::<(String, String)>::new());
        assert_eq!(
            prepared.auth,
            PreparedAuth::Digest(Credentials {
                username: "u".into(),
                password: "p".into()
            })
        );
    }

    #[test]
    fn an_api_key_goes_in_its_header_or_after_the_query() {
        let header = RequestTemplate {
            headers: vec![ParamItem::new("Accept", "*/*")],
            auth: RequestAuth::api_key("X-API-Key", "k1", ApiKeyIn::Header),
            ..get("http://h/a")
        };
        let prepared = prepare(&header, true).unwrap();
        assert_eq!(
            prepared.headers,
            [
                ("Accept".into(), "*/*".into()),
                ("X-API-Key".into(), "k1".into())
            ]
        );
        assert_eq!(prepared.url, "http://h/a");

        let query = RequestTemplate {
            auth: RequestAuth::api_key("api key", "a&b", ApiKeyIn::Query),
            ..get("http://h/a?page=2#top")
        };
        let prepared = prepare(&query, true).unwrap();
        assert_eq!(prepared.url, "http://h/a?page=2&api+key=a%26b");
        assert_eq!(prepared.headers, Vec::<(String, String)>::new());
        let bare = RequestTemplate {
            auth: RequestAuth::api_key("key", "v", ApiKeyIn::Query),
            ..get("http://h/a")
        };
        assert_eq!(prepare(&bare, true).unwrap().url, "http://h/a?key=v");
    }

    #[test]
    fn an_api_key_conflicts_only_with_its_own_name() {
        let header = RequestTemplate {
            headers: vec![ParamItem::new("x-api-key", "other")],
            auth: RequestAuth::api_key("X-API-Key", "k", ApiKeyIn::Header),
            ..get("http://h")
        };
        assert_eq!(
            prepare(&header, true).unwrap_err().to_string(),
            "Use either the API key or a header of the same name"
        );
        let query = RequestTemplate {
            auth: RequestAuth::api_key("key", "k", ApiKeyIn::Query),
            ..get("http://h/?key=1")
        };
        assert_eq!(
            prepare(&query, true).unwrap_err().to_string(),
            "Use either the API key or a query parameter of the same name"
        );
        let beside = RequestTemplate {
            headers: vec![ParamItem::new("Authorization", "Bearer t")],
            auth: RequestAuth::api_key("X-API-Key", "k", ApiKeyIn::Header),
            ..get("http://h")
        };
        assert!(
            prepare(&beside, true).is_ok(),
            "an Authorization header beside a key is the user's own"
        );
        let nameless = RequestTemplate {
            auth: RequestAuth::api_key("", "k", ApiKeyIn::Header),
            ..get("http://h")
        };
        assert!(matches!(
            prepare(&nameless, true),
            Err(PrepareError::Credentials(_))
        ));
    }

    #[test]
    fn auth_type_without_credentials_is_an_error() {
        let template = RequestTemplate {
            auth: RequestAuth {
                kind: Some(AuthKind::Basic),
                ..RequestAuth::default()
            },
            ..get("http://h")
        };
        assert!(matches!(
            prepare(&template, true),
            Err(PrepareError::Credentials(_))
        ));
    }

    #[test]
    fn all_cookies_disabled_sends_no_cookie_header() {
        let mut template = get("http://h");
        template.cookies = vec![ParamItem {
            is_enabled: false,
            ..ParamItem::new("a", "1")
        }];
        assert_eq!(
            prepare(&template, true).unwrap().headers,
            Vec::<(String, String)>::new()
        );
    }

    #[test]
    fn invalid_methods_are_rejected() {
        let template = RequestTemplate {
            method: "GE T".into(),
            ..get("http://h")
        };
        assert!(matches!(
            prepare(&template, true),
            Err(PrepareError::InvalidMethod)
        ));
    }

    #[test]
    fn binary_body_is_a_utf8_path() {
        let template = RequestTemplate {
            body_type: BodyType::Binary,
            body: vec![0xff],
            ..get("http://h")
        };
        assert!(matches!(
            prepare(&template, true),
            Err(PrepareError::BinaryPath)
        ));
    }
}
