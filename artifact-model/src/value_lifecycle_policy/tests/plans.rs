use super::*;
use crate::{
    bytecode::{ResourceDropPlan, ValueDropPlan, ValueTransferPlan},
    NativeValueDropPlan, NativeValueLifecycleConcrete, NativeValueLifecycleResolution,
};

#[test]
fn nullable_has_a_fixed_snapshot_root_drop() {
    let resolution = classify_value_lifecycle(
        &TypeRefIr::Nullable {
            inner: Box::new(TypeRefIr::builtin("number")),
        },
        &PositionalTypeEnvironment::empty(),
        &mut RejectingResolver,
        &mut budget(),
    )
    .unwrap();
    assert_eq!(
        resolution.lifecycle,
        NativeValueLifecycleConcrete::SnapshotShare {
            drop: NativeValueDropPlan::SnapshotRelease,
        }
    );
}

#[test]
fn std_task_builtins_are_ordinary_snapshots_and_void_stays_fail_closed() {
    for name in ["TaskRef", "TaskStatus", "TaskCancelResult"] {
        let resolution = classify_value_lifecycle(
            &TypeRefIr::builtin(name),
            &PositionalTypeEnvironment::empty(),
            &mut RejectingResolver,
            &mut budget(),
        )
        .unwrap();
        assert_eq!(
            resolution,
            NativeValueLifecycleResolution {
                lifecycle: NativeValueLifecycleConcrete::SnapshotShare {
                    drop: NativeValueDropPlan::SnapshotRelease,
                },
                embedding: crate::NativeValueEmbedding::Ordinary,
            }
        );
    }

    assert!(matches!(
        classify_value_lifecycle(
            &TypeRefIr::builtin("void"),
            &PositionalTypeEnvironment::empty(),
            &mut RejectingResolver,
            &mut budget(),
        ),
        Err(ValueLifecyclePolicyError::UnsupportedType {
            kind: "unregisteredBuiltin"
        })
    ));
}

#[test]
fn exception_and_catch_result_are_ordinary_snapshot_roots() {
    for ty in [
        TypeRefIr::Builtin {
            name: "Exception".to_string(),
            args: vec![TypeRefIr::builtin("string")],
        },
        TypeRefIr::Builtin {
            name: "CatchResult".to_string(),
            args: vec![TypeRefIr::builtin("bool"), TypeRefIr::builtin("string")],
        },
    ] {
        assert_eq!(
            classify_value_lifecycle(
                &ty,
                &PositionalTypeEnvironment::empty(),
                &mut RejectingResolver,
                &mut budget(),
            )
            .unwrap(),
            NativeValueLifecycleResolution {
                lifecycle: NativeValueLifecycleConcrete::SnapshotShare {
                    drop: NativeValueDropPlan::SnapshotRelease,
                },
                embedding: crate::NativeValueEmbedding::Ordinary,
            }
        );
    }
}

#[test]
fn plans_are_recomputed_and_recursive_shape_is_rejected() {
    let ty = TypeRefIr::builtin("string");
    assert!(verify_value_transfer_plan(
        &ty,
        &ValueTransferPlan::FromType { ty: ty.clone() },
        &PositionalTypeEnvironment::empty(),
        &mut RejectingResolver,
        &mut budget(),
    )
    .is_ok());

    assert!(matches!(
        verify_value_transfer_plan(
            &ty,
            &ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::RecursiveShape { shape_ref: 0 },
            },
            &PositionalTypeEnvironment::empty(),
            &mut RejectingResolver,
            &mut budget(),
        ),
        Err(ValueLifecyclePolicyError::RecursiveShapePlan)
    ));
    assert!(matches!(
        verify_value_transfer_plan(
            &ty,
            &ValueTransferPlan::FromType {
                ty: TypeRefIr::builtin("number")
            },
            &PositionalTypeEnvironment::empty(),
            &mut RejectingResolver,
            &mut budget()
        ),
        Err(ValueLifecyclePolicyError::PlanMismatch { .. })
    ));
    assert!(matches!(
        verify_value_transfer_plan(
            &ty,
            &ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::Trivial
            },
            &PositionalTypeEnvironment::empty(),
            &mut RejectingResolver,
            &mut budget()
        ),
        Err(ValueLifecyclePolicyError::PlanMismatch { .. })
    ));
    assert!(matches!(
        verify_value_transfer_plan(
            &ty,
            &ValueTransferPlan::ExplicitCloneLease {
                clone_adapter: crate::bytecode::NativeValueAdapterRef {
                    binding_key: "missing.clone".to_string()
                },
                drop: ResourceDropPlan::ResourceTableRelease
            },
            &PositionalTypeEnvironment::empty(),
            &mut RejectingResolver,
            &mut budget()
        ),
        Err(ValueLifecyclePolicyError::UnknownAdapter { .. })
    ));
}

#[test]
fn unresolved_owners_and_forbidden_container_children_fail_closed() {
    let dependency = TypeRefIr::PackageSymbol {
        symbol: crate::PackageSymbolRef {
            package: crate::PackageRefIr::Dependency {
                dependency_ref: "dep".to_string(),
            },
            symbol_path: "dep.Value".to_string(),
            abi_expectation: Some("abi".to_string()),
        },
    };
    assert!(matches!(
        normalize_value_lifecycle_type(
            &dependency,
            &PositionalTypeEnvironment::empty(),
            &mut budget()
        ),
        Err(ValueLifecyclePolicyError::UnnormalizedOwner { .. })
    ));

    for residual in [
        TypeRefIr::LocalType { type_index: 0 },
        TypeRefIr::PublicationType {
            module_path: "module".to_string(),
            type_index: 0,
        },
        TypeRefIr::ServiceSymbol {
            symbol: crate::ServiceSymbolRef {
                module_path: "service".to_string(),
                symbol: "Type".to_string(),
            },
        },
        TypeRefIr::DbObjectSymbol {
            symbol: crate::ServiceSymbolRef {
                module_path: "db".to_string(),
                symbol: "Object".to_string(),
            },
        },
    ] {
        assert!(matches!(
            normalize_value_lifecycle_type(
                &residual,
                &PositionalTypeEnvironment::empty(),
                &mut budget()
            ),
            Err(ValueLifecyclePolicyError::UnnormalizedOwner { .. })
        ));
    }

    let array_of_stream = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("string")],
        }],
    };
    assert!(matches!(
        classify_value_lifecycle(
            &array_of_stream,
            &PositionalTypeEnvironment::empty(),
            &mut RejectingResolver,
            &mut budget()
        ),
        Err(ValueLifecyclePolicyError::ArgumentPolicy { .. })
    ));
}
