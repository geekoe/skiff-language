use serde_json::Value;

use crate::error::{Result, RuntimeError};

pub(super) fn max_safe_json_integer() -> f64 {
    9_007_199_254_740_991.0
}

pub(super) fn integer_json(value: f64) -> Result<Value> {
    if value.is_finite() && value.fract() == 0.0 && value.abs() <= max_safe_json_integer() {
        return Ok(Value::Number((value as i64).into()));
    }
    Err(RuntimeError::Decode("expected safe integer".to_string()))
}

pub(super) fn number_json(value: f64) -> Result<Value> {
    if value.is_finite() && value.fract() == 0.0 && value.abs() <= max_safe_json_integer() {
        return Ok(Value::Number((value as i64).into()));
    }
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| RuntimeError::Decode("number is not finite".to_string()))
}
