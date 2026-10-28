//! HTTP Digest authentication (RFC 7616, RFC 2617), as httpx implements it: MD5, SHA, SHA-256
//! and SHA-512 with their `-sess` variants, `qop=auth` or no qop. `auth-int` is not supported.

use std::collections::HashMap;
use std::fmt::Write as _;

use md5::{Digest as _, Md5};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub algorithm: String,
    /// The qop values the server offers (empty when it sent none).
    pub qop: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DigestError {
    #[error("The server's Digest challenge is missing {0}")]
    Missing(&'static str),
    #[error("Digest algorithm {0} is not supported")]
    Algorithm(String),
    #[error("Digest qop auth-int is not supported")]
    AuthInt,
}

/// Split `key=value, key="quoted, value"` lists.
fn parameters(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq]
            .trim()
            .trim_start_matches(',')
            .trim()
            .to_ascii_lowercase();
        rest = rest[eq + 1..].trim_start();
        let value;
        if let Some(quoted) = rest.strip_prefix('"') {
            let mut text = String::new();
            let mut chars = quoted.char_indices();
            let mut end = quoted.len();
            while let Some((index, c)) = chars.next() {
                match c {
                    '\\' => {
                        if let Some((_, escaped)) = chars.next() {
                            text.push(escaped);
                        }
                    }
                    '"' => {
                        end = index + 1;
                        break;
                    }
                    other => text.push(other),
                }
            }
            value = text;
            rest = quoted[end..]
                .trim_start()
                .trim_start_matches(',')
                .trim_start();
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            value = rest[..end].trim().to_owned();
            rest = rest[end..].trim_start_matches(',').trim_start();
        }
        out.insert(key, value);
    }
    out
}

