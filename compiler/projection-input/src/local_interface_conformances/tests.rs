use skiff_artifact_model::{
    InterfaceInstantiationRef, NominalTypeRefBaseIr, PackageRefIr, PackageSymbolRef,
    ServiceSymbolRef, TypeRefIr,
};
use skiff_compiler_core::json_utils::canonical_json_bytes;

use super::*;

fn exact_interface(
    identity: TypeRefIr,
    canonical_type_args: Vec<TypeRefIr>,
) -> InterfaceInstantiationRef {
    InterfaceInstantiationRef {
        interface_abi_id: String::from_utf8(canonical_json_bytes(&identity).unwrap()).unwrap(),
        canonical_type_args,
    }
}

fn dependency_interface(symbol_path: &str) -> InterfaceInstantiationRef {
    exact_interface(
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "example.com/interfaces".to_string(),
                },
                symbol_path: symbol_path.to_string(),
                abi_expectation: Some("package-local-abi:interfaces".to_string()),
            },
        },
        vec![TypeRefIr::TypeParam {
            name: "T".to_string(),
        }],
    )
}

fn conformance(
    interface: InterfaceInstantiationRef,
    implementation_executables: Vec<ProjectionExecutableKey>,
) -> crate::ProjectionLocalInterfaceConformance {
    crate::ProjectionLocalInterfaceConformance::try_new(
        vec!["T".to_string()],
        ProjectionSourceSymbolKey::new("root.models", "Box"),
        interface,
        implementation_executables,
    )
    .unwrap()
}

#[test]
fn preserves_exact_dependency_identity_and_slot_order() {
    let slots = vec![
        ProjectionExecutableKey::new("root.models", 9),
        ProjectionExecutableKey::new("root.models", 2),
    ];
    let interface = dependency_interface("api.Mapper");
    let row = conformance(interface.clone(), slots.clone());

    assert_eq!(row.type_parameters(), &["T".to_string()]);
    assert_eq!(row.receiver().module_path(), "root.models");
    assert_eq!(row.receiver().symbol(), "Box");
    assert_eq!(row.interface(), &interface);
    assert_eq!(row.implementation_executables(), slots.as_slice());
}

#[test]
fn facts_canonically_sort_rows_without_reordering_slots() {
    let first = conformance(
        dependency_interface("api.Alpha"),
        vec![
            ProjectionExecutableKey::new("root.models", 7),
            ProjectionExecutableKey::new("root.models", 1),
        ],
    );
    let second = conformance(
        dependency_interface("api.Zeta"),
        vec![
            ProjectionExecutableKey::new("root.models", 4),
            ProjectionExecutableKey::new("root.models", 3),
        ],
    );
    let facts =
        ProjectionLocalInterfaceConformanceFacts::try_from_entries([second.clone(), first.clone()])
            .unwrap();
    let mut expected = vec![first, second];
    expected.sort_by_key(|row| canonical_sort_key(row).unwrap());

    assert_eq!(facts.conformances(), expected.as_slice());
    assert_eq!(
        facts.conformances()[0]
            .implementation_executables()
            .iter()
            .map(ProjectionExecutableKey::executable_index)
            .collect::<Vec<_>>(),
        if facts.conformances()[0].interface().interface_abi_id
            == dependency_interface("api.Alpha").interface_abi_id
        {
            vec![7, 1]
        } else {
            vec![4, 3]
        }
    );
}

#[test]
fn duplicate_exact_key_fails_even_when_targets_differ() {
    let interface = dependency_interface("api.Mapper");
    let error = ProjectionLocalInterfaceConformanceFacts::try_from_entries([
        conformance(
            interface.clone(),
            vec![ProjectionExecutableKey::new("root.models", 1)],
        ),
        conformance(
            interface,
            vec![ProjectionExecutableKey::new("root.models", 2)],
        ),
    ])
    .unwrap_err();

    assert!(matches!(
        error,
        ProjectionLocalInterfaceConformanceFactsError::DuplicateConformance { .. }
    ));
}

#[test]
fn marker_interface_allows_an_empty_slot_vector() {
    let marker = exact_interface(
        TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "root.markers".to_string(),
                symbol: "Serializable".to_string(),
            },
        },
        Vec::new(),
    );
    let row = ProjectionLocalInterfaceConformance::try_new(
        Vec::new(),
        ProjectionSourceSymbolKey::new("root.models", "User"),
        marker,
        Vec::new(),
    )
    .unwrap();
    let facts = ProjectionLocalInterfaceConformanceFacts::try_from_entries([row]).unwrap();

    assert_eq!(facts.len(), 1);
    assert!(facts.conformances()[0]
        .implementation_executables()
        .is_empty());
}

