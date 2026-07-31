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
mod tests;