impl Challenge {
    /// Parse a `WWW-Authenticate` value, or `None` if it is not a Digest challenge.
    pub fn parse(header: &str) -> Option<Result<Self, DigestError>> {
        let (scheme, rest) = header
            .trim()
            .split_once(char::is_whitespace)
            .unwrap_or((header.trim(), ""));
        if !scheme.eq_ignore_ascii_case("digest") {
            return None;
        }
        let mut fields = parameters(rest);
        let mut take = |key: &'static str| fields.remove(key);
        Some((|| {
            Ok(Self {
                realm: take("realm").ok_or(DigestError::Missing("realm"))?,
                nonce: take("nonce").ok_or(DigestError::Missing("nonce"))?,
                opaque: take("opaque"),
                algorithm: take("algorithm").unwrap_or_else(|| "MD5".to_owned()),
                qop: take("qop")
                    .map(|qop| {
                        qop.split(',')
                            .map(|v| v.trim().to_ascii_lowercase())
                            .filter(|v| !v.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hash {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing to a String cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

impl Hash {
    fn digest(self, data: &str) -> String {
        match self {
            Self::Md5 => hex(&Md5::digest(data.as_bytes())),
            Self::Sha1 => hex(&sha1::Sha1::digest(data.as_bytes())),
            Self::Sha256 => hex(&sha2::Sha256::digest(data.as_bytes())),
            Self::Sha512 => hex(&sha2::Sha512::digest(data.as_bytes())),
        }
    }
}

fn algorithm(name: &str) -> Result<(Hash, bool), DigestError> {
    let upper = name.to_ascii_uppercase();
    let (base, session) = upper
        .strip_suffix("-SESS")
        .map_or((upper.as_str(), false), |base| (base, true));
    let hash = match base {
        "MD5" => Hash::Md5,
        "SHA" => Hash::Sha1,
        "SHA-256" => Hash::Sha256,
        "SHA-512" => Hash::Sha512,
        _ => return Err(DigestError::Algorithm(name.to_owned())),
    };
    Ok((hash, session))
}

/// A random client nonce: 16 hex digits.
pub fn client_nonce() -> String {
    // OS randomness practically never fails; a predictable nonce still authenticates.
    let value = getrandom::u64().unwrap_or_else(|_| u64::from(std::process::id()));
    format!("{value:016x}")
}

/// The `Authorization` header value answering `challenge` for `method` and `uri`
/// (the request target: path and query).
pub fn authorization(
    challenge: &Challenge,
    username: &str,
    password: &str,
    method: &str,
    uri: &str,
    nonce_count: u32,
    cnonce: &str,
) -> Result<String, DigestError> {
    let (hash, session) = algorithm(&challenge.algorithm)?;
    let qop = if challenge.qop.is_empty() {
        None
    } else if challenge.qop.iter().any(|q| q == "auth") {
        Some("auth")
    } else {
        return Err(DigestError::AuthInt);
    };
    let mut ha1 = hash.digest(&format!("{username}:{}:{password}", challenge.realm));
    if session {
        ha1 = hash.digest(&format!("{ha1}:{}:{cnonce}", challenge.nonce));
    }
    let ha2 = hash.digest(&format!("{method}:{uri}"));
    let nc = format!("{nonce_count:08x}");
    let response = match qop {
        Some(qop) => hash.digest(&format!(
            "{ha1}:{}:{nc}:{cnonce}:{qop}:{ha2}",
            challenge.nonce
        )),
        None => hash.digest(&format!("{ha1}:{}:{ha2}", challenge.nonce)),
    };

    let quote = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
    let mut header = format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{response}\", algorithm={}",
        quote(username),
        quote(&challenge.realm),
        quote(&challenge.nonce),
        quote(uri),
        challenge.algorithm
    );
    if let Some(opaque) = &challenge.opaque {
        let _ = write!(header, ", opaque=\"{}\"", quote(opaque));
    }
    if let Some(qop) = qop {
        let _ = write!(header, ", qop={qop}, nc={nc}, cnonce=\"{}\"", quote(cnonce));
    }
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_of(header: &str) -> String {
        parameters(header.trim_start_matches("Digest "))["response"].clone()
    }

    #[test]
    fn rfc_2617_example() {
        let challenge = Challenge::parse(
            r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#,
        )
        .unwrap()
        .unwrap();
        let header = authorization(
            &challenge,
            "Mufasa",
            "Circle Of Life",
            "GET",
            "/dir/index.html",
            1,
            "0a4f113b",
        )
        .unwrap();
        assert_eq!(response_of(&header), "6629fae49393a05397450978507c4ef1");
        assert!(header.contains(r#"opaque="5ccc069c403ebaf9f0171e9517f40e41""#));
        assert!(header.contains("qop=auth, nc=00000001, cnonce=\"0a4f113b\""));
    }

    #[test]
    fn rfc_7616_examples_md5_and_sha256() {
        let base = r#"realm="http-auth@example.org", qop="auth, auth-int", nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#;
        let cnonce = "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ";
        for (algorithm, expected) in [
            ("MD5", "8ca523f5e9506fed4657c9700eebdbec"),
            (
                "SHA-256",
                "753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1",
            ),
        ] {
            let challenge = Challenge::parse(&format!("Digest {base}, algorithm={algorithm}"))
                .unwrap()
                .unwrap();
            let header = authorization(
                &challenge,
                "Mufasa",
                "Circle of Life",
                "GET",
                "/dir/index.html",
                1,
                cnonce,
            )
            .unwrap();
            assert_eq!(response_of(&header), expected, "{algorithm}");
        }
    }

    #[test]
    fn without_qop_the_legacy_formula_is_used() {
        let challenge = Challenge::parse(r#"Digest realm="r", nonce="n""#)
            .unwrap()
            .unwrap();
        let header = authorization(&challenge, "u", "p", "GET", "/", 1, "c").unwrap();
        let ha1 = Hash::Md5.digest("u:r:p");
        let ha2 = Hash::Md5.digest("GET:/");
        assert_eq!(
            response_of(&header),
            Hash::Md5.digest(&format!("{ha1}:n:{ha2}"))
        );
        assert!(!header.contains("qop"));
    }

    #[test]
    fn session_variants_hash_the_nonces_into_ha1() {
        let challenge =
            Challenge::parse(r#"Digest realm="r", nonce="n", qop="auth", algorithm=SHA-512-sess"#)
                .unwrap()
                .unwrap();
        let header = authorization(&challenge, "u", "p", "POST", "/x?y=1", 2, "c").unwrap();
        let ha1 = Hash::Sha512.digest(&format!("{}:n:c", Hash::Sha512.digest("u:r:p")));
        let ha2 = Hash::Sha512.digest("POST:/x?y=1");
        assert_eq!(
            response_of(&header),
            Hash::Sha512.digest(&format!("{ha1}:n:00000002:c:auth:{ha2}"))
        );
        assert!(header.contains("algorithm=SHA-512-sess"));
    }

    #[test]
    fn unsupported_challenges_are_errors() {
        assert!(Challenge::parse(r#"Basic realm="x""#).is_none());
        assert_eq!(
            Challenge::parse(r#"Digest nonce="n""#),
            Some(Err(DigestError::Missing("realm")))
        );
        let auth_int = Challenge::parse(r#"Digest realm="r", nonce="n", qop="auth-int""#)
            .unwrap()
            .unwrap();
        assert_eq!(
            authorization(&auth_int, "u", "p", "GET", "/", 1, "c"),
            Err(DigestError::AuthInt)
        );
        let unknown = Challenge::parse(r#"Digest realm="r", nonce="n", algorithm=SHA3"#)
            .unwrap()
            .unwrap();
        assert!(matches!(
            authorization(&unknown, "u", "p", "GET", "/", 1, "c"),
            Err(DigestError::Algorithm(_))
        ));
    }

    #[test]
    fn parameters_handle_quotes_escapes_and_bare_values() {
        let fields = parameters(r#"realm="a, \"b\"", algorithm=MD5,stale=FALSE , qop="auth""#);
        assert_eq!(fields["realm"], "a, \"b\"");
        assert_eq!(fields["algorithm"], "MD5");
        assert_eq!(fields["stale"], "FALSE");
        assert_eq!(fields["qop"], "auth");
        assert_eq!(client_nonce().len(), 16);
    }
}
