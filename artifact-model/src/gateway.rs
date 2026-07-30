use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

pub const WEBSOCKET_ENTRY_ID_PREFIX: &str = "skiff-websocket-entry-v1:sha256";
pub const WEBSOCKET_GATEWAY_ENTRY_KEY: &str = "websocket";
pub const WEBSOCKET_CONNECT_REQUEST_V1_TYPE: &str = "std.websocket.WebSocketConnectRequest";
pub const WEBSOCKET_CONNECTION_POLICY_V1_TYPE: &str = "std.websocket.WebSocketConnectionPolicy";
pub const WEBSOCKET_CONNECT_RESULT_V1_TYPE: &str = "std.websocket.WebSocketConnectResult";
pub const WEBSOCKET_JSON_RPC_TEXT_PROFILE: &str = "jsonrpc-2.0-text";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketEntryIdParseError {
    value: String,
}

impl fmt::Display for WebSocketEntryIdParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "WebSocket entry id {:?} must use {WEBSOCKET_ENTRY_ID_PREFIX}:<64 lowercase hex>",
            self.value
        )
    }
}

impl std::error::Error for WebSocketEntryIdParseError {}

/// Stable internal identity for the single compiler-owned WebSocket entry.
///
/// Authors never provide this value. The canonical producer derives it from
/// the exact service id and compiler-owned gateway entry key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct WebSocketEntryId(String);

impl WebSocketEntryId {
    pub fn parse(value: impl Into<String>) -> Result<Self, WebSocketEntryIdParseError> {
        let value = value.into();
        let valid = value
            .strip_prefix(WEBSOCKET_ENTRY_ID_PREFIX)
            .and_then(|suffix| suffix.strip_prefix(':'))
            .is_some_and(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            });
        if !valid {
            return Err(WebSocketEntryIdParseError { value });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl<'de> Deserialize<'de> for WebSocketEntryId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for WebSocketEntryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GatewayAdapterKind {
    TypedJson,
    RawHttp,
    #[serde(rename = "websocketConnect")]
    WebSocketConnect,
    #[serde(rename = "websocketJsonRpc")]
    WebSocketJsonRpc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(tag = "kind")]
pub enum GatewayAdapterSource {
    #[serde(rename = "http.request")]
    HttpRequest,
    #[serde(rename = "http.body")]
    HttpBody,
    #[serde(rename = "http.context")]
    HttpContext,
    #[serde(rename = "websocket.connectRequest")]
    WebSocketConnectRequest,
    #[serde(rename = "websocket.jsonRpcParams")]
    WebSocketJsonRpcParams,
    #[serde(rename = "websocket.connectionId")]
    WebSocketConnectionId,
    #[serde(rename = "websocket.businessIdentity")]
    WebSocketBusinessIdentity,
}

impl<'de> Deserialize<'de> for GatewayAdapterSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", deny_unknown_fields)]
        enum Wire {
            #[serde(rename = "http.request")]
            HttpRequest {},
            #[serde(rename = "http.body")]
            HttpBody {},
            #[serde(rename = "http.context")]
            HttpContext {},
            #[serde(rename = "websocket.connectRequest")]
            WebSocketConnectRequest {},
            #[serde(rename = "websocket.jsonRpcParams")]
            WebSocketJsonRpcParams {},
            #[serde(rename = "websocket.connectionId")]
            WebSocketConnectionId {},
            #[serde(rename = "websocket.businessIdentity")]
            WebSocketBusinessIdentity {},
        }

        Ok(match Wire::deserialize(deserializer)? {
            Wire::HttpRequest {} => Self::HttpRequest,
            Wire::HttpBody {} => Self::HttpBody,
            Wire::HttpContext {} => Self::HttpContext,
            Wire::WebSocketConnectRequest {} => Self::WebSocketConnectRequest,
            Wire::WebSocketJsonRpcParams {} => Self::WebSocketJsonRpcParams,
            Wire::WebSocketConnectionId {} => Self::WebSocketConnectionId,
            Wire::WebSocketBusinessIdentity {} => Self::WebSocketBusinessIdentity,
        })
    }
}

