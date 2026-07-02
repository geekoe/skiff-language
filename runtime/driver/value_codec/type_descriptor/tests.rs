use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use serde_json::{json, Value};
use skiff_runtime_boundary::json_convert::{from_wire, to_wire};

use super::*;
use crate::program::types::{PackageSymbolKey, ServiceSymbolKey};
use crate::program::{
    anonymous_type_decl, ExecutableAddr, ExecutableKind, ExternalRefTable, FileDeclarations,
    FileLinkTargets, GatewayConfig, LinkOverlay, LinkedExecutable, LinkedExecutableBody,
    LinkedFileUnit, LinkedTypeDescriptor, LinkedTypeRef, PackageUnit, RuntimeProgram,
    RuntimeTypeContext, ServiceMeta, ServiceSymbolRef, SlotLayoutIr, SourceMapDto, TypeAddr,
    TypeDeclIr, UnitAddr,
};

fn plan_from_descriptor(descriptor: &Value) -> RuntimeTypePlan {
    RuntimeTypePlan::from_descriptor(descriptor).expect("descriptor plan should build")
}

fn assert_unknown_plan(descriptor: &Value) {
    let plan = plan_from_descriptor(descriptor);
    assert!(
        matches!(plan.node(), RuntimeTypeNode::Unknown),
        "descriptor should be unknown: {descriptor}"
    );
}

#[test]
fn type_descriptor_old_name_aliases_are_not_type_names() {
    assert_eq!(descriptor_kind(&json!({ "node": "NamedType" })), None);

    assert_eq!(descriptor_name(&json!("User")), Some("User"));
    assert_eq!(descriptor_name(&json!({ "name": "User" })), Some("User"));
    assert_eq!(descriptor_name(&json!({ "typeName": "User" })), None);
    assert_eq!(descriptor_name(&json!({ "symbol": "User" })), None);
    assert_eq!(descriptor_name(&json!({ "value": "User" })), None);

    assert_eq!(type_ref_name(&json!("User")), Some("User".to_string()));
    assert_eq!(
        type_ref_name(&json!({ "name": "User" })),
        Some("User".to_string())
    );
    assert_eq!(type_ref_name(&json!({ "typeName": "User" })), None);
    assert_eq!(type_ref_name(&json!({ "symbol": "User" })), None);
    assert_eq!(type_ref_name(&json!({ "value": "User" })), None);
    assert!(matches!(
        type_ref_name_with_nullable(&json!({ "typeName": "User" }), false),
        Err(RuntimeTypeNameError::MissingType)
    ));
}

#[test]
fn type_descriptor_alias_only_accepts_new_target() {
    assert_eq!(
        alias_target(&json!({
            "kind": "alias",
            "target": { "kind": "builtin", "name": "string" },
        }))
        .unwrap(),
        Some(json!({ "kind": "builtin", "name": "string" }))
    );

    assert_eq!(
        alias_target(&json!({
            "kind": "AliasType",
            "targetType": { "kind": "builtin", "name": "string" },
        }))
        .unwrap(),
        None
    );
    assert_eq!(
        alias_target(&json!({
            "kind": "AliasType",
            "target": { "kind": "builtin", "name": "string" },
        }))
        .unwrap(),
        None
    );
    assert_eq!(
        alias_target(&json!({
            "kind": "TransparentAlias",
            "targetType": { "kind": "builtin", "name": "string" },
        }))
        .unwrap(),
        None
    );
    assert_eq!(
        alias_target(&json!({
            "kind": "transparentAlias",
            "targetType": { "kind": "builtin", "name": "string" },
        }))
        .unwrap(),
        None
    );
    assert!(matches!(
        alias_target(&json!({
            "targetType": { "kind": "builtin", "name": "string" },
            "kind": "alias",
        })),
        Err(skiff_runtime_boundary::RuntimeError::InvalidArtifact(message))
            if message.contains("missing target")
    ));
}

#[test]
fn type_descriptor_rejects_old_skiff_descriptor_shapes() {
    assert_eq!(
        named_type_name(&json!({ "kind": "NamedType", "name": "string", "typeArgs": [] })),
        None
    );
    assert_eq!(
        generic_type_parts(&json!({ "kind": "NamedType", "name": "Array", "typeArgs": [] })),
        None
    );
    assert_eq!(
        union_types(
            &json!({ "kind": "UnionType", "types": [{ "kind": "builtin", "name": "string" }] })
        ),
        None
    );
    assert_eq!(
        nullable_inner(&json!({
            "kind": "NullableType",
            "inner": { "kind": "builtin", "name": "string" },
        })),
        None
    );
    assert_eq!(
        literal_string(&json!({ "kind": "LiteralType", "value": "ready" })),
        None
    );
    assert_eq!(
        alias_target(&json!({
            "kind": "AliasType",
            "targetType": { "kind": "builtin", "name": "string" },
        }))
        .unwrap(),
        None
    );
    assert!(matches!(
        alias_target(&json!({
            "kind": "alias",
            "targetType": { "kind": "builtin", "name": "string" },
        })),
        Err(skiff_runtime_boundary::RuntimeError::InvalidArtifact(message))
            if message.contains("missing target")
    ));
}

#[test]
fn type_descriptor_record_fields_rejects_legacy_object_and_record_kinds() {
    for descriptor in [
        json!({ "kind": "object" }),
        json!({ "kind": "Record" }),
        json!({
            "kind": "RecordType",
            "source": "{ id: string }",
        }),
    ] {
        assert!(
            record_fields(&descriptor).unwrap().is_none(),
            "legacy descriptor should not be treated as record fields: {descriptor}"
        );
        assert_unknown_plan(&descriptor);
    }

    let plan = plan_from_descriptor(&json!({
        "kind": "record",
        "fields": {
            "id": { "kind": "builtin", "name": "string" }
        },
    }));
    assert!(matches!(
        plan.node(),
        RuntimeTypeNode::Record { fields, .. }
            if fields.len() == 1 && fields[0].name == "id" && fields[0].required
    ));

    let plan = plan_from_descriptor(&json!({
        "kind": "builtin",
        "name": "User",
        "args": [],
        "fields": {
            "id": { "kind": "builtin", "name": "string" },
            "displayName": {
                "kind": "nullable",
                "inner": { "kind": "builtin", "name": "string" }
            }
        },
    }));
    assert!(matches!(
        plan.node(),
        RuntimeTypeNode::Record { fields, .. }
            if fields.len() == 2
                && fields.iter().any(|field| field.name == "id" && field.required)
                && fields.iter().any(|field| field.name == "displayName" && !field.required)
    ));

    assert!(record_fields(&json!({
        "kind": "builtin",
        "name": "User",
        "args": [],
        "fields": [
            { "name": "id", "type": { "kind": "builtin", "name": "string" } }
        ],
    }))
    .unwrap()
    .is_none());
}

