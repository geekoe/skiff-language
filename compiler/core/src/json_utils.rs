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
mod tests {
    use serde_json::{json, Map, Number, Value};

    use super::*;

    #[test]
    fn compiler_canonical_helpers_delegate_number_normalization() {
        let mut value = Map::new();
        value.insert(
            "number".to_string(),
            Value::Number(Number::from_f64(1.0).expect("number")),
        );

        assert_eq!(
            canonical_json_bytes(&Value::Object(value)).expect("canonical bytes"),
            br#"{"number":1}"#
        );
    }

    #[test]
    fn stable_json_is_explicitly_sort_only() {
        let number = Number::from_f64(1.0).expect("number");
        let value = Value::Object(Map::from_iter([
            ("z".to_string(), json!(2)),
            ("a".to_string(), Value::Number(number)),
        ]));

        assert_eq!(stable_json_string(&value), r#"{"a":1.0,"z":2}"#);
        assert_eq!(
            serde_json::to_string(&canonical_json_value(&value)).expect("canonical JSON"),
            r#"{"a":1,"z":2}"#
        );
    }
}
