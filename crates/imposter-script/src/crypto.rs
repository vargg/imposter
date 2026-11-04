//! Digests, HMACs and encodings. Digests and HMACs take text or bytes and give bytes, so
//! signatures can be chained (AWS Signature 4 signs with the bytes of an earlier HMAC);
//! `hex` and `base64_encode` turn bytes into text.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use rhai::{Blob, Dynamic, Engine, EvalAltResult};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

type Result<T> = std::result::Result<T, Box<EvalAltResult>>;

/// The bytes of text (UTF-8) or of a blob.
fn bytes(value: &Dynamic, what: &str) -> Result<Vec<u8>> {
    if value.is_string() {
        return Ok(value.clone().into_string()?.into_bytes());
    }
    if value.is_blob() {
        return Ok(value.clone().into_blob()?);
    }
    Err(format!("{what} must be text or bytes, not {}", value.type_name()).into())
}

fn hmac<M: Mac + KeyInit>(key: &Dynamic, data: &Dynamic) -> Result<Blob> {
    let mut mac = <M as KeyInit>::new_from_slice(&bytes(key, "The key")?)
        .map_err(|_| "The key cannot be used for an HMAC".to_owned())?;
    mac.update(&bytes(data, "The data")?);
    Ok(mac.finalize().into_bytes().to_vec())
}

pub fn hex(data: &[u8]) -> String {
    use std::fmt::Write as _;
    data.iter()
        .fold(String::with_capacity(data.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// RFC 3986: every byte but the unreserved characters is written `%XX`.
pub fn url_encode(text: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

fn url_decode(text: &str) -> Result<String> {
    let mut out = Vec::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let pair = bytes
                .get(at + 1..at + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or_else(|| format!("Invalid percent escape at character {}", at + 1))?;
            out.push(pair);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Standard or URL-safe alphabet, padded or not.
fn base64_decode(text: &str) -> Result<Blob> {
    let text = text.trim();
    [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD]
        .iter()
        .find_map(|engine| engine.decode(text).ok())
        .ok_or_else(|| "Not valid base64".to_owned().into())
}

pub fn register(engine: &mut Engine) {
    engine
        .register_fn("sha256", |data: Dynamic| -> Result<Blob> {
            Ok(Sha256::digest(bytes(&data, "The data")?).to_vec())
        })
        .register_fn("sha512", |data: Dynamic| -> Result<Blob> {
            Ok(Sha512::digest(bytes(&data, "The data")?).to_vec())
        })
        .register_fn("sha1", |data: Dynamic| -> Result<Blob> {
            Ok(Sha1::digest(bytes(&data, "The data")?).to_vec())
        })
        .register_fn("md5", |data: Dynamic| -> Result<Blob> {
            Ok(md5::Md5::digest(bytes(&data, "The data")?).to_vec())
        })
        .register_fn("hmac_sha256", |key: Dynamic, data: Dynamic| {
            hmac::<Hmac<Sha256>>(&key, &data)
        })
        .register_fn("hmac_sha512", |key: Dynamic, data: Dynamic| {
            hmac::<Hmac<Sha512>>(&key, &data)
        })
        .register_fn("hmac_sha1", |key: Dynamic, data: Dynamic| {
            hmac::<Hmac<Sha1>>(&key, &data)
        })
        .register_fn("hex", |data: Dynamic| -> Result<String> {
            Ok(hex(&bytes(&data, "hex()'s argument")?))
        })
        .register_fn("base64_encode", |data: Dynamic| -> Result<String> {
            Ok(STANDARD.encode(bytes(&data, "base64_encode()'s argument")?))
        })
        .register_fn("base64url_encode", |data: Dynamic| -> Result<String> {
            Ok(URL_SAFE_NO_PAD.encode(bytes(&data, "base64url_encode()'s argument")?))
        })
        .register_fn("base64_decode", |text: &str| base64_decode(text))
        .register_fn("url_encode", |text: &str| url_encode(text))
        .register_fn("url_decode", |text: &str| url_decode(text));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encoding_keeps_only_unreserved_characters() {
        assert_eq!(url_encode("a b/ç~"), "a%20b%2F%C3%A7~");
        assert_eq!(url_decode("a%20b%2F%C3%A7~").unwrap(), "a b/ç~");
        assert!(url_decode("%zz").is_err());
        assert!(url_decode("100%").is_err());
    }

    #[test]
    fn base64_reads_both_alphabets_with_or_without_padding() {
        for text in ["+/8=", "+/8", "-_8=", "-_8"] {
            assert_eq!(base64_decode(text).unwrap(), vec![0xfb, 0xff], "{text}");
        }
        assert!(base64_decode("!!").is_err());
    }
}
