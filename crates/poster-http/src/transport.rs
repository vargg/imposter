//! Sending a prepared request: client policy, redirects, Digest retries, capped capture and
//! cancellation. Redirects are followed here rather than by reqwest so streamed uploads can be
//! rebuilt for 307/308 and the httpx rules apply (method changes, dropped credentials).

use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt as _;
use poster_core::execution::Response;
use poster_core::prepare::{Prepared, PreparedAuth, PreparedBody};
use poster_core::settings::AppSettings;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, LOCATION, WWW_AUTHENTICATE};
use reqwest::{Method, StatusCode, Url};
use tokio_util::sync::CancellationToken;

use crate::digest::{self, Challenge};
use crate::tls::{self, TlsSettings};
use crate::upload::{self, UploadError};

pub const MAX_REDIRECTS: usize = 20;

/// How one execution talks to the network, resolved from the effective settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// Applies separately to connecting and to each read, as in httpx.
    pub timeout: Duration,
    pub follow_redirects: bool,
    /// Explicit proxy; environment and system proxies are never used.
    pub proxy: Option<String>,
    pub tls: TlsSettings,
    pub identity_routing: Option<Box<IdentityRouting>>,
    /// Bytes of body to keep; `None` keeps everything.
    pub capture_limit: Option<u64>,
}

/// Captured before execution so redirects use the same settings and variable snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct IdentityRouting {
    pub settings: AppSettings,
    pub options: poster_core::model::Options,
    pub variables: poster_core::model::Variables,
}

impl IdentityRouting {
    fn apply(&self, url: &Url, tls: &mut TlsSettings) -> Result<(), SendError> {
        let settings = self.settings.resolve_for_url(url.as_str(), &self.options);
        let mut missing = std::collections::BTreeSet::new();
        let mut resolve = |text: &str| {
            poster_core::variables::substitute(
                text,
                |name| self.variables.get(name).map(String::as_str),
                &mut missing,
            )
        };
        tls.certificate = path_setting(&resolve(&settings.certificate_path));
        tls.key = path_setting(&resolve(&settings.key_file));
        tls.password = resolve(&settings.password);
        if !missing.is_empty() {
            return Err(SendError::new(
                ErrorKind::Tls,
                poster_core::variables::ResolveError::Unresolved(missing).to_string(),
            ));
        }
        Ok(())
    }
}

fn path_setting(value: &str) -> Option<std::path::PathBuf> {
    (!value.is_empty()).then(|| value.into())
}

