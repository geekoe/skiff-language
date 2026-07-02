use serde::Serialize;
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

pub fn value_sha256(value: &Value) -> String {
    let canonical = canonical_json_value(value);
    let bytes = serde_json::to_vec(&canonical).expect("artifact values serialize");
    sha256_hex(&bytes)
}

pub fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonical_json_value).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            let mut sorted = Map::new();
            for key in keys {
                if let Some(nested) = object.get(key) {
                    sorted.insert(key.clone(), canonical_json_value(nested));
                }
            }
            Value::Object(sorted)
        }
        Value::Number(number) => canonical_json_number(number),
        _ => value.clone(),
    }
}

pub fn canonical_json_bytes<T: Serialize>(value: &T) -> serde_json::Result<Vec<u8>> {
    let value = serde_json::to_value(value)?;
    let canonical = canonical_json_value(&value);
    serde_json::to_vec(&canonical)
}

pub fn canonical_json_number(number: &Number) -> Value {
    if let Some(value) = number.as_i64() {
        return Value::Number(Number::from(value));
    }
    if let Some(value) = number.as_u64() {
        return Value::Number(Number::from(value));
    }
    if let Some(value) = number.as_f64() {
        if value.is_finite()
            && value.fract() == 0.0
            && value >= i64::MIN as f64
            && value <= i64::MAX as f64
        {
            return Value::Number(Number::from(value as i64));
        }
    }
    Value::Number(number.clone())
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
