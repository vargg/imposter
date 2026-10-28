//! Bytes as base64 the way the Python app writes them: URL-safe alphabet with padding.
//! Reading also accepts the standard alphabet and missing padding, as Pydantic does.

use base64::Engine as _;
use base64::alphabet::URL_SAFE;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde::{Deserialize, Deserializer, Serializer};

const WRITE: GeneralPurpose = base64::engine::general_purpose::URL_SAFE;
const READ: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

pub fn encode(bytes: &[u8]) -> String {
    WRITE.encode(bytes)
}

pub fn decode(text: &str) -> Result<Vec<u8>, base64::DecodeError> {
    let url_safe: String = text
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            other => other,
        })
        .collect();
    READ.decode(url_safe)
}

pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&encode(bytes))
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let text = String::deserialize(deserializer)?;
    decode(&text).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BYTES: &[u8] = &[0xfb, 0xff, 0x00, 0x3e, 0x3f];

    #[test]
    fn writes_url_safe_with_padding_like_pydantic() {
        assert_eq!(encode(BYTES), "-_8APj8=");
        assert_eq!(encode(b""), "");
    }

    #[test]
    fn reads_both_alphabets_with_or_without_padding() {
        for text in ["-_8APj8=", "+/8APj8=", "-_8APj8", "+/8APj8"] {
            assert_eq!(decode(text).unwrap(), BYTES, "{text}");
        }
        assert!(decode("a b").is_err());
    }
}