impl Policy {
    pub fn from_settings(settings: &AppSettings) -> Self {
        Self {
            identity_routing: (!settings.host_identities.is_empty()).then(|| {
                Box::new(IdentityRouting {
                    settings: settings.clone(),
                    options: poster_core::model::Options::default(),
                    variables: poster_core::model::Variables::new(),
                })
            }),
            timeout: Duration::from_secs_f64(settings.timeout),
            follow_redirects: settings.follow_redirects,
            proxy: (!settings.proxy_url.is_empty()).then(|| settings.proxy_url.clone()),
            tls: TlsSettings {
                verify: settings.verify_ssl,
                ca_bundle: path_setting(&settings.ca_bundle),
                certificate: path_setting(&settings.certificate_path),
                key: path_setting(&settings.key_file),
                password: settings.password.clone(),
            },
            capture_limit: settings.download_limit_bytes(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidUrl,
    InvalidHeader,
    InvalidProxy,
    Tls,
    Connect,
    Timeout,
    TooManyRedirects,
    Upload,
    Digest,
    Body,
    Protocol,
}

impl ErrorKind {
    /// The stable name stored in history (`Response::error_kind`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUrl => "InvalidUrl",
            Self::InvalidHeader => "InvalidHeader",
            Self::InvalidProxy => "InvalidProxy",
            Self::Tls => "TlsError",
            Self::Connect => "ConnectError",
            Self::Timeout => "Timeout",
            Self::TooManyRedirects => "TooManyRedirects",
            Self::Upload => "UploadError",
            Self::Digest => "DigestError",
            Self::Body => "BodyError",
            Self::Protocol => "ProtocolError",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("Request cancelled")]
    Cancelled,
    #[error("{message}")]
    Failed { kind: ErrorKind, message: String },
}

impl SendError {
    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self::Failed {
            kind,
            message: message.into(),
        }
    }
}

impl From<UploadError> for SendError {
    fn from(error: UploadError) -> Self {
        Self::new(ErrorKind::Upload, error.to_string())
    }
}

/// An error's message with its causes, without the URL (it may carry credentials).
fn describe(error: reqwest::Error) -> (ErrorKind, String) {
    let error = error.without_url();
    let mut message = error.to_string();
    let mut kind = if error.is_timeout() {
        ErrorKind::Timeout
    } else if error.is_connect() {
        ErrorKind::Connect
    } else if error.is_body() || error.is_decode() {
        ErrorKind::Body
    } else {
        ErrorKind::Protocol
    };
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        // `io::Error::source` skips the error it wraps (often another `io::Error`), so walk
        // the wrapped errors explicitly.
        let mut layer = Some(cause);
        while let Some(current) = layer {
            if current.is::<UploadError>() {
                kind = ErrorKind::Upload;
            } else if current.is::<rustls::Error>() {
                kind = ErrorKind::Tls;
            }
            let io = current.downcast_ref::<std::io::Error>();
            if kind != ErrorKind::Tls
                && io.is_some_and(|io| io.kind() == std::io::ErrorKind::TimedOut)
            {
                kind = ErrorKind::Timeout;
            }
            layer = io
                .and_then(std::io::Error::get_ref)
                .map(|inner| inner as &(dyn std::error::Error + 'static));
        }
        source = cause.source();
    }
    (kind, message)
}

impl From<reqwest::Error> for SendError {
    fn from(error: reqwest::Error) -> Self {
        let (kind, message) = describe(error);
        Self::new(kind, message)
    }
}

fn client(policy: &Policy, tls: rustls::ClientConfig) -> Result<reqwest::Client, SendError> {
    let mut builder = reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(policy.timeout)
        .read_timeout(policy.timeout)
        .pool_max_idle_per_host(0)
        .user_agent(concat!("Poster/", env!("CARGO_PKG_VERSION")));
    builder = match &policy.proxy {
        Some(url) => builder.proxy(reqwest::Proxy::all(url.as_str()).map_err(|e| {
            SendError::new(
                ErrorKind::InvalidProxy,
                format!("Invalid proxy URL: {}", e.without_url()),
            )
        })?),
        None => builder.no_proxy(),
    };
    builder.build().map_err(Into::into)
}

fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, SendError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            SendError::new(
                ErrorKind::InvalidHeader,
                format!("Invalid header name: {name:?}"),
            )
        })?;
        let value = HeaderValue::from_bytes(value.as_bytes()).map_err(|_| {
            SendError::new(
                ErrorKind::InvalidHeader,
                format!("Invalid value for header {name}"),
            )
        })?;
        map.append(header, value);
    }
    Ok(map)
}

/// The body for one attempt. Streams are rebuilt so every attempt sends the whole body.
fn body(prepared: &PreparedBody, boundary: &str) -> Result<reqwest::Body, SendError> {
    Ok(match prepared {
        PreparedBody::Bytes(bytes) => reqwest::Body::from(bytes.clone()),
        PreparedBody::Multipart(fields) => {
            reqwest::Body::wrap_stream(upload::multipart_stream(fields, boundary)?)
        }
        PreparedBody::File(path) => reqwest::Body::wrap_stream(upload::file_stream(path.into())),
    })
}

struct Attempt<'a> {
    client: reqwest::Client,
    method: Method,
    url: Url,
    headers: Vec<(String, String)>,
    /// `None` once a redirect turned the request into a bodiless GET.
    body: Option<&'a PreparedBody>,
    boundary: &'a str,
}

