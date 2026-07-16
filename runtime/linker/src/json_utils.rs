use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) use skiff_canonical_json::canonical_json_value;

pub(crate) fn value_sha256(value: &Value) -> anyhow::Result<String> {
    let canonical = canonical_json_value(value);
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| anyhow::anyhow!("failed to serialize artifact JSON: {error}"))?;
    Ok(sha256_hex(&bytes))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Number, Value};

    use super::*;

    #[test]
    fn linker_hash_uses_shared_canonical_number_normalization() {
        let mut value = Map::new();
        value.insert(
            "number".to_string(),
            Value::Number(Number::from_f64(1.0).expect("number")),
        );

        assert_eq!(
            canonical_json_value(&Value::Object(value)),
            serde_json::json!({"number": 1})
        );
    }
}
