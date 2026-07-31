use serde::Serialize;
use serde_json::{Map, Number, Value};

/// Returns the canonical JSON value used by Skiff semantic identities.
///
/// Object keys are ordered recursively and integral JSON numbers are normalized
/// to their integer representation. Array order remains significant.
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

/// Normalizes a JSON number without applying any artifact-specific policy.
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

/// Serializes a value to canonical JSON bytes.
pub fn canonical_json_bytes<T: Serialize>(value: &T) -> serde_json::Result<Vec<u8>> {
    let value = serde_json::to_value(value)?;
    serde_json::to_vec(&canonical_json_value(&value))
}

#[cfg(test)]
mod tests;
