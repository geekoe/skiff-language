//! Test-support projection for legacy JSON descriptor fixtures.
//!
//! The descriptor bridge remains intentionally outside production. Its WebSocket records and
//! unions are projected from the artifact-owned canonical spec so fixture drift cannot establish
//! another schema owner.

use std::collections::BTreeSet;

use skiff_artifact_model::websocket_ingress::{
    canonical_websocket_shape_spec, CanonicalWebSocketShapeSpec, WebSocketShape,
    WebSocketShapeField, WebSocketShapeId, WebSocketShapeType,
};
use skiff_runtime_model::type_plan::{
    RuntimeRecordFieldPlan, RuntimeTypeIdentityPlan, RuntimeTypeNode, RuntimeTypePlan,
};

pub fn canonical_websocket_descriptor_plan(
    shape_id: WebSocketShapeId,
    context: Option<&RuntimeTypePlan>,
) -> Option<RuntimeTypePlan> {
    let spec = canonical_websocket_shape_spec();
    project_shape(spec, shape_id, context, &mut BTreeSet::new())
}

fn project_shape(
    spec: &CanonicalWebSocketShapeSpec,
    shape_id: WebSocketShapeId,
    context: Option<&RuntimeTypePlan>,
    active: &mut BTreeSet<WebSocketShapeId>,
) -> Option<RuntimeTypePlan> {
    if !active.insert(shape_id) {
        return None;
    }

    let name = shape_id.canonical_name();
    let projected = match spec.shape(shape_id) {
        WebSocketShape::Record { fields } => {
            let fields = fields
                .iter()
                .map(|field| project_field(spec, name, field, context, active))
                .collect::<Option<Vec<_>>>()?;
            record_plan(name, fields)
        }
        WebSocketShape::TaggedUnion {
            discriminator_field,
            variants,
        } => {
            let variants = variants
                .iter()
                .map(|variant| {
                    variant.tag(discriminator_field)?;
                    let variant_name = variant.canonical_name();
                    let fields = variant
                        .fields()
                        .iter()
                        .map(|field| project_field(spec, variant_name, field, context, active))
                        .collect::<Option<Vec<_>>>()?;
                    Some(record_plan(variant_name, fields))
                })
                .collect::<Option<Vec<_>>>()?;
            let union = union_plan(name, variants);
            if shape_id == WebSocketShapeId::Message {
                builtin_plan(
                    name,
                    RuntimeTypeNode::Representation {
                        type_name: name.to_string(),
                        payload: Box::new(union),
                    },
                )
            } else {
                union
            }
        }
    };

    active.remove(&shape_id);
    Some(projected)
}

fn project_field(
    spec: &CanonicalWebSocketShapeSpec,
    owner_name: &str,
    field: &WebSocketShapeField,
    context: Option<&RuntimeTypePlan>,
    active: &mut BTreeSet<WebSocketShapeId>,
) -> Option<RuntimeRecordFieldPlan> {
    let literal_union_name = format!("{owner_name}.{}", field.name());
    let ty = project_type(spec, field.ty(), context, active, &literal_union_name)?;
    let required = !matches!(ty.node, RuntimeTypeNode::Nullable(_));
    Some(RuntimeRecordFieldPlan {
        name: field.name().to_string(),
        ty,
        required,
        identity: None,
    })
}

