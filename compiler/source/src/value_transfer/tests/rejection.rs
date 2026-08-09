use std::collections::BTreeMap;

use skiff_artifact_model::{
    FunctionTypeParamIr, NativeValueLifecycleLookupError, PackageSchemaTypeId, ServiceSymbolRef,
    TypeDescriptorIr, TypeRefIr, ValueTransferPlanKind,
};

use super::*;

#[test]
fn resources_cannot_hide_in_ordinary_containers_or_nominals() {
    let mut facts = SourceValueTransferFacts::new();
    let stream = generic_builtin("Stream", vec![builtin("number")]);
    for ty in [
        generic_builtin("Array", vec![stream.clone()]),
        generic_builtin("Map", vec![builtin("string"), stream.clone()]),
        record([("events", stream.clone())]),
        TypeRefIr::Union {
            items: vec![builtin("null"), stream.clone()],
        },
        TypeRefIr::Nullable {
            inner: Box::new(stream.clone()),
        },
    ] {
        let error = plan(&facts, &ty).expect_err("ordinary aggregate must reject Stream");
        assert!(matches!(
            root_error(&error),
            SourceValueTransferError::StructuralPositionNotSnapshotShare {
                found: ValueTransferPlanKind::AffineResource,
                ..
            }
        ));
    }

    facts.insert_nominal(
        local_id(3),
        ordinary_fact(
            &[],
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([("events".to_string(), stream)]),
            },
        ),
    );
    let error = plan(&facts, &local(3)).expect_err("nominal record must reject Stream");
    assert!(matches!(
        root_error(&error),
        SourceValueTransferError::StructuralPositionNotSnapshotShare {
            found: ValueTransferPlanKind::AffineResource,
            ..
        }
    ));
}

#[test]
fn stream_argument_and_builtin_arity_fail_closed() {
    let facts = SourceValueTransferFacts::new();
    let nested = generic_builtin(
        "Stream",
        vec![generic_builtin("Stream", vec![builtin("number")])],
    );
    assert!(matches!(
        root_error(&plan(&facts, &nested).expect_err("Stream payload must be snapshot")),
        SourceValueTransferError::StructuralPositionNotSnapshotShare {
            found: ValueTransferPlanKind::AffineResource,
            ..
        }
    ));
    assert_eq!(
        plan(&facts, &generic_builtin("Array", vec![])),
        Err(SourceValueTransferError::BuiltinArityMismatch {
            builtin: "Array".to_string(),
            expected: 1,
            actual: 0,
        })
    );
}

#[test]
fn native_lifecycle_nesting_uses_the_registry_limit() {
    let facts = SourceValueTransferFacts::new();
    let mut ty = builtin("string");
    for _ in 0..=skiff_artifact_model::MAX_NATIVE_VALUE_LIFECYCLE_ARGUMENTS {
        ty = generic_builtin("Array", vec![ty]);
    }
    assert!(matches!(
        root_error(&plan(&facts, &ty).expect_err("native nesting must be bounded")),
        SourceValueTransferError::NativeLifecycleLookup {
            source: NativeValueLifecycleLookupError::NestingLimit,
            ..
        }
    ));
}

#[test]
fn unknown_and_unregistered_builtins_never_receive_guessed_plans() {
    let facts = SourceValueTransferFacts::new();
    assert_eq!(
        plan(&facts, &builtin("MysteryHandle")),
        Err(SourceValueTransferError::UnknownBuiltin {
            name: "MysteryHandle".to_string(),
        })
    );
    assert!(matches!(
        plan(&facts, &builtin("TaskRef")),
        Err(SourceValueTransferError::NativeLifecycleLookup {
            source: NativeValueLifecycleLookupError::Missing { .. },
            ..
        })
    ));
}

#[test]
fn callbacks_actors_db_schema_and_unregistered_native_nominals_are_rejected() {
    let mut facts = SourceValueTransferFacts::new();
    facts.insert_nominal(
        local_id(1),
        SourceValueTransferNominalFact {
            declaration_module: "app.model".to_string(),
            type_parameters: vec![],
            semantics: SourceValueTransferNominalSemantics::Actor,
        },
    );
    facts.insert_nominal(
        local_id(2),
        SourceValueTransferNominalFact {
            declaration_module: "app.model".to_string(),
            type_parameters: vec![],
            semantics: SourceValueTransferNominalSemantics::NativeOpaque,
        },
    );

    let callback = TypeRefIr::Function {
        params: vec![FunctionTypeParamIr {
            name: "value".to_string(),
            ty: builtin("string"),
        }],
        return_type: Box::new(builtin("void")),
    };
    assert_eq!(
        plan(&facts, &callback),
        Err(SourceValueTransferError::CallbackTypeUnsupported)
    );
    assert_eq!(
        plan(&facts, &local(1)),
        Err(SourceValueTransferError::ActorUnsupported {
            nominal: local_id(1),
        })
    );
    assert_eq!(
        plan(&facts, &local(2)),
        Err(SourceValueTransferError::NativeNominalNotRegistered {
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
        plan(&facts, &db),
        Err(SourceValueTransferError::DatabaseObjectUnsupported { .. })
    ));
    let schema = TypeRefIr::PackageSchema {
        package_id: "pkg.data".to_string(),
        stable_schema_key: "User".to_string(),
        package_schema_type_id: PackageSchemaTypeId::new("type:user"),
    };
    assert!(matches!(
        plan(&facts, &schema),
        Err(SourceValueTransferError::PackageSchemaUnsupported { .. })
    ));
}
