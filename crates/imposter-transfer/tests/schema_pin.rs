//! The bundled Postman schema is the pinned file, unchanged (`schemas/README.md`).
#![cfg(test)]

use std::fmt::Write;

use sha2::{Digest, Sha256};

#[test]
fn the_postman_schema_matches_its_pinned_hash() {
    let readme = include_str!("../schemas/README.md");
    let pinned = readme
        .lines()
        .find_map(|line| line.strip_prefix("SHA-256: `"))
        .and_then(|rest| rest.strip_suffix('`'))
        .expect("README.md names the pinned SHA-256");
    let digest = Sha256::digest(imposter_transfer::postman::SCHEMA.as_bytes());
    let actual = digest.iter().fold(String::new(), |mut hex, byte| {
        write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
        hex
    });
    assert_eq!(actual, pinned);
}