fn project_type(
    spec: &CanonicalWebSocketShapeSpec,
    ty: &WebSocketShapeType,
    context: Option<&RuntimeTypePlan>,
    active: &mut BTreeSet<WebSocketShapeId>,
    literal_union_name: &str,
) -> Option<RuntimeTypePlan> {
    match ty {
        WebSocketShapeType::String => Some(leaf_plan("string", RuntimeTypeNode::String)),
        WebSocketShapeType::Integer => Some(leaf_plan("integer", RuntimeTypeNode::Integer)),
        WebSocketShapeType::Context => context.cloned(),
        WebSocketShapeType::Shape(shape_id) => project_shape(spec, *shape_id, context, active),
        WebSocketShapeType::Array(item) => Some(builtin_plan(
            "Array",
            RuntimeTypeNode::Array(Box::new(project_type(
                spec,
                item,
                context,
                active,
                literal_union_name,
            )?)),
        )),
        WebSocketShapeType::Nullable(inner) => Some(RuntimeTypePlan {
            label: "nullable".to_string(),
            named_type_name: None,
            identity: RuntimeTypeIdentityPlan::default(),
            node: RuntimeTypeNode::Nullable(Box::new(project_type(
                spec,
                inner,
                context,
                active,
                literal_union_name,
            )?)),
        }),
        WebSocketShapeType::StringLiteral(value) => Some(RuntimeTypePlan {
            label: "literal".to_string(),
            named_type_name: None,
            identity: RuntimeTypeIdentityPlan::default(),
            node: RuntimeTypeNode::LiteralString((*value).to_string()),
        }),
        WebSocketShapeType::StringLiteralUnion(values) => Some(union_plan(
            literal_union_name,
            values
                .iter()
                .map(|value| RuntimeTypePlan {
                    label: "literal".to_string(),
                    named_type_name: None,
                    identity: RuntimeTypeIdentityPlan::default(),
                    node: RuntimeTypeNode::LiteralString((*value).to_string()),
                })
                .collect(),
        )),
    }
}

fn leaf_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
    builtin_plan(name, node)
}

fn record_plan(name: &str, fields: Vec<RuntimeRecordFieldPlan>) -> RuntimeTypePlan {
    builtin_plan(
        name,
        RuntimeTypeNode::Record {
            fields,
            boundary_record_kind: Some(name.to_string()),
        },
    )
}

fn union_plan(name: &str, variants: Vec<RuntimeTypePlan>) -> RuntimeTypePlan {
    builtin_plan(name, RuntimeTypeNode::Union(variants))
}

fn builtin_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "builtin".to_string(),
        named_type_name: Some(name.to_string()),
        identity: RuntimeTypeIdentityPlan::default(),
        node,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context_marker() -> RuntimeTypePlan {
        RuntimeTypePlan {
            label: "websocket-context-placeholder".to_string(),
            named_type_name: None,
            identity: RuntimeTypeIdentityPlan::default(),
            node: RuntimeTypeNode::Unknown,
        }
    }

    #[test]
    fn test_support_projects_every_canonical_websocket_shape() {
        let context = context_marker();
        let projected = canonical_websocket_shape_spec()
            .shapes()
            .map(|(shape_id, _)| {
                canonical_websocket_descriptor_plan(shape_id, Some(&context))
                    .expect("canonical WebSocket shape must project")
                    .named_type_name
                    .expect("canonical WebSocket shape must stay named")
            })
            .collect::<BTreeSet<_>>();
        let expected = canonical_websocket_shape_spec()
            .shapes()
            .map(|(shape_id, _)| shape_id.canonical_name().to_string())
            .collect::<BTreeSet<_>>();

        assert_eq!(projected, expected);
    }

    #[test]
    fn test_support_contract_builtin_vocabulary_remains_event_and_result() {
        let builtins = canonical_websocket_shape_spec()
            .contract_builtins()
            .iter()
            .map(|builtin| (builtin.name(), builtin.context_arity(), builtin.shape()))
            .collect::<Vec<_>>();

        assert_eq!(
            builtins,
            [
                (
                    "std.websocket.WebSocketIngressEvent",
                    1,
                    WebSocketShapeId::Event,
                ),
                (
                    "std.websocket.WebSocketConnectResult",
                    1,
                    WebSocketShapeId::Result,
                ),
            ]
        );
    }

    #[test]
    fn test_support_requires_the_canonical_context_placeholder() {
        assert!(canonical_websocket_descriptor_plan(WebSocketShapeId::Event, None).is_none());
        assert!(canonical_websocket_descriptor_plan(WebSocketShapeId::Result, None).is_none());
        assert!(
            canonical_websocket_descriptor_plan(WebSocketShapeId::Message, None).is_some(),
            "context-free nested shapes must not invent a placeholder"
        );
    }
}
