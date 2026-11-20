//! Network authority for an agent session. Every origin is tied to a literal address so
//! a project edit or a change in DNS cannot move a send outside the user's grant.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};

use reqwest::Url;
use reqwest::dns::{Name, Resolve, Resolving};

pub const CAPTURE_LIMIT: u64 = 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Destinations {
    origins: Vec<Url>,
    pins: BTreeMap<String, IpAddr>,
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum Denied {
    #[error(
        "Use an HTTP(S) origin with no credentials, path or query, and append =IP for a DNS name"
    )]
    InvalidOrigin,
    #[error("Each granted hostname must use one IP address")]
    ConflictingPin,
    #[error("Destination is outside the execution grant")]
    Destination,
    #[error(
        "The execution grant does not allow proxies, custom certificates or disabled TLS verification"
    )]
    Settings,
    #[error("The execution grant does not allow file uploads")]
    File,
    #[error(
        "The execution grant does not allow HTTP connection upgrades, tunneling or a custom Host header"
    )]
    Tunnel,
}

impl Destinations {
    /// Add an origin as `http://IP:PORT` or `https://NAME:PORT=IP`.
    pub fn allow(&mut self, entry: &str) -> Result<(), Denied> {
        let (origin, pin) = entry.split_once('=').map_or((entry, None), |(origin, ip)| {
            (origin, Some(ip.parse::<IpAddr>()))
        });
        let url = Url::parse(origin).map_err(|_| Denied::InvalidOrigin)?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Denied::InvalidOrigin);
        }
        match (url.host(), pin) {
            (Some(url::Host::Domain(name)), Some(Ok(ip))) => {
                if self.pins.get(name).is_some_and(|previous| *previous != ip) {
                    return Err(Denied::ConflictingPin);
                }
                self.pins.insert(name.to_owned(), ip);
            }
            (Some(url::Host::Ipv4(_) | url::Host::Ipv6(_)), None) => {}
            _ => return Err(Denied::InvalidOrigin),
        }
        self.origins.push(url);
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }

    pub fn check_url(&self, url: &str) -> Result<(), Denied> {
        let url = Url::parse(url).map_err(|_| Denied::Destination)?;
        self.check(&url)
    }

    pub(crate) fn check(&self, url: &Url) -> Result<(), Denied> {
        if url.username().is_empty()
            && url.password().is_none()
            && self
                .origins
                .iter()
                .any(|origin| origin.origin() == url.origin())
        {
            Ok(())
        } else {
            Err(Denied::Destination)
        }
    }

    pub(crate) fn configure(&self, mut builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        builder = builder.dns_resolver(std::sync::Arc::new(PinsOnly));
        for (host, ip) in &self.pins {
            // Reqwest uses the URL's port with this override, preserving the HTTP host
            // and TLS name while selecting the granted IP without looking up DNS.
            builder = builder.resolve(host, SocketAddr::new(*ip, 0));
        }
        builder
    }
}

struct PinsOnly;

impl Resolve for PinsOnly {
    fn resolve(&self, _name: Name) -> Resolving {
        Box::pin(async { Err(Box::new(Denied::Destination) as _) })
    }
}
