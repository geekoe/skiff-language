use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::error::{Result, RuntimeError};

pub const MIN_EPOCH_MILLIS: i64 = -62_167_219_200_000;
pub const MAX_EPOCH_MILLIS: i64 = 253_402_300_799_999;

pub fn is_valid_epoch_millis(ms: i64) -> bool {
    (MIN_EPOCH_MILLIS..=MAX_EPOCH_MILLIS).contains(&ms)
}

pub fn validate_epoch_millis(ms: i64, target: &str) -> Result<i64> {
    if is_valid_epoch_millis(ms) {
        Ok(ms)
    } else {
        Err(RuntimeError::decode_target(
            target,
            format!("{target} Date is outside RFC3339 year range 0000..9999"),
        ))
    }
}

pub fn parse_rfc3339_millis(input: &str, target: &str) -> Result<i64> {
    if has_leap_second(input) {
        return Err(RuntimeError::decode_target(
            target,
            format!("{target} rejects RFC3339 leap seconds"),
        ));
    }
    let parsed = OffsetDateTime::parse(input, &Rfc3339).map_err(|error| {
        RuntimeError::decode_target(target, format!("{target} requires RFC3339 Date: {error}"))
    })?;
    let seconds = parsed.unix_timestamp();
    let millis = seconds
        .checked_mul(1000)
        .and_then(|value| value.checked_add(i64::from(parsed.nanosecond() / 1_000_000)))
        .ok_or_else(|| {
            RuntimeError::decode_target(
                target,
                format!("{target} Date epoch milliseconds overflow"),
            )
        })?;
    validate_epoch_millis(millis, target)
}

pub fn try_parse_rfc3339_millis(input: &str) -> Option<i64> {
    parse_rfc3339_millis(input, "Date.parse").ok()
}

pub fn format_epoch_millis(ms: i64, target: &str) -> Result<String> {
    validate_epoch_millis(ms, target)?;
    let seconds = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let date = OffsetDateTime::from_unix_timestamp(seconds)
        .map_err(|error| {
            RuntimeError::decode_target(target, format!("{target} Date cannot format: {error}"))
        })?
        .replace_nanosecond((millis as u32) * 1_000_000)
        .map_err(|error| {
            RuntimeError::decode_target(target, format!("{target} Date cannot format: {error}"))
        })?;
    Ok(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        date.year(),
        u8::from(date.month()),
        date.day(),
        date.hour(),
        date.minute(),
        date.second(),
        millis,
    ))
}

fn has_leap_second(input: &str) -> bool {
    let Some(time_start) = input.find(['T', 't']) else {
        return false;
    };
    let time = &input[time_start + 1..];
    let mut parts = time.splitn(3, ':');
    let (Some(_hour), Some(_minute), Some(seconds_and_rest)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    seconds_and_rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        == "60"
}
