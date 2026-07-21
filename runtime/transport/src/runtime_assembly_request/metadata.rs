use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use skiff_canonical_json::canonical_json_number;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestClientSessionFrameHeader {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestDeadlineFrameHeader {
    #[serde(deserialize_with = "deserialize_safe_unsigned_integer")]
    pub timeout_ms: u64,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestTraceFrameHeader {
    pub trace_id: String,
    pub span_id: String,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_span_id: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub sampled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestNameValueFrameHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyHttpRequestFrameHeader {
    pub method: String,
    pub url: String,
    pub path: String,
    pub query: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    pub headers: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeAssemblyHttpAdapterKindFrameHeader {
    TypedJson,
    RawHttp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RuntimeAssemblyHttpAdapterCallableFrameHeader {
    ServiceFunction {
        module_path: String,
        symbol: String,
    },
    PackageFunction {
        package_id: String,
        symbol_path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAssemblyHttpAdapterSourceFrameHeader {
    pub kind: RuntimeAssemblyHttpAdapterSourceKindFrameHeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeAssemblyHttpAdapterSourceKindFrameHeader {
    #[serde(rename = "http.request")]
    Request,
    #[serde(rename = "http.body")]
    Body,
    #[serde(rename = "http.context")]
    Context,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyHttpAdapterArgFrameHeader {
    pub param: String,
    pub source: RuntimeAssemblyHttpAdapterSourceFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyHttpAdapterFrameHeader {
    pub kind: RuntimeAssemblyHttpAdapterKindFrameHeader,
    pub handler: RuntimeAssemblyHttpAdapterCallableFrameHeader,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub guard: Option<RuntimeAssemblyHttpAdapterCallableFrameHeader>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub pre: Option<RuntimeAssemblyHttpAdapterCallableFrameHeader>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapter_args: Vec<RuntimeAssemblyHttpAdapterArgFrameHeader>,
}

impl RuntimeAssemblyHttpAdapterFrameHeader {
    fn validate(&self) -> Result<(), String> {
        validate_adapter_params(
            self.adapter_args.iter().map(|arg| arg.param.as_str()),
            "httpAdapter.adapterArgs",
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeAssemblyWebSocketAdapterKindFrameHeader {
    Connect,
    Receive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketAdapterSourceFrameHeader {
    pub kind: RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader {
    #[serde(rename = "websocket.connectRequest")]
    ConnectRequest,
    #[serde(rename = "websocket.receiveEvent")]
    ReceiveEvent,
    #[serde(rename = "websocket.connection")]
    Connection,
    #[serde(rename = "websocket.connectionContext")]
    ConnectionContext,
    #[serde(rename = "websocket.message")]
    Message,
    #[serde(rename = "websocket.messageBody")]
    MessageBody,
    #[serde(rename = "websocket.connectionId")]
    ConnectionId,
    #[serde(rename = "websocket.businessIdentity")]
    BusinessIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketAdapterArgFrameHeader {
    pub param: String,
    pub source: RuntimeAssemblyWebSocketAdapterSourceFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RuntimeAssemblyWebSocketContextExpectationFrameHeader {
    Null,
    Typed {
        connect_operation_abi_id: String,
        context_type_identity: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyWebSocketContextExpectationFrameHeader {
    kind: RuntimeAssemblyWebSocketContextExpectationKindFrameHeader,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    connect_operation_abi_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    context_type_identity: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum RuntimeAssemblyWebSocketContextExpectationKindFrameHeader {
    Null,
    Typed,
}

impl<'de> Deserialize<'de> for RuntimeAssemblyWebSocketContextExpectationFrameHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw =
            RawRuntimeAssemblyWebSocketContextExpectationFrameHeader::deserialize(deserializer)?;
        match (
            raw.kind,
            raw.connect_operation_abi_id,
            raw.context_type_identity,
        ) {
            (RuntimeAssemblyWebSocketContextExpectationKindFrameHeader::Null, None, None) => {
                Ok(Self::Null)
            }
            (
                RuntimeAssemblyWebSocketContextExpectationKindFrameHeader::Typed,
                Some(connect_operation_abi_id),
                Some(context_type_identity),
            ) => Ok(Self::Typed {
                connect_operation_abi_id,
                context_type_identity,
            }),
            (RuntimeAssemblyWebSocketContextExpectationKindFrameHeader::Null, _, _) => Err(
                de::Error::custom("null contextExpectation must not carry typed fields"),
            ),
            (RuntimeAssemblyWebSocketContextExpectationKindFrameHeader::Typed, _, _) => Err(
                de::Error::custom("typed contextExpectation requires both typed fields"),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketContextCodecFrameHeader {
    pub operation_abi_id: String,
    pub context_type_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketConnectRequestFrameHeader {
    pub connection_id: String,
    pub url: String,
    pub query: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    pub headers: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    pub cookies: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeAssemblyWebSocketMessageTagFrameHeader {
    Text,
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeAssemblyWebSocketMessageEncodingFrameHeader {
    Utf8,
    #[serde(rename = "binary")]
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketMessageFrameHeader {
    pub tag: RuntimeAssemblyWebSocketMessageTagFrameHeader,
    pub encoding: RuntimeAssemblyWebSocketMessageEncodingFrameHeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeAssemblyWebSocketPayloadSegmentKindFrameHeader {
    #[serde(rename = "websocket.context")]
    Context,
    #[serde(rename = "websocket.message")]
    Message,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketPayloadSegmentFrameHeader {
    pub kind: RuntimeAssemblyWebSocketPayloadSegmentKindFrameHeader,
    #[serde(deserialize_with = "deserialize_safe_unsigned_integer")]
    pub offset: u64,
    #[serde(deserialize_with = "deserialize_safe_unsigned_integer")]
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketReceiveEventFrameHeader {
    pub connection_id: String,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub business_identity: Option<String>,
    pub message: RuntimeAssemblyWebSocketMessageFrameHeader,
    pub payload_segments: Vec<RuntimeAssemblyWebSocketPayloadSegmentFrameHeader>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_codec: Option<RuntimeAssemblyWebSocketContextCodecFrameHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketAdapterFrameHeader {
    pub kind: RuntimeAssemblyWebSocketAdapterKindFrameHeader,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapter_args: Vec<RuntimeAssemblyWebSocketAdapterArgFrameHeader>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_expectation: Option<RuntimeAssemblyWebSocketContextExpectationFrameHeader>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub connect_request: Option<RuntimeAssemblyWebSocketConnectRequestFrameHeader>,
    #[serde(
        rename = "receiveEvent",
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub receive_event: Option<RuntimeAssemblyWebSocketReceiveEventFrameHeader>,
}

impl RuntimeAssemblyWebSocketAdapterFrameHeader {
    fn validate(&self) -> Result<(), String> {
        validate_adapter_params(
            self.adapter_args.iter().map(|arg| arg.param.as_str()),
            "websocketAdapter.adapterArgs",
        )?;
        match (
            self.kind,
            self.connect_request.is_some(),
            self.receive_event.is_some(),
        ) {
            (RuntimeAssemblyWebSocketAdapterKindFrameHeader::Connect, true, false)
            | (RuntimeAssemblyWebSocketAdapterKindFrameHeader::Receive, false, true) => Ok(()),
            (RuntimeAssemblyWebSocketAdapterKindFrameHeader::Connect, _, _) => {
                Err("websocketAdapter kind connect requires only connectRequest".to_string())
            }
            (RuntimeAssemblyWebSocketAdapterKindFrameHeader::Receive, _, _) => {
                Err("websocketAdapter kind receive requires only receiveEvent".to_string())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestTestEffectDoubleFrameHeader {
    #[serde(
        default,
        deserialize_with = "deserialize_present_json_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub expect_request: Option<Value>,
    #[serde(deserialize_with = "deserialize_canonical_json_value")]
    pub response: Value,
}

pub(super) fn deserialize_present_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

pub(super) fn deserialize_optional_activation_identity<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let valid = value
        .strip_prefix("skiff-runtime-activation-v1:opaque:")
        .is_some_and(|opaque| {
            !opaque.is_empty()
                && opaque.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
                })
        });
    if !valid {
        return Err(de::Error::custom(
            "activationIdentity must be skiff-runtime-activation-v1:opaque:<opaque id>",
        ));
    }
    Ok(Some(value))
}

pub(super) fn deserialize_optional_gateway_entry_identity<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let valid = value
        .strip_prefix("skiff-gateway-v1:sha256:")
        .is_some_and(is_lower_hex_64);
    if !valid {
        return Err(de::Error::custom(
            "gatewayEntryIdentity must be skiff-gateway-v1:sha256:<64 lowercase hex>",
        ));
    }
    Ok(Some(value))
}

pub(super) fn deserialize_optional_http_adapter<'de, D>(
    deserializer: D,
) -> Result<Option<RuntimeAssemblyHttpAdapterFrameHeader>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = RuntimeAssemblyHttpAdapterFrameHeader::deserialize(deserializer)?;
    value.validate().map_err(de::Error::custom)?;
    Ok(Some(value))
}

pub(super) fn deserialize_optional_websocket_adapter<'de, D>(
    deserializer: D,
) -> Result<Option<RuntimeAssemblyWebSocketAdapterFrameHeader>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = RuntimeAssemblyWebSocketAdapterFrameHeader::deserialize(deserializer)?;
    value.validate().map_err(de::Error::custom)?;
    Ok(Some(value))
}

pub(super) fn deserialize_test_effect_doubles<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, Vec<RuntimeAssemblyRequestTestEffectDoubleFrameHeader>>, D::Error>
where
    D: Deserializer<'de>,
{
    let doubles =
        HashMap::<String, Vec<RuntimeAssemblyRequestTestEffectDoubleFrameHeader>>::deserialize(
            deserializer,
        )?;
    for (target, sequence) in &doubles {
        if sequence.is_empty() {
            return Err(de::Error::custom(format!(
                "testEffectDoubles.{target} must be a non-empty array"
            )));
        }
    }
    Ok(doubles)
}

fn deserialize_present_json_value<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_canonical_json_value(deserializer).map(Some)
}

fn deserialize_canonical_json_value<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    normalize_opaque_json(Value::deserialize(deserializer)?).map_err(de::Error::custom)
}

fn normalize_opaque_json(value: Value) -> Result<Value, String> {
    match value {
        Value::Array(values) => values
            .into_iter()
            .map(normalize_opaque_json)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(object) => {
            let mut normalized = Map::new();
            for (key, value) in object {
                normalized.insert(key, normalize_opaque_json(value)?);
            }
            Ok(Value::Object(normalized))
        }
        Value::Number(number) => {
            let normalized = canonical_json_number(&number);
            if normalized
                .as_i64()
                .is_some_and(|value| value.unsigned_abs() > MAX_SAFE_INTEGER)
                || normalized
                    .as_u64()
                    .is_some_and(|value| value > MAX_SAFE_INTEGER)
                || normalized.as_f64().is_some_and(|value| {
                    value.is_finite()
                        && value.fract() == 0.0
                        && value.abs() > MAX_SAFE_INTEGER as f64
                })
            {
                return Err("JSON integer exceeds Number.MAX_SAFE_INTEGER".to_string());
            }
            Ok(normalized)
        }
        value => Ok(value),
    }
}

fn deserialize_safe_unsigned_integer<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(SafeUnsignedIntegerVisitor)
}

struct SafeUnsignedIntegerVisitor;

impl de::Visitor<'_> for SafeUnsignedIntegerVisitor {
    type Value = u64;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a non-negative safe integer other than -0")
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value <= MAX_SAFE_INTEGER {
            Ok(value)
        } else {
            Err(E::custom("integer exceeds Number.MAX_SAFE_INTEGER"))
        }
    }
}

fn validate_adapter_params<'a>(
    params: impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for param in params {
        if param.chars().all(is_ecmascript_whitespace) {
            return Err(format!("{label} param must be non-blank"));
        }
        if !seen.insert(param) {
            return Err(format!("{label} has duplicate param {param}"));
        }
    }
    Ok(())
}

fn is_ecmascript_whitespace(value: char) -> bool {
    matches!(
        value,
        '\u{0009}'
            | '\u{000a}'
            | '\u{000b}'
            | '\u{000c}'
            | '\u{000d}'
            | '\u{0020}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}
