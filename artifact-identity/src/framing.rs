use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{ArtifactIdentityError, Result};

pub(crate) fn canonical_ir_bytes<T: Serialize>(
    value: &T,
    map_error: fn(serde_json::Error) -> ArtifactIdentityError,
) -> Result<Vec<u8>> {
    skiff_canonical_json::canonical_json_bytes(value).map_err(map_error)
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Frames an identity from its canonical prefix and an already-derived hash.
pub fn framed_identity(prefix: &str, hash: &str) -> String {
    format!("{prefix}:{hash}")
}

pub(crate) fn hash_field(hasher: &mut Sha256, label: &str, value: &str) {
    hash_framed_bytes(hasher, label, value.as_bytes());
}

pub(crate) fn hash_bytes(hasher: &mut Sha256, label: &str, value: &[u8]) {
    hash_framed_bytes(hasher, label, value);
}

fn hash_framed_bytes(hasher: &mut Sha256, label: &str, value: &[u8]) {
    hasher.update(label.as_bytes());
    hasher.update([0]);
    hasher.update(value.len().to_le_bytes());
    hasher.update(value);
    hasher.update([0xff]);
}
