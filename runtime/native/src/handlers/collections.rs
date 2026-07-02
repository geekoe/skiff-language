use serde_json::{Map, Value};

use crate::error::Result;

pub(crate) fn array_empty(_args: &[Value]) -> Result<Value> {
    Ok(Value::Array(Vec::new()))
}

pub(crate) fn map_empty(_args: &[Value]) -> Result<Value> {
    Ok(Value::Object(Map::new()))
}
