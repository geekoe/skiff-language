//! E2b bridge: runtime-form recoverable expected-type plan (E2a wire
//! `expectedTypePlan`, the serde projection of
//! `skiff_runtime_model::recoverable::RuntimeRecoverableExpectedTypePlan`)
//! into the artifact-form `RecoverableExpectedTypePlan` stored by
//! task-control (`ActorActivationSnapshot`).
//!
//! The runtime form is the only plan the submission side can freeze from the
//! linked program. Execution never decodes with this projection: attempt
//! decode uses the linked expected plans of the frozen execution image (the
//! Runtime cold-activates an Actor from the `actor.owner.invoke` bootstrap
//! against its linked `create` declaration). The artifact projection is a
//! deterministic store witness; the verbatim runtime plan is preserved in
//! `ActorActivationSnapshot.expected_type_plan_runtime`.
//!
//! Projection contract:
//! - structural nodes map to canonical `TypeRefIr` builtins / record / union /
//!   nullable / literal shapes;
//! - `Representation` and `AnyInterface` (which carry only identity strings in
//!   the runtime form) project to `TypeRefIr::Builtin` whose name is the
//!   namespaced canonical identity string, with the identity also recorded in
//!   `root_type_identity_ref`;
//! - `Unresolved` nodes fail closed (E2a create plans never contain them).

use std::collections::BTreeMap;

use serde::Deserialize;
use skiff_artifact_model::{
    LiteralIr, RecoverableExpectedTypePlan, RecoverableExpectedTypeRoot,
    RecoverableTypeIdentityRef, TypeRefIr,
};

#[allow(dead_code)] // DTO fields are consumed by serde validation / projection.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeExpectedTypePlanDto {
    label: String,
    #[serde(default)]
    identity: Option<RuntimeTypeIdentityRefDto>,
    node: RuntimeExpectedTypeNodeDto,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
