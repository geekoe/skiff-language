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
