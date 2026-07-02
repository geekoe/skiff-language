use serde_json::Value;

use crate::error::{Result, RuntimeError};

use super::support::is_safe_integer;

pub(crate) fn string_join(args: &[Value]) -> Result<Value> {
    let items = args
        .first()
        .and_then(Value::as_array)
        .ok_or_else(|| RuntimeError::Decode("string.join requires an array".to_string()))?;
    let separator = args.get(1).and_then(Value::as_str).ok_or_else(|| {
        RuntimeError::Decode("string.join separator must be a string".to_string())
    })?;
    let mut strings = Vec::with_capacity(items.len());
    for item in items {
        let value = item.as_str().ok_or_else(|| {
            RuntimeError::Decode("string.join items must all be strings".to_string())
        })?;
        strings.push(value);
    }
    Ok(Value::String(strings.join(separator)))
}

pub(crate) fn string_split(args: &[Value]) -> Result<Value> {
    let value = args.first().and_then(Value::as_str).ok_or_else(|| {
        RuntimeError::Decode("std.string.split value must be a string".to_string())
    })?;
    let separator = args.get(1).and_then(Value::as_str).ok_or_else(|| {
        RuntimeError::Decode("std.string.split separator must be a string".to_string())
    })?;
    if separator.is_empty() {
        return Err(RuntimeError::Decode(
            "std.string.split separator must not be empty".to_string(),
        ));
    }
    Ok(Value::Array(
        value
            .split(separator)
            .map(|part| Value::String(part.to_string()))
            .collect(),
    ))
}

pub(crate) fn string_is_ascii_digits(args: &[Value]) -> Result<Value> {
    let value = args.first().and_then(Value::as_str).ok_or_else(|| {
        RuntimeError::Decode("std.string.isAsciiDigits requires a string".to_string())
    })?;
    Ok(Value::Bool(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
    ))
}

pub(crate) fn string_truncate_utf8_bytes(args: &[Value]) -> Result<Value> {
    let value = args.first().and_then(Value::as_str).ok_or_else(|| {
        RuntimeError::Decode("std.string.truncateUtf8Bytes value must be a string".to_string())
    })?;
    let max_bytes = args.get(1).and_then(Value::as_f64).ok_or_else(|| {
        RuntimeError::Decode("std.string.truncateUtf8Bytes maxBytes must be a number".to_string())
    })?;
    if !max_bytes.is_finite() {
        return Err(RuntimeError::Decode(
            "std.string.truncateUtf8Bytes maxBytes must be finite".to_string(),
        ));
    }
    if max_bytes <= 0.0 {
        return Ok(Value::String(String::new()));
    }
    if !is_safe_integer(max_bytes) {
        return Err(RuntimeError::Decode(
            "std.string.truncateUtf8Bytes maxBytes must be a safe integer".to_string(),
        ));
    }
    let max_bytes = usize::try_from(max_bytes as u64).map_err(|_| {
        RuntimeError::Decode(
            "std.string.truncateUtf8Bytes maxBytes must fit within system size".to_string(),
        )
    })?;
    if value.len() <= max_bytes {
        return Ok(Value::String(value.to_string()));
    }

    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    Ok(Value::String(value[..end].to_string()))
}

pub(crate) fn string_encode_query_component(args: &[Value]) -> Result<Value> {
    let value = args.first().and_then(Value::as_str).ok_or_else(|| {
        RuntimeError::Decode("std.string.encodeQueryComponent requires a string".to_string())
    })?;
    Ok(Value::String(percent_encode_query_component(value)))
}

pub(crate) fn string_encode_path(args: &[Value]) -> Result<Value> {
    let value = args.first().and_then(Value::as_str).ok_or_else(|| {
        RuntimeError::Decode("std.string.encodePath requires a string".to_string())
    })?;
    Ok(Value::String(percent_encode_path(value)))
}

fn percent_encode_query_component(value: &str) -> String {
    percent_encode(value, false)
}

fn percent_encode_path(value: &str) -> String {
    percent_encode(value, true)
}

fn percent_encode(value: &str, keep_slash: bool) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (keep_slash && byte == b'/')
        {
            output.push(byte as char);
        } else {
            output.push('%');
            output.push(HEX[(byte >> 4) as usize] as char);
            output.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    output
}