enum RuntimeTypeIdentityRefDto {
    RuntimeNamedType(RuntimeNamedTypeDto),
    ServiceSymbol(RuntimeServiceSymbolDto),
    PackageSymbol(RuntimePackageSymbolDto),
    ArtifactType(RuntimeArtifactTypeDto),
    Interface(RuntimeInterfaceTypeDto),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeNamedTypeDto {
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeServiceRefDto {
    service_id: String,
    #[allow(dead_code)]
    #[serde(default)]
    version: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    build_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeServiceSymbolDto {
    #[serde(default)]
    service: Option<RuntimeServiceRefDto>,
    module_path: String,
    symbol: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimePackageSymbolDto {
    package_ref: String,
    symbol_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeArtifactTypeDto {
    artifact_identity: String,
    type_identity: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeInterfaceTypeDto {
    interface_identity: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum RuntimeExpectedTypeNodeDto {
    Alias {
        target: Box<RuntimeExpectedTypePlanDto>,
    },
    Nullable {
        inner: Box<RuntimeExpectedTypePlanDto>,
    },
    Union {
        items: Vec<RuntimeExpectedTypePlanDto>,
    },
    LiteralString {
        value: String,
    },
    Representation {
        identity: RuntimeTypeIdentityRefDto,
        payload: Box<RuntimeExpectedTypePlanDto>,
    },
    Json,
    JsonObject,
    Bytes,
    Date,
    String,
    TaskRef,
    Bool,
    Number,
    Integer,
    Null,
    Stream {
        item: Box<RuntimeExpectedTypePlanDto>,
    },
    Array {
        item: Box<RuntimeExpectedTypePlanDto>,
    },
    Map {
        key: Box<RuntimeExpectedTypePlanDto>,
        value: Box<RuntimeExpectedTypePlanDto>,
    },
    Record {
        fields: Vec<RuntimeExpectedRecordFieldDto>,
        #[allow(dead_code)]
        #[serde(default)]
        boundary_record_kind: Option<String>,
    },
    AnyInterface {
        expected: RuntimeExpectedAnyInterfaceDto,
    },
    Unresolved {
        diagnostic_label: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeExpectedRecordFieldDto {
    name: String,
    ty: RuntimeExpectedTypePlanDto,
    #[allow(dead_code)]
    required: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeExpectedAnyInterfaceDto {
    interface_identity: String,
    #[allow(dead_code)]
    method_projection_identity: String,
}

/// Projects the E2a wire runtime-form plan JSON into the task-control store
/// artifact form. Fails closed on malformed JSON or unresolved nodes.
pub fn project_runtime_expected_type_plan(
    value: &serde_json::Value,
) -> Result<RecoverableExpectedTypePlan, String> {
    let plan: RuntimeExpectedTypePlanDto = serde_json::from_value(value.clone())
        .map_err(|error| format!("actor expected-type plan projection failed: {error}"))?;
    project_plan(&plan)
}

fn project_plan(plan: &RuntimeExpectedTypePlanDto) -> Result<RecoverableExpectedTypePlan, String> {
    let root = project_node(&plan.node)?;
    Ok(RecoverableExpectedTypePlan {
        root: RecoverableExpectedTypeRoot::TypeRef { ty: root },
        root_type_identity_ref: plan
            .identity
            .as_ref()
            .map(runtime_identity_string)
            .map(RecoverableTypeIdentityRef),
        runtime_carrier_check_required: false,
        interface_projection_refs: Vec::new(),
        interface_method_refs: Vec::new(),
        field_refs: Vec::new(),
        union_branch_refs: Vec::new(),
    })
}

fn project_node(node: &RuntimeExpectedTypeNodeDto) -> Result<TypeRefIr, String> {
    match node {
        RuntimeExpectedTypeNodeDto::Alias { target } => project_node(&target.node),
        RuntimeExpectedTypeNodeDto::Nullable { inner } => Ok(TypeRefIr::Nullable {
            inner: Box::new(project_node(&inner.node)?),
        }),
        RuntimeExpectedTypeNodeDto::Union { items } => Ok(TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| project_node(&item.node))
                .collect::<Result<Vec<_>, _>>()?,
        }),
        RuntimeExpectedTypeNodeDto::LiteralString { value } => Ok(TypeRefIr::Literal {
            value: LiteralIr::String {
                value: value.clone(),
            },
        }),
        RuntimeExpectedTypeNodeDto::Representation { identity, payload } => {
            Ok(TypeRefIr::Builtin {
                name: runtime_identity_string(identity),
                args: vec![project_node(&payload.node)?],
            })
        }
        RuntimeExpectedTypeNodeDto::Json => Ok(TypeRefIr::builtin("Json")),
        RuntimeExpectedTypeNodeDto::JsonObject => Ok(TypeRefIr::builtin("JsonObject")),
        RuntimeExpectedTypeNodeDto::Bytes => Ok(TypeRefIr::builtin("bytes")),
        RuntimeExpectedTypeNodeDto::Date => Ok(TypeRefIr::builtin("date")),
        RuntimeExpectedTypeNodeDto::String => Ok(TypeRefIr::builtin("string")),
        RuntimeExpectedTypeNodeDto::TaskRef => Ok(TypeRefIr::builtin("TaskRef")),
        RuntimeExpectedTypeNodeDto::Bool => Ok(TypeRefIr::builtin("bool")),
        RuntimeExpectedTypeNodeDto::Number => Ok(TypeRefIr::builtin("number")),
        RuntimeExpectedTypeNodeDto::Integer => Ok(TypeRefIr::builtin("integer")),
        RuntimeExpectedTypeNodeDto::Null => Ok(TypeRefIr::builtin("null")),
        RuntimeExpectedTypeNodeDto::Stream { item } => Ok(TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![project_node(&item.node)?],
        }),
        RuntimeExpectedTypeNodeDto::Array { item } => Ok(TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![project_node(&item.node)?],
        }),
        RuntimeExpectedTypeNodeDto::Map { key, value } => Ok(TypeRefIr::Builtin {
            name: "Map".to_string(),
            args: vec![project_node(&key.node)?, project_node(&value.node)?],
        }),
        RuntimeExpectedTypeNodeDto::Record { fields, .. } => {
            let mut projected = BTreeMap::new();
            for field in fields {
                let name = field.name.clone();
                if projected
                    .insert(name.clone(), project_node(&field.ty.node)?)
                    .is_some()
                {
                    return Err(format!(
                        "actor expected-type plan record has duplicate field {name}"
                    ));
                }
            }
            Ok(TypeRefIr::Record { fields: projected })
        }
        RuntimeExpectedTypeNodeDto::AnyInterface { expected } => Ok(TypeRefIr::Builtin {
            name: format!("interface:{}", expected.interface_identity),
            args: Vec::new(),
        }),
        RuntimeExpectedTypeNodeDto::Unresolved { diagnostic_label } => Err(format!(
            "actor expected-type plan contains an unresolved node ({diagnostic_label})"
        )),
    }
}

/// Canonical namespaced identity string for one runtime identity ref
/// (deterministic, collision-free across identity kinds).
fn runtime_identity_string(identity: &RuntimeTypeIdentityRefDto) -> String {
    match identity {
        RuntimeTypeIdentityRefDto::RuntimeNamedType(value) => format!("type:{}", value.name),
        RuntimeTypeIdentityRefDto::ServiceSymbol(value) => {
            let service = value
                .service
                .as_ref()
                .map(|service| service.service_id.as_str())
                .unwrap_or("*");
            format!("service:{service}:{}:{}", value.module_path, value.symbol)
        }
        RuntimeTypeIdentityRefDto::PackageSymbol(value) => {
            format!("package:{}:{}", value.package_ref, value.symbol_path)
        }
        RuntimeTypeIdentityRefDto::ArtifactType(value) => format!(
            "artifact:{}:{}",
            value.artifact_identity, value.type_identity
        ),
        RuntimeTypeIdentityRefDto::Interface(value) => {
            format!("interface:{}", value.interface_identity)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::project_runtime_expected_type_plan;
    use serde_json::json;
    use skiff_artifact_model::{LiteralIr, RecoverableExpectedTypeRoot, TypeRefIr};

    #[test]
    fn projects_create_params_record_with_builtins() {
        let plan = json!({
            "label": "record",
            "node": {
                "kind": "record",
                "fields": [
                    {
                        "name": "name",
                        "ty": { "label": "string", "node": { "kind": "string" } },
                        "required": true
                    },
                    {
                        "name": "count",
                        "ty": { "label": "integer", "node": { "kind": "integer" } },
                        "required": true
                    },
                    {
                        "name": "tags",
                        "ty": {
                            "label": "Array<string>",
                            "node": {
                                "kind": "array",
                                "item": { "label": "string", "node": { "kind": "string" } }
                            }
                        },
                        "required": false
                    }
                ]
            }
        });
        let projected = project_runtime_expected_type_plan(&plan).expect("projection");
        let RecoverableExpectedTypeRoot::TypeRef {
            ty: TypeRefIr::Record { fields },
        } = &projected.root
        else {
            panic!("expected record root, got {:?}", projected.root);
        };
        assert_eq!(fields.len(), 3);
        assert_eq!(fields.get("name"), Some(&TypeRefIr::builtin("string")));
        assert_eq!(
            fields.get("tags"),
            Some(&TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::builtin("string")],
            })
        );
        assert!(projected.root_type_identity_ref.is_none());
    }

    #[test]
    fn projects_named_and_interface_nodes_with_identity_strings() {
        let plan = json!({
            "label": "DocHub",
            "identity": { "kind": "runtimeNamedType", "value": { "name": "DocHub" } },
            "node": {
                "kind": "representation",
                "identity": { "kind": "runtimeNamedType", "value": { "name": "DocHub" } },
                "payload": { "label": "record", "node": { "kind": "record", "fields": [] } }
            }
        });
        let projected = project_runtime_expected_type_plan(&plan).expect("projection");
        assert_eq!(
            projected
                .root_type_identity_ref
                .as_ref()
                .map(|id| id.0.as_str()),
            Some("type:DocHub")
        );
        let RecoverableExpectedTypeRoot::TypeRef {
            ty: TypeRefIr::Builtin { name, args },
        } = &projected.root
        else {
            panic!("expected builtin root, got {:?}", projected.root);
        };
        assert_eq!(name, "type:DocHub");
        assert_eq!(args.len(), 1);
        assert!(matches!(
            args.first(),
            Some(TypeRefIr::Record { fields }) if fields.is_empty()
        ));
    }

    #[test]
    fn projects_literal_union_nullable_and_rejects_unresolved() {
        let plan = json!({
            "label": "union",
            "node": {
                "kind": "union",
                "items": [
                    { "label": "null", "node": { "kind": "null" } },
                    { "label": "literal", "node": { "kind": "literalString", "value": "x" } }
                ]
            }
        });
        let projected = project_runtime_expected_type_plan(&plan).expect("projection");
        assert_eq!(
            projected.root,
            RecoverableExpectedTypeRoot::TypeRef {
                ty: TypeRefIr::Union {
                    items: vec![
                        TypeRefIr::builtin("null"),
                        TypeRefIr::Literal {
                            value: LiteralIr::String {
                                value: "x".to_string()
                            }
                        },
                    ],
                }
            }
        );

        let unresolved = json!({
            "label": "unknown",
            "node": { "kind": "unresolved", "diagnosticLabel": "unknown" }
        });
        assert!(project_runtime_expected_type_plan(&unresolved).is_err());
    }
}