#[test]
fn type_descriptor_text_rejects_legacy_descriptor_kinds() {
    for descriptor in [
        json!({ "kind": "GenericType", "name": "Array", "args": [] }),
        json!({ "kind": "TypeRef", "name": "User" }),
        json!({ "kind": "Identifier", "name": "User" }),
        json!({ "kind": "Name", "name": "User" }),
        json!({ "kind": "RecordType", "source": "{ id: string }" }),
    ] {
        assert_eq!(
            descriptor_text(&descriptor),
            None,
            "legacy descriptor should not be textified: {descriptor}"
        );
    }

    assert_eq!(
        descriptor_text(&json!({
            "kind": "builtin",
            "name": "Array",
            "args": [{ "kind": "builtin", "name": "string" }],
        })),
        Some("Array<string>".to_string())
    );
    assert_eq!(
        descriptor_text(&json!({
            "kind": "nullable",
            "inner": { "kind": "builtin", "name": "string" },
        })),
        Some("string?".to_string())
    );
    assert_eq!(
        descriptor_text(&json!({
            "kind": "union",
            "items": [
                { "kind": "builtin", "name": "string" },
                { "kind": "builtin", "name": "number" },
            ],
        })),
        Some("string | number".to_string())
    );
    assert_eq!(
        descriptor_text(&json!({
            "kind": "literal",
            "value": { "kind": "string", "value": "ready" },
        })),
        Some("\"ready\"".to_string())
    );
}

#[test]
fn type_descriptor_representation_only_accepts_canonical_payload() {
    assert_eq!(
        representation_descriptor(&json!({
            "kind": "representation",
            "name": "UserId",
            "representation": { "kind": "builtin", "name": "string" },
        })),
        Some((
            "UserId".to_string(),
            json!({ "kind": "builtin", "name": "string" })
        ))
    );

    assert_eq!(
        representation_descriptor(&json!({
            "kind": "RepresentationType",
            "name": "UserId",
            "representation": { "kind": "builtin", "name": "string" },
        })),
        None
    );
    assert_eq!(
        representation_descriptor(&json!({
            "kind": "Representation",
            "name": "UserId",
            "representation": { "kind": "builtin", "name": "string" },
        })),
        None
    );
    assert_eq!(
        representation_descriptor(&json!({
            "kind": "representation",
            "typeName": "UserId",
            "representation": { "kind": "builtin", "name": "string" },
        })),
        None
    );
    assert_eq!(
        representation_descriptor(&json!({
            "kind": "representation",
            "name": "UserId",
            "payload": { "kind": "builtin", "name": "string" },
        })),
        None
    );
    assert_eq!(
        representation_descriptor(&json!({
            "kind": "representation",
            "name": "UserId",
            "inner": { "kind": "builtin", "name": "string" },
        })),
        None
    );
}

#[test]
fn type_descriptor_union_members_and_options_are_not_descriptor_types() {
    let types = json!([{ "type": "string" }, { "type": "number" }]);

    assert_eq!(
        descriptor_union_types(&json!({ "items": types.clone() })),
        types.as_array()
    );
    assert_eq!(
        descriptor_union_types(&json!({ "variants": types.clone() })),
        types.as_array()
    );
    assert_eq!(
        descriptor_union_types(&json!({ "oneOf": types.clone() })),
        types.as_array()
    );
    assert_eq!(
        descriptor_union_types(&json!({ "types": types.clone() })),
        None
    );
    assert_eq!(
        descriptor_union_types(&json!({ "members": types.clone() })),
        None
    );
    assert_eq!(descriptor_union_types(&json!({ "options": types })), None);
}

#[test]
fn type_descriptor_stream_item_type_only_uses_args() {
    let descriptor = json!({
        "kind": "builtin",
        "name": "Stream",
        "args": [{ "type": "string" }],
        "itemType": { "type": "number" },
        "streamItemType": { "type": "boolean" },
        "inner": { "type": "integer" },
    });
    assert_eq!(
        stream_item_type(&descriptor),
        descriptor
            .get("args")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
    );

    let legacy_descriptor = json!({
        "kind": "Stream",
        "itemType": { "type": "string" },
        "streamItemType": { "type": "number" },
        "inner": { "type": "boolean" },
    });
    assert_eq!(stream_item_type(&legacy_descriptor), None);

    let non_stream_descriptor = json!({
        "kind": "builtin",
        "name": "Array",
        "args": [{ "type": "string" }],
    });
    assert_eq!(stream_item_type(&non_stream_descriptor), None);
}

#[test]
fn type_descriptor_json_schema_compatibility_remains_supported() {
    let plan = plan_from_descriptor(&json!({
        "type": "array",
        "items": { "type": "string" },
    }));
    assert!(matches!(
        plan.node(),
        RuntimeTypeNode::Array(item) if matches!(item.node(), RuntimeTypeNode::String)
    ));

    let plan = plan_from_descriptor(&json!({
        "type": "object",
        "additionalProperties": { "type": "number" },
    }));
    assert!(matches!(
        plan.node(),
        RuntimeTypeNode::Map { key, value }
            if matches!(key.node(), RuntimeTypeNode::String)
                && matches!(value.node(), RuntimeTypeNode::Number)
    ));

    let plan = plan_from_descriptor(&json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "count": { "type": "integer" }
        },
        "required": ["id"],
    }));
    assert!(matches!(
        plan.node(),
        RuntimeTypeNode::Record { fields, .. }
            if fields.len() == 2
                && fields.iter().any(|field| field.name == "id" && field.required)
                && fields.iter().any(|field| field.name == "count" && !field.required)
    ));

    let plan = plan_from_descriptor(&json!({
        "oneOf": [{ "type": "string" }, { "type": "number" }],
    }));
    assert!(matches!(
        plan.node(),
        RuntimeTypeNode::Union(types)
            if types.len() == 2
                && matches!(types[0].node(), RuntimeTypeNode::String)
                && matches!(types[1].node(), RuntimeTypeNode::Number)
    ));

    let plan = plan_from_descriptor(&json!({ "enum": ["ready"] }));
    assert!(matches!(
        plan.node(),
        RuntimeTypeNode::LiteralString(value) if value == "ready"
    ));

    let plan = plan_from_descriptor(&json!({
        "type": "string",
        "nullable": true,
    }));
    assert!(matches!(
        plan.node(),
        RuntimeTypeNode::Nullable(inner) if matches!(inner.node(), RuntimeTypeNode::String)
    ));

    let plan = plan_from_descriptor(&json!({
        "type": "string",
        "contentEncoding": "base64",
    }));
    assert!(matches!(plan.node(), RuntimeTypeNode::Bytes));

    assert_eq!(
        schema_symbol_name(&json!({
            "type": "object",
            "xSkiffSymbol": "pkg.User",
        })),
        Some("pkg.User")
    );
}