impl Attempt<'_> {
    async fn send(&self, authorization: Option<String>) -> Result<reqwest::Response, SendError> {
        let mut headers = header_map(&self.headers)?;
        if let Some(value) = authorization {
            let value = HeaderValue::from_str(&value).map_err(|_| {
                SendError::new(
                    ErrorKind::Digest,
                    "Digest credentials contain invalid characters",
                )
            })?;
            headers.append(reqwest::header::AUTHORIZATION, value);
        }
        let mut request = self
            .client
            .request(self.method.clone(), self.url.clone())
            .headers(headers);
        if let Some(prepared) = self.body {
            request = request.body(body(prepared, self.boundary)?);
        }
        Ok(request.send().await?)
    }

    /// One hop, answering a Digest challenge once if credentials are configured.
    async fn send_authenticated(
        &self,
        auth: &PreparedAuth,
    ) -> Result<reqwest::Response, SendError> {
        let first = self.send(None).await?;
        let PreparedAuth::Digest(credentials) = auth else {
            return Ok(first);
        };
        if first.status() != StatusCode::UNAUTHORIZED {
            return Ok(first);
        }
        let challenge = first
            .headers()
            .get_all(WWW_AUTHENTICATE)
            .iter()
            .find_map(|value| Challenge::parse(value.to_str().ok()?));
        let Some(challenge) = challenge else {
            return Ok(first);
        };
        let challenge = challenge.map_err(|e| SendError::new(ErrorKind::Digest, e.to_string()))?;
        drop(first);
        let uri = match self.url.query() {
            Some(query) => format!("{}?{query}", self.url.path()),
            None => self.url.path().to_owned(),
        };
        let header = digest::authorization(
            &challenge,
            &credentials.username,
            &credentials.password,
            self.method.as_str(),
            &uri,
            1,
            &digest::client_nonce(),
        )
        .map_err(|e| SendError::new(ErrorKind::Digest, e.to_string()))?;
        self.send(Some(header)).await
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

fn is_https_upgrade(from: &Url, to: &Url) -> bool {
    from.scheme() == "http"
        && to.scheme() == "https"
        && from.host_str() == to.host_str()
        && from.port_or_known_default() == Some(80)
        && to.port_or_known_default() == Some(443)
}

fn remove_header(headers: &mut Vec<(String, String)>, name: &str) {
    headers.retain(|(header, _)| !header.eq_ignore_ascii_case(name));
}

/// httpx's redirect method rules: 303 becomes GET (except HEAD); 301 and 302 turn POST into GET.
fn redirect_method(status: StatusCode, method: &Method) -> Method {
    let to_get = (status == StatusCode::SEE_OTHER && method != Method::HEAD)
        || (matches!(status, StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND)
            && method == Method::POST);
    if to_get { Method::GET } else { method.clone() }
}

async fn capture(
    mut response: reqwest::Response,
    limit: Option<u64>,
) -> Result<Response, SendError> {
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let limit = limit.map(|limit| usize::try_from(limit).unwrap_or(usize::MAX));
    let mut body = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = response.chunk().await? {
        if let Some(limit) = limit
            && body.len() + chunk.len() > limit
        {
            body.extend_from_slice(&chunk[..limit - body.len()]);
            truncated = true;
            break;
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Response {
        status: status.as_u16(),
        reason: status.canonical_reason().unwrap_or_default().to_owned(),
        headers,
        body,
        truncated,
        ..Response::default()
    })
}

async fn client_for(policy: &Policy, url: &Url) -> Result<reqwest::Client, SendError> {
    let mut settings = policy.tls.clone();
    if let Some(routing) = &policy.identity_routing {
        routing.apply(url, &mut settings)?;
    }
    let tls = tokio::task::spawn_blocking(move || tls::client_config(&settings))
        .await
        .map_err(|_| SendError::new(ErrorKind::Tls, "Loading certificates failed unexpectedly"))?
        .map_err(|e| SendError::new(ErrorKind::Tls, e.to_string()))?;
    client(policy, tls)
}

async fn exchange(prepared: &Prepared, policy: &Policy) -> Result<Response, SendError> {
    match &prepared.body {
        PreparedBody::Multipart(fields) => {
            upload::validate(upload::multipart_files(fields)).await?;
        }
        PreparedBody::File(path) => {
            upload::validate([std::path::Path::new(path)]).await?;
        }
        PreparedBody::Bytes(_) => {}
    }

    let boundary = upload::boundary();
    let mut headers = prepared.headers.clone();
    if matches!(prepared.body, PreparedBody::Multipart(_)) {
        remove_header(&mut headers, "content-type");
        headers.push((
            "Content-Type".to_owned(),
            format!("multipart/form-data; boundary={boundary}"),
        ));
    }
    let method = Method::from_bytes(prepared.method.as_bytes()).map_err(|_| {
        SendError::new(
            ErrorKind::Protocol,
            "Request method must be a nonempty HTTP token",
        )
    })?;
    let url = Url::parse(&prepared.url)
        .map_err(|e| SendError::new(ErrorKind::InvalidUrl, format!("Invalid URL: {e}")))?;
    let mut attempt = Attempt {
        client: client_for(policy, &url).await?,
        method,
        url,
        headers,
        body: Some(&prepared.body),
        boundary: &boundary,
    };

    let mut redirects = 0;
    loop {
        let response = attempt.send_authenticated(&prepared.auth).await?;
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok());
        let (true, true, Some(location)) = (
            policy.follow_redirects,
            response.status().is_redirection(),
            location,
        ) else {
            return capture(response, policy.capture_limit).await;
        };
        redirects += 1;
        if redirects > MAX_REDIRECTS {
            return Err(SendError::new(
                ErrorKind::TooManyRedirects,
                format!("Exceeded {MAX_REDIRECTS} redirects"),
            ));
        }
        let next = attempt.url.join(location).map_err(|e| {
            SendError::new(
                ErrorKind::InvalidUrl,
                format!("Invalid redirect location: {e}"),
            )
        })?;
        let method = redirect_method(response.status(), &attempt.method);
        drop(response);
        if method != attempt.method && method == Method::GET {
            attempt.body = None;
            remove_header(&mut attempt.headers, "content-length");
            remove_header(&mut attempt.headers, "transfer-encoding");
        }
        if !same_origin(&attempt.url, &next) && !is_https_upgrade(&attempt.url, &next) {
            remove_header(&mut attempt.headers, "authorization");
        }
        // As httpx does: explicit cookies are not carried to the next location.
        remove_header(&mut attempt.headers, "cookie");
        attempt.client = client_for(policy, &next).await?;
        attempt.method = method;
        attempt.url = next;
    }
}

/// Send `prepared` and capture the response. Cancelling drops the connection at once.
pub async fn send(
    prepared: &Prepared,
    policy: &Policy,
    cancel: &CancellationToken,
) -> Result<Response, SendError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(SendError::Cancelled),
        result = exchange(prepared, policy) => result,
    }
}

