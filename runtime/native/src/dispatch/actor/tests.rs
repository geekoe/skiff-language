use serde_json::{json, Map, Value};

use super::actor_id_key;

#[test]
fn actor_id_key_uses_canonical_value_bytes() {
    let left = json!({"a": 1, "b": {"x": 2, "y": 3}});
    let mut nested = Map::new();
    nested.insert("y".to_string(), json!(3));
    nested.insert("x".to_string(), json!(2));
    let mut right = Map::new();
    right.insert("b".to_string(), Value::Object(nested));
    right.insert("a".to_string(), json!(1));

    assert_eq!(
        actor_id_key(&left).expect("left id encodes"),
        actor_id_key(&Value::Object(right)).expect("right id encodes")
    );
}