impl GatewayAdapterSource {
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::HttpRequest => "http.request",
            Self::HttpBody => "http.body",
            Self::HttpContext => "http.context",
            Self::WebSocketConnectRequest => "websocket.connectRequest",
            Self::WebSocketJsonRpcParams => "websocket.jsonRpcParams",
            Self::WebSocketConnectionId => "websocket.connectionId",
            Self::WebSocketBusinessIdentity => "websocket.businessIdentity",
        }
    }

    /// Whether selecting this source changes the external protocol view.
    ///
    /// HTTP context remains in the deployment execution plan. WebSocket
    /// connection and business-identity sources are part of the closed
    /// protocol capability surface even though their formal parameter names
    /// and order remain deployment-only facts.
    pub fn is_external_protocol_source(self) -> bool {
        matches!(
            self,
            Self::HttpRequest
                | Self::HttpBody
                | Self::WebSocketConnectRequest
                | Self::WebSocketJsonRpcParams
                | Self::WebSocketConnectionId
                | Self::WebSocketBusinessIdentity
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayAdapterArg {
    pub param: String,
    pub source: GatewayAdapterSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayAdapterArgValidationError {
    InvalidParam {
        param: String,
    },
    DuplicateParam {
        param: String,
    },
    SourceNotAllowed {
        kind: GatewayAdapterKind,
        source: GatewayAdapterSource,
    },
    HttpContextRequiresPre,
}

impl fmt::Display for GatewayAdapterArgValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParam { param } => write!(
                formatter,
                "gateway adapter param {param:?} must be non-empty and contain no whitespace or control characters"
            ),
            Self::DuplicateParam { param } => {
                write!(formatter, "duplicate gateway adapter param {param}")
            }
            Self::SourceNotAllowed { kind, source } => write!(
                formatter,
                "gateway adapter source {} is not allowed for {kind:?}",
                source.wire_name()
            ),
            Self::HttpContextRequiresPre => {
                formatter.write_str("http.context requires an HTTP pre callable")
            }
        }
    }
}

impl std::error::Error for GatewayAdapterArgValidationError {}

pub fn validate_gateway_adapter_args(
    kind: GatewayAdapterKind,
    has_http_pre: bool,
    args: &[GatewayAdapterArg],
) -> Result<(), GatewayAdapterArgValidationError> {
    let mut params = BTreeSet::new();
    for arg in args {
        if arg.param.is_empty()
            || arg
                .param
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(GatewayAdapterArgValidationError::InvalidParam {
                param: arg.param.clone(),
            });
        }
        if !params.insert(arg.param.as_str()) {
            return Err(GatewayAdapterArgValidationError::DuplicateParam {
                param: arg.param.clone(),
            });
        }
        if !adapter_source_is_allowed(kind, arg.source) {
            return Err(GatewayAdapterArgValidationError::SourceNotAllowed {
                kind,
                source: arg.source,
            });
        }
        if arg.source == GatewayAdapterSource::HttpContext && !has_http_pre {
            return Err(GatewayAdapterArgValidationError::HttpContextRequiresPre);
        }
    }
    Ok(())
}

fn adapter_source_is_allowed(kind: GatewayAdapterKind, source: GatewayAdapterSource) -> bool {
    match kind {
        GatewayAdapterKind::TypedJson => matches!(
            source,
            GatewayAdapterSource::HttpRequest
                | GatewayAdapterSource::HttpBody
                | GatewayAdapterSource::HttpContext
        ),
        GatewayAdapterKind::RawHttp => matches!(
            source,
            GatewayAdapterSource::HttpRequest | GatewayAdapterSource::HttpContext
        ),
        GatewayAdapterKind::WebSocketConnect => matches!(
            source,
            GatewayAdapterSource::WebSocketConnectRequest
                | GatewayAdapterSource::WebSocketConnectionId
        ),
        GatewayAdapterKind::WebSocketJsonRpc => matches!(
            source,
            GatewayAdapterSource::WebSocketJsonRpcParams
                | GatewayAdapterSource::WebSocketConnectionId
                | GatewayAdapterSource::WebSocketBusinessIdentity
        ),
    }
}

