use serde_json::Value;
use sha2::{Digest, Sha256};

pub use skiff_canonical_json::{canonical_json_bytes, canonical_json_number, canonical_json_value};

pub fn value_sha256(value: &Value) -> String {
    let canonical = canonical_json_value(value);
    let bytes = serde_json::to_vec(&canonical).expect("artifact values serialize");
    sha256_hex(&bytes)
}

pub fn stable_json_string(value: &Value) -> String {
    serde_json::to_string(&sort_json_value(value)).expect("stable JSON value must be serializable")
}

pub fn sort_json_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(sort_json_value).collect()),
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), sort_json_value(&map[key]));
            }
            Value::Object(sorted)
        }
        _ => value.clone(),
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests;
