use super::registry::builtin_entry;
use super::*;
use crate::{PackageRefIr, TypeRefIr};

fn builtin(name: &str, args: Vec<TypeRefIr>) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: name.to_string(),
        args,
    }
}

#[test]
fn built_in_identity_and_entries_are_deterministic() {
    let registry = native_value_lifecycle_registry();
    assert_eq!(
        registry.identity().registry_id,
        NATIVE_VALUE_LIFECYCLE_REGISTRY_ID
    );
    assert_eq!(
        registry.identity().version,
        NATIVE_VALUE_LIFECYCLE_REGISTRY_VERSION
    );
    assert_eq!(
        registry.identity().fingerprint,
        NATIVE_VALUE_LIFECYCLE_REGISTRY_FINGERPRINT
    );
    assert_eq!(registry.entries().len(), 6);

    let reversed = NativeValueLifecycleRegistry::new(
        NATIVE_VALUE_LIFECYCLE_REGISTRY_ID,
        NATIVE_VALUE_LIFECYCLE_REGISTRY_VERSION,
        registry.entries().iter().cloned().rev().collect(),
    )
    .unwrap();
    assert_eq!(registry.identity(), reversed.identity());
    assert_eq!(registry.entries(), reversed.entries());
    let wire = serde_json::to_value(registry.identity()).unwrap();
    assert_eq!(
        serde_json::from_value::<NativeValueLifecycleRegistryIdentity>(wire).unwrap(),
        registry.identity().clone()
    );
}

#[test]
fn initial_registry_contains_only_audited_scalars_and_stream() {
    for name in ["null", "bool", "number", "integer", "Date"] {
        assert_eq!(
            native_value_lifecycle_registry().lookup(&builtin(name, Vec::new())),
            Ok(NativeValueLifecycleResolution {
                lifecycle: NativeValueLifecycleConcrete::SnapshotShare {
                    drop: NativeValueDropPlan::Trivial
                },
                embedding: NativeValueEmbedding::Ordinary,
            })
        );
    }
    assert_eq!(
        native_value_lifecycle_registry()
            .lookup(&builtin("Stream", vec![builtin("number", Vec::new())])),
        Ok(NativeValueLifecycleResolution {
            lifecycle: NativeValueLifecycleConcrete::AffineResource {
                drop: NativeResourceDropPlan::ResourceTableRelease
            },
            embedding: NativeValueEmbedding::Forbidden,
        })
    );
    assert!(matches!(
        native_value_lifecycle_registry().lookup(&builtin("string", Vec::new())),
        Err(NativeValueLifecycleLookupError::Missing { .. })
    ));
}

#[test]
fn lookup_reports_arity_and_argument_policy_failures() {
    assert!(matches!(
        native_value_lifecycle_registry().lookup(&builtin("Stream", Vec::new())),
        Err(NativeValueLifecycleLookupError::ArityMismatch {
            expected,
            actual: 0,
            ..
        }) if expected == vec![1]
    ));
    assert!(matches!(
        native_value_lifecycle_registry()
            .lookup(&builtin("Stream", vec![builtin("string", Vec::new())])),
        Err(NativeValueLifecycleLookupError::Argument { index: 0, .. })
    ));
    assert!(matches!(
        native_value_lifecycle_registry().lookup(&builtin(
            "Stream",
            vec![builtin("Stream", vec![builtin("number", Vec::new())])]
        )),
        Err(NativeValueLifecycleLookupError::ArgumentPolicyMismatch {
            index: 0,
            actual: NativeValueLifecycleKind::AffineResource,
            ..
        })
    ));
}

#[test]
fn constructor_rejects_duplicate_arity_and_invalid_from_type() {
    let entry = builtin_entry(
        "Box",
        vec![NativeValueArgumentPolicy::RequireSnapshotShare],
        NativeValueLifecycleTemplate::FromType { argument_index: 0 },
        NativeValueEmbedding::Ordinary,
    );
    assert!(matches!(
        NativeValueLifecycleRegistry::new("id", "v1", vec![entry.clone(), entry]),
        Err(NativeValueLifecycleRegistryError::DuplicateConstructorArity { .. })
    ));
    let invalid = builtin_entry(
        "Token",
        vec![NativeValueArgumentPolicy::Phantom],
        NativeValueLifecycleTemplate::FromType { argument_index: 0 },
        NativeValueEmbedding::Privileged,
    );
    assert!(matches!(
        NativeValueLifecycleRegistry::new("id", "v1", vec![invalid]),
        Err(NativeValueLifecycleRegistryError::InvalidFromType { .. })
    ));

    let out_of_bounds = builtin_entry(
        "Token",
        Vec::new(),
        NativeValueLifecycleTemplate::FromType { argument_index: 0 },
        NativeValueEmbedding::Privileged,
    );
    assert!(matches!(
        NativeValueLifecycleRegistry::new("id", "v1", vec![out_of_bounds]),
        Err(NativeValueLifecycleRegistryError::InvalidFromType { .. })
    ));

    let too_many_arguments = builtin_entry(
        "Wide",
        vec![NativeValueArgumentPolicy::Phantom; MAX_NATIVE_VALUE_LIFECYCLE_ARGUMENTS + 1],
        NativeValueLifecycleTemplate::SnapshotShare {
            drop: NativeValueDropPlan::Trivial,
        },
        NativeValueEmbedding::Ordinary,
    );
    assert!(matches!(
        NativeValueLifecycleRegistry::new("id", "v1", vec![too_many_arguments]),
        Err(NativeValueLifecycleRegistryError::TooManyArguments { .. })
    ));
}