/// Strict entry-local description of the supported external JSON vocabulary.
///
/// This intentionally has no nominal reference, source path, public path,
/// package identity, runtime codec plan or untyped JSON escape hatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GatewayExternalSchema {
    Null,
    String,
    Number,
    Integer,
    Boolean,
    Bytes,
    Array {
        items: Box<GatewayExternalSchema>,
    },
    Record {
        fields: BTreeMap<String, GatewayExternalSchema>,
        required: Vec<String>,
    },
    ClosedUnion {
        branches: Vec<GatewayExternalSchema>,
    },
    Nullable {
        inner: Box<GatewayExternalSchema>,
    },
    StringLiteral {
        value: String,
    },
}

/// Compiler-owned structural definition of the WebSocket connect v1 types.
///
/// A package symbol name alone is not sufficient to claim the v1 protocol
/// surface: the linked std schema must project to this exact closed shape.
pub fn canonical_websocket_connect_schema(name: &str) -> Option<GatewayExternalSchema> {
    let string = || GatewayExternalSchema::String;
    let integer = || GatewayExternalSchema::Integer;
    let nullable = |inner| GatewayExternalSchema::Nullable {
        inner: Box::new(inner),
    };
    let array = |items| GatewayExternalSchema::Array {
        items: Box::new(items),
    };
    let literal = |value: &str| GatewayExternalSchema::StringLiteral {
        value: value.to_string(),
    };
    let record = |fields: BTreeMap<String, GatewayExternalSchema>, required: &[&str]| {
        GatewayExternalSchema::Record {
            fields,
            required: required.iter().map(|field| (*field).to_string()).collect(),
        }
    };
    let name_value = || {
        record(
            BTreeMap::from([
                ("name".to_string(), string()),
                ("value".to_string(), string()),
            ]),
            &["name", "value"],
        )
    };
    let policy = || {
        record(
            BTreeMap::from([
                ("closeCode".to_string(), nullable(integer())),
                ("closeReason".to_string(), nullable(string())),
                ("maxConnections".to_string(), integer()),
                (
                    "overflow".to_string(),
                    GatewayExternalSchema::ClosedUnion {
                        branches: vec![literal("close-oldest"), literal("reject-new")],
                    },
                ),
            ]),
            &["maxConnections", "overflow"],
        )
    };

    match name {
        WEBSOCKET_CONNECT_REQUEST_V1_TYPE => Some(record(
            BTreeMap::from([
                ("connectionId".to_string(), string()),
                ("cookies".to_string(), array(name_value())),
                ("gatewayEntryIdentity".to_string(), string()),
                ("headers".to_string(), array(name_value())),
                ("query".to_string(), array(name_value())),
                ("url".to_string(), string()),
                ("version".to_string(), nullable(string())),
                ("websocketEntryId".to_string(), string()),
            ]),
            &[
                "connectionId",
                "cookies",
                "gatewayEntryIdentity",
                "headers",
                "query",
                "url",
                "websocketEntryId",
            ],
        )),
        WEBSOCKET_CONNECTION_POLICY_V1_TYPE => Some(policy()),
        WEBSOCKET_CONNECT_RESULT_V1_TYPE => Some(GatewayExternalSchema::ClosedUnion {
            branches: vec![
                record(
                    BTreeMap::from([
                        ("businessIdentity".to_string(), nullable(string())),
                        ("connectionPolicy".to_string(), nullable(policy())),
                        ("tag".to_string(), literal("accept")),
                    ]),
                    &["tag"],
                ),
                record(
                    BTreeMap::from([
                        ("code".to_string(), integer()),
                        ("reason".to_string(), string()),
                        ("tag".to_string(), literal("reject")),
                    ]),
                    &["code", "reason", "tag"],
                ),
            ],
        }),
        _ => None,
    }
}

impl<'de> Deserialize<'de> for GatewayExternalSchema {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
        enum Wire {
            Null {},
            String {},
            Number {},
            Integer {},
            Boolean {},
            Bytes {},
            Array {
                items: Box<GatewayExternalSchema>,
            },
            Record {
                fields: StrictSchemaFields,
                required: Vec<String>,
            },
            ClosedUnion {
                branches: Vec<GatewayExternalSchema>,
            },
            Nullable {
                inner: Box<GatewayExternalSchema>,
            },
            StringLiteral {
                value: String,
            },
        }

