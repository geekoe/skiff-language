use std::fmt;

use serde::{de, Deserialize, Deserializer, Serialize};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BytecodeRequestClientSessionFrameHeader {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BytecodeRequestDeadlineFrameHeader {
    #[serde(deserialize_with = "deserialize_safe_unsigned_integer")]
    pub timeout_ms: u64,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BytecodeRequestTraceFrameHeader {
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
pub struct BytecodeRequestNameValueFrameHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BytecodeHttpRequestFrameHeader {
    pub method: String,
    pub url: String,
    pub path: String,
    pub query: Vec<BytecodeRequestNameValueFrameHeader>,
    pub headers: Vec<BytecodeRequestNameValueFrameHeader>,
}

pub(super) fn deserialize_present_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
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