/// Collect a body stream; for tests and previews of what an upload sends.
pub async fn collect(stream: upload::BodyStream) -> Result<Vec<u8>, UploadError> {
    let chunks: Vec<Result<Bytes, UploadError>> = stream.collect().await;
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend_from_slice(&chunk?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_methods_follow_httpx() {
        assert_eq!(
            redirect_method(StatusCode::SEE_OTHER, &Method::PUT),
            Method::GET
        );
        assert_eq!(
            redirect_method(StatusCode::SEE_OTHER, &Method::HEAD),
            Method::HEAD
        );
        assert_eq!(
            redirect_method(StatusCode::FOUND, &Method::POST),
            Method::GET
        );
        assert_eq!(
            redirect_method(StatusCode::FOUND, &Method::PUT),
            Method::PUT
        );
        assert_eq!(
            redirect_method(StatusCode::MOVED_PERMANENTLY, &Method::POST),
            Method::GET
        );
        assert_eq!(
            redirect_method(StatusCode::TEMPORARY_REDIRECT, &Method::POST),
            Method::POST
        );
        assert_eq!(
            redirect_method(StatusCode::PERMANENT_REDIRECT, &Method::POST),
            Method::POST
        );
    }

    #[test]
    fn origins_and_https_upgrades() {
        let url = |text: &str| Url::parse(text).unwrap();
        assert!(same_origin(&url("http://h/a"), &url("http://h:80/b")));
        assert!(!same_origin(&url("http://h/a"), &url("http://other/a")));
        assert!(!same_origin(&url("http://h/a"), &url("https://h/a")));
        assert!(is_https_upgrade(&url("http://h/a"), &url("https://h/a")));
        assert!(!is_https_upgrade(
            &url("http://h:8080/a"),
            &url("https://h/a")
        ));
    }

    #[test]
    fn policy_from_settings_maps_empty_paths_to_none() {
        let settings = AppSettings {
            ca_bundle: "/ca.pem".into(),
            download_limit: 0,
            ..AppSettings::default()
        };
        let policy = Policy::from_settings(&settings);
        assert_eq!(
            policy.tls.ca_bundle.as_deref(),
            Some(std::path::Path::new("/ca.pem"))
        );
        assert_eq!(policy.tls.certificate, None);
        assert_eq!(policy.proxy, None);
        assert_eq!(policy.capture_limit, None);
        assert_eq!(policy.timeout, Duration::from_secs(30));
    }

    #[test]
    fn invalid_header_names_and_values_are_reported() {
        let error = header_map(&[("bad name".into(), "v".into())]).unwrap_err();
        assert!(matches!(
            error,
            SendError::Failed {
                kind: ErrorKind::InvalidHeader,
                ..
            }
        ));
        let error = header_map(&[("ok".into(), "line\r\nbreak".into())]).unwrap_err();
        assert!(matches!(
            error,
            SendError::Failed {
                kind: ErrorKind::InvalidHeader,
                ..
            }
        ));
        assert_eq!(
            header_map(&[("X".into(), "ünï".into())]).unwrap()["x"].as_bytes(),
            "ünï".as_bytes()
        );
    }
}
