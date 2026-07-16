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
mod tests {
    use serde_json::{json, Map, Number, Value};

    use super::*;

    #[test]
    fn recursively_orders_keys_without_reordering_arrays() {
        let mut nested = Map::new();
        nested.insert("z".to_string(), json!(1));
        nested.insert("a".to_string(), json!(2));
        let mut root = Map::new();
        root.insert("z".to_string(), Value::Object(nested));
        root.insert("a".to_string(), json!([{"b": 1, "a": 2}, 3]));

        assert_eq!(
            canonical_json_bytes(&Value::Object(root)).expect("canonical bytes"),
            br#"{"a":[{"a":2,"b":1},3],"z":{"a":2,"z":1}}"#
        );
    }

    #[test]
    fn normalizes_integral_floats_but_preserves_fractional_numbers() {
        let integral = Number::from_f64(42.0).expect("integral float");
        let fractional = Number::from_f64(42.5).expect("fractional float");

        assert_eq!(canonical_json_number(&integral), json!(42));
        assert_eq!(
            canonical_json_number(&fractional),
            Value::Number(fractional)
        );
    }

    #[test]
    fn insertion_order_does_not_change_bytes() {
        let left = json!({"a": 1, "b": {"x": 2, "y": 3}});
        let mut nested = Map::new();
        nested.insert("y".to_string(), json!(3));
        nested.insert("x".to_string(), json!(2));
        let mut right = Map::new();
        right.insert("b".to_string(), Value::Object(nested));
        right.insert("a".to_string(), json!(1));

        assert_eq!(
            canonical_json_bytes(&left).expect("left bytes"),
            canonical_json_bytes(&Value::Object(right)).expect("right bytes")
        );
    }
}
