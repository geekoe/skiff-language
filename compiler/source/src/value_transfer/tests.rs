use std::collections::BTreeMap;

use skiff_artifact_model::{
    FunctionTypeParamIr, InterfaceInstantiationRef, LiteralIr, NominalTypeRefBaseIr,
    PackageSchemaTypeId, ServiceSymbolRef, TypeDescriptorIr, TypeRefIr,
};

use super::*;

fn builtin(name: &str) -> TypeRefIr {
    TypeRefIr::builtin(name)
}

fn generic_builtin(name: &str, args: Vec<TypeRefIr>) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: name.to_string(),
        args,
    }
}

fn local(type_index: u32) -> TypeRefIr {
    TypeRefIr::LocalType { type_index }
}

fn local_id(type_index: u32) -> SourceValueTransferNominalId {
    SourceValueTransferNominalId::Local {
        module_path: "app.model".to_string(),
        type_index,
    }
}

fn ordinary_fact(
    type_parameters: &[&str],
    descriptor: TypeDescriptorIr,
) -> SourceValueTransferNominalFact {
    SourceValueTransferNominalFact {
        declaration_module: "app.model".to_string(),
        type_parameters: type_parameters
            .iter()
            .map(|parameter| (*parameter).to_string())
            .collect(),
        semantics: SourceValueTransferNominalSemantics::Ordinary(descriptor),
    }
}

fn root_error(error: &SourceValueTransferError) -> &SourceValueTransferError {
    match error {
        SourceValueTransferError::AtStructuralPosition { source, .. } => root_error(source),
        other => other,
    }
}

#[test]
fn canonical_scalars_literals_and_structural_aggregates_snapshot_share() {
    let facts = SourceValueTransferFacts::new();
    let structural = TypeRefIr::Record {
        fields: BTreeMap::from([
            (
                "labels".to_string(),
                generic_builtin("Array", vec![builtin("string")]),
            ),
            (
                "lookup".to_string(),
                generic_builtin(
                    "Map",
                    vec![
                        builtin("string"),
                        TypeRefIr::Nullable {
                            inner: Box::new(builtin("number")),
                        },
                    ],
                ),
            ),
        ]),
    };

    for ty in [
        builtin("bool"),
        builtin("bytes"),
        TypeRefIr::Literal {
            value: LiteralIr::String {
                value: "ready".to_string(),
            },
        },
        structural,
        TypeRefIr::Union {
            items: vec![builtin("integer"), builtin("null")],
        },
    ] {
        assert_eq!(
            facts.classify("app.model", &ty),
            Ok(SourceValueTransferKind::SnapshotShare)
        );
    }
}

#[test]
fn stream_is_affine_but_cannot_hide_in_an_ordinary_record() {
    let mut facts = SourceValueTransferFacts::new();
    let stream = generic_builtin("Stream", vec![builtin("string")]);
    assert_eq!(
        facts.classify("app.model", &stream),
        Ok(SourceValueTransferKind::AffineResource)
    );

    let record = TypeRefIr::Record {
        fields: BTreeMap::from([("events".to_string(), stream)]),
    };
    assert_eq!(
        facts.classify("app.model", &record),
        Err(
            SourceValueTransferError::StructuralPositionNotSnapshotShare {
                position: SourceValueTransferPosition::AnonymousRecordField {
                    field: "events".to_string(),
                },
                found: SourceValueTransferKind::AffineResource,
            }
        )
    );

    facts.insert_nominal(
        local_id(3),
        ordinary_fact(
            &[],
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "events".to_string(),
                    generic_builtin("Stream", vec![builtin("string")]),
                )]),
            },
        ),
    );
    let error = facts
        .classify("app.model", &local(3))
        .expect_err("a nominal record must not hide a resource");
    assert!(matches!(
        root_error(&error),
        SourceValueTransferError::StructuralPositionNotSnapshotShare {
            found: SourceValueTransferKind::AffineResource,
            ..
        }
    ));
}

