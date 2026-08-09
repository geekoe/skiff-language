use std::collections::BTreeMap;

use skiff_artifact_model::{NominalTypeRefBaseIr, TypeDescriptorIr, TypeRefIr, ValueTransferPlan};

use super::*;

#[test]
fn ordinary_nominals_prove_fields_and_keep_transparent_alias_drop() {
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
    facts.insert_nominal(
        local_id(1),
        ordinary_fact(
            &[],
            TypeDescriptorIr::Alias {
                target: builtin("number"),
            },
        ),
    );

    let boxed = applied_local(0, vec![builtin("string")]);
    assert_eq!(plan(&facts, &boxed), Ok(snapshot_release()));
    assert_eq!(
        plan(&facts, &local(1)),
        Ok(ValueTransferPlan::SnapshotShare {
            drop: skiff_artifact_model::ValueDropPlan::Trivial,
        })
    );

    let nested = applied_local(0, vec![boxed]);
    assert_eq!(
        plan(&facts, &nested),
        Ok(snapshot_release()),
        "substituted generic arguments are proven once and do not create a false declaration cycle"
    );

    let boxed_stream = applied_local(0, vec![generic_builtin("Stream", vec![builtin("number")])]);
    assert!(matches!(
        root_error(
            &plan(&facts, &boxed_stream)
                .expect_err("ordinary nominal cannot accept a resource type argument")
        ),
        SourceValueTransferError::StructuralPositionNotSnapshotShare {
            found: skiff_artifact_model::ValueTransferPlanKind::AffineResource,
            ..
        }
    ));
}

#[test]
fn legal_binders_produce_owner_stable_from_type_expressions() {
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
    let binders = vec!["T".to_string()];
    let parameter = TypeRefIr::TypeParam {
        name: "T".to_string(),
    };
    assert_eq!(
        relocatable_plan(&facts, &parameter, &binders),
        Ok(ValueTransferPlan::FromType {
            ty: parameter.clone(),
        })
    );

    let array = generic_builtin("Array", vec![parameter.clone()]);
    assert_eq!(
        relocatable_plan(&facts, &array, &binders),
        Ok(ValueTransferPlan::FromType { ty: array })
    );

    let boxed = applied_local(0, vec![parameter.clone()]);
    assert_eq!(
        relocatable_plan(&facts, &boxed, &binders),
        Ok(ValueTransferPlan::FromType {
            ty: TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PublicationType {
                    module_path: "app.model".to_string(),
                    type_index: 0,
                },
                arguments: vec![parameter],
            },
        })
    );

    let hidden_resource = record([
        (
            "deferred",
            TypeRefIr::TypeParam {
                name: "T".to_string(),
            },
        ),
        ("events", generic_builtin("Stream", vec![builtin("number")])),
    ]);
    assert!(matches!(
        root_error(
            &relocatable_plan(&facts, &hidden_resource, &binders)
                .expect_err("known resource must not be masked by a deferred field")
        ),
        SourceValueTransferError::StructuralPositionNotSnapshotShare {
            found: skiff_artifact_model::ValueTransferPlanKind::AffineResource,
            ..
        }
    ));
}

#[test]
fn undeclared_or_invalid_binders_fail_closed() {
    let facts = SourceValueTransferFacts::new();
    let parameter = TypeRefIr::TypeParam {
        name: "T".to_string(),
    };
    assert_eq!(
        plan(&facts, &parameter),
        Err(SourceValueTransferError::UnresolvedTypeParameter {
            name: "T".to_string(),
        })
    );
    assert_eq!(
        relocatable_plan(&facts, &parameter, &["U".to_string()]),
        Err(SourceValueTransferError::UnresolvedTypeParameter {
            name: "T".to_string(),
        })
    );
    assert_eq!(
        relocatable_plan(&facts, &parameter, &["T".to_string(), "T".to_string()],),
        Err(SourceValueTransferError::InvalidRelocatableTypeParameter {
            name: "T".to_string(),
        },)
    );
}

#[test]
fn recursive_missing_and_wrong_arity_nominals_have_stable_errors() {
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

    let cycle = plan(&facts, &local(0)).expect_err("recursive nominal must fail closed");
    assert_eq!(
        root_error(&cycle),
        &SourceValueTransferError::RecursiveNominal {
            nominal: local_id(0),
        }
    );
    assert_eq!(
        plan(&facts, &local(404)),
        Err(SourceValueTransferError::MissingNominalFacts {
            nominal: local_id(404),
        })
    );
    assert_eq!(
        plan(&facts, &local(1)),
        Err(SourceValueTransferError::NominalArityMismatch {
            nominal: local_id(1),
            expected: 1,
            actual: 0,
        })
    );
}