#[test]
fn constructor_requires_exact_adapter_role_and_abi() {
    let invalid = builtin_entry(
        "Lease",
        Vec::new(),
        NativeValueLifecycleTemplate::ExplicitCloneLease {
            clone_adapter: NativeValueLifecycleAdapter {
                binding_key: "lease.clone".to_string(),
                role: NativeValueAdapterRole::ResourceDrop,
                abi_version: 1,
            },
            drop: NativeResourceDropPlan::ResourceTableRelease,
        },
        NativeValueEmbedding::Forbidden,
    );
    assert!(matches!(
        NativeValueLifecycleRegistry::new("id", "v1", vec![invalid]),
        Err(NativeValueLifecycleRegistryError::InvalidAdapter { .. })
    ));
}

#[test]
fn package_constructor_and_phantom_argument_are_exact() {
    let constructor = NativeValueTypeConstructor::PackageSymbol {
        package_id: "pkg.crypto".to_string(),
        symbol_path: "Handle".to_string(),
        abi_identity: "abi:handle:v1".to_string(),
    };
    let registry = NativeValueLifecycleRegistry::new(
        "id",
        "v1",
        vec![NativeValueLifecycleEntry {
            pattern: NativeValueTypePattern {
                constructor,
                argument_policies: vec![NativeValueArgumentPolicy::Phantom],
            },
            lifecycle: NativeValueLifecycleTemplate::MoveOnly {
                drop: NativeValueDropPlan::Trivial,
            },
            embedding: NativeValueEmbedding::Privileged,
        }],
    )
    .unwrap();
    let ty = TypeRefIr::AppliedNominal {
        base: crate::NominalTypeRefBaseIr::PackageSymbol {
            symbol: crate::PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "pkg.crypto".to_string(),
                },
                symbol_path: "Handle".to_string(),
                abi_expectation: Some("abi:handle:v1".to_string()),
            },
        },
        arguments: vec![builtin("not-registered", Vec::new())],
    };
    assert_eq!(
        registry.lookup(&ty),
        Ok(NativeValueLifecycleResolution {
            lifecycle: NativeValueLifecycleConcrete::MoveOnly {
                drop: NativeValueDropPlan::Trivial,
            },
            embedding: NativeValueEmbedding::Privileged,
        })
    );

    let missing_abi = TypeRefIr::PackageSymbol {
        symbol: crate::PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: "pkg.crypto".to_string(),
            },
            symbol_path: "Handle".to_string(),
            abi_expectation: None,
        },
    };
    assert!(matches!(
        registry.lookup(&missing_abi),
        Err(NativeValueLifecycleLookupError::UnsupportedType { .. })
    ));
}

#[test]
fn adapter_keys_have_one_global_role_and_abi() {
    let adapter = NativeValueLifecycleAdapter {
        binding_key: "lease.clone".to_string(),
        role: NativeValueAdapterRole::CloneLease,
        abi_version: 1,
    };
    let first = builtin_entry(
        "LeaseA",
        Vec::new(),
        NativeValueLifecycleTemplate::ExplicitCloneLease {
            clone_adapter: adapter.clone(),
            drop: NativeResourceDropPlan::ResourceTableRelease,
        },
        NativeValueEmbedding::Forbidden,
    );
    let registry = NativeValueLifecycleRegistry::new("id", "v1", vec![first.clone()]).unwrap();
    assert_eq!(registry.adapter("lease.clone"), Some(&adapter));

    let second = builtin_entry(
        "LeaseB",
        Vec::new(),
        NativeValueLifecycleTemplate::ExplicitCloneLease {
            clone_adapter: NativeValueLifecycleAdapter {
                abi_version: 2,
                ..adapter
            },
            drop: NativeResourceDropPlan::ResourceTableRelease,
        },
        NativeValueEmbedding::Forbidden,
    );
    assert!(matches!(
        NativeValueLifecycleRegistry::new("id", "v1", vec![first, second]),
        Err(NativeValueLifecycleRegistryError::ConflictingAdapter { .. })
    ));
}

#[test]
fn adapter_binding_keys_reject_whitespace_and_controls() {
    for binding_key in ["lease clone", "lease\nclone", "lease\u{7f}clone"] {
        let entry = builtin_entry(
            "Lease",
            Vec::new(),
            NativeValueLifecycleTemplate::ExplicitCloneLease {
                clone_adapter: NativeValueLifecycleAdapter {
                    binding_key: binding_key.to_string(),
                    role: NativeValueAdapterRole::CloneLease,
                    abi_version: 1,
                },
                drop: NativeResourceDropPlan::ResourceTableRelease,
            },
            NativeValueEmbedding::Forbidden,
        );
        assert!(matches!(
            NativeValueLifecycleRegistry::new("id", "v1", vec![entry]),
            Err(NativeValueLifecycleRegistryError::InvalidAdapter { .. })
        ));
    }
}
