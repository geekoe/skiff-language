use serde_json::Value;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use super::RequestEnvelope;

pub fn request_deadline_ms(request: &RequestEnvelope) -> Option<u64> {
    let timeout_ms = request
        .extra
        .get("deadline")
        .and_then(Value::as_object)
        .and_then(|deadline| deadline.get("timeoutMs"))
        .and_then(Value::as_u64);
    let Some(expires_at) = request
        .extra
        .get("deadline")
        .and_then(Value::as_object)
        .and_then(|deadline| deadline.get("expiresAt"))
        .and_then(Value::as_str)
    else {
        return timeout_ms;
    };
    let Ok(expires_at) = OffsetDateTime::parse(expires_at, &Rfc3339) else {
        return timeout_ms;
    };
    let now = OffsetDateTime::now_utc();
    if expires_at <= now {
        return Some(0);
    }
    let remaining_ms = (expires_at - now).whole_milliseconds();
    let remaining_ms = remaining_ms.try_into().unwrap_or(u64::MAX);
    Some(timeout_ms.map_or(remaining_ms, |timeout_ms| timeout_ms.min(remaining_ms)))
}

#[cfg(test)]
mod tests;
