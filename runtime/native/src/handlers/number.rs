use serde_json::{Number, Value};

use crate::error::{Result, RuntimeError};

use super::support::{is_integer, is_safe_integer, number_decode, number_value};

pub(crate) fn number_parse(args: &[Value]) -> Result<Value> {
    let value = args.first().and_then(Value::as_str).unwrap_or("").trim();
    if value.is_empty() {
        return Ok(Value::Null);
    }
    let parsed = value
        .parse::<f64>()
        .map_err(|_| number_decode("number.parse", "number.parse requires a numeric string"))?;
    if !parsed.is_finite() {
        return Err(number_decode(
            "number.parse",
            "number.parse requires a finite numeric string",
        ));
    }
    Ok(number_value(parsed))
}

pub(crate) fn number_is_integer(args: &[Value]) -> Result<Value> {
    let value = args
        .first()
        .and_then(Value::as_f64)
        .ok_or_else(|| RuntimeError::Decode("number.isInteger requires a number".to_string()))?;
    Ok(Value::Bool(is_integer(value)))
}

pub(crate) fn number_is_safe_integer(args: &[Value]) -> Result<Value> {
    let value = args.first().and_then(Value::as_f64).ok_or_else(|| {
        RuntimeError::Decode("number.isSafeInteger requires a number".to_string())
    })?;
    Ok(Value::Bool(is_safe_integer(value)))
}

pub(crate) fn number_assert_safe_integer(args: &[Value]) -> Result<Value> {
    let value = args.first().and_then(Value::as_f64).ok_or_else(|| {
        RuntimeError::Decode("number.assertSafeInteger requires a number".to_string())
    })?;
    if !is_safe_integer(value) {
        return Err(number_decode(
            "number.assertSafeInteger",
            "number.assertSafeInteger requires a safe integer",
        ));
    }
    Ok(Value::Number(Number::from(value as i64)))
}
