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

/// Frames an identity from its canonical prefix and an already-derived hash.
pub fn framed_identity(prefix: &str, hash: &str) -> String {
    format!("{prefix}:{hash}")
}
