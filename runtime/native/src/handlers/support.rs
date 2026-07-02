use serde_json::{Number, Value};

use crate::error::{Result, RuntimeError};

pub(super) fn number_value(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

pub(super) fn is_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0
}

pub(super) fn is_safe_integer(value: f64) -> bool {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    is_integer(value) && value.abs() <= MAX_SAFE_INTEGER
}

pub(super) fn safe_integer_number(value: i64, target: &str) -> Result<Value> {
    const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
    if value < -MAX_SAFE_INTEGER || value > MAX_SAFE_INTEGER {
        return Err(time_decode(
            target,
            format!("{target} requires a safe integer"),
        ));
    }
    Ok(Value::Number(Number::from(value)))
}

pub(super) fn json_integer_i64(value: Option<&Value>, target: &str) -> Result<i64> {
    let Some(value) = value else {
        return Err(time_decode(target, format!("{target} requires an integer")));
    };
    if let Some(value) = value.as_i64() {
        return Ok(value);
    }
    let number = value
        .as_f64()
        .ok_or_else(|| time_decode(target, format!("{target} requires an integer")))?;
    if !is_safe_integer(number) {
        return Err(time_decode(
            target,
            format!("{target} requires a safe integer"),
        ));
    }
    Ok(number as i64)
}

pub(super) fn number_decode(target: impl Into<String>, message: impl Into<String>) -> RuntimeError {
    RuntimeError::decode_target(target, message)
}

pub(super) fn time_decode(target: impl Into<String>, message: impl Into<String>) -> RuntimeError {
    RuntimeError::decode_target(target, message)
}
