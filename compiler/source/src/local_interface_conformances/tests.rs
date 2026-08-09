use skiff_artifact_identity::interface_instantiation_ref;
use skiff_artifact_model::{PackageRefIr, PackageSymbolRef, ServiceSymbolRef};

use super::*;

fn receiver() -> SourceSymbolKey {
    SourceSymbolKey::new("models", "Box")
}

fn receiver_type() -> TypeRefIr {
    TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: "models".to_string(),
            symbol: "Box".to_string(),
        },
    }
}

fn source_interface(arguments: Vec<TypeRefIr>) -> InterfaceInstantiationRef {
    interface_instantiation_ref(
        TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "interfaces".to_string(),
                symbol: "Ordered".to_string(),
            },
        },
        arguments,
    )
}

#[test]
fn read_only_facts_preserve_declared_slot_order_and_marker_rows() {
    let ordered = SourceLocalInterfaceConformance::try_new(
        vec!["T".to_string()],
        receiver(),
        receiver_type(),
        source_interface(vec![TypeRefIr::TypeParam {
            name: "T".to_string(),
        }]),
        vec![
            SourceSymbolKey::new("models", "Box<T>.zeta"),
            SourceSymbolKey::new("models", "Box<T>.alpha"),
        ],
    )
    .unwrap();
    let marker = SourceLocalInterfaceConformance::try_new(
        Vec::new(),
        SourceSymbolKey::new("models", "MarkerValue"),
        TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "models".to_string(),
                symbol: "MarkerValue".to_string(),
            },
        },
        interface_instantiation_ref(
            TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "interfaces".to_string(),
                    symbol: "Marker".to_string(),
                },
            },
            Vec::new(),
        ),
        Vec::new(),
    )
    .unwrap();

    let mut expected = vec![marker.clone(), ordered.clone()];
    expected.sort_by_key(|row| canonical_sort_key(row).unwrap());
    let facts =
        SourceLocalInterfaceConformanceFacts::try_from_entries([marker.clone(), ordered.clone()])
            .unwrap();
    assert_eq!(facts.conformances(), expected.as_slice());
    let ordered = facts
        .iter()
        .find(|row| row.receiver() == &receiver())
        .expect("ordered conformance is present");

    assert_eq!(ordered.receiver_type_parameters(), &["T".to_string()]);
    assert_eq!(ordered.receiver_type(), &receiver_type());
    assert_eq!(
        ordered.implementation_methods(),
        &[
            SourceSymbolKey::new("models", "Box<T>.zeta"),
            SourceSymbolKey::new("models", "Box<T>.alpha"),
        ]
    );
    assert!(facts
        .iter()
        .find(|row| row.receiver().symbol() == "MarkerValue")
        .expect("marker conformance is present")
        .implementation_methods()
        .is_empty());
    assert_eq!((&facts).into_iter().count(), 2);
}

#[test]
fn dependency_alias_and_residual_type_parameter_fail_closed() {
    let alias_error = SourceLocalInterfaceConformance::try_new(
        Vec::new(),
        receiver(),
        receiver_type(),
        interface_instantiation_ref(
            TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::Dependency {
                        dependency_ref: "dep".to_string(),
                    },
                    symbol_path: "api.Protocol".to_string(),
                    abi_expectation: Some("abi".to_string()),
                },
            },
            Vec::new(),
        ),
        Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(
        alias_error,
        SourceLocalInterfaceConformanceError::DependencyAliasIdentity { .. }
    ));

    let type_parameter_error = SourceLocalInterfaceConformance::try_new(
        vec!["T".to_string()],
        receiver(),
        receiver_type(),
        source_interface(vec![TypeRefIr::TypeParam {
            name: "U".to_string(),
        }]),
        Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(
        type_parameter_error,
        SourceLocalInterfaceConformanceError::ResidualTypeParameter { ref name, .. }
            if name == "U"
    ));
}

#[test]
fn duplicate_canonical_conformance_key_is_rejected() {
    let row = SourceLocalInterfaceConformance::try_new(
        Vec::new(),
        receiver(),
        receiver_type(),
        source_interface(Vec::new()),
        Vec::new(),
    )
    .unwrap();

    let error =
        SourceLocalInterfaceConformanceFacts::try_from_entries([row.clone(), row]).unwrap_err();
    assert!(matches!(
        error,
        SourceLocalInterfaceConformanceFactsError::DuplicateConformance { .. }
    ));
}
