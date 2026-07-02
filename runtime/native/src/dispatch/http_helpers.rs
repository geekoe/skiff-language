use std::collections::BTreeSet;

use serde_json::Value;

use crate::runtime_value_facade::bytes_value;

pub(super) fn http_response_wire(status: u16, headers: Value, body: Vec<u8>) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("status".to_string(), Value::Number(status.into()));
    object.insert("headers".to_string(), headers);
    object.insert("body".to_string(), bytes_value(&body));
    Value::Object(object)
}

pub(super) fn json_headers() -> Value {
    Value::Array(vec![json_header(
        "content-type",
        "application/json; charset=utf-8",
    )])
}

pub(super) fn ensure_json_content_type(headers: &mut Value) {
    let Some(items) = headers.as_array_mut() else {
        return;
    };
    if items.iter().any(|item| {
        item.as_object()
            .and_then(|object| object.get("name"))
            .and_then(Value::as_str)
            .is_some_and(|name| name.eq_ignore_ascii_case("content-type"))
    }) {
        return;
    }
    items.push(json_header(
        "content-type",
        "application/json; charset=utf-8",
    ));
}

fn json_header(name: &str, value: &str) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("name".to_string(), Value::String(name.to_string()));
    object.insert("value".to_string(), Value::String(value.to_string()));
    Value::Object(object)
}

#[derive(Clone, Copy)]
pub(super) enum NameMatch {
    Exact,
    AsciiCaseInsensitive,
}

pub(super) fn optional_string_value(value: Option<String>) -> Value {
    value.map(Value::String).unwrap_or(Value::Null)
}

pub(super) fn first_name_value(
    object: &Value,
    field: &str,
    name: &str,
    name_match: NameMatch,
) -> Option<String> {
    name_values(object, field, name, name_match)
        .into_iter()
        .next()
}

pub(super) fn name_values(
    object: &Value,
    field: &str,
    name: &str,
    name_match: NameMatch,
) -> Vec<String> {
    object
        .as_object()
        .and_then(|object| object.get(field))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let object = item.as_object()?;
            let item_name = object.get("name").and_then(Value::as_str)?;
            if !name_matches(item_name, name, name_match) {
                return None;
            }
            object
                .get("value")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

fn name_matches(actual: &str, expected: &str, name_match: NameMatch) -> bool {
    match name_match {
        NameMatch::Exact => actual == expected,
        NameMatch::AsciiCaseInsensitive => actual.eq_ignore_ascii_case(expected),
    }
}

pub(super) fn cookie_value(headers: &[String], name: &str) -> Option<String> {
    headers
        .iter()
        .flat_map(|header| header.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(cookie_name, value)| {
            (cookie_name.trim() == name).then(|| value.trim().to_string())
        })
}

pub(super) fn http_method_not_allowed_wire(allow: &str) -> Value {
    http_response_wire(
        405,
        Value::Array(vec![json_header("allow", allow)]),
        Vec::new(),
    )
}

pub(super) fn forwardable_headers(headers: &[Value]) -> Value {
    let mut blocked = BTreeSet::from([
        "connection".to_string(),
        "content-length".to_string(),
        "keep-alive".to_string(),
        "proxy-authenticate".to_string(),
        "proxy-authorization".to_string(),
        "te".to_string(),
        "trailer".to_string(),
        "transfer-encoding".to_string(),
        "upgrade".to_string(),
    ]);
    for header in headers {
        let Some(name) = header
            .as_object()
            .and_then(|object| object.get("name"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if !name.eq_ignore_ascii_case("connection") {
            continue;
        }
        if let Some(value) = header
            .as_object()
            .and_then(|object| object.get("value"))
            .and_then(Value::as_str)
        {
            blocked.extend(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .map(|token| token.to_ascii_lowercase()),
            );
        }
    }
    Value::Array(
        headers
            .iter()
            .filter(|header| {
                header
                    .as_object()
                    .and_then(|object| object.get("name"))
                    .and_then(Value::as_str)
                    .map(|name| !blocked.contains(&name.to_ascii_lowercase()))
                    .unwrap_or(false)
            })
            .cloned()
            .collect(),
    )
}

pub(super) fn sse_headers() -> Value {
    Value::Array(vec![
        json_header("content-type", "text/event-stream; charset=utf-8"),
        json_header("cache-control", "no-cache"),
        json_header("connection", "keep-alive"),
    ])
}