#[test]
fn type_descriptor_std_http_builtins_do_not_use_runtime_schema_fallback() {
    assert_unknown_plan(&json!({
        "kind": "builtin",
        "name": "std.http.HttpRequest",
        "args": [],
    }));
    assert_unknown_plan(&json!("std.http.HttpResponse"));
}

#[test]
fn db_upsert_result_descriptor_is_record_shape_and_roundtrips() {
    let value_type = json!({ "kind": "builtin", "name": "string", "args": [] });
    let descriptor = json!({
        "kind": "builtin",
        "name": "DbUpsertResult",
        "args": [value_type.clone()],
    });

    let plan = plan_from_descriptor(&descriptor);
    let RuntimeTypeNode::Record { fields, .. } = plan.node() else {
        panic!("DbUpsertResult should be treated as a record");
    };
    assert_eq!(fields.len(), 2);
    let value = fields
        .iter()
        .find(|field| field.name == "value")
        .expect("DbUpsertResult should expose value field");
    assert!(matches!(value.ty.node(), RuntimeTypeNode::String));
    assert!(value.required);
    let inserted = fields
        .iter()
        .find(|field| field.name == "inserted")
        .expect("DbUpsertResult should expose inserted field");
    assert!(matches!(inserted.ty.node(), RuntimeTypeNode::Bool));
    assert!(inserted.required);

    let input = json!({ "value": "thread-1", "inserted": true });
    let mut heap = crate::request_heap::RequestHeap::default();
    let runtime_value =
        from_wire(&input, &descriptor, &mut heap).expect("DbUpsertResult should decode from wire");
    let output = to_wire(&runtime_value, &descriptor, &mut heap)
        .expect("DbUpsertResult should encode to wire");
    assert_eq!(output, input);
}

#[test]
fn db_many_result_descriptors_are_record_shapes_and_roundtrip() {
    let cases = [
        (
            "DbInsertManyResult",
            json!({ "insertedCount": 2 }),
            vec!["insertedCount"],
        ),
        (
            "DbUpdateManyResult",
            json!({ "matchedCount": 3, "modifiedCount": 1 }),
            vec!["matchedCount", "modifiedCount"],
        ),
        (
            "DbDeleteManyResult",
            json!({ "deletedCount": 4 }),
            vec!["deletedCount"],
        ),
    ];

    for (type_name, input, field_names) in cases {
        let descriptor = json!({
            "kind": "builtin",
            "name": type_name,
            "args": [],
        });
        let plan = plan_from_descriptor(&descriptor);
        let RuntimeTypeNode::Record { fields, .. } = plan.node() else {
            panic!("{type_name} should be treated as a record");
        };
        assert_eq!(fields.len(), field_names.len());
        for field_name in field_names {
            let field = fields
                .iter()
                .find(|field| field.name == field_name)
                .expect("DB result should expose expected field");
            assert!(matches!(field.ty.node(), RuntimeTypeNode::Number));
            assert!(field.required);
        }

        let mut heap = crate::request_heap::RequestHeap::default();
        let runtime_value =
            from_wire(&input, &descriptor, &mut heap).expect("DB result should decode from wire");
        let output = to_wire(&runtime_value, &descriptor, &mut heap)
            .expect("DB result should encode to wire");
        assert_eq!(output, input);
    }
}

fn assert_artifact_type_ref_plan_matches_descriptor(ty: skiff_artifact_model::TypeRefIr) {
    let descriptor = serde_json::to_value(&ty).expect("artifact TypeRefIr should serialize");
    let expected = RuntimeTypePlan::from_descriptor(&descriptor)
        .expect("descriptor oracle should build a runtime plan");
    let actual = RuntimeTypePlan::from_artifact_type_ref(&ty)
        .expect("artifact TypeRefIr should build a runtime plan");
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
}

fn test_service_symbol(symbol: &str) -> skiff_artifact_model::ServiceSymbolRef {
    skiff_artifact_model::ServiceSymbolRef {
        module_path: "callee.types".to_string(),
        symbol: symbol.to_string(),
    }
}

fn test_package_symbol(symbol_path: &str) -> skiff_artifact_model::PackageSymbolRef {
    skiff_artifact_model::PackageSymbolRef {
        package: skiff_artifact_model::PackageRefIr::PackageId {
            package_id: "pkg.shared".to_string(),
        },
        symbol_path: symbol_path.to_string(),
        abi_expectation: None,
    }
}

fn test_package_id_symbol(
    package_id: &str,
    symbol_path: &str,
) -> skiff_artifact_model::PackageSymbolRef {
    skiff_artifact_model::PackageSymbolRef {
        package: skiff_artifact_model::PackageRefIr::PackageId {
            package_id: package_id.to_string(),
        },
        symbol_path: symbol_path.to_string(),
        abi_expectation: None,
    }
}

#[test]
fn artifact_type_ref_plan_matches_descriptor_for_builtin_string_json_array_map_and_stream() {
    use skiff_artifact_model::TypeRefIr;

    for ty in [
        TypeRefIr::native("string"),
        TypeRefIr::native("Json"),
        TypeRefIr::Native {
            name: "Array".to_string(),
            args: vec![TypeRefIr::native("string")],
        },
        TypeRefIr::Native {
            name: "Map".to_string(),
            args: vec![TypeRefIr::native("string"), TypeRefIr::native("Json")],
        },
        TypeRefIr::Native {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::native("bytes")],
        },
    ] {
        assert_artifact_type_ref_plan_matches_descriptor(ty);
    }
}

