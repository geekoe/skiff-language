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
mod tests {
    use serde_json::json;

    use super::*;
    use crate::type_descriptor::RuntimeTypePlanDescriptorExt;

    #[test]
    fn map_key_shape_uses_type_plan() {
        let representation = json!({
            "kind": "representation",
            "name": "UserId",
            "representation": { "kind": "builtin", "name": "string", "args": [] }
        });
        let expected = RuntimeMapKeyShape::PlainString;

        let plan = RuntimeTypePlan::from_descriptor(&representation).expect("map key plan");
        assert_eq!(
            RuntimeMapKeyShape::for_plan(&plan).expect("map key shape"),
            expected
        );
    }

    #[test]
    fn map_key_shape_erases_custom_named_keys_to_plain_strings() {
        let descriptor = json!({ "kind": "builtin", "name": "UserId", "args": [] });
        let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("custom key plan");
        let shape = RuntimeMapKeyShape::for_plan(&plan).expect("custom key shape");

        assert_eq!(
            shape.decode_runtime_key("u1".to_string()),
            RuntimeValueKey::string("u1")
        );
    }

    #[test]
    fn map_key_shape_rejects_numeric_representation_payloads() {
        let descriptor = json!({
            "kind": "representation",
            "name": "NumericId",
            "representation": { "kind": "builtin", "name": "number", "args": [] }
        });

        let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("numeric key plan");
        let error = RuntimeMapKeyShape::for_plan(&plan)
            .expect_err("numeric key payload should be rejected");

        assert!(error
            .to_string()
            .contains("Map key representation payload must be string"));
    }
}
