use skiff_artifact_model::{
    websocket_ingress::{
        canonical_websocket_shape_spec, CanonicalWebSocketShapeSpec, WebSocketContractBuiltin,
        WebSocketShape, WebSocketShapeField, WebSocketShapeId, WebSocketShapeType,
    },
    TypeRefIr,
};
use skiff_runtime_boundary::{
    type_descriptor::RuntimeTypePlanDescriptorExt,
    websocket_shape_descriptor::canonical_websocket_descriptor_plan,
};
use skiff_runtime_model::type_plan::{RuntimeTypeNode, RuntimeTypePlan};

use crate::type_plan::RuntimeTypePlanLinkedExt;

const CONTEXT_MARKER: &str = "p5-f24d.ContextPlaceholder";

#[derive(Clone, Debug, PartialEq, Eq)]
enum ShapeFingerprint {
    String,
    Integer,
    Context,
    Array(Box<Self>),
    Nullable(Box<Self>),
    Literal(String),
    NamedRecord {
        name: String,
        fields: Vec<FieldFingerprint>,
    },
    NamedUnion {
        name: String,
        variants: Vec<Self>,
    },
    Representation {
        name: String,
        payload: Box<Self>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FieldFingerprint {
    name: String,
    required: bool,
    ty: ShapeFingerprint,
}

#[test]
fn websocket_shape_parity_matches_spec_for_linked_and_descriptor_consumers() {
    let spec = canonical_websocket_shape_spec();
    let builtin_vocabulary = spec
        .contract_builtins()
        .iter()
        .map(|builtin| (builtin.name(), builtin.context_arity(), builtin.shape()))
        .collect::<Vec<_>>();
    assert_eq!(
        builtin_vocabulary,
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

    for builtin in [
        WebSocketContractBuiltin::Event,
        WebSocketContractBuiltin::Result,
    ] {
        let builtin = spec.contract_builtin(builtin);
        let expected = spec_shape_fingerprint(spec, builtin.shape());
        let linked = linked_builtin_plan(builtin.name());
        let descriptor = descriptor_builtin_plan(builtin.name());
        let direct_descriptor =
            canonical_websocket_descriptor_plan(builtin.shape(), Some(&context_marker_plan()))
                .expect("test-support descriptor must project canonical WebSocket shape");

        assert_eq!(plan_shape_fingerprint(&linked), expected);
        assert_eq!(plan_shape_fingerprint(&descriptor), expected);
        assert_eq!(plan_shape_fingerprint(&direct_descriptor), expected);
    }
}

#[test]
fn websocket_shape_parity_detects_bidirectional_field_order_nullable_tag_and_context_drift() {
    let spec = canonical_websocket_shape_spec();
    let expected = canonical_corpus(spec);
    let linked = linked_corpus(spec);
    let descriptor = descriptor_corpus(spec);
    assert_eq!(linked, expected);
    assert_eq!(descriptor, expected);

    let mutations: [fn(&mut [ShapeFingerprint]) -> bool; 5] = [
        mutate_field_name,
        mutate_field_order,
        mutate_nullable,
        mutate_tag,
        mutate_context,
    ];
    for mutate in mutations {
        let mut linked_drift = linked.clone();
        assert!(mutate(&mut linked_drift), "linked mutation must apply");
        assert_ne!(linked_drift, expected, "linked drift must be detected");

        let mut descriptor_drift = descriptor.clone();
        assert!(
            mutate(&mut descriptor_drift),
            "descriptor mutation must apply"
        );
        assert_ne!(
            descriptor_drift, expected,
            "test-support descriptor drift must be detected"
        );
    }
}

fn canonical_corpus(spec: &CanonicalWebSocketShapeSpec) -> Vec<ShapeFingerprint> {
    spec.contract_builtins()
        .iter()
        .map(|builtin| spec_shape_fingerprint(spec, builtin.shape()))
        .collect()
}

fn linked_corpus(spec: &CanonicalWebSocketShapeSpec) -> Vec<ShapeFingerprint> {
    spec.contract_builtins()
        .iter()
        .map(|builtin| plan_shape_fingerprint(&linked_builtin_plan(builtin.name())))
        .collect()
}

fn descriptor_corpus(spec: &CanonicalWebSocketShapeSpec) -> Vec<ShapeFingerprint> {
    spec.contract_builtins()
        .iter()
        .map(|builtin| plan_shape_fingerprint(&descriptor_builtin_plan(builtin.name())))
        .collect()
}

fn linked_builtin_plan(name: &str) -> RuntimeTypePlan {
    let type_ref = TypeRefIr::Native {
        name: name.to_string(),
        args: vec![TypeRefIr::native(CONTEXT_MARKER)],
    };
    <RuntimeTypePlan as RuntimeTypePlanLinkedExt>::from_artifact_type_ref(&type_ref)
        .expect("linked WebSocket builtin plan must build")
}

fn descriptor_builtin_plan(name: &str) -> RuntimeTypePlan {
    let descriptor = serde_json::json!({
        "kind": "builtin",
        "name": name,
        "args": [{ "kind": "builtin", "name": CONTEXT_MARKER, "args": [] }],
    });
    <RuntimeTypePlan as RuntimeTypePlanDescriptorExt>::from_descriptor(&descriptor)
        .expect("test-support WebSocket descriptor plan must build")
}

fn context_marker_plan() -> RuntimeTypePlan {
    let type_ref = TypeRefIr::native(CONTEXT_MARKER);
    <RuntimeTypePlan as RuntimeTypePlanLinkedExt>::from_artifact_type_ref(&type_ref)
        .expect("Context placeholder plan must build")
}

fn spec_shape_fingerprint(
    spec: &CanonicalWebSocketShapeSpec,
    shape_id: WebSocketShapeId,
) -> ShapeFingerprint {
    let name = shape_id.canonical_name().to_string();
    match spec.shape(shape_id) {
        WebSocketShape::Record { fields } => ShapeFingerprint::NamedRecord {
            name,
            fields: fields
                .iter()
                .map(|field| spec_field_fingerprint(spec, shape_id.canonical_name(), field))
                .collect(),
        },
        WebSocketShape::TaggedUnion { variants, .. } => {
            let union = ShapeFingerprint::NamedUnion {
                name: name.clone(),
                variants: variants
                    .iter()
                    .map(|variant| ShapeFingerprint::NamedRecord {
                        name: variant.canonical_name().to_string(),
                        fields: variant
                            .fields()
                            .iter()
                            .map(|field| {
                                spec_field_fingerprint(spec, variant.canonical_name(), field)
                            })
                            .collect(),
                    })
                    .collect(),
            };
            if shape_id == WebSocketShapeId::Message {
                ShapeFingerprint::Representation {
                    name,
                    payload: Box::new(union),
                }
            } else {
                union
            }
        }
    }
}

fn spec_field_fingerprint(
    spec: &CanonicalWebSocketShapeSpec,
    owner_name: &str,
    field: &WebSocketShapeField,
) -> FieldFingerprint {
    let ty = spec_type_fingerprint(spec, field.ty(), &format!("{owner_name}.{}", field.name()));
    FieldFingerprint {
        name: field.name().to_string(),
        required: !matches!(ty, ShapeFingerprint::Nullable(_)),
        ty,
    }
}

fn spec_type_fingerprint(
    spec: &CanonicalWebSocketShapeSpec,
    ty: &WebSocketShapeType,
    literal_union_name: &str,
) -> ShapeFingerprint {
    match ty {
        WebSocketShapeType::String => ShapeFingerprint::String,
        WebSocketShapeType::Integer => ShapeFingerprint::Integer,
        WebSocketShapeType::Context => ShapeFingerprint::Context,
        WebSocketShapeType::Shape(shape_id) => spec_shape_fingerprint(spec, *shape_id),
        WebSocketShapeType::Array(item) => ShapeFingerprint::Array(Box::new(
            spec_type_fingerprint(spec, item, literal_union_name),
        )),
        WebSocketShapeType::Nullable(inner) => ShapeFingerprint::Nullable(Box::new(
            spec_type_fingerprint(spec, inner, literal_union_name),
        )),
        WebSocketShapeType::StringLiteral(value) => ShapeFingerprint::Literal((*value).to_string()),
        WebSocketShapeType::StringLiteralUnion(values) => ShapeFingerprint::NamedUnion {
            name: literal_union_name.to_string(),
            variants: values
                .iter()
                .map(|value| ShapeFingerprint::Literal((*value).to_string()))
                .collect(),
        },
    }
}

fn plan_shape_fingerprint(plan: &RuntimeTypePlan) -> ShapeFingerprint {
    if plan.named_type_name.as_deref() == Some(CONTEXT_MARKER)
        && matches!(plan.node, RuntimeTypeNode::Unknown)
    {
        return ShapeFingerprint::Context;
    }
    match &plan.node {
        RuntimeTypeNode::String => ShapeFingerprint::String,
        RuntimeTypeNode::Integer => ShapeFingerprint::Integer,
        RuntimeTypeNode::Array(item) => {
            ShapeFingerprint::Array(Box::new(plan_shape_fingerprint(item)))
        }
        RuntimeTypeNode::Nullable(inner) => {
            ShapeFingerprint::Nullable(Box::new(plan_shape_fingerprint(inner)))
        }
        RuntimeTypeNode::LiteralString(value) => ShapeFingerprint::Literal(value.clone()),
        RuntimeTypeNode::Record { fields, .. } => ShapeFingerprint::NamedRecord {
            name: plan
                .named_type_name
                .clone()
                .expect("canonical WebSocket record must stay named"),
            fields: fields
                .iter()
                .map(|field| FieldFingerprint {
                    name: field.name.clone(),
                    required: field.required,
                    ty: plan_shape_fingerprint(&field.ty),
                })
                .collect(),
        },
        RuntimeTypeNode::Union(variants) => ShapeFingerprint::NamedUnion {
            name: plan
                .named_type_name
                .clone()
                .expect("canonical WebSocket union must stay named"),
            variants: variants.iter().map(plan_shape_fingerprint).collect(),
        },
        RuntimeTypeNode::Representation { type_name, payload } => {
            ShapeFingerprint::Representation {
                name: type_name.clone(),
                payload: Box::new(plan_shape_fingerprint(payload)),
            }
        }
        other => panic!("unexpected canonical WebSocket runtime node: {other:?}"),
    }
}

fn mutate_field_name(corpus: &mut [ShapeFingerprint]) -> bool {
    mutate_first(corpus, &mut |shape| match shape {
        ShapeFingerprint::NamedRecord { fields, .. } if !fields.is_empty() => {
            fields[0].name.push_str("Drift");
            true
        }
        _ => false,
    })
}

fn mutate_field_order(corpus: &mut [ShapeFingerprint]) -> bool {
    mutate_first(corpus, &mut |shape| match shape {
        ShapeFingerprint::NamedRecord { fields, .. } if fields.len() > 1 => {
            fields.swap(0, 1);
            true
        }
        _ => false,
    })
}

fn mutate_nullable(corpus: &mut [ShapeFingerprint]) -> bool {
    mutate_first(corpus, &mut |shape| {
        let ShapeFingerprint::Nullable(inner) = shape else {
            return false;
        };
        *shape = (**inner).clone();
        true
    })
}

fn mutate_tag(corpus: &mut [ShapeFingerprint]) -> bool {
    mutate_first(corpus, &mut |shape| {
        let ShapeFingerprint::Literal(value) = shape else {
            return false;
        };
        value.push_str("-drift");
        true
    })
}

fn mutate_context(corpus: &mut [ShapeFingerprint]) -> bool {
    mutate_first(corpus, &mut |shape| {
        if *shape != ShapeFingerprint::Context {
            return false;
        }
        *shape = ShapeFingerprint::String;
        true
    })
}

fn mutate_first(
    corpus: &mut [ShapeFingerprint],
    mutate: &mut impl FnMut(&mut ShapeFingerprint) -> bool,
) -> bool {
    corpus.iter_mut().any(|shape| mutate_shape(shape, mutate))
}

fn mutate_shape(
    shape: &mut ShapeFingerprint,
    mutate: &mut impl FnMut(&mut ShapeFingerprint) -> bool,
) -> bool {
    if mutate(shape) {
        return true;
    }
    match shape {
        ShapeFingerprint::Array(inner)
        | ShapeFingerprint::Nullable(inner)
        | ShapeFingerprint::Representation { payload: inner, .. } => mutate_shape(inner, mutate),
        ShapeFingerprint::NamedRecord { fields, .. } => fields
            .iter_mut()
            .any(|field| mutate_shape(&mut field.ty, mutate)),
        ShapeFingerprint::NamedUnion { variants, .. } => variants
            .iter_mut()
            .any(|variant| mutate_shape(variant, mutate)),
        ShapeFingerprint::String
        | ShapeFingerprint::Integer
        | ShapeFingerprint::Context
        | ShapeFingerprint::Literal(_) => false,
    }
}
