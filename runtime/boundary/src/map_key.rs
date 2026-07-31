use crate::{
    error::{Result, RuntimeError},
    runtime_value::RuntimeValueKey,
    type_descriptor::{is_builtin_named_type, RuntimeTypeNode, RuntimeTypePlan},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeMapKeyShape {
    PlainString,
}

impl RuntimeMapKeyShape {
    pub fn for_plan(key_type: &RuntimeTypePlan) -> Result<Self> {
        match key_type.node() {
            RuntimeTypeNode::String | RuntimeTypeNode::Json => Ok(Self::PlainString),
            RuntimeTypeNode::Representation { payload, .. } => {
                if !is_string_key_payload_plan(payload) {
                    return Err(RuntimeError::Decode(
                        "Map key representation payload must be string".to_string(),
                    ));
                }
                Ok(Self::PlainString)
            }
            RuntimeTypeNode::Alias(target) => Self::for_plan(target),
            _ => Err(RuntimeError::Decode(
                "Map key type must be string or representation over string".to_string(),
            )),
        }
        .or_else(|error| match key_type.named_type_name() {
            Some(type_name) if !is_builtin_named_type(type_name) => Ok(Self::PlainString),
            _ => Err(error),
        })
    }

    pub fn encode_runtime_key<'a>(&self, key: &'a RuntimeValueKey) -> Result<&'a str> {
        match self {
            Self::PlainString => match key {
                RuntimeValueKey::String(value) => Ok(value.as_str()),
            },
        }
    }

    pub fn decode_runtime_key(&self, value: String) -> RuntimeValueKey {
        match self {
            Self::PlainString => RuntimeValueKey::string(value),
        }
    }
}

fn is_string_key_payload_plan(payload_type: &RuntimeTypePlan) -> bool {
    match payload_type.node() {
        RuntimeTypeNode::Alias(target) => is_string_key_payload_plan(target),
        RuntimeTypeNode::String => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
