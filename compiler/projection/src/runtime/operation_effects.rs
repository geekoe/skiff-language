use serde::Serialize;
use skiff_artifact_model::TypeRefIr;

const EFFECT_SUMMARY_SCHEMA_VERSION: &str = "skiff-effect-summary-v1";

/// A single inferred effect of a service operation.
///
/// The effect model is not implemented yet (every summary is currently emitted
/// with `precision: "placeholder"` and no effects), but the type is a closed,
/// typed enum rather than `serde_json::Value`: the schema projection layer must
/// not use `Value` as an internal protocol. Variants are added here as the
/// effect-inference passes that produce them land. Serialization is
/// tag/content shaped (`{ "kind": ... }`) so artifact bytes are stable.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Effect {}

/// Effect summary for a service operation. `produces`/`emits` are only present
/// for stream-returning operations. Field order matches the former `json!`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectSummary {
    schema_version: &'static str,
    precision: &'static str,
    effects: Vec<Effect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    produces: Option<TypeRefIr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    emits: Option<TypeRefIr>,
}

pub fn effect_summary() -> EffectSummary {
    EffectSummary {
        schema_version: EFFECT_SUMMARY_SCHEMA_VERSION,
        precision: "placeholder",
        effects: Vec::new(),
        produces: None,
        emits: None,
    }
}

pub fn effect_summary_for_signature(return_type: &TypeRefIr) -> EffectSummary {
    let mut summary = effect_summary();
    if let TypeRefIr::Native { name, args } = return_type {
        if name == "Stream" && args.len() == 1 {
            summary.produces = Some(return_type.clone());
            summary.emits = Some(args[0].clone());
        }
    }
    summary
}
