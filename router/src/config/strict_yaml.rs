//! Strict YAML object parsing for the frozen Router process config contract.
//!
//! Mirrors the TypeScript `parseStrictYamlObject` contract (C-config): duplicate
//! keys, anchors, aliases and custom tags are rejected; every key must be a
//! plain `[A-Za-z_][A-Za-z0-9_-]*` segment; dotted keys are rejected; plain
//! scalars are resolved with the YAML 1.2 core schema used by the `yaml` npm
//! package (quoted scalars are always strings).
//!
//! Syntax coverage comes from `unsafe-libyaml` (already in the workspace lock
//! through `serde_yaml`). The raw parser is encapsulated in a small safe
//! wrapper that copies event data out of libyaml's buffers; all rejection and
//! value-construction logic below is safe Rust.

use std::collections::HashSet;
use std::ffi::CStr;
use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::slice;

#[allow(clippy::unsafe_removed_from_name)]
use unsafe_libyaml as sys;

/// A JSON-compatible value produced by the strict YAML parser. Numbers carry
/// JavaScript `Number` semantics (an `f64`), matching the `yaml` npm package's
/// core schema resolution.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub(crate) fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(entries) => entries
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub(crate) fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            JsonValue::Object(entries) => Some(entries),
            _ => None,
        }
    }

    pub(crate) fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(items) => Some(items),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::String(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn as_number(&self) -> Option<f64> {
        match self {
            JsonValue::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn is_null(&self) -> bool {
        matches!(self, JsonValue::Null)
    }
}

/// Parses a strict YAML object. On success the root is guaranteed to be an
/// object; every failure returns a message formatted like the TypeScript
/// parser (the caller prefixes it with the config path label).
pub(crate) fn parse_strict_yaml_object(text: &str, label: &str) -> Result<JsonValue, String> {
    let mut parser = Parser::new(text.as_bytes())?;
    let mut frames: Vec<Frame> = Vec::new();
    let mut root: Option<JsonValue> = None;
    let mut document_finished = false;
    let mut stream_end = false;
    // The TypeScript parser rejects YAML features in a deterministic walk
    // order; aliases win over anchors/tags when both are present. We defer
    // anchor/tag errors until parsing completes and return the alias error as
    // soon as an alias event arrives, matching the frozen corpus cases.
    let mut feature_error: Option<String> = None;

    while !stream_end {
        let event = parser
            .next()
            .map_err(|problem| format!("{label} config YAML parse error: {problem}"))?;
        match event {
            Event::StreamStart | Event::DocumentStart => {}
            Event::DocumentEnd => {
                document_finished = true;
            }
            Event::StreamEnd => {
                stream_end = true;
            }
            Event::Alias => {
                return Err(format!("{label} config YAML aliases are not supported"));
            }
            Event::Scalar {
                anchor,
                tag,
                value,
                style,
            } => {
                if document_finished {
                    continue;
                }
                if feature_error.is_none() {
                    if anchor.is_some() {
                        feature_error =
                            Some(format!("{label} config YAML anchors are not supported"));
                    } else if tag.is_some() {
                        feature_error = Some(format!("{label} config YAML tags are not supported"));
                    }
                }
                let path = current_node_path(&frames);
                let resolved = resolve_scalar(&value, style, label, &path)?;
                attach_scalar(&mut frames, &mut root, resolved, label)?;
            }
            Event::SequenceStart { anchor, tag } => {
                if document_finished {
                    continue;
                }
                if feature_error.is_none() {
                    if anchor.is_some() {
                        feature_error =
                            Some(format!("{label} config YAML anchors are not supported"));
                    } else if tag.is_some() {
                        feature_error = Some(format!("{label} config YAML tags are not supported"));
                    }
                }
                ensure_collection_allowed_as_key(&frames, label)?;
                let path = current_node_path(&frames);
                frames.push(Frame::Sequence(SequenceFrame {
                    items: Vec::new(),
                    path,
                }));
            }
            Event::MappingStart { anchor, tag } => {
                if document_finished {
                    continue;
                }
                if feature_error.is_none() {
                    if anchor.is_some() {
                        feature_error =
                            Some(format!("{label} config YAML anchors are not supported"));
                    } else if tag.is_some() {
                        feature_error = Some(format!("{label} config YAML tags are not supported"));
                    }
                }
                ensure_collection_allowed_as_key(&frames, label)?;
                let path = current_node_path(&frames);
                frames.push(Frame::Mapping(MappingFrame {
                    entries: Vec::new(),
                    seen: HashSet::new(),
                    path,
                    next_is_key: true,
                    pending_key: None,
                }));
            }
            Event::SequenceEnd => {
                if document_finished {
                    continue;
                }
                let completed = match frames.pop() {
                    Some(Frame::Sequence(sequence)) => JsonValue::Array(sequence.items),
                    _ => {
                        return Err(format!(
                            "{label} config YAML parse error: unexpected sequence end"
                        ));
                    }
                };
                attach_value(&mut frames, &mut root, completed, label)?;
            }
            Event::MappingEnd => {
                if document_finished {
                    continue;
                }
                let completed = match frames.pop() {
                    Some(Frame::Mapping(mapping)) => JsonValue::Object(mapping.entries),
                    _ => {
                        return Err(format!(
                            "{label} config YAML parse error: unexpected mapping end"
                        ));
                    }
                };
                attach_value(&mut frames, &mut root, completed, label)?;
            }
        }
    }

    if let Some(error) = feature_error {
        return Err(error);
    }
    match root {
        Some(JsonValue::Object(entries)) => Ok(JsonValue::Object(entries)),
        _ => Err(format!("{label} config root must be an object")),
    }
}

enum Frame {
    Mapping(MappingFrame),
    Sequence(SequenceFrame),
}

struct MappingFrame {
    entries: Vec<(String, JsonValue)>,
    seen: HashSet<String>,
    path: String,
    next_is_key: bool,
    pending_key: Option<String>,
}

struct SequenceFrame {
    items: Vec<JsonValue>,
    path: String,
}

fn ensure_collection_allowed_as_key(frames: &[Frame], label: &str) -> Result<(), String> {
    if let Some(Frame::Mapping(mapping)) = frames.last() {
        if mapping.next_is_key {
            return Err(format!(
                "{label} config key at {} must be a string",
                mapping.path
            ));
        }
    }
    Ok(())
}

fn attach_scalar(
    frames: &mut [Frame],
    root: &mut Option<JsonValue>,
    resolved: JsonValue,
    label: &str,
) -> Result<(), String> {
    let Some(Frame::Mapping(mapping)) = frames.last_mut() else {
        return attach_value(frames, root, resolved, label);
    };
    if !mapping.next_is_key {
        let key = mapping
            .pending_key
            .take()
            .expect("mapping value must follow a mapping key");
        mapping.entries.push((key, resolved));
        mapping.next_is_key = true;
        return Ok(());
    }

    let JsonValue::String(key) = resolved else {
        return Err(format!(
            "{label} config key at {} must be a string",
            mapping.path
        ));
    };
    if key.is_empty() {
        return Err(format!("{label} invalid config key {}", mapping.path));
    }
    let dotted = if mapping.path.is_empty() {
        key.clone()
    } else {
        format!("{}.{}", mapping.path, key)
    };
    if key.contains('.') {
        return Err(format!(
            "{label} invalid config key {dotted}: dotted YAML keys are not supported"
        ));
    }
    if !is_valid_key_segment(&key) {
        return Err(format!("{label} invalid config key {dotted}"));
    }
    if !mapping.seen.insert(key.clone()) {
        return Err(format!("{label} config YAML parse error: duplicate key"));
    }
    mapping.pending_key = Some(key);
    mapping.next_is_key = false;
    Ok(())
}

fn attach_value(
    frames: &mut [Frame],
    root: &mut Option<JsonValue>,
    value: JsonValue,
    label: &str,
) -> Result<(), String> {
    match frames.last_mut() {
        None => {
            *root = Some(value);
            Ok(())
        }
        Some(Frame::Sequence(sequence)) => {
            sequence.items.push(value);
            Ok(())
        }
        Some(Frame::Mapping(mapping)) => {
            if mapping.next_is_key {
                // libyaml never emits a value where a key is expected; this is
                // a defensive terminal for malformed event streams.
                Err(format!("{label} config {} must be an object", mapping.path))
            } else {
                let key = mapping
                    .pending_key
                    .take()
                    .expect("mapping value must follow a mapping key");
                mapping.entries.push((key, value));
                mapping.next_is_key = true;
                Ok(())
            }
        }
    }
}

fn current_node_path(frames: &[Frame]) -> String {
    match frames.last() {
        None => String::new(),
        Some(Frame::Sequence(sequence)) => format!("{}[{}]", sequence.path, sequence.items.len()),
        Some(Frame::Mapping(mapping)) => {
            if mapping.next_is_key {
                mapping.path.clone()
            } else {
                let key = mapping.pending_key.as_deref().unwrap_or("");
                if mapping.path.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", mapping.path, key)
                }
            }
        }
    }
}

fn is_valid_key_segment(key: &str) -> bool {
    let mut characters = key.chars();
    match characters.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    characters
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
}

fn resolve_scalar(
    value: &[u8],
    style: sys::yaml_scalar_style_t,
    label: &str,
    path: &str,
) -> Result<JsonValue, String> {
    let text = std::str::from_utf8(value)
        .map_err(|_| format!("{label} config {path} must be JSON-compatible"))?;
    if style != sys::YAML_PLAIN_SCALAR_STYLE {
        return Ok(JsonValue::String(text.to_string()));
    }
    if matches!(text, "" | "~" | "null" | "Null" | "NULL") {
        return Ok(JsonValue::Null);
    }
    if matches!(text, "true" | "True" | "TRUE") {
        return Ok(JsonValue::Bool(true));
    }
    if matches!(text, "false" | "False" | "FALSE") {
        return Ok(JsonValue::Bool(false));
    }
    if is_core_int(text) {
        return Ok(JsonValue::Number(parse_core_int(text)));
    }
    if is_core_float(text) {
        return Ok(JsonValue::Number(parse_core_float(text)));
    }
    Ok(JsonValue::String(text.to_string()))
}

/// YAML 1.2 core schema integer forms as implemented by the `yaml` npm package:
/// `[-+]?[0-9]+`, `0o[0-7]+`, `0x[0-9a-fA-F]+`.
fn is_core_int(text: &str) -> bool {
    if let Some(rest) = text.strip_prefix("0o") {
        return !rest.is_empty() && rest.bytes().all(|byte| (b'0'..=b'7').contains(&byte));
    }
    if let Some(rest) = text.strip_prefix("0x") {
        return !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// JavaScript `parseInt` semantics: sign is applied after radix conversion and
/// the result is the nearest `f64` (the config checks reject anything beyond
/// the safe-integer range, so exactness only matters within 2^53).
fn parse_core_int(text: &str) -> f64 {
    let (radix, digits, negative) = if let Some(rest) = text.strip_prefix("0o") {
        (8, rest, false)
    } else if let Some(rest) = text.strip_prefix("0x") {
        (16, rest, false)
    } else if let Some(rest) = text.strip_prefix('-') {
        (10, rest, true)
    } else {
        (10, text.strip_prefix('+').unwrap_or(text), false)
    };
    let mut value = 0.0;
    for character in digits.chars() {
        let digit = character.to_digit(radix).unwrap_or(0) as f64;
        value = value * radix as f64 + digit;
    }
    if negative {
        -value
    } else {
        value
    }
}

/// YAML 1.2 core schema float forms as implemented by the `yaml` npm package.
fn is_core_float(text: &str) -> bool {
    if is_core_nan_or_inf(text) {
        return true;
    }
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    if let Some(exp_index) = unsigned.find(['e', 'E']) {
        let (mantissa, exponent) = unsigned.split_at(exp_index);
        let exponent = &exponent[1..];
        let exponent_digits = exponent.strip_prefix(['-', '+']).unwrap_or(exponent);
        if exponent_digits.is_empty() || !exponent_digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            return false;
        }
        return is_core_exp_mantissa(mantissa);
    }
    is_core_plain_float(unsigned)
}

/// `(?:\.[0-9]+|[0-9]+\.[0-9]*)`
fn is_core_plain_float(text: &str) -> bool {
    if let Some(rest) = text.strip_prefix('.') {
        return !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit());
    }
    let Some(dot) = text.find('.') else {
        return false;
    };
    let (integer_part, fraction) = text.split_at(dot);
    let fraction = &fraction[1..];
    if integer_part.is_empty() || !integer_part.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    !fraction.contains('.') && fraction.bytes().all(|byte| byte.is_ascii_digit())
}

/// `(?:\.[0-9]+|[0-9]+(?:\.[0-9]*)?)` used before an exponent.
fn is_core_exp_mantissa(text: &str) -> bool {
    if let Some(rest) = text.strip_prefix('.') {
        return !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit());
    }
    let Some(dot) = text.find('.') else {
        return !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    };
    let (integer_part, fraction) = text.split_at(dot);
    let fraction = &fraction[1..];
    !integer_part.is_empty()
        && integer_part.bytes().all(|byte| byte.is_ascii_digit())
        && !fraction.contains('.')
        && fraction.bytes().all(|byte| byte.is_ascii_digit())
}

