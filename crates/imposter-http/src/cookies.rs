//! The cookie jar: the cookies servers set, kept so later requests send them back where
//! their domain, path and `Secure` flag allow (RFC 6265, through `cookie_store`). A jar
//! belongs to one collections folder and environment; this module neither reads nor writes
//! it anywhere, it only keeps, matches and lists cookies.

use std::sync::{Arc, Mutex, PoisonError};

use cookie_store::{Cookie, CookieExpiration, CookieStore};
use reqwest::Url;

/// Cookies, session cookies included: a jar is kept until cleared, since a login's session
/// cookie usually has no expiry and is the one worth keeping. Expired cookies are dropped.
#[derive(Debug, Clone, Default)]
pub struct Jar(CookieStore);

#[derive(Debug, thiserror::Error)]
#[error("The stored cookie jar cannot be read: {0}")]
pub struct JarError(String);

/// One cookie as the jar holds it, for listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCookie {
    /// The host it was set by (`host_only`), or the domain it covers with its subdomains.
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub name: String,
    pub value: String,
    /// Seconds since the Unix epoch, UTC; `None` for a session cookie.
    pub expires: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
}

impl Jar {
    /// A jar from its stored JSON; empty text is an empty jar.
    pub fn from_json(text: &str) -> Result<Self, JarError> {
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let cookies: Vec<Cookie<'static>> =
            serde_json::from_str(text).map_err(|e| JarError(e.to_string()))?;
        CookieStore::from_cookies(cookies.into_iter().map(Ok::<_, JarError>), false).map(Self)
    }

    /// The jar as JSON, without expired cookies.
    pub fn to_json(&self) -> String {
        let cookies: Vec<&Cookie<'static>> = self.0.iter_unexpired().collect();
        // Serializing cookies (strings and dates) cannot fail; an empty list is the fallback.
        serde_json::to_string(&cookies).unwrap_or_else(|_| "[]".to_owned())
    }

    /// The cookies to send to `url`, as name and value: longer paths first, as RFC 6265
    /// orders them, then by name (the jar keeps no creation time to order by). None for a
    /// URL that does not parse.
    pub fn cookies_for(&self, url: &str) -> Vec<(String, String)> {
        let Ok(url) = Url::parse(url) else {
            return Vec::new();
        };
        let mut cookies = self.0.matches(&url);
        cookies.sort_by(|a, b| {
            let length = |c: &Cookie<'_>| String::from(&c.path).len();
            length(b)
                .cmp(&length(a))
                .then_with(|| a.name().cmp(b.name()))
        });
        cookies
            .into_iter()
            .map(|c| (c.name().to_owned(), c.value().to_owned()))
            .collect()
    }

    /// Keep what one `Set-Cookie` header sets, as received from `url`. A cookie for another
    /// domain or with a bad syntax is ignored, as browsers ignore it; an expired one removes
    /// the cookie it names. Returns whether the jar changed.
    pub fn receive(&mut self, url: &str, set_cookie: &str) -> bool {
        // `parse`, not `store_response_cookies`, which logs each cookie's value at debug
        // (`imposter-http/tests/logging.rs` catches that).
        Url::parse(url).is_ok_and(|url| self.0.parse(set_cookie, &url).is_ok())
    }

    /// Every cookie, by domain, path and name.
    pub fn cookies(&self) -> Vec<StoredCookie> {
        let mut cookies: Vec<StoredCookie> = self
            .0
            .iter_unexpired()
            .map(|c| StoredCookie {
                domain: c
                    .domain
                    .as_cow()
                    .map(std::borrow::Cow::into_owned)
                    .unwrap_or_default(),
                host_only: matches!(c.domain, cookie_store::CookieDomain::HostOnly(_)),
                path: String::from(&c.path),
                name: c.name().to_owned(),
                value: c.value().to_owned(),
                expires: match c.expires {
                    CookieExpiration::AtUtc(at) => Some(at.unix_timestamp()),
                    CookieExpiration::SessionEnd => None,
                },
                secure: c.secure().unwrap_or(false),
                http_only: c.http_only().unwrap_or(false),
            })
            .collect();
        cookies.sort_by(|a, b| (&a.domain, &a.path, &a.name).cmp(&(&b.domain, &b.path, &b.name)));
        cookies
    }

