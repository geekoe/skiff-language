//! Minimal UTC RFC3339 helpers for the health projection.
//!
//! The router avoids pulling a date-time crate into the production binary;
//! the runtime health frame `observedAt` is emitted in a fixed UTC shape
//! (`YYYY-MM-DDTHH:MM:SS[.mmm]Z`) and the loop-risk `observedAt`/`fresh`
//! projection only needs millisecond resolution UTC.

use std::time::{SystemTime, UNIX_EPOCH};

const MILLIS_PER_DAY: u64 = 86_400_000;

/// Formats a `SystemTime` as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC).
pub fn format_iso_millis(time: SystemTime) -> String {
    let millis = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let days = millis / MILLIS_PER_DAY;
    let (year, month, day) = civil_from_days(days as i64);
    let remainder = millis % MILLIS_PER_DAY;
    let hour = remainder / 3_600_000;
    let minute = (remainder % 3_600_000) / 60_000;
    let second = (remainder % 60_000) / 1_000;
    let millisecond = remainder % 1_000;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millisecond:03}Z")
}

/// Parses the UTC RFC3339 shapes emitted by the runtime and router
/// (`YYYY-MM-DDTHH:MM:SSZ` with 0 or 1-9 fractional-second digits, truncated
/// to milliseconds) into epoch milliseconds. Returns `None` for any other
/// shape (fail-closed freshness).
pub fn parse_iso_utc_millis(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let year = parse_fixed(&bytes[0..4])?;
    let month = parse_fixed(&bytes[5..7])?;
    let day = parse_fixed(&bytes[8..10])?;
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let hour = parse_fixed(&bytes[11..13])?;
    let minute = parse_fixed(&bytes[14..16])?;
    let second = parse_fixed(&bytes[17..19])?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut index = 19;
    let mut millis = 0_u64;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let mut digits = 0_u32;
        // RFC3339 permits any fractional precision; the runtime emits
        // 0/3/6/9 digits. Accept 1-9 digits and truncate to milliseconds
        // (the first three digits; fewer are right-padded with zeros).
        while digits < 9 {
            let digit = bytes.get(index).copied()?;
            if !digit.is_ascii_digit() {
                break;
            }
            if digits < 3 {
                millis = millis * 10 + u64::from(digit - b'0');
            }
            index += 1;
            digits += 1;
        }
        if digits == 0 {
            return None;
        }
        while digits < 3 {
            millis *= 10;
            digits += 1;
        }
    }
    if bytes.get(index) != Some(&b'Z') || bytes.len() != index + 1 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    Some(
        days as u64 * MILLIS_PER_DAY
            + u64::from(hour) * 3_600_000
            + u64::from(minute) * 60_000
            + u64::from(second) * 1_000
            + millis,
    )
}

fn parse_fixed(bytes: &[u8]) -> Option<u32> {
    if bytes.iter().any(|byte| !byte.is_ascii_digit()) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse::<u32>().ok()
}

/// Days since 1970-01-01 from a proleptic Gregorian civil date.
fn days_from_civil(year: u32, month: u32, day: u32) -> Option<u32> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = i64::from(year);
    let month = i64::from(month);
    let day = i64::from(day);
    let adjusted_year = if month <= 2 { year - 1 } else { year };
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    u32::try_from(days).ok()
}

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_runtime_and_router_shapes() {
        assert_eq!(
            parse_iso_utc_millis("2026-08-02T00:00:00Z"),
            Some(1_785_628_800_000)
        );
        assert_eq!(
            parse_iso_utc_millis("2026-08-02T00:00:00.123Z"),
            Some(1_785_628_800_123)
        );
    }

    #[test]
    fn parse_accepts_one_to_nine_fractional_digits_and_truncates_to_millis() {
        assert_eq!(
            parse_iso_utc_millis("2026-08-02T00:00:00.1Z"),
            Some(1_785_628_800_100)
        );
        assert_eq!(
            parse_iso_utc_millis("2026-08-02T00:00:00.12Z"),
            Some(1_785_628_800_120)
        );
        assert_eq!(
            parse_iso_utc_millis("2026-08-02T00:00:00.123456Z"),
            Some(1_785_628_800_123)
        );
        assert_eq!(
            parse_iso_utc_millis("2026-08-02T00:00:00.123456789Z"),
            Some(1_785_628_800_123)
        );
        // Six/four-digit truncation never rounds: extra digits are dropped.
        assert_eq!(
            parse_iso_utc_millis("2026-08-02T00:00:00.999999Z"),
            Some(1_785_628_800_999)
        );
    }

    #[test]
    fn parse_rejects_invalid_fractional_shapes() {
        for value in [
            "2026-08-02T00:00:00.Z",
            "2026-08-02T00:00:00..1Z",
            "2026-08-02T00:00:00.1aZ",
            "2026-08-02T00:00:00.1234567890Z",
            "2026-08-02T00:00:00.123Z ",
        ] {
            assert_eq!(parse_iso_utc_millis(value), None, "{value}");
        }
    }

    #[test]
    fn parse_rejects_malformed_shapes() {
        for value in [
            "2026-08-02T00:00:00",
            "2026-08-02 00:00:00Z",
            "2026-08-02T00:00:00+08:00",
            "2026-13-02T00:00:00Z",
            "not-a-time",
            "2026-08-02T24:00:00Z",
        ] {
            assert_eq!(parse_iso_utc_millis(value), None, "{value}");
        }
    }

    #[test]
    fn format_round_trips_through_parse() {
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(1_785_628_800_123);
        let formatted = format_iso_millis(time);
        assert_eq!(formatted, "2026-08-02T00:00:00.123Z");
        assert_eq!(parse_iso_utc_millis(&formatted), Some(1_785_628_800_123));
    }

    #[test]
    fn format_covers_epoch_and_recent_date() {
        assert_eq!(
            format_iso_millis(SystemTime::UNIX_EPOCH),
            "1970-01-01T00:00:00.000Z"
        );
        assert_eq!(
            format_iso_millis(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86_400)),
            "1970-01-02T00:00:00.000Z"
        );
    }
}
