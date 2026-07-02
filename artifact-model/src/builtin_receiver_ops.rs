use std::fmt;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

pub const RECEIVER_BUILTIN_CAPABILITY_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BuiltinReceiverRoot {
    Array,
    Map,
    JsonObject,
    #[serde(rename = "string")]
    StringText,
    #[serde(rename = "number")]
    Number,
    Date,
    Duration,
    #[serde(rename = "bytes")]
    Bytes,
}

impl BuiltinReceiverRoot {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Array => "Array",
            Self::Map => "Map",
            Self::JsonObject => "JsonObject",
            Self::StringText => "string",
            Self::Number => "number",
            Self::Date => "Date",
            Self::Duration => "Duration",
            Self::Bytes => "bytes",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BuiltinReceiverMethod {
    #[serde(rename = "length")]
    Length,
    #[serde(rename = "push")]
    Push,
    #[serde(rename = "set")]
    Set,
    #[serde(rename = "pop")]
    Pop,
    #[serde(rename = "clone")]
    Clone,
    #[serde(rename = "get")]
    Get,
    #[serde(rename = "has")]
    Has,
    #[serde(rename = "delete")]
    Delete,
    #[serde(rename = "keys")]
    Keys,
    #[serde(rename = "contains")]
    Contains,
    #[serde(rename = "replaceAll")]
    ReplaceAll,
    #[serde(rename = "concat")]
    Concat,
    #[serde(rename = "startsWith")]
    StartsWith,
    #[serde(rename = "endsWith")]
    EndsWith,
    #[serde(rename = "lowercase")]
    Lowercase,
    #[serde(rename = "floor")]
    Floor,
    #[serde(rename = "ceil")]
    Ceil,
    #[serde(rename = "round")]
    Round,
    #[serde(rename = "toEpochMilliseconds")]
    ToEpochMilliseconds,
    #[serde(rename = "toISOString")]
    ToIsoString,
    #[serde(rename = "addMilliseconds")]
    AddMilliseconds,
    #[serde(rename = "diffMilliseconds")]
    DiffMilliseconds,
    #[serde(rename = "compare")]
    Compare,
    #[serde(rename = "isBefore")]
    IsBefore,
    #[serde(rename = "isAfter")]
    IsAfter,
    #[serde(rename = "toMilliseconds")]
    ToMilliseconds,
    #[serde(rename = "toBase64")]
    ToBase64,
    #[serde(rename = "toHex")]
    ToHex,
    #[serde(rename = "toUtf8String")]
    ToUtf8String,
}

impl BuiltinReceiverMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Length => "length",
            Self::Push => "push",
            Self::Set => "set",
            Self::Pop => "pop",
            Self::Clone => "clone",
            Self::Get => "get",
            Self::Has => "has",
            Self::Delete => "delete",
            Self::Keys => "keys",
            Self::Contains => "contains",
            Self::ReplaceAll => "replaceAll",
            Self::Concat => "concat",
            Self::StartsWith => "startsWith",
            Self::EndsWith => "endsWith",
            Self::Lowercase => "lowercase",
            Self::Floor => "floor",
            Self::Ceil => "ceil",
            Self::Round => "round",
            Self::ToEpochMilliseconds => "toEpochMilliseconds",
            Self::ToIsoString => "toISOString",
            Self::AddMilliseconds => "addMilliseconds",
            Self::DiffMilliseconds => "diffMilliseconds",
            Self::Compare => "compare",
            Self::IsBefore => "isBefore",
            Self::IsAfter => "isAfter",
            Self::ToMilliseconds => "toMilliseconds",
            Self::ToBase64 => "toBase64",
            Self::ToHex => "toHex",
            Self::ToUtf8String => "toUtf8String",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BuiltinReceiverThrowSemantics {
    Never,
    Decode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BuiltinReceiverSupportStatus {
    Supported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinReceiverPublicReturnType {
    Fixed(&'static str),
    Receiver,
    ArrayItem,
    MapValue,
    MapKeyArray,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinReceiverOp {
    pub receiver: BuiltinReceiverRoot,
    pub method: BuiltinReceiverMethod,
    pub signature_version: u32,
    pub canonical_key: &'static str,
}

impl BuiltinReceiverOp {
    pub fn new(
        receiver: BuiltinReceiverRoot,
        method: BuiltinReceiverMethod,
        signature_version: u32,
    ) -> Option<Self> {
        builtin_receiver_op(receiver, method, signature_version)
    }
}

impl Serialize for BuiltinReceiverOp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Fields {
            receiver: BuiltinReceiverRoot,
            method: BuiltinReceiverMethod,
            signature_version: u32,
            canonical_key: &'static str,
        }

        Fields {
            receiver: self.receiver,
            method: self.method,
            signature_version: self.signature_version,
            canonical_key: self.canonical_key,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BuiltinReceiverOp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Fields {
            receiver: BuiltinReceiverRoot,
            method: BuiltinReceiverMethod,
            signature_version: u32,
            canonical_key: String,
        }

        let fields = Fields::deserialize(deserializer)?;
        validate_receiver_builtin_fields(
            fields.receiver,
            fields.method,
            fields.signature_version,
            &fields.canonical_key,
        )
        .map_err(D::Error::custom)
        .map(|spec| spec.op)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinReceiverOpSpec {
    pub op: BuiltinReceiverOp,
    pub introduced_capability_version: u32,
    pub support_status: BuiltinReceiverSupportStatus,
    pub public_return_type: BuiltinReceiverPublicReturnType,
    pub mutates_receiver: bool,
    pub throws: BuiltinReceiverThrowSemantics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuiltinReceiverSupportError {
    CanonicalKeyMismatch {
        expected: String,
        actual: String,
    },
    UnknownOp {
        receiver: BuiltinReceiverRoot,
        method: BuiltinReceiverMethod,
        signature_version: u32,
    },
    UnsupportedSignatureVersion {
        receiver: BuiltinReceiverRoot,
        method: BuiltinReceiverMethod,
        signature_version: u32,
    },
}

impl fmt::Display for BuiltinReceiverSupportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalKeyMismatch { expected, actual } => write!(
                f,
                "receiver builtin canonicalKey mismatch: expected {expected}, got {actual}"
            ),
            Self::UnknownOp {
                receiver,
                method,
                signature_version,
            } => write!(
                f,
                "unknown receiver builtin op {}.{}@{}",
                receiver.as_str(),
                method.as_str(),
                signature_version
            ),
            Self::UnsupportedSignatureVersion {
                receiver,
                method,
                signature_version,
            } => write!(
                f,
                "unsupported receiver builtin signatureVersion {} for {}.{}",
                signature_version,
                receiver.as_str(),
                method.as_str()
            ),
        }
    }
}

impl std::error::Error for BuiltinReceiverSupportError {}

pub const SUPPORTED_RECEIVER_BUILTIN_OPS: &[BuiltinReceiverOpSpec] = &[
    spec(
        BuiltinReceiverRoot::Array,
        BuiltinReceiverMethod::Length,
        BuiltinReceiverPublicReturnType::Fixed("number"),
        false,
        BuiltinReceiverThrowSemantics::Never,
    ),
    spec(
        BuiltinReceiverRoot::Array,
        BuiltinReceiverMethod::Push,
        BuiltinReceiverPublicReturnType::Fixed("null"),
        true,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Array,
        BuiltinReceiverMethod::Set,
        BuiltinReceiverPublicReturnType::Fixed("null"),
        true,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Array,
        BuiltinReceiverMethod::Pop,
        BuiltinReceiverPublicReturnType::ArrayItem,
        true,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Array,
        BuiltinReceiverMethod::Clone,
        BuiltinReceiverPublicReturnType::Receiver,
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Map,
        BuiltinReceiverMethod::Length,
        BuiltinReceiverPublicReturnType::Fixed("number"),
        false,
        BuiltinReceiverThrowSemantics::Never,
    ),
    spec(
        BuiltinReceiverRoot::Map,
        BuiltinReceiverMethod::Get,
        BuiltinReceiverPublicReturnType::MapValue,
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Map,
        BuiltinReceiverMethod::Has,
        BuiltinReceiverPublicReturnType::Fixed("bool"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Map,
        BuiltinReceiverMethod::Set,
        BuiltinReceiverPublicReturnType::Fixed("null"),
        true,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Map,
        BuiltinReceiverMethod::Delete,
        BuiltinReceiverPublicReturnType::Fixed("bool"),
        true,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Map,
        BuiltinReceiverMethod::Keys,
        BuiltinReceiverPublicReturnType::MapKeyArray,
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Map,
        BuiltinReceiverMethod::Clone,
        BuiltinReceiverPublicReturnType::Receiver,
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::JsonObject,
        BuiltinReceiverMethod::Length,
        BuiltinReceiverPublicReturnType::Fixed("number"),
        false,
        BuiltinReceiverThrowSemantics::Never,
    ),
    spec(
        BuiltinReceiverRoot::JsonObject,
        BuiltinReceiverMethod::Get,
        BuiltinReceiverPublicReturnType::Fixed("Json"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::JsonObject,
        BuiltinReceiverMethod::Has,
        BuiltinReceiverPublicReturnType::Fixed("bool"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::JsonObject,
        BuiltinReceiverMethod::Set,
        BuiltinReceiverPublicReturnType::Fixed("null"),
        true,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::JsonObject,
        BuiltinReceiverMethod::Delete,
        BuiltinReceiverPublicReturnType::Fixed("bool"),
        true,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::JsonObject,
        BuiltinReceiverMethod::Clone,
        BuiltinReceiverPublicReturnType::Fixed("JsonObject"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::StringText,
        BuiltinReceiverMethod::Length,
        BuiltinReceiverPublicReturnType::Fixed("number"),
        false,
        BuiltinReceiverThrowSemantics::Never,
    ),
    spec(
        BuiltinReceiverRoot::StringText,
        BuiltinReceiverMethod::Contains,
        BuiltinReceiverPublicReturnType::Fixed("bool"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::StringText,
        BuiltinReceiverMethod::ReplaceAll,
        BuiltinReceiverPublicReturnType::Fixed("string"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::StringText,
        BuiltinReceiverMethod::Concat,
        BuiltinReceiverPublicReturnType::Fixed("string"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::StringText,
        BuiltinReceiverMethod::StartsWith,
        BuiltinReceiverPublicReturnType::Fixed("bool"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::StringText,
        BuiltinReceiverMethod::EndsWith,
        BuiltinReceiverPublicReturnType::Fixed("bool"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::StringText,
        BuiltinReceiverMethod::Lowercase,
        BuiltinReceiverPublicReturnType::Fixed("string"),
        false,
        BuiltinReceiverThrowSemantics::Never,
    ),
    spec(
        BuiltinReceiverRoot::Number,
        BuiltinReceiverMethod::Floor,
        BuiltinReceiverPublicReturnType::Fixed("number"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Number,
        BuiltinReceiverMethod::Ceil,
        BuiltinReceiverPublicReturnType::Fixed("number"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Number,
        BuiltinReceiverMethod::Round,
        BuiltinReceiverPublicReturnType::Fixed("number"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Date,
        BuiltinReceiverMethod::ToEpochMilliseconds,
        BuiltinReceiverPublicReturnType::Fixed("integer"),
        false,
        BuiltinReceiverThrowSemantics::Never,
    ),
    spec(
        BuiltinReceiverRoot::Date,
        BuiltinReceiverMethod::ToIsoString,
        BuiltinReceiverPublicReturnType::Fixed("string"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Date,
        BuiltinReceiverMethod::AddMilliseconds,
        BuiltinReceiverPublicReturnType::Fixed("Date"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Date,
        BuiltinReceiverMethod::DiffMilliseconds,
        BuiltinReceiverPublicReturnType::Fixed("integer"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Date,
        BuiltinReceiverMethod::Compare,
        BuiltinReceiverPublicReturnType::Fixed("integer"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Date,
        BuiltinReceiverMethod::IsBefore,
        BuiltinReceiverPublicReturnType::Fixed("bool"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Date,
        BuiltinReceiverMethod::IsAfter,
        BuiltinReceiverPublicReturnType::Fixed("bool"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Duration,
        BuiltinReceiverMethod::ToMilliseconds,
        BuiltinReceiverPublicReturnType::Fixed("integer"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
    spec(
        BuiltinReceiverRoot::Bytes,
        BuiltinReceiverMethod::Length,
        BuiltinReceiverPublicReturnType::Fixed("number"),
        false,
        BuiltinReceiverThrowSemantics::Never,
    ),
    spec(
        BuiltinReceiverRoot::Bytes,
        BuiltinReceiverMethod::ToBase64,
        BuiltinReceiverPublicReturnType::Fixed("string"),
        false,
        BuiltinReceiverThrowSemantics::Never,
    ),
    spec(
        BuiltinReceiverRoot::Bytes,
        BuiltinReceiverMethod::ToHex,
        BuiltinReceiverPublicReturnType::Fixed("string"),
        false,
        BuiltinReceiverThrowSemantics::Never,
    ),
    spec(
        BuiltinReceiverRoot::Bytes,
        BuiltinReceiverMethod::ToUtf8String,
        BuiltinReceiverPublicReturnType::Fixed("string"),
        false,
        BuiltinReceiverThrowSemantics::Decode,
    ),
];

const fn spec(
    receiver: BuiltinReceiverRoot,
    method: BuiltinReceiverMethod,
    public_return_type: BuiltinReceiverPublicReturnType,
    mutates_receiver: bool,
    throws: BuiltinReceiverThrowSemantics,
) -> BuiltinReceiverOpSpec {
    BuiltinReceiverOpSpec {
        op: BuiltinReceiverOp {
            receiver,
            method,
            signature_version: 1,
            canonical_key: canonical_key(receiver, method, 1),
        },
        introduced_capability_version: RECEIVER_BUILTIN_CAPABILITY_VERSION,
        support_status: BuiltinReceiverSupportStatus::Supported,
        public_return_type,
        mutates_receiver,
        throws,
    }
}

pub fn builtin_receiver_op(
    receiver: BuiltinReceiverRoot,
    method: BuiltinReceiverMethod,
    signature_version: u32,
) -> Option<BuiltinReceiverOp> {
    SUPPORTED_RECEIVER_BUILTIN_OPS
        .iter()
        .find(|spec| {
            spec.op.receiver == receiver
                && spec.op.method == method
                && spec.op.signature_version == signature_version
        })
        .map(|spec| spec.op)
}

pub fn canonical_receiver_builtin_key(
    receiver: BuiltinReceiverRoot,
    method: BuiltinReceiverMethod,
    signature_version: u32,
) -> String {
    format!(
        "receiver:{}.{}@{}",
        receiver.as_str(),
        method.as_str(),
        signature_version
    )
}

pub fn validate_supported_receiver_builtin_op(
    op: &BuiltinReceiverOp,
) -> Result<&'static BuiltinReceiverOpSpec, BuiltinReceiverSupportError> {
    validate_receiver_builtin_fields(
        op.receiver,
        op.method,
        op.signature_version,
        op.canonical_key,
    )
}

pub fn validate_receiver_builtin_fields(
    receiver: BuiltinReceiverRoot,
    method: BuiltinReceiverMethod,
    signature_version: u32,
    canonical_key: &str,
) -> Result<&'static BuiltinReceiverOpSpec, BuiltinReceiverSupportError> {
    let expected = canonical_receiver_builtin_key(receiver, method, signature_version);
    if canonical_key != expected {
        return Err(BuiltinReceiverSupportError::CanonicalKeyMismatch {
            expected,
            actual: canonical_key.to_string(),
        });
    }

    let receiver_method_exists = SUPPORTED_RECEIVER_BUILTIN_OPS
        .iter()
        .any(|spec| spec.op.receiver == receiver && spec.op.method == method);
    if !receiver_method_exists {
        return Err(BuiltinReceiverSupportError::UnknownOp {
            receiver,
            method,
            signature_version,
        });
    }

    SUPPORTED_RECEIVER_BUILTIN_OPS
        .iter()
        .find(|spec| {
            spec.op.receiver == receiver
                && spec.op.method == method
                && spec.op.signature_version == signature_version
        })
        .ok_or(BuiltinReceiverSupportError::UnsupportedSignatureVersion {
            receiver,
            method,
            signature_version,
        })
}

pub fn builtin_receiver_op_by_name(root: &str, method: &str) -> Option<BuiltinReceiverOp> {
    let receiver = receiver_root_by_name(root)?;
    let method = receiver_method_by_name(method)?;
    builtin_receiver_op(receiver, method, 1)
}

pub fn builtin_receiver_op_spec_by_name(
    root: &str,
    method: &str,
) -> Option<&'static BuiltinReceiverOpSpec> {
    let receiver = receiver_root_by_name(root)?;
    let method = receiver_method_by_name(method)?;
    SUPPORTED_RECEIVER_BUILTIN_OPS.iter().find(|spec| {
        spec.op.receiver == receiver && spec.op.method == method && spec.op.signature_version == 1
    })
}

pub fn receiver_root_by_name(root: &str) -> Option<BuiltinReceiverRoot> {
    match canonical_runtime_receiver_root(root) {
        "Array" => Some(BuiltinReceiverRoot::Array),
        "Map" => Some(BuiltinReceiverRoot::Map),
        "JsonObject" => Some(BuiltinReceiverRoot::JsonObject),
        "string" => Some(BuiltinReceiverRoot::StringText),
        "integer" | "number" => Some(BuiltinReceiverRoot::Number),
        "Date" => Some(BuiltinReceiverRoot::Date),
        "Duration" => Some(BuiltinReceiverRoot::Duration),
        "bytes" => Some(BuiltinReceiverRoot::Bytes),
        _ => None,
    }
}

pub fn receiver_method_by_name(method: &str) -> Option<BuiltinReceiverMethod> {
    match method {
        "length" => Some(BuiltinReceiverMethod::Length),
        "push" => Some(BuiltinReceiverMethod::Push),
        "set" => Some(BuiltinReceiverMethod::Set),
        "pop" => Some(BuiltinReceiverMethod::Pop),
        "clone" => Some(BuiltinReceiverMethod::Clone),
        "get" => Some(BuiltinReceiverMethod::Get),
        "has" => Some(BuiltinReceiverMethod::Has),
        "delete" => Some(BuiltinReceiverMethod::Delete),
        "keys" => Some(BuiltinReceiverMethod::Keys),
        "contains" => Some(BuiltinReceiverMethod::Contains),
        "replaceAll" => Some(BuiltinReceiverMethod::ReplaceAll),
        "concat" => Some(BuiltinReceiverMethod::Concat),
        "startsWith" => Some(BuiltinReceiverMethod::StartsWith),
        "endsWith" => Some(BuiltinReceiverMethod::EndsWith),
        "lowercase" => Some(BuiltinReceiverMethod::Lowercase),
        "floor" => Some(BuiltinReceiverMethod::Floor),
        "ceil" => Some(BuiltinReceiverMethod::Ceil),
        "round" => Some(BuiltinReceiverMethod::Round),
        "toEpochMilliseconds" => Some(BuiltinReceiverMethod::ToEpochMilliseconds),
        "toISOString" => Some(BuiltinReceiverMethod::ToIsoString),
        "addMilliseconds" => Some(BuiltinReceiverMethod::AddMilliseconds),
        "diffMilliseconds" => Some(BuiltinReceiverMethod::DiffMilliseconds),
        "compare" => Some(BuiltinReceiverMethod::Compare),
        "isBefore" => Some(BuiltinReceiverMethod::IsBefore),
        "isAfter" => Some(BuiltinReceiverMethod::IsAfter),
        "toMilliseconds" => Some(BuiltinReceiverMethod::ToMilliseconds),
        "toBase64" => Some(BuiltinReceiverMethod::ToBase64),
        "toHex" => Some(BuiltinReceiverMethod::ToHex),
        "toUtf8String" => Some(BuiltinReceiverMethod::ToUtf8String),
        _ => None,
    }
}

pub fn canonical_runtime_receiver_root(root: &str) -> &str {
    let root = root.trim();
    match root {
        "std.collection.Array" => "Array",
        "std.collection.Map" => "Map",
        "std.bytes.bytes" => "bytes",
        "std.time.Duration" => "Duration",
        _ => root,
    }
}

const fn canonical_key(
    receiver: BuiltinReceiverRoot,
    method: BuiltinReceiverMethod,
    signature_version: u32,
) -> &'static str {
    match (receiver, method, signature_version) {
        (BuiltinReceiverRoot::Array, BuiltinReceiverMethod::Length, 1) => "receiver:Array.length@1",
        (BuiltinReceiverRoot::Array, BuiltinReceiverMethod::Push, 1) => "receiver:Array.push@1",
        (BuiltinReceiverRoot::Array, BuiltinReceiverMethod::Set, 1) => "receiver:Array.set@1",
        (BuiltinReceiverRoot::Array, BuiltinReceiverMethod::Pop, 1) => "receiver:Array.pop@1",
        (BuiltinReceiverRoot::Array, BuiltinReceiverMethod::Clone, 1) => "receiver:Array.clone@1",
        (BuiltinReceiverRoot::Map, BuiltinReceiverMethod::Length, 1) => "receiver:Map.length@1",
        (BuiltinReceiverRoot::Map, BuiltinReceiverMethod::Get, 1) => "receiver:Map.get@1",
        (BuiltinReceiverRoot::Map, BuiltinReceiverMethod::Has, 1) => "receiver:Map.has@1",
        (BuiltinReceiverRoot::Map, BuiltinReceiverMethod::Set, 1) => "receiver:Map.set@1",
        (BuiltinReceiverRoot::Map, BuiltinReceiverMethod::Delete, 1) => "receiver:Map.delete@1",
        (BuiltinReceiverRoot::Map, BuiltinReceiverMethod::Keys, 1) => "receiver:Map.keys@1",
        (BuiltinReceiverRoot::Map, BuiltinReceiverMethod::Clone, 1) => "receiver:Map.clone@1",
        (BuiltinReceiverRoot::JsonObject, BuiltinReceiverMethod::Length, 1) => {
            "receiver:JsonObject.length@1"
        }
        (BuiltinReceiverRoot::JsonObject, BuiltinReceiverMethod::Get, 1) => {
            "receiver:JsonObject.get@1"
        }
        (BuiltinReceiverRoot::JsonObject, BuiltinReceiverMethod::Has, 1) => {
            "receiver:JsonObject.has@1"
        }
        (BuiltinReceiverRoot::JsonObject, BuiltinReceiverMethod::Set, 1) => {
            "receiver:JsonObject.set@1"
        }
        (BuiltinReceiverRoot::JsonObject, BuiltinReceiverMethod::Delete, 1) => {
            "receiver:JsonObject.delete@1"
        }
        (BuiltinReceiverRoot::JsonObject, BuiltinReceiverMethod::Clone, 1) => {
            "receiver:JsonObject.clone@1"
        }
        (BuiltinReceiverRoot::StringText, BuiltinReceiverMethod::Length, 1) => {
            "receiver:string.length@1"
        }
        (BuiltinReceiverRoot::StringText, BuiltinReceiverMethod::Contains, 1) => {
            "receiver:string.contains@1"
        }
        (BuiltinReceiverRoot::StringText, BuiltinReceiverMethod::ReplaceAll, 1) => {
            "receiver:string.replaceAll@1"
        }
        (BuiltinReceiverRoot::StringText, BuiltinReceiverMethod::Concat, 1) => {
            "receiver:string.concat@1"
        }
        (BuiltinReceiverRoot::StringText, BuiltinReceiverMethod::StartsWith, 1) => {
            "receiver:string.startsWith@1"
        }
        (BuiltinReceiverRoot::StringText, BuiltinReceiverMethod::EndsWith, 1) => {
            "receiver:string.endsWith@1"
        }
        (BuiltinReceiverRoot::StringText, BuiltinReceiverMethod::Lowercase, 1) => {
            "receiver:string.lowercase@1"
        }
        (BuiltinReceiverRoot::Number, BuiltinReceiverMethod::Floor, 1) => "receiver:number.floor@1",
        (BuiltinReceiverRoot::Number, BuiltinReceiverMethod::Ceil, 1) => "receiver:number.ceil@1",
        (BuiltinReceiverRoot::Number, BuiltinReceiverMethod::Round, 1) => "receiver:number.round@1",
        (BuiltinReceiverRoot::Date, BuiltinReceiverMethod::ToEpochMilliseconds, 1) => {
            "receiver:Date.toEpochMilliseconds@1"
        }
        (BuiltinReceiverRoot::Date, BuiltinReceiverMethod::ToIsoString, 1) => {
            "receiver:Date.toISOString@1"
        }
        (BuiltinReceiverRoot::Date, BuiltinReceiverMethod::AddMilliseconds, 1) => {
            "receiver:Date.addMilliseconds@1"
        }
        (BuiltinReceiverRoot::Date, BuiltinReceiverMethod::DiffMilliseconds, 1) => {
            "receiver:Date.diffMilliseconds@1"
        }
        (BuiltinReceiverRoot::Date, BuiltinReceiverMethod::Compare, 1) => "receiver:Date.compare@1",
        (BuiltinReceiverRoot::Date, BuiltinReceiverMethod::IsBefore, 1) => {
            "receiver:Date.isBefore@1"
        }
        (BuiltinReceiverRoot::Date, BuiltinReceiverMethod::IsAfter, 1) => "receiver:Date.isAfter@1",
        (BuiltinReceiverRoot::Duration, BuiltinReceiverMethod::ToMilliseconds, 1) => {
            "receiver:Duration.toMilliseconds@1"
        }
        (BuiltinReceiverRoot::Bytes, BuiltinReceiverMethod::Length, 1) => "receiver:bytes.length@1",
        (BuiltinReceiverRoot::Bytes, BuiltinReceiverMethod::ToBase64, 1) => {
            "receiver:bytes.toBase64@1"
        }
        (BuiltinReceiverRoot::Bytes, BuiltinReceiverMethod::ToHex, 1) => "receiver:bytes.toHex@1",
        (BuiltinReceiverRoot::Bytes, BuiltinReceiverMethod::ToUtf8String, 1) => {
            "receiver:bytes.toUtf8String@1"
        }
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_and_duration_receiver_ops_publish_integer_return_types() {
        for (root, method) in [
            ("Date", "toEpochMilliseconds"),
            ("Date", "diffMilliseconds"),
            ("Date", "compare"),
            ("Duration", "toMilliseconds"),
        ] {
            let spec = builtin_receiver_op_spec_by_name(root, method)
                .expect("builtin receiver op spec should exist");
            assert_eq!(
                spec.public_return_type,
                BuiltinReceiverPublicReturnType::Fixed("integer"),
                "{root}.{method} should publish integer return type"
            );
        }
    }

    #[test]
    fn string_replace_all_receiver_op_is_supported() {
        let spec = builtin_receiver_op_spec_by_name("string", "replaceAll")
            .expect("string.replaceAll receiver op should exist");

        assert_eq!(
            spec.public_return_type,
            BuiltinReceiverPublicReturnType::Fixed("string")
        );
        assert_eq!(spec.op.canonical_key, "receiver:string.replaceAll@1");
    }
}