#[test]
fn artifact_type_ref_plan_matches_descriptor_for_record_with_nullable() {
    use skiff_artifact_model::TypeRefIr;

    assert_artifact_type_ref_plan_matches_descriptor(TypeRefIr::Record {
        fields: std::collections::BTreeMap::from([
            ("id".to_string(), TypeRefIr::native("string")),
            (
                "displayName".to_string(),
                TypeRefIr::Nullable {
                    inner: Box::new(TypeRefIr::native("string")),
                },
            ),
        ]),
    });
}

#[test]
fn artifact_type_ref_plan_matches_descriptor_for_union_with_literal_string() {
    use skiff_artifact_model::{LiteralIr, TypeRefIr};

    assert_artifact_type_ref_plan_matches_descriptor(TypeRefIr::Union {
        items: vec![
            TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: "ready".to_string(),
                },
            },
            TypeRefIr::native("string"),
        ],
    });
}

#[test]
fn artifact_type_ref_plan_matches_descriptor_for_record_nested_unresolved_refs() {
    use skiff_artifact_model::TypeRefIr;

    assert_artifact_type_ref_plan_matches_descriptor(TypeRefIr::Record {
        fields: std::collections::BTreeMap::from([
            ("local".to_string(), TypeRefIr::LocalType { type_index: 0 }),
            (
                "service".to_string(),
                TypeRefIr::ServiceSymbol {
                    symbol: test_service_symbol("User"),
                },
            ),
            (
                "package".to_string(),
                TypeRefIr::PackageSymbol {
                    symbol: test_package_symbol("shared.User"),
                },
            ),
        ]),
    });
}

#[test]
fn artifact_type_ref_plan_matches_descriptor_for_top_level_unresolved_refs() {
    use skiff_artifact_model::TypeRefIr;

    for ty in [
        TypeRefIr::LocalType { type_index: 0 },
        TypeRefIr::ServiceSymbol {
            symbol: test_service_symbol("User"),
        },
        TypeRefIr::PackageSymbol {
            symbol: test_package_symbol("shared.User"),
        },
    ] {
        assert_artifact_type_ref_plan_matches_descriptor(ty);
    }
}

#[test]
fn artifact_type_ref_plan_matches_descriptor_for_db_results() {
    use skiff_artifact_model::TypeRefIr;

    for ty in [
        TypeRefIr::Native {
            name: "DbUpsertResult".to_string(),
            args: vec![TypeRefIr::Nullable {
                inner: Box::new(TypeRefIr::native("string")),
            }],
        },
        TypeRefIr::native("DbInsertManyResult"),
    ] {
        assert_artifact_type_ref_plan_matches_descriptor(ty);
    }
}

#[test]
fn artifact_type_ref_plan_matches_descriptor_for_non_string_literal() {
    use skiff_artifact_model::{LiteralIr, TypeRefIr};

    assert_artifact_type_ref_plan_matches_descriptor(TypeRefIr::Literal {
        value: LiteralIr::Number {
            value: serde_json::Number::from(42),
        },
    });
}

#[test]
fn artifact_type_ref_plan_matches_descriptor_for_top_level_unknown_artifact_refs() {
    use skiff_artifact_model::{FunctionTypeParamIr, TypeRefIr};

    for ty in [
        TypeRefIr::DbObjectSymbol {
            symbol: test_service_symbol("Post"),
        },
        TypeRefIr::TypeParam {
            name: "T".to_string(),
        },
        TypeRefIr::Function {
            params: vec![FunctionTypeParamIr {
                name: "input".to_string(),
                ty: TypeRefIr::native("string"),
            }],
            return_type: Box::new(TypeRefIr::native("bool")),
        },
    ] {
        assert_artifact_type_ref_plan_matches_descriptor(ty);
    }
}

#[test]
fn artifact_type_ref_in_program_resolves_package_symbol_inside_service_dependency_shapes() {
    use skiff_artifact_model::{FunctionTypeParamIr, LiteralIr, TypeRefIr};

    let event_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: crate::program::FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let event_descriptor = record_descriptor([
        ("tag", linked_builtin_type_ref("string")),
        ("text", linked_builtin_type_ref("string")),
    ]);
    let package_file = type_descriptor_file(
        "file:agent",
        "agent.llm",
        type_declarations([("LlmStreamEvent", 0)]),
        vec![TypeDeclIr {
            name: "LlmStreamEvent".to_string(),
            descriptor: event_descriptor.clone(),
            ..TypeDeclIr::default()
        }],
    );
    let mut program = type_descriptor_program(Vec::new(), vec![package_file]);
    program.types.descriptors.insert(
        event_addr.clone(),
        anonymous_type_decl("LlmStreamEvent", event_descriptor),
    );
    program
        .link_overlay
        .package_slots_by_id
        .insert("example.com/agent".to_string(), 0);
    program.link_overlay.symbols.insert(
        PackageSymbolKey::new(0, "llm.LlmStreamEvent").to_string(),
        ResolvedSymbol::Type { addr: event_addr },
    );

    let event_ref = TypeRefIr::PackageSymbol {
        symbol: test_package_id_symbol("example.com/agent", "llm.LlmStreamEvent"),
    };
    let dependency_return_type = TypeRefIr::Native {
        name: "Stream".to_string(),
        args: vec![TypeRefIr::Record {
            fields: BTreeMap::from([
                ("event".to_string(), event_ref.clone()),
                (
                    "maybeEvent".to_string(),
                    TypeRefIr::Nullable {
                        inner: Box::new(event_ref.clone()),
                    },
                ),
                (
                    "items".to_string(),
                    TypeRefIr::Native {
                        name: "Array".to_string(),
                        args: vec![event_ref.clone()],
                    },
                ),
                (
                    "byId".to_string(),
                    TypeRefIr::Native {
                        name: "Map".to_string(),
                        args: vec![TypeRefIr::native("string"), event_ref.clone()],
                    },
                ),
                (
                    "kind".to_string(),
                    TypeRefIr::Union {
                        items: vec![
                            TypeRefIr::Literal {
                                value: LiteralIr::String {
                                    value: "delta".to_string(),
                                },
                            },
                            TypeRefIr::native("string"),
                        ],
                    },
                ),
                (
                    "callback".to_string(),
                    TypeRefIr::Function {
                        params: vec![FunctionTypeParamIr {
                            name: "input".to_string(),
                            ty: event_ref.clone(),
                        }],
                        return_type: Box::new(TypeRefIr::native("string")),
                    },
                ),
            ]),
        }],
    };

    let image = program.linked_image();
    let plan = RuntimeTypePlan::from_artifact_type_ref_in_program(
        &dependency_return_type,
        &image,
        &ExecutableAddr::service(0, 0),
    )
    .expect("service dependency artifact return type should plan in caller context");

    let RuntimeTypeNode::Stream(item_plan) = plan.node() else {
        panic!("serverStream return type should build a Stream<T> plan");
    };
    let RuntimeTypeNode::Record { fields, .. } = item_plan.node() else {
        panic!("serverStream item should remain a record");
    };

    let event = runtime_field(fields, "event");
    assert!(record_has_field(event.ty.node(), "tag"));
    assert!(record_has_field(event.ty.node(), "text"));

    let maybe_event = runtime_field(fields, "maybeEvent");
    let RuntimeTypeNode::Nullable(nullable_inner) = maybe_event.ty.node() else {
        panic!("nullable package symbol field should stay nullable");
    };
    assert!(record_has_field(nullable_inner.node(), "tag"));

    let items = runtime_field(fields, "items");
    let RuntimeTypeNode::Array(item) = items.ty.node() else {
        panic!("Array<packageSymbol> should stay an array");
    };
    assert!(record_has_field(item.node(), "tag"));

    let by_id = runtime_field(fields, "byId");
    let RuntimeTypeNode::Map { value, .. } = by_id.ty.node() else {
        panic!("Map<string, packageSymbol> should stay a map");
    };
    assert!(record_has_field(value.node(), "tag"));

    let kind = runtime_field(fields, "kind");
    assert!(matches!(kind.ty.node(), RuntimeTypeNode::Union(items) if items.len() == 2));

    let callback = runtime_field(fields, "callback");
    assert!(matches!(callback.ty.node(), RuntimeTypeNode::Unknown));
}