    /// Remove one cookie; returns whether it was there.
    pub fn remove(&mut self, domain: &str, path: &str, name: &str) -> bool {
        self.0.remove(domain, path, name).is_some()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn len(&self) -> usize {
        self.0.iter_unexpired().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl PartialEq for Jar {
    fn eq(&self, other: &Self) -> bool {
        self.cookies() == other.cookies()
    }
}

/// One jar shared by every hop of an exchange, and by the requests of a run: what one
/// response sets, the next request sends.
#[derive(Clone, Default)]
pub struct SharedJar(Arc<Mutex<Jar>>);

impl SharedJar {
    pub fn new(jar: Jar) -> Self {
        Self(Arc::new(Mutex::new(jar)))
    }

    /// Work on the jar. A panic elsewhere while it was held leaves cookies, not broken
    /// state, so the jar stays usable.
    pub fn with<T>(&self, work: impl FnOnce(&mut Jar) -> T) -> T {
        work(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// A copy of the jar as it is now.
    pub fn snapshot(&self) -> Jar {
        self.with(|jar| jar.clone())
    }
}

/// The same jar, not an equal one: two runs with equal cookies still keep their own.
impl PartialEq for SharedJar {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for SharedJar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Values stay out of debug output, which can reach logs.
        write!(f, "SharedJar({} cookies)", self.with(|jar| jar.len()))
    }
}

/// The names in a `Cookie` header's value (`a=1; b=2`).
pub(crate) fn names_in(header: &str) -> impl Iterator<Item = &str> {
    header
        .split(';')
        .filter_map(|pair| pair.split_once('=').map(|(name, _)| name.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_go_where_their_domain_path_and_secure_flag_allow() {
        let mut jar = Jar::default();
        assert!(jar.receive("http://api.test/login", "session=abc; Path=/"));
        assert!(jar.receive("http://api.test/login", "scoped=1; Path=/admin"));
        assert!(jar.receive("http://api.test/", "wide=1; Domain=api.test"));
        assert!(jar.receive("https://api.test/", "safe=1; Secure"));
        assert!(
            !jar.receive("http://api.test/", "other=1; Domain=elsewhere.test"),
            "another domain's cookie is ignored"
        );
        assert!(!jar.receive("http://api.test/", "no equals sign"));

        let names = |url: &str| {
            jar.cookies_for(url)
                .into_iter()
                .map(|(n, _)| n)
                .collect::<Vec<_>>()
        };
        assert_eq!(names("http://api.test/users"), ["session", "wide"]);
        assert_eq!(
            names("http://api.test/admin/users"),
            ["scoped", "session", "wide"],
            "longer paths first"
        );
        assert_eq!(names("http://www.api.test/"), ["wide"]);
        assert_eq!(names("https://api.test/"), ["safe", "session", "wide"]);
        assert_eq!(names("http://other.test/"), [] as [String; 0]);
        assert_eq!(names("not a url"), [] as [String; 0]);
    }

    #[test]
    fn an_expired_cookie_removes_the_one_it_names() {
        let mut jar = Jar::default();
        jar.receive("http://api.test/", "session=abc");
        assert_eq!(jar.len(), 1);
        jar.receive("http://api.test/", "session=; Max-Age=0");
        assert!(jar.is_empty());
    }

    #[test]
    fn the_jar_round_trips_through_json_with_session_cookies() {
        let mut jar = Jar::default();
        jar.receive("http://api.test/", "session=abc; HttpOnly");
        jar.receive(
            "https://api.test/a",
            "kept=1; Path=/a; Secure; Expires=Fri, 01 Jan 2100 00:00:00 GMT",
        );
        let json = jar.to_json();
        let back = Jar::from_json(&json).unwrap();
        assert_eq!(back, jar);
        assert_eq!(
            back.cookies(),
            [
                StoredCookie {
                    domain: "api.test".into(),
                    host_only: true,
                    path: "/".into(),
                    name: "session".into(),
                    value: "abc".into(),
                    expires: None,
                    secure: false,
                    http_only: true,
                },
                StoredCookie {
                    domain: "api.test".into(),
                    host_only: true,
                    path: "/a".into(),
                    name: "kept".into(),
                    value: "1".into(),
                    expires: Some(4_102_444_800),
                    secure: true,
                    http_only: false,
                },
            ]
        );
        assert!(Jar::from_json("").unwrap().is_empty());
        assert!(Jar::from_json("{").is_err());
    }

    #[test]
    fn removing_and_clearing() {
        let mut jar = Jar::default();
        jar.receive("http://api.test/", "a=1");
        jar.receive("http://api.test/", "b=2");
        assert!(jar.remove("api.test", "/", "a"));
        assert!(!jar.remove("api.test", "/", "a"));
        assert_eq!(jar.cookies().len(), 1);
        jar.clear();
        assert!(jar.is_empty());
    }

    #[test]
    fn a_shared_jar_is_one_jar_and_hides_values_from_debug() {
        let shared = SharedJar::new(Jar::default());
        let same = shared.clone();
        same.with(|jar| jar.receive("http://api.test/", "token=secret-value"));
        assert_eq!(shared.snapshot().len(), 1);
        assert_eq!(shared, same);
        assert_ne!(shared, SharedJar::new(shared.snapshot()));
        assert_eq!(format!("{shared:?}"), "SharedJar(1 cookies)");
    }

    #[test]
    fn names_in_a_cookie_header() {
        assert_eq!(
            names_in("a=1; b = 2;c=x=y; bare").collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
    }
}