#[test]
fn exact_generic_nominal_recursively_proves_ordinary_fields() {
    let mut facts = SourceValueTransferFacts::new();
    facts.insert_nominal(
        local_id(0),
        ordinary_fact(
            &["T"],
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "value".to_string(),
                    TypeRefIr::TypeParam {
                        name: "T".to_string(),
                    },
                )]),
            },
        ),
    );
    let boxed_string = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
        arguments: vec![builtin("string")],
    };
    assert_eq!(
        facts.classify("app.model", &boxed_string),
        Ok(SourceValueTransferKind::SnapshotShare)
    );
    let nested_box = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
        arguments: vec![boxed_string],
    };
    assert_eq!(
        facts.classify("app.model", &nested_box),
        Ok(SourceValueTransferKind::SnapshotShare),
        "finite repeated generic instantiations are not declaration cycles"
    );

    let boxed_stream = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
        arguments: vec![generic_builtin("Stream", vec![builtin("string")])],
    };
    let error = facts
        .classify("app.model", &boxed_stream)
        .expect_err("resource type argument must not hide in an ordinary nominal");
    assert!(matches!(
        root_error(&error),
        SourceValueTransferError::StructuralPositionNotSnapshotShare {
            found: SourceValueTransferKind::AffineResource,
            ..
        }
    ));
}

#[test]
fn recursive_nominal_fails_with_exact_cycle_identity() {
    let mut facts = SourceValueTransferFacts::new();
    facts.insert_nominal(
        local_id(0),
        ordinary_fact(
            &[],
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "next".to_string(),
                    TypeRefIr::Nullable {
                        inner: Box::new(local(0)),
                    },
                )]),
            },
        ),
    );

    let error = facts
        .classify("app.model", &local(0))
        .expect_err("recursive nominal must fail closed");
    assert_eq!(
        root_error(&error),
        &SourceValueTransferError::RecursiveNominal {
            nominal: local_id(0),
        }
    );
}

#[test]
fn unknown_missing_and_wrong_arity_types_are_stable_errors() {
    let mut facts = SourceValueTransferFacts::new();
    facts.insert_nominal(
        local_id(1),
        ordinary_fact(
            &["T"],
            TypeDescriptorIr::Representation {
                representation: TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
            },
        ),
    );

    assert_eq!(
        facts.classify("app.model", &builtin("unknown")),
        Err(SourceValueTransferError::UnknownBuiltin {
            name: "unknown".to_string(),
        })
    );
    assert_eq!(
        facts.classify("app.model", &builtin("MysteryHandle")),
        Err(SourceValueTransferError::UnknownBuiltin {
            name: "MysteryHandle".to_string(),
        })
    );
    assert_eq!(
        facts.classify("app.model", &local(404)),
        Err(SourceValueTransferError::MissingNominalFacts {
            nominal: local_id(404),
        })
    );
    assert_eq!(
        facts.classify("app.model", &generic_builtin("Array", vec![])),
        Err(SourceValueTransferError::BuiltinArityMismatch {
            builtin: "Array".to_string(),
            expected: 1,
            actual: 0,
        })
    );
    assert_eq!(
        facts.classify("app.model", &local(1)),
        Err(SourceValueTransferError::NominalArityMismatch {
            nominal: local_id(1),
            expected: 1,
            actual: 0,
        })
    );
}

#[test]
fn cross_module_local_type_arguments_require_exact_externalization() {
    let mut facts = SourceValueTransferFacts::new();
    let identity = SourceValueTransferNominalId::ServiceSymbol {
        module_path: "dependency.model".to_string(),
        symbol: "Box".to_string(),
    };
    facts.insert_nominal(
        identity.clone(),
        SourceValueTransferNominalFact {
            declaration_module: "dependency.model".to_string(),
            type_parameters: vec!["T".to_string()],
            semantics: SourceValueTransferNominalSemantics::Ordinary(
                TypeDescriptorIr::Representation {
                    representation: TypeRefIr::TypeParam {
                        name: "T".to_string(),
                    },
                },
            ),
        },
    );
    let ty = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "dependency.model".to_string(),
                symbol: "Box".to_string(),
            },
        },
        arguments: vec![local(0)],
    };

    assert_eq!(
        facts.classify("app.model", &ty),
        Err(SourceValueTransferError::CrossModuleLocalTypeArgument {
            nominal: identity,
            index: 0,
            argument_module: "app.model".to_string(),
            declaration_module: "dependency.model".to_string(),
        })
    );
}