/// `[-+]?\.(inf|Inf|INF)` and `.nan|.NaN|.NAN` (sign is only allowed on inf).
fn is_core_nan_or_inf(text: &str) -> bool {
    if matches!(text, ".nan" | ".NaN" | ".NAN") {
        return true;
    }
    let signed = text.strip_prefix(['-', '+']).unwrap_or(text);
    matches!(signed, ".inf" | ".Inf" | ".INF")
}

fn parse_core_float(text: &str) -> f64 {
    if is_core_nan_or_inf(text) {
        if matches!(text, ".nan" | ".NaN" | ".NAN") {
            return f64::NAN;
        }
        return if text.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    match text.parse::<f64>() {
        Ok(value) => value,
        // JavaScript `parseFloat` overflows to Infinity.
        Err(_) => {
            if text.starts_with('-') {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        }
    }
}

/// A minimal safe wrapper over `unsafe-libyaml`'s event parser. Event data is
/// copied out of libyaml-owned buffers before the event is deleted.
struct Parser {
    pinned: Box<ParserPinned>,
}

struct ParserPinned {
    sys: MaybeUninit<sys::yaml_parser_t>,
    input: Vec<u8>,
}

enum Event {
    StreamStart,
    StreamEnd,
    DocumentStart,
    DocumentEnd,
    Alias,
    Scalar {
        anchor: Option<Vec<u8>>,
        tag: Option<Vec<u8>>,
        value: Vec<u8>,
        style: sys::yaml_scalar_style_t,
    },
    SequenceStart {
        anchor: Option<Vec<u8>>,
        tag: Option<Vec<u8>>,
    },
    SequenceEnd,
    MappingStart {
        anchor: Option<Vec<u8>>,
        tag: Option<Vec<u8>>,
    },
    MappingEnd,
}

impl Parser {
    fn new(input: &[u8]) -> Result<Self, String> {
        let mut pinned = Box::new(ParserPinned {
            sys: MaybeUninit::uninit(),
            input: input.to_vec(),
        });
        // SAFETY: `sys` is an uninitialized slot; `yaml_parser_initialize`
        // fully initializes it before any other parser function is called.
        let parser = pinned.sys.as_mut_ptr();
        let initialized = unsafe { sys::yaml_parser_initialize(parser) };
        if !initialized.ok {
            return Err("YAML parser initialization failed".to_string());
        }
        // SAFETY: the parser is initialized. The input pointer refers to
        // `pinned.input`, whose heap buffer outlives the parser (both live in
        // the same Box) and is never mutated or resized.
        unsafe {
            sys::yaml_parser_set_encoding(parser, sys::YAML_UTF8_ENCODING);
            sys::yaml_parser_set_input_string(
                parser,
                pinned.input.as_ptr(),
                pinned.input.len() as u64,
            );
        }
        Ok(Parser { pinned })
    }

    fn next(&mut self) -> Result<Event, String> {
        let mut event = MaybeUninit::<sys::yaml_event_t>::uninit();
        // SAFETY: `sys` is initialized by `Parser::new`; `event` is a zeroed
        // slot that libyaml fills before returning success.
        let parser = self.pinned.sys.as_mut_ptr();
        let event_ptr = event.as_mut_ptr();
        if unsafe { (&(*parser)).error } != sys::YAML_NO_ERROR {
            return Err(parser_problem(parser));
        }
        let parsed = unsafe { sys::yaml_parser_parse(parser, event_ptr) };
        if !parsed.ok {
            return Err(parser_problem(parser));
        }
        // SAFETY: `event_ptr` was successfully filled by `yaml_parser_parse`.
        let converted = unsafe { convert_event(&*event_ptr) };
        // SAFETY: the event was produced by `yaml_parser_parse`; libyaml owns
        // any heap data inside it (anchor/tag copies were already taken).
        unsafe { sys::yaml_event_delete(event_ptr) };
        Ok(converted)
    }
}

impl Drop for ParserPinned {
    fn drop(&mut self) {
        // SAFETY: `sys` was initialized in `Parser::new`; delete frees all
        // parser-owned buffers.
        unsafe { sys::yaml_parser_delete(self.sys.as_mut_ptr()) };
    }
}

fn parser_problem(parser: *mut sys::yaml_parser_t) -> String {
    // SAFETY: `problem` is a NUL-terminated string owned by the parser, or
    // null when no problem description is available (the prefix type exposes
    // the field through `yaml_parser_t`'s `Deref`).
    unsafe {
        let problem = (&(*parser)).problem;
        if problem.is_null() {
            return "YAML parse error".to_string();
        }
        CStr::from_ptr(problem).to_string_lossy().into_owned()
    }
}

// SAFETY: the returned event copies every libyaml-owned byte range (value,
// anchor, tag) before the event is deleted; nothing references libyaml memory.
unsafe fn convert_event(event: &sys::yaml_event_t) -> Event {
    match event.type_ {
        sys::YAML_STREAM_START_EVENT => Event::StreamStart,
        sys::YAML_STREAM_END_EVENT => Event::StreamEnd,
        sys::YAML_DOCUMENT_START_EVENT => Event::DocumentStart,
        sys::YAML_DOCUMENT_END_EVENT => Event::DocumentEnd,
        sys::YAML_ALIAS_EVENT => Event::Alias,
        sys::YAML_SCALAR_EVENT => Event::Scalar {
            anchor: copy_cstr(event.data.scalar.anchor),
            tag: copy_cstr(event.data.scalar.tag),
            value: copy_bytes(event.data.scalar.value, event.data.scalar.length),
            style: event.data.scalar.style,
        },
        sys::YAML_SEQUENCE_START_EVENT => Event::SequenceStart {
            anchor: copy_cstr(event.data.sequence_start.anchor),
            tag: copy_cstr(event.data.sequence_start.tag),
        },
        sys::YAML_SEQUENCE_END_EVENT => Event::SequenceEnd,
        sys::YAML_MAPPING_START_EVENT => Event::MappingStart {
            anchor: copy_cstr(event.data.mapping_start.anchor),
            tag: copy_cstr(event.data.mapping_start.tag),
        },
        sys::YAML_MAPPING_END_EVENT => Event::MappingEnd,
        sys::YAML_NO_EVENT | _ => Event::StreamEnd,
    }
}

// SAFETY: `pointer` must be a NUL-terminated string owned by libyaml (or
// null); the bytes are copied before the owning event is deleted.
unsafe fn copy_cstr(pointer: *mut u8) -> Option<Vec<u8>> {
    let non_null = NonNull::new(pointer)?;
    Some(
        CStr::from_ptr(non_null.as_ptr() as *const i8)
            .to_bytes()
            .to_vec(),
    )
}

// SAFETY: `pointer` must point to `length` valid bytes owned by libyaml (or
// be null when length is zero); the bytes are copied before deletion.
unsafe fn copy_bytes(pointer: *mut u8, length: u64) -> Vec<u8> {
    if pointer.is_null() {
        return Vec::new();
    }
    slice::from_raw_parts(pointer, length as usize).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<JsonValue, String> {
        parse_strict_yaml_object(text, "router config /tmp/router.yml")
    }

    #[test]
    fn parses_maps_sequences_and_core_scalars() {
        let value = parse(
            r#"
profile: dev
count: 3
ratio: 1.5
enabled: true
missing:
hex: 0x10
quoted: "4000"
list: [a, b]
http:
  port: 4000
"#,
        )
        .expect("valid YAML must parse");
        assert_eq!(
            value.get("profile").and_then(JsonValue::as_str),
            Some("dev")
        );
        assert_eq!(value.get("count").and_then(JsonValue::as_number), Some(3.0));
        assert_eq!(value.get("ratio").and_then(JsonValue::as_number), Some(1.5));
        assert!(matches!(value.get("enabled"), Some(JsonValue::Bool(true))));
        assert!(matches!(value.get("missing"), Some(JsonValue::Null)));
        assert_eq!(value.get("hex").and_then(JsonValue::as_number), Some(16.0));
        assert_eq!(
            value.get("quoted").and_then(JsonValue::as_str),
            Some("4000")
        );
        let list = value.get("list").and_then(JsonValue::as_array).unwrap();
        assert_eq!(list.len(), 2);
        let http = value.get("http").and_then(JsonValue::as_object).unwrap();
        assert_eq!(http[0].0, "port");
        assert_eq!(http[0].1.as_number(), Some(4000.0));
    }

    #[test]
    fn core_schema_keeps_non_core_scalars_as_strings() {
        let value = parse("yes: yes\non: on\nmaybe: y\nleading: 0123\nnan: .nan\n").unwrap();
        assert_eq!(value.get("yes").and_then(JsonValue::as_str), Some("yes"));
        assert_eq!(value.get("on").and_then(JsonValue::as_str), Some("on"));
        assert_eq!(value.get("maybe").and_then(JsonValue::as_str), Some("y"));
        assert_eq!(
            value.get("leading").and_then(JsonValue::as_number),
            Some(123.0)
        );
        assert!(value
            .get("nan")
            .and_then(JsonValue::as_number)
            .unwrap()
            .is_nan());
    }

    #[test]
    fn rejects_duplicate_keys() {
        let error = parse("a: 1\na: 2\n").expect_err("duplicate key must fail");
        assert!(error.contains("duplicate key"), "{error}");
    }

    #[test]
    fn rejects_anchors_aliases_and_tags() {
        let error = parse("host: &anchor 127.0.0.1\n").expect_err("anchor must fail");
        assert!(error.contains("YAML anchors are not supported"), "{error}");
        let error = parse("base: &b 1\nalias: *b\n").expect_err("alias must fail");
        assert!(error.contains("YAML aliases are not supported"), "{error}");
        let error = parse("value: !!str 20000\n").expect_err("tag must fail");
        assert!(error.contains("YAML tags are not supported"), "{error}");
    }

    #[test]
    fn rejects_dotted_and_invalid_keys() {
        let error = parse("a.b: 1\n").expect_err("dotted key must fail");
        assert!(
            error.contains("dotted YAML keys are not supported"),
            "{error}"
        );
        let error = parse("bad-key!$: 1\n").expect_err("invalid key must fail");
        assert!(error.contains("invalid config key"), "{error}");
    }

    #[test]
    fn rejects_non_object_roots() {
        let error = parse("[a, b]\n").expect_err("list root must fail");
        assert!(error.contains("config root must be an object"), "{error}");
        let error = parse("just-a-string\n").expect_err("scalar root must fail");
        assert!(error.contains("config root must be an object"), "{error}");
        let error = parse("").expect_err("empty root must fail");
        assert!(error.contains("config root must be an object"), "{error}");
    }

    #[test]
    fn rejects_malformed_yaml() {
        let error = parse("artifactsPath: [unclosed\n").expect_err("malformed YAML must fail");
        assert!(error.contains("YAML parse error"), "{error}");
    }

    #[test]
    fn rejects_non_string_keys() {
        let error = parse("4000: value\n").expect_err("numeric key must fail");
        assert!(error.contains("config key at  must be a string"), "{error}");
    }

    #[test]
    fn comments_and_blank_lines_are_supported() {
        let value = parse("# leading comment\nprofile: dev # trailing\n\nhttp:\n  port: 4000\n")
            .expect("comments must parse");
        assert_eq!(
            value.get("profile").and_then(JsonValue::as_str),
            Some("dev")
        );
    }

    #[test]
    fn secret_like_plain_scalars_are_not_mistaken_for_yaml_features() {
        let value = parse("mongoUrl: mongodb://user:p!ss&word@127.0.0.1/skiff\nbucket: a&b*c\n")
            .expect("plain scalars with & ! * must stay strings");
        assert_eq!(
            value.get("mongoUrl").and_then(JsonValue::as_str),
            Some("mongodb://user:p!ss&word@127.0.0.1/skiff")
        );
        assert_eq!(
            value.get("bucket").and_then(JsonValue::as_str),
            Some("a&b*c")
        );
    }
}
