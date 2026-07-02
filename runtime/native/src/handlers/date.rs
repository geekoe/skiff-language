use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use skiff_runtime_boundary::date_value::{
    format_epoch_millis, parse_rfc3339_millis, try_parse_rfc3339_millis, validate_epoch_millis,
};

use crate::error::Result;

use super::support::{json_integer_i64, time_decode};

pub(crate) fn date_now(_args: &[Value]) -> Result<Value> {
    let ms = now_epoch_millis();
    Ok(Value::String(format_epoch_millis(ms, "Date.now")?))
}

pub(crate) fn date_from_epoch_milliseconds(args: &[Value]) -> Result<Value> {
    let ms = json_integer_i64(args.first(), "Date.fromEpochMilliseconds")?;
    let ms = validate_epoch_millis(ms, "Date.fromEpochMilliseconds")?;
    Ok(Value::String(format_epoch_millis(
        ms,
        "Date.fromEpochMilliseconds",
    )?))
}

pub(crate) fn date_parse(args: &[Value]) -> Result<Value> {
    let value = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| time_decode("Date.parse", "Date.parse requires a string"))?;
    Ok(try_parse_rfc3339_millis(value)
        .map(|ms| format_epoch_millis(ms, "Date.parse"))
        .transpose()?
        .map(Value::String)
        .unwrap_or(Value::Null))
}

pub(crate) fn date_require_parse(args: &[Value]) -> Result<Value> {
    let value = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| time_decode("Date.requireParse", "Date.requireParse requires a string"))?;
    let ms = parse_rfc3339_millis(value, "Date.requireParse")?;
    Ok(Value::String(format_epoch_millis(ms, "Date.requireParse")?))
}

fn now_epoch_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            let millis = duration.as_millis().min(i64::MAX as u128);
            millis as i64
        }
        Err(error) => {
            let millis = error.duration().as_millis().min(i64::MAX as u128);
            -(millis as i64)
        }
    }
}
