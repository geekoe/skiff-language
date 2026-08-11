use std::collections::HashSet;
use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use skiff_canonical_json::canonical_json_number;

use crate::{
    protocol::{BINARY_FRAME_HEADER_ENCODING_JSON, BINARY_FRAME_MAGIC, BINARY_FRAME_VERSION},
    BinaryFrameError, TransportError,
};

const FIXED_HEADER_BYTES: usize = 14;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub(super) fn decode_bytecode_request_json_frame(
    frame: &[u8],
) -> Result<(Value, Vec<u8>), BinaryFrameError> {
    decode_bytecode_json_frame(frame, "runtimeAssembly request.start")
}

pub(super) fn decode_bytecode_json_frame(
    frame: &[u8],
    label: &str,
) -> Result<(Value, Vec<u8>), BinaryFrameError> {
    let (header, payload) = canonical_frame_parts(frame)?;
    let header = serde_json::from_slice::<StrictCanonicalJsonValue>(header)
        .map_err(|error| {
            TransportError::decode(format!("invalid {label} frame header JSON: {error}"))
        })?
        .0;
    if !header.is_object() {
        return Err(TransportError::decode(format!(
            "invalid {label} frame: header must be an object"
        )));
    }
    Ok((header, payload.to_vec()))
}

fn canonical_frame_parts(frame: &[u8]) -> Result<(&[u8], &[u8]), BinaryFrameError> {
    if frame.len() < FIXED_HEADER_BYTES {
        return Err(TransportError::decode(
            "invalid skiff binary frame: frame is too short",
        ));
    }
    if frame[0..4] != BINARY_FRAME_MAGIC {
        return Err(TransportError::decode(
            "invalid skiff binary frame: expected skiff binary frame magic",
        ));
    }
    if frame[4] != BINARY_FRAME_VERSION {
        return Err(TransportError::decode(format!(
            "invalid skiff binary frame: unsupported frame version {}",
            frame[4]
        )));
    }
    if frame[5] != BINARY_FRAME_HEADER_ENCODING_JSON {
        return Err(TransportError::decode(format!(
            "invalid skiff binary frame: unsupported header encoding {}",
            frame[5]
        )));
    }
    let header_length = u32::from_be_bytes([frame[6], frame[7], frame[8], frame[9]]) as usize;
    let payload_length = u32::from_be_bytes([frame[10], frame[11], frame[12], frame[13]]) as usize;
    if header_length == 0 {
        return Err(TransportError::decode(
            "invalid skiff binary frame: header must not be empty",
        ));
    }
    let expected_length = FIXED_HEADER_BYTES
        .checked_add(header_length)
        .and_then(|length| length.checked_add(payload_length))
        .ok_or_else(|| {
            TransportError::decode("invalid skiff binary frame: frame length overflow")
        })?;
    if frame.len() != expected_length {
        return Err(TransportError::decode(format!(
            "invalid skiff binary frame: frame length {} does not match header length {} plus payload length {}",
            frame.len(), header_length, payload_length
        )));
    }
    let payload_start = FIXED_HEADER_BYTES + header_length;
    Ok((
        &frame[FIXED_HEADER_BYTES..payload_start],
        &frame[payload_start..],
    ))
}

struct StrictCanonicalJsonValue(Value);

impl<'de> Deserialize<'de> for StrictCanonicalJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictCanonicalJsonVisitor)
    }
}

struct StrictCanonicalJsonVisitor;

impl<'de> Visitor<'de> for StrictCanonicalJsonVisitor {
    type Value = StrictCanonicalJsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("canonical request JSON without duplicate keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictCanonicalJsonValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value.unsigned_abs() > MAX_SAFE_INTEGER {
            return Err(E::custom("JSON integer exceeds Number.MAX_SAFE_INTEGER"));
        }
        Ok(StrictCanonicalJsonValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value > MAX_SAFE_INTEGER {
            return Err(E::custom("JSON integer exceeds Number.MAX_SAFE_INTEGER"));
        }
        Ok(StrictCanonicalJsonValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let number =
            Number::from_f64(value).ok_or_else(|| E::custom("JSON number must be finite"))?;
        let normalized = if value == 0.0 && value.is_sign_negative() {
            Value::Number(number)
        } else {
            canonical_json_number(&number)
        };
        reject_unsafe_normalized_integer::<E>(&normalized)?;
        Ok(StrictCanonicalJsonValue(normalized))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictCanonicalJsonValue(Value::String(value.to_string())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictCanonicalJsonValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictCanonicalJsonValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictCanonicalJsonValue(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        StrictCanonicalJsonValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictCanonicalJsonValue>()? {
            values.push(value.0);
        }
        Ok(StrictCanonicalJsonValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!(
                    "duplicate JSON object key {key}"
                )));
            }
            values.insert(key, object.next_value::<StrictCanonicalJsonValue>()?.0);
        }
        Ok(StrictCanonicalJsonValue(Value::Object(values)))
    }
}

fn reject_unsafe_normalized_integer<E>(value: &Value) -> Result<(), E>
where
    E: de::Error,
{
    let Value::Number(number) = value else {
        return Ok(());
    };
    if let Some(value) = number.as_i64() {
        if value.unsigned_abs() > MAX_SAFE_INTEGER {
            return Err(E::custom("JSON integer exceeds Number.MAX_SAFE_INTEGER"));
        }
        return Ok(());
    }
    if let Some(value) = number.as_u64() {
        if value > MAX_SAFE_INTEGER {
            return Err(E::custom("JSON integer exceeds Number.MAX_SAFE_INTEGER"));
        }
        return Ok(());
    }
    if number.as_f64().is_some_and(|value| {
        value.is_finite() && value.fract() == 0.0 && value.abs() > MAX_SAFE_INTEGER as f64
    }) {
        return Err(E::custom("JSON integer exceeds Number.MAX_SAFE_INTEGER"));
    }
    Ok(())
}
