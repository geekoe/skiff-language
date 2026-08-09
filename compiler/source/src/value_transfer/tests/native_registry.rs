use skiff_artifact_model::{
    NativeResourceDropPlan, NativeValueAdapterRole, NativeValueArgumentPolicy, NativeValueDropPlan,
    NativeValueEmbedding, NativeValueLifecycleAdapter, NativeValueLifecycleEntry,
    NativeValueLifecycleLookupError, NativeValueLifecycleRegistry, NativeValueLifecycleTemplate,
    NativeValueTypeConstructor, NativeValueTypePattern, ResourceDropPlan, ValueDropPlan,
    ValueTransferPlan, ValueTransferPlanKind,
};

use super::*;

fn package_entry(
    symbol_path: &str,
    abi: &str,
    policies: Vec<NativeValueArgumentPolicy>,
    lifecycle: NativeValueLifecycleTemplate,
    embedding: NativeValueEmbedding,
) -> NativeValueLifecycleEntry {
    NativeValueLifecycleEntry {
        pattern: NativeValueTypePattern {
            constructor: NativeValueTypeConstructor::PackageSymbol {
                package_id: "pkg.lifecycle".to_string(),
                symbol_path: symbol_path.to_string(),
                abi_identity: abi.to_string(),
            },
            argument_policies: policies,
        },
        lifecycle,
        embedding,
    }
}

fn adapter(binding_key: &str, role: NativeValueAdapterRole) -> NativeValueLifecycleAdapter {
    NativeValueLifecycleAdapter {
        binding_key: binding_key.to_string(),
        role,
        abi_version: 3,
    }
}

fn custom_registry() -> NativeValueLifecycleRegistry {
    let mut entries = skiff_artifact_model::native_value_lifecycle_registry()
        .entries()
        .to_vec();
    entries.extend([
        package_entry(
            "MoveToken",
            "abi:move:v1",
            vec![],
            NativeValueLifecycleTemplate::MoveOnly {
                drop: NativeValueDropPlan::NativeAdapter {
                    adapter: adapter("token.drop", NativeValueAdapterRole::ValueDrop),
                },
            },
            NativeValueEmbedding::Forbidden,
        ),
        package_entry(
            "Lease",
            "abi:lease:v1",
            vec![],
            NativeValueLifecycleTemplate::ExplicitCloneLease {
                clone_adapter: adapter("lease.clone", NativeValueAdapterRole::CloneLease),
                drop: NativeResourceDropPlan::NativeAdapter {
                    adapter: adapter("lease.drop", NativeValueAdapterRole::ResourceDrop),
                },
            },
            NativeValueEmbedding::Forbidden,
        ),
        package_entry(
            "Forward",
            "abi:forward:v1",
            vec![NativeValueArgumentPolicy::RequireSnapshotShare],
            NativeValueLifecycleTemplate::FromType { argument_index: 0 },
            NativeValueEmbedding::Ordinary,
        ),
        package_entry(
            "PrivilegedSnapshot",
            "abi:privileged:v1",
            vec![],
            NativeValueLifecycleTemplate::SnapshotShare {
                drop: NativeValueDropPlan::Trivial,
            },
            NativeValueEmbedding::Privileged,
        ),
    ]);
    NativeValueLifecycleRegistry::new("source-test-lifecycle", "v1", entries)
        .expect("test lifecycle registry is valid")
}

#[test]
fn exact_registry_adapters_map_to_complete_artifact_plans() {
    let registry = custom_registry();
    let facts = SourceValueTransferFacts::new();
    let move_token = exact_package_type("MoveToken", "abi:move:v1", vec![]);
    let lease = exact_package_type("Lease", "abi:lease:v1", vec![]);

    let move_plan = plan_with_registry(&facts, &registry, &move_token, &[])
        .expect("registered move-only package type has a plan");
    assert_eq!(
        move_plan,
        ValueTransferPlan::MoveOnly {
            drop: ValueDropPlan::NativeAdapter {
                adapter: skiff_artifact_model::NativeValueAdapterRef {
                    binding_key: "token.drop".to_string(),
                },
            },
        }
    );
    let lease_plan = plan_with_registry(&facts, &registry, &lease, &[])
        .expect("registered explicit lease has a plan");
    assert_eq!(
        lease_plan,
        ValueTransferPlan::ExplicitCloneLease {
            clone_adapter: skiff_artifact_model::NativeValueAdapterRef {
                binding_key: "lease.clone".to_string(),
            },
            drop: ResourceDropPlan::NativeAdapter {
                adapter: skiff_artifact_model::NativeValueAdapterRef {
                    binding_key: "lease.drop".to_string(),
                },
            },
        }
    );
    assert_no_recursive_shape(&move_plan);
    assert_no_recursive_shape(&lease_plan);

    for (field, ty, expected) in [
        ("token", move_token, ValueTransferPlanKind::MoveOnly),
        ("lease", lease, ValueTransferPlanKind::ExplicitCloneLease),
    ] {
        assert!(matches!(
            plan_with_registry(&facts, &registry, &record([(field, ty)]), &[]),
            Err(SourceValueTransferError::StructuralPositionNotSnapshotShare {
                found,
                ..
            }) if found == expected
        ));
    }
}