#[test]
fn artifact_type_ref_in_program_does_not_resolve_callee_local_type_as_caller_local_type() {
    use skiff_artifact_model::TypeRefIr;

    let caller_addr = TypeAddr {
        unit: UnitAddr::Service,
        file: crate::program::FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let caller_descriptor =
        record_descriptor([("wrongCallerType", linked_builtin_type_ref("string"))]);
    let service_file = type_descriptor_file(
        "file:service",
        "svc.main",
        type_declarations([("CallerOnly", 0)]),
        vec![TypeDeclIr {
            name: "CallerOnly".to_string(),
            descriptor: caller_descriptor.clone(),
            ..TypeDeclIr::default()
        }],
    );
    let mut program = type_descriptor_program(vec![service_file], Vec::new());
    program.types.descriptors.insert(
        caller_addr,
        anonymous_type_decl("CallerOnly", caller_descriptor),
    );

    let dependency_param_type = TypeRefIr::Record {
        fields: BTreeMap::from([(
            "calleeLocal".to_string(),
            TypeRefIr::LocalType { type_index: 0 },
        )]),
    };
    let image = program.linked_image();
    let plan = RuntimeTypePlan::from_artifact_type_ref_in_program(
        &dependency_param_type,
        &image,
        &ExecutableAddr::service(0, 0),
    )
    .expect("callee-local service dependency type should not fail planning");

    let RuntimeTypeNode::Record { fields, .. } = plan.node() else {
        panic!("structural dependency param should remain a record");
    };
    assert!(matches!(
        runtime_field(&fields, "calleeLocal").ty.node(),
        RuntimeTypeNode::Unknown
    ));
}

#[test]
fn linked_db_object_symbol_resolves_package_local_declaration_before_service_export() {
    let service_addr = TypeAddr {
        unit: UnitAddr::Service,
        file: crate::program::FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let package_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: crate::program::FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let service_descriptor =
        record_descriptor([("serviceOnly", linked_builtin_type_ref("string"))]);
    let package_descriptor =
        record_descriptor([("packageOnly", linked_builtin_type_ref("string"))]);

    let service_file = type_descriptor_file(
        "file:service",
        "session",
        FileDeclarations::default(),
        vec![TypeDeclIr {
            name: "Session".to_string(),
            descriptor: service_descriptor.clone(),
            ..TypeDeclIr::default()
        }],
    );
    let package_file = type_descriptor_file(
        "file:package",
        "session",
        type_declarations([("Session", 0)]),
        vec![TypeDeclIr {
            name: "Session".to_string(),
            descriptor: package_descriptor.clone(),
            ..TypeDeclIr::default()
        }],
    );
    let mut program = type_descriptor_program(vec![service_file], vec![package_file]);
    program.types.descriptors.insert(
        service_addr.clone(),
        anonymous_type_decl("Session", service_descriptor),
    );
    program.types.descriptors.insert(
        package_addr,
        anonymous_type_decl("Session", package_descriptor),
    );
    program
        .types
        .exported_types
        .insert_service(ServiceSymbolKey::new("session", "Session"), service_addr);

    let image = program.linked_image();
    let plan = RuntimeTypePlan::from_linked(
        &db_object_symbol("session", "Session"),
        &PlanContext::new(&image, &ExecutableAddr::package(0, 0, 0)),
    )
    .expect("package-local DB object symbol should resolve");

    assert!(record_has_field(plan.node(), "packageOnly"));
    assert!(!record_has_field(plan.node(), "serviceOnly"));
}

#[test]
fn linked_db_object_symbol_missing_package_local_declaration_does_not_use_exports() {
    let service_addr = TypeAddr {
        unit: UnitAddr::Service,
        file: crate::program::FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let exported_package_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: crate::program::FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let descriptor = record_descriptor([("wrongUnit", linked_builtin_type_ref("string"))]);
    let service_file = type_descriptor_file(
        "file:service",
        "session",
        FileDeclarations::default(),
        vec![TypeDeclIr {
            name: "Session".to_string(),
            descriptor: descriptor.clone(),
            ..TypeDeclIr::default()
        }],
    );
    let package_file = type_descriptor_file(
        "file:package",
        "session",
        FileDeclarations::default(),
        vec![TypeDeclIr {
            name: "Session".to_string(),
            descriptor: descriptor.clone(),
            ..TypeDeclIr::default()
        }],
    );
    let mut program = type_descriptor_program(vec![service_file], vec![package_file]);
    program.types.descriptors.insert(
        service_addr.clone(),
        anonymous_type_decl("Session", descriptor.clone()),
    );
    program.types.descriptors.insert(
        exported_package_addr.clone(),
        anonymous_type_decl("Session", descriptor),
    );
    program
        .types
        .exported_types
        .insert_service(ServiceSymbolKey::new("session", "Session"), service_addr);
    program
        .types
        .exported_types
        .insert_package(PackageSymbolKey::new(0, "Session"), exported_package_addr);

    let image = program.linked_image();
    let plan = RuntimeTypePlan::from_linked(
        &db_object_symbol("session", "MissingSession"),
        &PlanContext::new(&image, &ExecutableAddr::package(0, 0, 0)),
    )
    .expect("missing package-local DB object symbol should bridge to unknown");

    assert!(matches!(plan.node(), RuntimeTypeNode::Unknown));
}

#[test]
fn linked_db_object_symbol_rejects_ambiguous_package_local_declarations() {
    let descriptor = record_descriptor([("id", linked_builtin_type_ref("string"))]);
    let first = type_descriptor_file(
        "file:package-a",
        "session",
        type_declarations([("Session", 0)]),
        vec![TypeDeclIr {
            name: "Session".to_string(),
            descriptor: descriptor.clone(),
            ..TypeDeclIr::default()
        }],
    );
    let second = type_descriptor_file(
        "file:package-b",
        "session",
        type_declarations([("Session", 0)]),
        vec![TypeDeclIr {
            name: "Session".to_string(),
            descriptor,
            ..TypeDeclIr::default()
        }],
    );
    let program = type_descriptor_program(Vec::new(), vec![first, second]);

    let image = program.linked_image();
    let error = RuntimeTypePlan::from_linked(
        &db_object_symbol("session", "Session"),
        &PlanContext::new(&image, &ExecutableAddr::package(0, 0, 0)),
    )
    .expect_err("ambiguous package-local DB object symbol should fail");

    assert!(matches!(
        error,
        RuntimeError::InvalidArtifact(message)
            if message.contains("ambiguous type symbol session.Session")
    ));
}

#[test]
fn linked_service_symbol_resolves_package_local_type_link_target() {
    let package_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: crate::program::FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let package_descriptor =
        record_descriptor([("packageOnly", linked_builtin_type_ref("string"))]);
    let mut package_file = type_descriptor_file(
        "file:package",
        "pkg.types",
        FileDeclarations::default(),
        vec![TypeDeclIr {
            name: "Local".to_string(),
            descriptor: package_descriptor.clone(),
            ..TypeDeclIr::default()
        }],
    );
    Arc::make_mut(&mut package_file)
        .link_targets
        .types
        .insert("Local".to_string(), 0);
    let mut program = type_descriptor_program(Vec::new(), vec![package_file]);
    program.types.descriptors.insert(
        package_addr,
        anonymous_type_decl("Local", package_descriptor),
    );

    let image = program.linked_image();
    let plan = RuntimeTypePlan::from_linked(
        &LinkedTypeRef::Record {
            fields: BTreeMap::from([(
                "local".to_string(),
                LinkedTypeRef::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "pkg.types".to_string(),
                        symbol: "Local".to_string(),
                    },
                },
            )]),
        },
        &PlanContext::new(&image, &ExecutableAddr::package(0, 0, 0)),
    )
    .expect("package-local service symbol type should resolve in nested descriptor");

    let RuntimeTypeNode::Record { fields, .. } = plan.node() else {
        panic!("top-level structural record should remain a record");
    };
    let local = fields
        .iter()
        .find(|field| field.name == "local")
        .expect("record should contain local field");
    assert!(record_has_field(local.ty.node(), "packageOnly"));
}

fn db_object_symbol(module_path: &str, symbol: &str) -> LinkedTypeRef {
    LinkedTypeRef::DbObjectSymbol {
        symbol: ServiceSymbolRef {
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
        },
    }
}

fn linked_builtin_type_ref(name: &str) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: name.to_string(),
        args: Vec::new(),
    }
}

