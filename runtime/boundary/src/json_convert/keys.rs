use crate::{
    error::Result, map_key::RuntimeMapKeyShape, runtime_value::RuntimeValueKey,
    type_descriptor::RuntimeTypePlan,
};

pub(super) fn require_plain_runtime_key(key: &RuntimeValueKey) -> Result<()> {
    match key {
        RuntimeValueKey::String(_) => Ok(()),
    }
}

pub(super) fn runtime_field_name_from_map_key(key: &RuntimeValueKey) -> Result<String> {
    match key {
        RuntimeValueKey::String(value) => Ok(value.clone()),
    }
}

pub(super) fn runtime_key_from_wire_key_plan(
    key: &str,
    key_type: &RuntimeTypePlan,
) -> Result<RuntimeValueKey> {
    Ok(RuntimeMapKeyShape::for_plan(key_type)?.decode_runtime_key(key.to_string()))
}

pub(super) fn wire_key_from_runtime_key_plan(
    key: &RuntimeValueKey,
    key_type: &RuntimeTypePlan,
) -> Result<String> {
    RuntimeMapKeyShape::for_plan(key_type)?
        .encode_runtime_key(key)
        .map(str::to_string)
}