        Ok(match Wire::deserialize(deserializer)? {
            Wire::Null {} => Self::Null,
            Wire::String {} => Self::String,
            Wire::Number {} => Self::Number,
            Wire::Integer {} => Self::Integer,
            Wire::Boolean {} => Self::Boolean,
            Wire::Bytes {} => Self::Bytes,
            Wire::Array { items } => Self::Array { items },
            Wire::Record { fields, required } => Self::Record {
                fields: fields.0,
                required,
            },
            Wire::ClosedUnion { branches } => Self::ClosedUnion { branches },
            Wire::Nullable { inner } => Self::Nullable { inner },
            Wire::StringLiteral { value } => Self::StringLiteral { value },
        })
    }
}

struct StrictSchemaFields(BTreeMap<String, GatewayExternalSchema>);

impl<'de> Deserialize<'de> for StrictSchemaFields {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FieldsVisitor;

        impl<'de> Visitor<'de> for FieldsVisitor {
            type Value = StrictSchemaFields;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a gateway external schema field map with unique keys")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut fields = BTreeMap::new();
                while let Some((name, schema)) =
                    access.next_entry::<String, GatewayExternalSchema>()?
                {
                    if fields.insert(name.clone(), schema).is_some() {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate gateway external schema field {name:?}"
                        )));
                    }
                }
                Ok(StrictSchemaFields(fields))
            }
        }

        deserializer.deserialize_map(FieldsVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GatewayDispatchMode {
    Unary,
    ServerStream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayHttpProtocolSurface {
    pub adapter_kind: GatewayAdapterKind,
    pub dispatch_mode: GatewayDispatchMode,
    pub external_sources: Vec<GatewayAdapterSource>,
    pub request_body_schema: Option<GatewayExternalSchema>,
    pub response_schema: Option<GatewayExternalSchema>,
    pub stream_item_schema: Option<GatewayExternalSchema>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayWebSocketConnectProtocolSurface {
    pub connect_request_shape: GatewayWebSocketShapeVersion,
    pub connect_result_shape: GatewayWebSocketShapeVersion,
    pub connection_policy_shape: GatewayWebSocketShapeVersion,
    pub external_sources: Vec<GatewayAdapterSource>,
    pub downlink_frames: Vec<GatewayWebSocketDownlinkFrame>,
    pub rpc_profiles: Vec<GatewayWebSocketRpcProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum GatewayWebSocketRpcProfile {
    #[serde(rename = "jsonrpc-2.0-text")]
    #[allow(non_camel_case_types)]
    JsonRpc2_0Text,
}

impl GatewayWebSocketRpcProfile {
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::JsonRpc2_0Text => WEBSOCKET_JSON_RPC_TEXT_PROFILE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayWebSocketJsonRpcProtocolSurface {
    pub profile: GatewayWebSocketRpcProfile,
    pub dispatch_mode: GatewayDispatchMode,
    pub external_sources: Vec<GatewayAdapterSource>,
    pub params_schema: GatewayExternalSchema,
    pub result_schema: GatewayExternalSchema,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GatewayWebSocketShapeVersion {
    V1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GatewayWebSocketDownlinkFrame {
    Text,
    Binary,
}

impl GatewayWebSocketDownlinkFrame {
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Binary => "binary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "surface",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum GatewayProtocolSurface {
    Http(GatewayHttpProtocolSurface),
    #[serde(rename = "websocketConnect")]
    WebSocketConnect(GatewayWebSocketConnectProtocolSurface),
    #[serde(rename = "websocketJsonRpc")]
    WebSocketJsonRpc(GatewayWebSocketJsonRpcProtocolSurface),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GatewayExternalErrorProjectionKind {
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GatewayExternalErrorProjectionVersion {
    V1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayExternalErrorProjection {
    pub kind: GatewayExternalErrorProjectionKind,
    pub version: GatewayExternalErrorProjectionVersion,
}

impl GatewayExternalErrorProjection {
    pub const FIXED_V1: Self = Self {
        kind: GatewayExternalErrorProjectionKind::Fixed,
        version: GatewayExternalErrorProjectionVersion::V1,
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayEntryProtocolSurface {
    pub protocol: GatewayProtocolSurface,
    pub external_error_projection: GatewayExternalErrorProjection,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{GatewayEntryIdentity, GatewayEntryKey, GATEWAY_ENTRY_IDENTITY_PREFIX};

    #[test]
    fn gateway_key_and_identity_are_distinct_validated_types() {
        let key = GatewayEntryKey::parse("chat.entry").expect("valid opaque key");
        let identity = GatewayEntryIdentity::parse(format!(
            "{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}",
            "a".repeat(64)
        ))
        .expect("valid content identity");
        assert_eq!(key.as_str(), "chat.entry");
        assert_ne!(key.as_str(), identity.as_str());

        for invalid in ["", " ", "two words", "line\nbreak", "nul\0byte"] {
            assert!(GatewayEntryKey::parse(invalid).is_err(), "{invalid:?}");
            assert!(
                serde_json::from_value::<GatewayEntryKey>(json!(invalid)).is_err(),
                "{invalid:?}"
            );
        }

        let digest = "a".repeat(64);
        for invalid in [
            String::new(),
            format!("skiff-gateway-v1:sha256:{digest}"),
            format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "a".repeat(63)),
            format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "A".repeat(64)),
            format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "g".repeat(64)),
        ] {
            assert!(GatewayEntryIdentity::parse(&invalid).is_err(), "{invalid}");
            assert!(
                serde_json::from_value::<GatewayEntryIdentity>(json!(invalid)).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn websocket_entry_id_has_an_exact_independent_lexical_frame() {
        let digest = "b".repeat(64);
        let valid = format!("{WEBSOCKET_ENTRY_ID_PREFIX}:{digest}");
        let parsed = WebSocketEntryId::parse(&valid).expect("valid WebSocket entry id");
        assert_eq!(parsed.as_str(), valid);
        assert_eq!(
            serde_json::from_value::<WebSocketEntryId>(json!(valid))
                .unwrap()
                .as_str(),
            parsed.as_str()
        );

        for invalid in [
            String::new(),
            format!("skiff-websocket-v1:sha256:{digest}"),
            format!("{WEBSOCKET_ENTRY_ID_PREFIX}:{}", "b".repeat(63)),
            format!("{WEBSOCKET_ENTRY_ID_PREFIX}:{}", "b".repeat(65)),
            format!("{WEBSOCKET_ENTRY_ID_PREFIX}:{}", "B".repeat(64)),
            format!("{WEBSOCKET_ENTRY_ID_PREFIX}:{}", "g".repeat(64)),
            format!("{WEBSOCKET_ENTRY_ID_PREFIX}:{digest}:extra"),
        ] {
            assert!(WebSocketEntryId::parse(&invalid).is_err(), "{invalid}");
            assert!(
                serde_json::from_value::<WebSocketEntryId>(json!(invalid)).is_err(),
                "serde accepted {invalid}"
            );
        }
    }

    #[test]
    fn gateway_adapter_source_vocabulary_and_args_are_strict() {
        let all_sources = [
            ("http.request", GatewayAdapterSource::HttpRequest),
            ("http.body", GatewayAdapterSource::HttpBody),
            ("http.context", GatewayAdapterSource::HttpContext),
            (
                "websocket.connectRequest",
                GatewayAdapterSource::WebSocketConnectRequest,
            ),
            (
                "websocket.jsonRpcParams",
                GatewayAdapterSource::WebSocketJsonRpcParams,
            ),
            (
                "websocket.connectionId",
                GatewayAdapterSource::WebSocketConnectionId,
            ),
            (
                "websocket.businessIdentity",
                GatewayAdapterSource::WebSocketBusinessIdentity,
            ),
        ];
        for (wire, source) in all_sources {
            let value = serde_json::to_value(source).expect("source serialization");
            assert_eq!(value, json!({ "kind": wire }));
            assert_eq!(
                serde_json::from_value::<GatewayAdapterSource>(value).expect("source parse"),
                source
            );
        }
        assert!(!GatewayAdapterSource::HttpContext.is_external_protocol_source());
        for source in [
            GatewayAdapterSource::WebSocketJsonRpcParams,
            GatewayAdapterSource::WebSocketConnectionId,
            GatewayAdapterSource::WebSocketBusinessIdentity,
        ] {
            assert!(source.is_external_protocol_source(), "{source:?}");
        }
        assert!(
            serde_json::from_value::<GatewayAdapterSource>(json!({ "kind": "http.query" }))
                .is_err()
        );
        assert!(serde_json::from_value::<GatewayAdapterSource>(
            json!({ "kind": "http.body", "path": "payload" })
        )
        .is_err());
        assert_eq!(
            serde_json::from_value::<GatewayAdapterKind>(json!("websocketConnect")).unwrap(),
            GatewayAdapterKind::WebSocketConnect
        );
        assert_eq!(
            serde_json::from_value::<GatewayAdapterKind>(json!("websocketJsonRpc")).unwrap(),
            GatewayAdapterKind::WebSocketJsonRpc
        );
        for invalid in [
            "webSocketConnect",
            "websocket",
            "websocketReceive",
            "webSocketJsonRpc",
        ] {
            assert!(
                serde_json::from_value::<GatewayAdapterKind>(json!(invalid)).is_err(),
                "{invalid}"
            );
        }
        assert!(serde_json::from_value::<GatewayAdapterSource>(
            json!({ "kind": "websocket.message" })
        )
        .is_err());

        let typed = [GatewayAdapterArg {
            param: "body".to_string(),
            source: GatewayAdapterSource::HttpBody,
        }];
        validate_gateway_adapter_args(GatewayAdapterKind::TypedJson, false, &typed)
            .expect("typed body source");
        assert!(validate_gateway_adapter_args(GatewayAdapterKind::RawHttp, false, &typed).is_err());
        assert!(validate_gateway_adapter_args(
            GatewayAdapterKind::TypedJson,
            false,
            &[GatewayAdapterArg {
                param: "context".to_string(),
                source: GatewayAdapterSource::HttpContext,
            }]
        )
        .is_err());
        validate_gateway_adapter_args(
            GatewayAdapterKind::WebSocketConnect,
            false,
            &[
                GatewayAdapterArg {
                    param: "request".to_string(),
                    source: GatewayAdapterSource::WebSocketConnectRequest,
                },
                GatewayAdapterArg {
                    param: "connectionId".to_string(),
                    source: GatewayAdapterSource::WebSocketConnectionId,
                },
            ],
        )
        .expect("WebSocket connect sources");
        validate_gateway_adapter_args(
            GatewayAdapterKind::WebSocketJsonRpc,
            false,
            &[
                GatewayAdapterArg {
                    param: "params".to_string(),
                    source: GatewayAdapterSource::WebSocketJsonRpcParams,
                },
                GatewayAdapterArg {
                    param: "connectionId".to_string(),
                    source: GatewayAdapterSource::WebSocketConnectionId,
                },
                GatewayAdapterArg {
                    param: "businessIdentity".to_string(),
                    source: GatewayAdapterSource::WebSocketBusinessIdentity,
                },
            ],
        )
        .expect("WebSocket JSON-RPC sources");
        assert!(validate_gateway_adapter_args(
            GatewayAdapterKind::WebSocketJsonRpc,
            false,
            &[GatewayAdapterArg {
                param: "request".to_string(),
                source: GatewayAdapterSource::WebSocketConnectRequest,
            }]
        )
        .is_err());
        assert!(validate_gateway_adapter_args(
            GatewayAdapterKind::WebSocketConnect,
            false,
            &[GatewayAdapterArg {
                param: "request".to_string(),
                source: GatewayAdapterSource::HttpRequest,
            }]
        )
        .is_err());
        assert!(validate_gateway_adapter_args(
            GatewayAdapterKind::TypedJson,
            false,
            &[
                GatewayAdapterArg {
                    param: "body".to_string(),
                    source: GatewayAdapterSource::HttpBody,
                },
                GatewayAdapterArg {
                    param: "body".to_string(),
                    source: GatewayAdapterSource::HttpRequest,
                },
            ]
        )
        .is_err());
        assert!(serde_json::from_value::<GatewayAdapterArg>(json!({
            "param": "body",
            "source": { "kind": "http.body" },
            "targetType": "PrivateRequest"
        }))
        .is_err());
    }

    #[test]
    fn websocket_json_rpc_protocol_surface_has_one_closed_profile_and_schema_shape() {
        let surface = GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::WebSocketJsonRpc(
                GatewayWebSocketJsonRpcProtocolSurface {
                    profile: GatewayWebSocketRpcProfile::JsonRpc2_0Text,
                    dispatch_mode: GatewayDispatchMode::Unary,
                    external_sources: vec![GatewayAdapterSource::WebSocketJsonRpcParams],
                    params_schema: GatewayExternalSchema::Record {
                        fields: BTreeMap::from([("id".to_string(), GatewayExternalSchema::String)]),
                        required: vec!["id".to_string()],
                    },
                    result_schema: GatewayExternalSchema::Null,
                },
            ),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        };
        let value = serde_json::to_value(&surface).unwrap();
        assert_eq!(
            value["protocol"]["surface"]["profile"],
            json!("jsonrpc-2.0-text")
        );
        assert_eq!(
            serde_json::from_value::<GatewayEntryProtocolSurface>(value).unwrap(),
            surface
        );
        assert_eq!(
            GatewayWebSocketRpcProfile::JsonRpc2_0Text.wire_name(),
            WEBSOCKET_JSON_RPC_TEXT_PROFILE
        );
        assert!(
            serde_json::from_value::<GatewayWebSocketRpcProfile>(json!("jsonrpc-1.0-text"))
                .is_err()
        );
    }

    #[test]
    fn gateway_schema_has_no_nominal_or_untyped_escape_fields() {
        let strict_record = json!({
            "kind": "record",
            "fields": {
                "id": { "kind": "string" },
                "state": {
                    "kind": "closedUnion",
                    "branches": [
                        { "kind": "stringLiteral", "value": "open" },
                        { "kind": "stringLiteral", "value": "closed" }
                    ]
                }
            },
            "required": ["id"]
        });
        serde_json::from_value::<GatewayExternalSchema>(strict_record.clone())
            .expect("closed schema vocabulary");

        for forbidden in [
            ("packageSchemaTypeId", json!("package-type")),
            ("typeRefIr", json!({ "kind": "builtin", "name": "User" })),
            ("publicPath", json!("types.User")),
            ("sourcePath", json!("internal.user")),
            ("nominalName", json!("User")),
            ("value", json!({ "arbitrary": true })),
        ] {
            let mut forged = strict_record.clone();
            forged
                .as_object_mut()
                .expect("record object")
                .insert(forbidden.0.to_string(), forbidden.1);
            assert!(
                serde_json::from_value::<GatewayExternalSchema>(forged).is_err(),
                "{} must be rejected",
                forbidden.0
            );
        }

        let vocabulary = [
            json!({ "kind": "null" }),
            json!({ "kind": "string" }),
            json!({ "kind": "number" }),
            json!({ "kind": "integer" }),
            json!({ "kind": "boolean" }),
            json!({ "kind": "bytes" }),
            json!({ "kind": "array", "items": { "kind": "string" } }),
            json!({
                "kind": "nullable",
                "inner": { "kind": "integer" }
            }),
            json!({
                "kind": "stringLiteral",
                "value": ""
            }),
        ];
        for schema in vocabulary {
            let parsed = serde_json::from_value::<GatewayExternalSchema>(schema.clone())
                .expect("supported external schema vocabulary");
            assert_eq!(
                serde_json::to_value(parsed).expect("schema serialization"),
                schema
            );
        }

        assert!(serde_json::from_value::<GatewayExternalSchema>(
            json!({ "kind": "string", "packageId": "example.com/private" })
        )
        .is_err());
        assert!(serde_json::from_value::<GatewayExternalSchema>(json!({
            "kind": "record",
            "fields": {},
            "required": [],
            "additionalProperties": true
        }))
        .is_err());
        assert!(
            serde_json::from_str::<GatewayExternalSchema>(
                r#"{"kind":"record","fields":{"id":{"kind":"string"},"id":{"kind":"integer"}},"required":["id"]}"#
            )
            .is_err(),
            "duplicate record field keys must not be silently overwritten"
        );
    }

    #[test]
    fn gateway_surface_dto_rejects_unknown_fields_and_enum_values() {
        let surface = GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
                adapter_kind: GatewayAdapterKind::TypedJson,
                dispatch_mode: GatewayDispatchMode::Unary,
                external_sources: vec![GatewayAdapterSource::HttpBody],
                request_body_schema: Some(GatewayExternalSchema::String),
                response_schema: Some(GatewayExternalSchema::Boolean),
                stream_item_schema: None,
            }),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        };
        let mut wire = serde_json::to_value(&surface).expect("surface serialization");
        wire.as_object_mut()
            .expect("surface object")
            .insert("handler".to_string(), json!("internal.handle"));
        assert!(serde_json::from_value::<GatewayEntryProtocolSurface>(wire).is_err());

        let mut wire = serde_json::to_value(&surface).expect("surface serialization");
        wire["protocol"]["surface"]["adapterKind"] = json!("graphql");
        assert!(serde_json::from_value::<GatewayEntryProtocolSurface>(wire).is_err());

        let websocket = GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::WebSocketConnect(
                GatewayWebSocketConnectProtocolSurface {
                    connect_request_shape: GatewayWebSocketShapeVersion::V1,
                    connect_result_shape: GatewayWebSocketShapeVersion::V1,
                    connection_policy_shape: GatewayWebSocketShapeVersion::V1,
                    external_sources: vec![
                        GatewayAdapterSource::WebSocketConnectRequest,
                        GatewayAdapterSource::WebSocketConnectionId,
                    ],
                    downlink_frames: vec![
                        GatewayWebSocketDownlinkFrame::Binary,
                        GatewayWebSocketDownlinkFrame::Text,
                    ],
                    rpc_profiles: vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text],
                },
            ),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        };
        assert_eq!(
            serde_json::to_value(&websocket).unwrap(),
            json!({
                "protocol": {
                    "kind": "websocketConnect",
                    "surface": {
                        "connectRequestShape": "v1",
                        "connectResultShape": "v1",
                        "connectionPolicyShape": "v1",
                        "externalSources": [
                            { "kind": "websocket.connectRequest" },
                            { "kind": "websocket.connectionId" }
                        ],
                        "downlinkFrames": ["binary", "text"],
                        "rpcProfiles": ["jsonrpc-2.0-text"]
                    }
                },
                "externalErrorProjection": {
                    "kind": "fixed",
                    "version": "v1"
                }
            })
        );
        let mut unknown = serde_json::to_value(websocket).unwrap();
        unknown["protocol"]["surface"]["receive"] = json!(true);
        assert!(serde_json::from_value::<GatewayEntryProtocolSurface>(unknown).is_err());

        for invalid in ["webSocketConnect", "websocket", "websocketReceive"] {
            let mut wrong_kind = serde_json::to_value(&surface).unwrap();
            wrong_kind["protocol"]["kind"] = json!(invalid);
            assert!(
                serde_json::from_value::<GatewayEntryProtocolSurface>(wrong_kind).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn canonical_websocket_v1_shapes_are_closed_and_exact() {
        let request =
            canonical_websocket_connect_schema(WEBSOCKET_CONNECT_REQUEST_V1_TYPE).unwrap();
        let GatewayExternalSchema::Record { fields, required } = request else {
            panic!("connect request must be a record");
        };
        assert_eq!(
            fields.keys().map(String::as_str).collect::<Vec<_>>(),
            vec![
                "connectionId",
                "cookies",
                "gatewayEntryIdentity",
                "headers",
                "query",
                "url",
                "version",
                "websocketEntryId"
            ]
        );
        assert!(!required.contains(&"version".to_string()));
        assert!(required.contains(&"websocketEntryId".to_string()));
        assert!(required.contains(&"gatewayEntryIdentity".to_string()));

        let result = canonical_websocket_connect_schema(WEBSOCKET_CONNECT_RESULT_V1_TYPE).unwrap();
        let GatewayExternalSchema::ClosedUnion { branches } = result else {
            panic!("connect result must be a closed union");
        };
        assert_eq!(branches.len(), 2);
        assert!(
            canonical_websocket_connect_schema("std.websocket.WebSocketIngressEvent").is_none()
        );
    }
}