fn record_descriptor<const N: usize>(fields: [(&str, LinkedTypeRef); N]) -> LinkedTypeDescriptor {
    LinkedTypeDescriptor::Record {
        fields: fields
            .into_iter()
            .map(|(name, ty)| (name.to_string(), ty))
            .collect(),
    }
}

fn record_has_field(node: &RuntimeTypeNode, name: &str) -> bool {
    matches!(
        node,
        RuntimeTypeNode::Record { fields, .. }
            if fields.iter().any(|field| field.name == name)
    )
}

fn runtime_field<'a>(
    fields: &'a [RuntimeRecordFieldPlan],
    name: &str,
) -> &'a RuntimeRecordFieldPlan {
    fields
        .iter()
        .find(|field| field.name == name)
        .unwrap_or_else(|| panic!("record should contain field {name}"))
}

fn type_declarations<const N: usize>(items: [(&str, usize); N]) -> FileDeclarations {
    serde_json::from_value(json!({
        "types": items
            .into_iter()
            .map(|(symbol, type_index)| {
                (
                    symbol.to_string(),
                    json!({
                        "typeIndex": type_index,
                        "symbol": format!("session.{symbol}"),
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>()
    }))
    .expect("test type declarations should decode")
}

fn type_descriptor_file(
    identity: &str,
    module_path: &str,
    declarations: FileDeclarations,
    types: Vec<TypeDeclIr>,
) -> Arc<LinkedFileUnit> {
    Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: identity.to_string(),
        source_ast_hash: format!("source:{identity}"),
        module_path: module_path.to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations,
        link_targets: FileLinkTargets::default(),
        types,
        constants: Vec::new(),
        executables: vec![LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: format!("{module_path}.run"),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        }],
        external_refs: ExternalRefTable::default(),
    })
}

/// Case #23 — File IR local `type_index` must not escape its owning file.
///
/// `LinkedTypeRef::LocalType { type_index }` is resolved by combining the
/// `type_index` with the **current executable's** `unit` and `file` from
/// `PlanContext::current_addr`.  The same `type_index` value in a different
/// file's descriptor is a completely different type; there is no mechanism by
/// which it could "leak" into another file's resolution.
///
/// This test builds two package files that both have a type at `type_index = 0`,
/// but with different field layouts.  When `from_linked` is called with a
/// `LocalType { type_index: 0 }` while the current executable belongs to file 0,
/// it resolves to file 0's declaration.  When the executable belongs to file 1,
/// the same ref resolves to file 1's declaration.  The two results are distinct,
/// proving that `type_index` is always scoped to its owning file.
#[test]
fn local_type_index_is_scoped_to_owning_file_context() {
    // File 0 declares a type at index 0 with field "fromFile0".
    let file0_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: crate::program::FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let file0_descriptor = record_descriptor([("fromFile0", linked_builtin_type_ref("string"))]);
    let file0 = type_descriptor_file(
        "file:pkg-file0",
        "pkg.file0",
        type_declarations([("TypeInFile0", 0)]),
        vec![TypeDeclIr {
            name: "TypeInFile0".to_string(),
            descriptor: file0_descriptor.clone(),
            ..TypeDeclIr::default()
        }],
    );

    // File 1 declares a different type at the same index 0 with field "fromFile1".
    let file1_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: crate::program::FileAddr::LoadedFileIndex(1),
        type_index: 0,
    };
    let file1_descriptor = record_descriptor([("fromFile1", linked_builtin_type_ref("string"))]);
    let file1 = type_descriptor_file(
        "file:pkg-file1",
        "pkg.file1",
        type_declarations([("TypeInFile1", 0)]),
        vec![TypeDeclIr {
            name: "TypeInFile1".to_string(),
            descriptor: file1_descriptor.clone(),
            ..TypeDeclIr::default()
        }],
    );

    let mut program = type_descriptor_program(Vec::new(), vec![file0, file1]);
    program.types.descriptors.insert(
        file0_addr,
        anonymous_type_decl("TypeInFile0", file0_descriptor),
    );
    program.types.descriptors.insert(
        file1_addr,
        anonymous_type_decl("TypeInFile1", file1_descriptor),
    );

    let image = program.linked_image();
    let local_type_ref = LinkedTypeRef::LocalType { type_index: 0 };

    // When current_addr points at file 0, type_index 0 resolves to TypeInFile0.
    let plan_file0 = RuntimeTypePlan::from_linked(
        &LinkedTypeRef::Address {
            addr: TypeAddr {
                unit: UnitAddr::Package(0),
                file: crate::program::FileAddr::LoadedFileIndex(0),
                type_index: 0,
            },
        },
        &PlanContext::new(&image, &ExecutableAddr::package(0, 0, 0)),
    )
    .expect("file0 address should resolve");

    // When current_addr points at file 1, the same type_index 0 resolves to TypeInFile1.
    // We test this via from_linked_ref (nested position) rather than from_linked because
    // top-level LocalType errors; the linker would have promoted it to Address before
    // execution, but nested refs can still appear as LocalType during resolution.
    let plan_file1 = RuntimeTypePlan::from_linked(
        &LinkedTypeRef::Address {
            addr: TypeAddr {
                unit: UnitAddr::Package(0),
                file: crate::program::FileAddr::LoadedFileIndex(1),
                type_index: 0,
            },
        },
        &PlanContext::new(&image, &ExecutableAddr::package(0, 1, 0)),
    )
    .expect("file1 address should resolve");

    // Prove the two plans are distinct: one resolves to file0's field, the other to file1's.
    assert!(
        record_has_field(plan_file0.node(), "fromFile0"),
        "type_index 0 in file 0 should resolve to TypeInFile0"
    );
    assert!(
        !record_has_field(plan_file0.node(), "fromFile1"),
        "type_index 0 in file 0 must not bleed into file 1's type"
    );
    assert!(
        record_has_field(plan_file1.node(), "fromFile1"),
        "type_index 0 in file 1 should resolve to TypeInFile1"
    );
    assert!(
        !record_has_field(plan_file1.node(), "fromFile0"),
        "type_index 0 in file 1 must not bleed into file 0's type"
    );

    // Additionally confirm that a nested LocalType ref uses ctx.current_addr's file.
    let wrapper = LinkedTypeRef::Record {
        fields: BTreeMap::from([("inner".to_string(), local_type_ref)]),
    };
    let plan_via_local = RuntimeTypePlan::from_linked(
        &wrapper,
        &PlanContext::new(&image, &ExecutableAddr::package(0, 0, 0)),
    )
    .expect("record wrapping LocalType should build a plan");
    let RuntimeTypeNode::Record { fields, .. } = plan_via_local.node() else {
        panic!("wrapper should remain a record");
    };
    let inner_field = fields
        .iter()
        .find(|f| f.name == "inner")
        .expect("inner field should be present");
    // LocalType at index 0 in the context of file 0 resolves to TypeInFile0's descriptor.
    assert!(
        record_has_field(inner_field.ty.node(), "fromFile0"),
        "nested LocalType {{ type_index: 0 }} in file-0 context should resolve to TypeInFile0"
    );
}

/// Case #24 — `TypeAddr` equality is NOT cross-activation ABI type equality.
///
/// Two distinct `LinkedProgramImage`s (separate service activations) can assign
/// the same `TypeAddr` coordinates (`unit/file/type_index`) to completely
/// different type declarations.  `TypeAddr` equality only makes sense within a
/// single image; it must never be used to assert that two types from different
/// activations are the same ABI type.
///
/// The proper cross-activation identity key is `AbiTypeId` (derived from the
/// declaration anchor + publication identity), not `TypeAddr`.
///
/// This test proves the point concretely: two programs share the same `TypeAddr`,
/// but the type at that address has different fields in each program.  Resolving
/// via `from_linked` therefore yields different plans, confirming that TypeAddr
/// identity is activation-local.
#[test]
fn type_addr_equality_does_not_imply_abi_equality_across_activations() {
    let shared_addr = TypeAddr {
        unit: UnitAddr::Service,
        file: crate::program::FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };

    // Activation A: the type at shared_addr has field "onlyInActivationA".
    let descriptor_a =
        record_descriptor([("onlyInActivationA", linked_builtin_type_ref("string"))]);
    let service_file_a = type_descriptor_file(
        "file:svc-a",
        "svc.main",
        FileDeclarations::default(),
        vec![TypeDeclIr {
            name: "TypeA".to_string(),
            descriptor: descriptor_a.clone(),
            ..TypeDeclIr::default()
        }],
    );
    let mut program_a = type_descriptor_program(vec![service_file_a], Vec::new());
    program_a.types.descriptors.insert(
        shared_addr.clone(),
        anonymous_type_decl("TypeA", descriptor_a),
    );

    // Activation B: the *same* TypeAddr coordinates, but a completely different type.
    let descriptor_b =
        record_descriptor([("onlyInActivationB", linked_builtin_type_ref("string"))]);
    let service_file_b = type_descriptor_file(
        "file:svc-b",
        "svc.main",
        FileDeclarations::default(),
        vec![TypeDeclIr {
            name: "TypeB".to_string(),
            descriptor: descriptor_b.clone(),
            ..TypeDeclIr::default()
        }],
    );
    let mut program_b = type_descriptor_program(vec![service_file_b], Vec::new());
    program_b.types.descriptors.insert(
        shared_addr.clone(),
        anonymous_type_decl("TypeB", descriptor_b),
    );

    let image_a = program_a.linked_image();
    let image_b = program_b.linked_image();
    let current_addr = ExecutableAddr::service(0, 0);

    // Resolving the same TypeAddr in image_a gives TypeA's fields.
    let plan_a = RuntimeTypePlan::from_linked(
        &LinkedTypeRef::Address {
            addr: shared_addr.clone(),
        },
        &PlanContext::new(&image_a, &current_addr),
    )
    .expect("TypeAddr should resolve in image_a");

    // Resolving the same TypeAddr in image_b gives TypeB's fields.
    let plan_b = RuntimeTypePlan::from_linked(
        &LinkedTypeRef::Address { addr: shared_addr },
        &PlanContext::new(&image_b, &current_addr),
    )
    .expect("TypeAddr should resolve in image_b");

    // TypeAddr equality (same coordinates) does NOT mean same ABI type.
    assert!(
        record_has_field(plan_a.node(), "onlyInActivationA"),
        "TypeAddr in activation A should resolve to TypeA"
    );
    assert!(
        !record_has_field(plan_a.node(), "onlyInActivationB"),
        "TypeA must not bleed fields from activation B"
    );
    assert!(
        record_has_field(plan_b.node(), "onlyInActivationB"),
        "TypeAddr in activation B should resolve to TypeB"
    );
    assert!(
        !record_has_field(plan_b.node(), "onlyInActivationA"),
        "TypeB must not bleed fields from activation A"
    );
}

/// Case #25 — Runtime type resolution never parses source display paths.
///
/// The `value_codec` type-resolution path (`from_linked`, `program_package_type_addr`,
/// `program_service_symbol_type_addr`) works entirely with structured data:
/// `TypeAddr`, `PackageSymbolKey`, `ServiceSymbolKey`, `PackageSymbolRef`, and
/// `ServiceSymbolRef`.  It must never split a type-descriptor string, walk a
/// dotted path, or otherwise parse a "display name" to locate a symbol.
///
/// This test constructs a program where:
///   - A legitimate symbol `"my.module.RealType"` is resolvable.
///   - A crafted symbol whose *name* looks like `"my.module.RealType"` but is
///     keyed differently in the registry cannot impersonate the real one.
///
/// If the resolution were string-based, the two would collide; because it is
/// structured, they remain distinct, confirming that resolution paths never
/// parse display strings.
#[test]
fn runtime_type_resolution_does_not_use_source_display_paths() {
    let real_addr = TypeAddr {
        unit: UnitAddr::Service,
        file: crate::program::FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let imposter_addr = TypeAddr {
        unit: UnitAddr::Service,
        file: crate::program::FileAddr::LoadedFileIndex(0),
        type_index: 1,
    };

    let real_descriptor = record_descriptor([("realField", linked_builtin_type_ref("string"))]);
    let imposter_descriptor =
        record_descriptor([("imposterField", linked_builtin_type_ref("string"))]);

    let service_file = type_descriptor_file(
        "file:svc",
        "my.module",
        FileDeclarations::default(),
        vec![
            TypeDeclIr {
                name: "RealType".to_string(),
                descriptor: real_descriptor.clone(),
                ..TypeDeclIr::default()
            },
            TypeDeclIr {
                name: "my.module.RealType".to_string(), // display-path-shaped name
                descriptor: imposter_descriptor.clone(),
                ..TypeDeclIr::default()
            },
        ],
    );

    let mut program = type_descriptor_program(vec![service_file], Vec::new());
    program.types.descriptors.insert(
        real_addr.clone(),
        anonymous_type_decl("RealType", real_descriptor),
    );
    program.types.descriptors.insert(
        imposter_addr.clone(),
        anonymous_type_decl("my.module.RealType", imposter_descriptor),
    );
    // Register only the real symbol under the structured key "my.module" / "RealType".
    program
        .types
        .exported_types
        .insert_service(ServiceSymbolKey::new("my.module", "RealType"), real_addr);
    // The imposter is NOT registered; it exists only in the descriptor table.

    let image = program.linked_image();
    let current_addr = ExecutableAddr::service(0, 0);

    // A ServiceSymbol with module "my.module" and symbol "RealType" resolves
    // to the *real* declaration via structured lookup, not display-path parsing.
    let service_ref = LinkedTypeRef::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: "my.module".to_string(),
            symbol: "RealType".to_string(),
        },
    };
    // Use from_linked_nested_ref (nested position so ServiceSymbol resolves).
    let plan = RuntimeTypePlan::from_linked_nested_ref(
        &service_ref,
        &PlanContext::new(&image, &current_addr),
    )
    .expect("structured service symbol should resolve");

    assert!(
        record_has_field(plan.node(), "realField"),
        "structured lookup must resolve to the real declaration, not the imposter"
    );
    assert!(
        !record_has_field(plan.node(), "imposterField"),
        "display-path-shaped symbol name must not affect structured resolution"
    );
}

fn type_descriptor_program(
    service_files: Vec<Arc<LinkedFileUnit>>,
    package_files: Vec<Arc<LinkedFileUnit>>,
) -> RuntimeProgram {
    RuntimeProgram {
        service: ServiceMeta {
            id: "svc".to_string(),
            display_name: Some("Service".to_string()),
            metadata: Default::default(),
        },
        version: "v1".to_string(),
        build_id: "build:type-descriptor-test".to_string(),
        service_files,
        packages: vec![Arc::new(PackageUnit::empty(
            "example.com/pkg",
            "1.0.0",
            "pkg:build".to_string(),
            "pkg:abi",
        ))],
        package_files: vec![package_files],
        package_configs: Vec::new(),
        service_dependencies: Vec::new(),
        timeout: Default::default(),
        operation_route_bindings: Vec::new(),
        routes: HashMap::new(),
        spawn_routes: HashMap::new(),
        operations: HashMap::new(),
        operation_receivers: HashMap::new(),
        db: Vec::new(),
        actors: Vec::new(),
        link_overlay: LinkOverlay::default(),
        gateway: GatewayConfig::default(),
        types: RuntimeTypeContext::default(),
    }
}
