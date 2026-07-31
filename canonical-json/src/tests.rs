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
