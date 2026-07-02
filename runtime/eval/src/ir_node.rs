use serde_json::Value;

pub fn node_kind(node: &Value) -> Option<&str> {
    node.get("kind").and_then(Value::as_str)
}

pub fn slot_of(node: &Value) -> Option<usize> {
    slot_field(node, "slot")
}

fn slot_field(node: &Value, key: &str) -> Option<usize> {
    node.get(key).and_then(value_as_usize)
}

fn value_as_usize(value: &Value) -> Option<usize> {
    value
        .as_u64()
        .and_then(|number| usize::try_from(number).ok())
}
