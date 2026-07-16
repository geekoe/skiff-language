use serde::Serialize;
use skiff_artifact_model::{CallableEffectSummary, TypeRefIr};

/// Runtime operation decoration. The callable effect is a semantic fact from
/// source compilation; stream carrier details remain a projection concern.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationEffectProjection {
    effects: CallableEffectSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    produces: Option<TypeRefIr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    emits: Option<TypeRefIr>,
}

pub fn operation_effect_projection(effects: CallableEffectSummary) -> OperationEffectProjection {
    OperationEffectProjection {
        effects,
        produces: None,
        emits: None,
    }
}

pub fn operation_effect_projection_for_signature(
    effects: CallableEffectSummary,
    return_type: &TypeRefIr,
) -> OperationEffectProjection {
    let mut projection = operation_effect_projection(effects);
    if let TypeRefIr::Native { name, args } = return_type {
        if name == "Stream" && args.len() == 1 {
            projection.produces = Some(return_type.clone());
            projection.emits = Some(args[0].clone());
        }
    }
    projection
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn runtime_projection_carries_typed_unknown_without_placeholder_precision() {
        let value = serde_json::to_value(operation_effect_projection(
            CallableEffectSummary::analysis_pending(),
        ))
        .expect("effect projection serializes");
        assert_eq!(
            value,
            json!({
                "effects": {
                    "kind": "unknown",
                    "reason": "analysisPending"
                }
            })
        );
        assert!(value.get("precision").is_none());
    }

    #[test]
    fn stream_carrier_projection_does_not_replace_callable_effects() {
        let stream = TypeRefIr::Native {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::native("string")],
        };
        let value = serde_json::to_value(operation_effect_projection_for_signature(
            CallableEffectSummary::analysis_pending(),
            &stream,
        ))
        .expect("stream effect projection serializes");
        assert_eq!(value["effects"]["kind"], "unknown");
        assert_eq!(value["produces"], serde_json::to_value(&stream).unwrap());
        assert_eq!(
            value["emits"],
            serde_json::to_value(TypeRefIr::native("string")).unwrap()
        );
    }
}