#[test]
fn native_semantics_are_explicit_and_never_derived_from_builtin_kind() {
    let mut facts = SourceValueTransferFacts::new();
    let task_ref = SourceValueTransferNativeTypeId::CompilerBuiltin {
        canonical_name: "TaskRef".to_string(),
    };
    assert_eq!(
        facts.classify("app.model", &builtin("TaskRef")),
        Err(SourceValueTransferError::MissingNativeSemantics {
            native_type: task_ref.clone(),
            category: SourceValueTransferNativeCategory::Opaque,
        })
    );

    facts.insert_native_semantics(task_ref, SourceValueTransferKind::MoveOnly);
    assert_eq!(
        facts.classify("app.model", &builtin("TaskRef")),
        Ok(SourceValueTransferKind::MoveOnly)
    );

    let lease_id = local_id(4);
    facts.insert_nominal(
        lease_id.clone(),
        SourceValueTransferNominalFact {
            declaration_module: "app.model".to_string(),
            type_parameters: vec![],
            semantics: SourceValueTransferNominalSemantics::NativeOpaque,
        },
    );
    assert!(matches!(
        facts.classify("app.model", &local(4)),
        Err(SourceValueTransferError::MissingNativeSemantics { .. })
    ));
    facts.insert_native_semantics(
        SourceValueTransferNativeTypeId::Nominal(lease_id),
        SourceValueTransferKind::ExplicitCloneLease,
    );
    assert_eq!(
        facts.classify("app.model", &local(4)),
        Ok(SourceValueTransferKind::ExplicitCloneLease)
    );
}

#[test]
fn unsupported_capabilities_callbacks_actors_db_and_schema_fail_closed() {
    let mut facts = SourceValueTransferFacts::new();
    facts.insert_nominal(
        local_id(2),
        SourceValueTransferNominalFact {
            declaration_module: "app.model".to_string(),
            type_parameters: vec![],
            semantics: SourceValueTransferNominalSemantics::Actor,
        },
    );

    assert!(matches!(
        facts.classify("app.model", &builtin("ClientCapability")),
        Err(SourceValueTransferError::MissingNativeSemantics {
            category: SourceValueTransferNativeCategory::Capability,
            ..
        })
    ));
    assert_eq!(
        facts.classify(
            "app.model",
            &TypeRefIr::Function {
                params: vec![FunctionTypeParamIr {
                    name: "value".to_string(),
                    ty: builtin("string"),
                }],
                return_type: Box::new(builtin("void")),
            }
        ),
        Err(SourceValueTransferError::CallbackTypeUnsupported)
    );
    assert_eq!(
        facts.classify("app.model", &local(2)),
        Err(SourceValueTransferError::ActorUnsupported {
            nominal: local_id(2),
        })
    );

    let db = TypeRefIr::DbObjectSymbol {
        symbol: ServiceSymbolRef {
            module_path: "app.data".to_string(),
            symbol: "Users".to_string(),
        },
    };
    assert!(matches!(
        facts.classify("app.model", &db),
        Err(SourceValueTransferError::DatabaseObjectUnsupported { .. })
    ));

    let schema = TypeRefIr::PackageSchema {
        package_id: "dep.pkg".to_string(),
        stable_schema_key: "User".to_string(),
        package_schema_type_id: PackageSchemaTypeId::new("type:user"),
    };
    assert!(matches!(
        facts.classify("app.model", &schema),
        Err(SourceValueTransferError::PackageSchemaUnsupported { .. })
    ));
}

#[test]
fn any_interface_is_snapshot_share_without_becoming_a_callback_plan() {
    let facts = SourceValueTransferFacts::new();
    let any_interface = TypeRefIr::AnyInterface {
        interface: InterfaceInstantiationRef {
            interface_abi_id: "exact-interface-abi".to_string(),
            canonical_type_args: vec![builtin("string")],
        },
    };
    assert_eq!(
        facts.classify("app.model", &any_interface),
        Ok(SourceValueTransferKind::SnapshotShare)
    );
}
