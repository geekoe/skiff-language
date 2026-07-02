use hmac::{Hmac, Mac};
use serde_json::Value;
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::encode_base64;

pub(crate) fn crypto_hmac_sha1_base64(args: &[Value]) -> Result<Value> {
    let key = args.first().and_then(Value::as_str).ok_or_else(|| {
        RuntimeError::Decode("std.crypto.hmacSha1Base64 key must be a string".to_string())
    })?;
    let text = args.get(1).and_then(Value::as_str).ok_or_else(|| {
        RuntimeError::Decode("std.crypto.hmacSha1Base64 text must be a string".to_string())
    })?;
    let mut mac = Hmac::<Sha1>::new_from_slice(key.as_bytes()).map_err(|error| {
        RuntimeError::Decode(format!("std.crypto.hmacSha1Base64 setup failed: {error}"))
    })?;
    mac.update(text.as_bytes());
    Ok(Value::String(encode_base64(&mac.finalize().into_bytes())))
}

pub(crate) fn crypto_sha256(args: &[Value]) -> Result<Value> {
    let text = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| RuntimeError::Decode("std.crypto.sha256 requires a string".to_string()))?;
    Ok(Value::String(hex::encode(Sha256::digest(text.as_bytes()))))
}

pub(crate) fn crypto_random_token(_args: &[Value]) -> Result<Value> {
    Ok(Value::String(format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )))
}

pub(crate) fn crypto_uuid(_args: &[Value]) -> Result<Value> {
    Ok(Value::String(uuid::Uuid::new_v4().to_string()))
}

pub(crate) fn crypto_uuid_simple(_args: &[Value]) -> Result<Value> {
    Ok(Value::String(uuid::Uuid::new_v4().simple().to_string()))
}
