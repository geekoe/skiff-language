use super::*;

#[test]
fn date_parse_accepts_offsets_and_formats_utc() {
    let millis = parse_rfc3339_millis("2026-06-04T12:34:56.789+08:00", "test").unwrap();
    assert_eq!(
        format_epoch_millis(millis, "test").unwrap(),
        "2026-06-04T04:34:56.789Z"
    );
}

#[test]
fn date_parse_rejects_leap_second() {
    assert!(parse_rfc3339_millis("2026-06-04T23:59:60Z", "test").is_err());
    assert!(try_parse_rfc3339_millis("2026-06-04T23:59:60Z").is_none());
}

#[test]
fn date_range_matches_rfc3339_four_digit_years() {
    assert_eq!(
        format_epoch_millis(MIN_EPOCH_MILLIS, "test").unwrap(),
        "0000-01-01T00:00:00.000Z"
    );
    assert_eq!(
        format_epoch_millis(MAX_EPOCH_MILLIS, "test").unwrap(),
        "9999-12-31T23:59:59.999Z"
    );
    assert!(validate_epoch_millis(MIN_EPOCH_MILLIS - 1, "test").is_err());
    assert!(validate_epoch_millis(MAX_EPOCH_MILLIS + 1, "test").is_err());
}