#[test]
fn malformed_and_noncanonical_interface_identities_fail_closed() {
    let malformed = ProjectionLocalInterfaceConformance::try_new(
        Vec::new(),
        ProjectionSourceSymbolKey::new("root.models", "User"),
        InterfaceInstantiationRef {
            interface_abi_id: "not-json".to_string(),
            canonical_type_args: Vec::new(),
        },
        Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(
        malformed,
        ProjectionLocalInterfaceConformanceError::InvalidInterfaceIdentity { .. }
    ));

    let identity = TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: "root.api".to_string(),
            symbol: "Display".to_string(),
        },
    };
    let canonical = String::from_utf8(canonical_json_bytes(&identity).unwrap()).unwrap();
    let noncanonical = ProjectionLocalInterfaceConformance::try_new(
        Vec::new(),
        ProjectionSourceSymbolKey::new("root.models", "User"),
        InterfaceInstantiationRef {
            interface_abi_id: format!(" {canonical}"),
            canonical_type_args: Vec::new(),
        },
        Vec::new(),
    )
    .unwrap_err();
    assert_eq!(
        noncanonical,
        ProjectionLocalInterfaceConformanceError::NonCanonicalInterfaceIdentity
    );
}

#[test]
fn unstable_local_identity_is_rejected_but_publication_identity_is_allowed() {
    let unstable = ProjectionLocalInterfaceConformance::try_new(
        Vec::new(),
        ProjectionSourceSymbolKey::new("root.models", "User"),
        exact_interface(TypeRefIr::LocalType { type_index: 0 }, Vec::new()),
        Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(
        unstable,
        ProjectionLocalInterfaceConformanceError::UnstableLocalInterfaceIdentity { .. }
    ));

    let unstable_base = ProjectionLocalInterfaceConformance::try_new(
        Vec::new(),
        ProjectionSourceSymbolKey::new("root.models", "User"),
        exact_interface(
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
                arguments: vec![TypeRefIr::builtin("string")],
            },
            Vec::new(),
        ),
        Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(
        unstable_base,
        ProjectionLocalInterfaceConformanceError::UnstableLocalInterfaceIdentity { .. }
    ));

    let unstable_argument = ProjectionLocalInterfaceConformance::try_new(
        Vec::new(),
        ProjectionSourceSymbolKey::new("root.models", "User"),
        exact_interface(
            TypeRefIr::PublicationType {
                module_path: "root.api".to_string(),
                type_index: 3,
            },
            vec![TypeRefIr::LocalType { type_index: 1 }],
        ),
        Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(
        unstable_argument,
        ProjectionLocalInterfaceConformanceError::UnstableLocalInterfaceIdentity { .. }
    ));

    ProjectionLocalInterfaceConformance::try_new(
        Vec::new(),
        ProjectionSourceSymbolKey::new("root.models", "User"),
        exact_interface(
            TypeRefIr::PublicationType {
                module_path: "root.api".to_string(),
                type_index: 3,
            },
            Vec::new(),
        ),
        Vec::new(),
    )
    .expect("publication-scoped identity is owner-stable");
}

#[test]
fn package_symbols_require_package_id_and_nonempty_abi_expectation() {
    let dependency_alias = exact_interface(
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "interfaces".to_string(),
                },
                symbol_path: "api.Mapper".to_string(),
                abi_expectation: Some("abi".to_string()),
            },
        },
        Vec::new(),
    );
    let error = ProjectionLocalInterfaceConformance::try_new(
        Vec::new(),
        ProjectionSourceSymbolKey::new("root.models", "User"),
        dependency_alias,
        Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ProjectionLocalInterfaceConformanceError::DependencyAliasInterfaceIdentity { .. }
    ));

    let missing_expectation = exact_interface(
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "example.com/interfaces".to_string(),
                },
                symbol_path: "api.Mapper".to_string(),
                abi_expectation: None,
            },
        },
        Vec::new(),
    );
    let error = ProjectionLocalInterfaceConformance::try_new(
        Vec::new(),
        ProjectionSourceSymbolKey::new("root.models", "User"),
        missing_expectation,
        Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ProjectionLocalInterfaceConformanceError::MissingPackageAbiExpectation { .. }
    ));
}

#[test]
fn projection_source_facts_exposes_checked_conformances() {
    let row = conformance(dependency_interface("api.Mapper"), Vec::new());
    let conformances = ProjectionLocalInterfaceConformanceFacts::try_from_entries([row]).unwrap();
    let source = crate::ProjectionSourceFacts::new(crate::ProjectionSourceFactsParts {
        publication_api_seed: Default::default(),
        export_bindings: Default::default(),
        config_requirements: Default::default(),
        abi_ids: Default::default(),
        callable_effects: Default::default(),
        callable_semantic_facts: Default::default(),
        local_interface_conformances: conformances.clone(),
    });

    assert_eq!(source.local_interface_conformances(), &conformances);
}
