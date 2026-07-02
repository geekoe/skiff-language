use serde_json::Value;

use crate::error::Result;

use super::support::{json_integer_i64, safe_integer_number, time_decode};

pub(crate) fn duration_milliseconds(args: &[Value]) -> Result<Value> {
    let ms = json_integer_i64(args.first(), "Duration.milliseconds")?;
    safe_integer_number(ms, "Duration.milliseconds")
}

pub(crate) fn duration_seconds(args: &[Value]) -> Result<Value> {
    let seconds = json_integer_i64(args.first(), "Duration.seconds")?;
    let ms = seconds
        .checked_mul(1_000)
        .ok_or_else(|| time_decode("Duration.seconds", "Duration.seconds overflow"))?;
    safe_integer_number(ms, "Duration.seconds")
}

pub(crate) fn duration_to_milliseconds(args: &[Value]) -> Result<Value> {
    let ms = json_integer_i64(args.first(), "Duration.toMilliseconds")?;
    safe_integer_number(ms, "Duration.toMilliseconds")
}
