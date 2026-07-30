use serde_json::{json, Value};

pub(super) fn named(name: &str) -> Value {
    json!({ "kind": "builtin", "name": name, "args": [] })
}

pub(super) fn generic(name: &str, args: Vec<Value>) -> Value {
    json!({ "kind": "builtin", "name": name, "args": args })
}

pub(super) fn array(inner: Value) -> Value {
    json!({ "kind": "builtin", "name": "Array", "args": [inner] })
}

pub(super) fn map(key: Value, value: Value) -> Value {
    json!({ "kind": "builtin", "name": "Map", "args": [key, value] })
}

pub(super) fn union(types: Vec<Value>) -> Value {
    json!({ "kind": "union", "items": types })
}

pub(super) fn nullable(inner: Value) -> Value {
    json!({ "kind": "nullable", "inner": inner })
}

pub(super) fn record(name: &str, fields: Vec<(&str, Value)>) -> Value {
    json!({
        "kind": "builtin",
        "name": name,
        "args": [],
        "fields": fields
            .into_iter()
            .map(|(name, ty)| (name.to_string(), ty))
            .collect::<serde_json::Map<_, _>>()
    })
}

pub(super) fn alias(name: &str, target: Value) -> Value {
    json!({ "kind": "alias", "name": name, "target": target })
}

pub(super) fn representation(name: &str, payload: Value) -> Value {
    json!({ "kind": "representation", "name": name, "representation": payload })
}
