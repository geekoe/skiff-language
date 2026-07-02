use serde_json::{Map, Value};

pub const STREAM_ID_KEY: &str = "__skiffStreamId";

pub fn stream_value(id: &str) -> Value {
    let mut object = Map::new();
    object.insert(STREAM_ID_KEY.to_string(), Value::String(id.to_string()));
    Value::Object(object)
}

pub fn is_stream_value(value: &Value) -> bool {
    stream_id(value).is_some()
}

pub fn stream_id(value: &Value) -> Option<&str> {
    value.as_object()?.get(STREAM_ID_KEY)?.as_str()
}
