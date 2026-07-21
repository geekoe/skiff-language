use std::collections::HashSet;
use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

use crate::{
    protocol::{BINARY_FRAME_HEADER_ENCODING_JSON, BINARY_FRAME_MAGIC, BINARY_FRAME_VERSION},
    BinaryFrameError, TransportError,
};

const FIXED_HEADER_BYTES: usize = 14;

pub(super) fn reject_runtime_assembly_request_header_duplicates(
    frame: &[u8],
) -> Result<(), BinaryFrameError> {
    let Some(header) = canonical_header_slice(frame) else {
        return Ok(());
    };
    serde_json::from_slice::<DuplicateRejectingJsonValue>(header).map_err(|error| {
        TransportError::decode(format!(
            "invalid runtimeAssembly request.start frame header JSON: {error}"
        ))
    })?;
    Ok(())
}

fn canonical_header_slice(frame: &[u8]) -> Option<&[u8]> {
    if frame.len() < FIXED_HEADER_BYTES
        || frame[0..4] != BINARY_FRAME_MAGIC
        || frame[4] != BINARY_FRAME_VERSION
        || frame[5] != BINARY_FRAME_HEADER_ENCODING_JSON
    {
        return None;
    }
    let header_length = u32::from_be_bytes([frame[6], frame[7], frame[8], frame[9]]) as usize;
    let payload_length = u32::from_be_bytes([frame[10], frame[11], frame[12], frame[13]]) as usize;
    let expected_length = FIXED_HEADER_BYTES
        .checked_add(header_length)?
        .checked_add(payload_length)?;
    if header_length == 0 || frame.len() != expected_length {
        return None;
    }
    Some(&frame[FIXED_HEADER_BYTES..FIXED_HEADER_BYTES + header_length])
}

struct DuplicateRejectingJsonValue;

impl<'de> Deserialize<'de> for DuplicateRejectingJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(DuplicateRejectingJsonVisitor)
    }
}

struct DuplicateRejectingJsonVisitor;

impl<'de> Visitor<'de> for DuplicateRejectingJsonVisitor {
    type Value = DuplicateRejectingJsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingJsonValue)
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingJsonValue)
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingJsonValue)
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingJsonValue)
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingJsonValue)
    }

    fn visit_string<E>(self, _value: String) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingJsonValue)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingJsonValue)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingJsonValue)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        DuplicateRejectingJsonValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence
            .next_element::<DuplicateRejectingJsonValue>()?
            .is_some()
        {}
        Ok(DuplicateRejectingJsonValue)
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        while let Some(key) = object.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!(
                    "duplicate JSON object key {key}"
                )));
            }
            object.next_value::<DuplicateRejectingJsonValue>()?;
        }
        Ok(DuplicateRejectingJsonValue)
    }
}