#[test]
fn registry_from_type_substitutes_concrete_and_defers_exact_binders() {
    let registry = custom_registry();
    let facts = SourceValueTransferFacts::new();
    let concrete = exact_package_type("Forward", "abi:forward:v1", vec![builtin("string")]);
    assert_eq!(
        plan_with_registry(&facts, &registry, &concrete, &[]),
        Ok(snapshot_release())
    );

    let parameter = TypeRefIr::TypeParam {
        name: "T".to_string(),
    };
    let generic = exact_package_type("Forward", "abi:forward:v1", vec![parameter.clone()]);
    assert_eq!(
        plan_with_registry(&facts, &registry, &generic, &["T".to_string()]),
        Ok(ValueTransferPlan::FromType { ty: generic })
    );
}

#[test]
fn privileged_snapshot_is_valid_at_root_but_not_in_ordinary_aggregate() {
    let registry = custom_registry();
    let facts = SourceValueTransferFacts::new();
    let privileged = exact_package_type("PrivilegedSnapshot", "abi:privileged:v1", vec![]);
    assert_eq!(
        plan_with_registry(&facts, &registry, &privileged, &[]),
        Ok(ValueTransferPlan::SnapshotShare {
            drop: ValueDropPlan::Trivial,
        })
    );
    assert_eq!(
        plan_with_registry(
            &facts,
            &registry,
            &record([("secret", privileged.clone())]),
            &[],
        ),
        Err(SourceValueTransferError::StructuralPositionNotOrdinary {
            position: SourceValueTransferPosition::AnonymousRecordField {
                field: "secret".to_string(),
            },
            found: NativeValueEmbedding::Privileged,
        })
    );
    for container in [
        generic_builtin("Array", vec![privileged.clone()]),
        generic_builtin("Map", vec![builtin("string"), privileged]),
    ] {
        assert!(matches!(
            plan_with_registry(&facts, &registry, &container, &[]),
            Err(SourceValueTransferError::StructuralPositionNotOrdinary {
                position: SourceValueTransferPosition::NativeArgument { .. },
                found: NativeValueEmbedding::Privileged,
            })
        ));
    }
}

#[test]
fn package_registry_identity_and_arity_are_exact() {
    let registry = custom_registry();
    let facts = SourceValueTransferFacts::new();
    let wrong_arity = exact_package_type("MoveToken", "abi:move:v1", vec![builtin("string")]);
    assert!(matches!(
        plan_with_registry(&facts, &registry, &wrong_arity, &[]),
        Err(SourceValueTransferError::NativeLifecycleLookup {
            source: NativeValueLifecycleLookupError::ArityMismatch { actual: 1, .. },
            ..
        })
    ));

    let dependency = TypeRefIr::PackageSymbol {
        symbol: skiff_artifact_model::PackageSymbolRef {
            package: skiff_artifact_model::PackageRefIr::Dependency {
                dependency_ref: "crypto".to_string(),
            },
            symbol_path: "MoveToken".to_string(),
            abi_expectation: Some("abi:move:v1".to_string()),
        },
    };
    assert_eq!(
        plan_with_registry(&facts, &registry, &dependency, &[]),
        Err(SourceValueTransferError::UnresolvedPackageDependency {
            dependency_ref: "crypto".to_string(),
        })
    );

    let missing_abi = TypeRefIr::PackageSymbol {
        symbol: skiff_artifact_model::PackageSymbolRef {
            package: skiff_artifact_model::PackageRefIr::PackageId {
                package_id: "pkg.lifecycle".to_string(),
            },
            symbol_path: "MoveToken".to_string(),
            abi_expectation: None,
        },
    };
    assert!(matches!(
        plan_with_registry(&facts, &registry, &missing_abi, &[]),
        Err(SourceValueTransferError::MissingPackageSymbolAbi { .. })
    ));
}
