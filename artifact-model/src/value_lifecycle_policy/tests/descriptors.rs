use super::*;
use crate::{NativeValueDropPlan, NativeValueLifecycleConcrete};

#[test]
fn exact_instantiations_are_memoized_and_alias_cycles_are_structured() {
    let symbol = crate::PackageSymbolRef {
        package: crate::PackageRefIr::PackageId {
            package_id: "skiff.run/example".to_string(),
        },
        symbol_path: "example.Value".to_string(),
        abi_expectation: Some("abi-v1".to_string()),
    };
    let ty = TypeRefIr::PackageSymbol {
        symbol: symbol.clone(),
    };
    let mut resolver = PackageResolver {
        calls: 0,
        descriptor: PackageDescriptor::AliasString,
    };
    let aggregate = TypeRefIr::Record {
        fields: [
            ("left".to_string(), ty.clone()),
            ("right".to_string(), ty.clone()),
        ]
        .into_iter()
        .collect(),
    };
    classify_value_lifecycle(
        &aggregate,
        &PositionalTypeEnvironment::empty(),
        &mut resolver,
        &mut budget(),
    )
    .unwrap();
    assert_eq!(resolver.calls, 1);

    let mut cyclic = PackageResolver {
        calls: 0,
        descriptor: PackageDescriptor::Cycle,
    };
    assert!(matches!(
        classify_value_lifecycle(
            &ty,
            &PositionalTypeEnvironment::empty(),
            &mut cyclic,
            &mut budget()
        ),
        Err(ValueLifecyclePolicyError::DescriptorCycle { .. })
    ));
}

#[test]
fn representation_cannot_hide_a_resource_child() {
    let symbol = crate::PackageSymbolRef {
        package: crate::PackageRefIr::PackageId {
            package_id: "skiff.run/example".to_string(),
        },
        symbol_path: "example.StreamWrapper".to_string(),
        abi_expectation: Some("abi-v1".to_string()),
    };
    let mut resolver = PackageResolver {
        calls: 0,
        descriptor: PackageDescriptor::RepresentationStream,
    };
    assert!(matches!(
        classify_value_lifecycle(
            &TypeRefIr::PackageSymbol { symbol },
            &PositionalTypeEnvironment::empty(),
            &mut resolver,
            &mut budget()
        ),
        Err(ValueLifecyclePolicyError::ArgumentPolicy { .. })
    ));
}

#[test]
fn schema_cycles_callback_interfaces_and_enumerations_are_explicit() {
    let schema_type = |key: &str| TypeRefIr::PackageSchema {
        package_id: "skiff.run/schema".to_string(),
        stable_schema_key: key.to_string(),
        package_schema_type_id: PackageSchemaTypeId::new(format!("{key}-id")),
    };
    assert!(matches!(
        classify_value_lifecycle(
            &schema_type("cycle"),
            &PositionalTypeEnvironment::empty(),
            &mut SchemaResolver {
                mode: SchemaMode::Cycle
            },
            &mut budget()
        ),
        Err(ValueLifecyclePolicyError::DescriptorCycle { .. })
    ));
    assert!(matches!(
        classify_value_lifecycle(
            &schema_type("callback"),
            &PositionalTypeEnvironment::empty(),
            &mut SchemaResolver {
                mode: SchemaMode::Callback
            },
            &mut budget()
        ),
        Err(ValueLifecyclePolicyError::UnsupportedType {
            kind: "bareCallbackInterfaceDescriptor"
        })
    ));

    let existential = classify_value_lifecycle(
        &schema_type("existential"),
        &PositionalTypeEnvironment::empty(),
        &mut SchemaResolver {
            mode: SchemaMode::AnyInterface { argument_count: 1 },
        },
        &mut budget(),
    )
    .unwrap();
    assert_eq!(
        existential.lifecycle,
        NativeValueLifecycleConcrete::SnapshotShare {
            drop: NativeValueDropPlan::SnapshotRelease
        }
    );
    assert!(matches!(
        classify_value_lifecycle(
            &schema_type("wrong-arity"),
            &PositionalTypeEnvironment::empty(),
            &mut SchemaResolver {
                mode: SchemaMode::AnyInterface { argument_count: 0 }
            },
            &mut budget()
        ),
        Err(ValueLifecyclePolicyError::Authority { .. })
    ));

    let enumeration = classify_value_lifecycle(
        &schema_type("enum"),
        &PositionalTypeEnvironment::empty(),
        &mut SchemaResolver {
            mode: SchemaMode::Enumeration,
        },
        &mut budget(),
    )
    .unwrap();
    assert_eq!(
        enumeration.lifecycle,
        NativeValueLifecycleConcrete::SnapshotShare {
            drop: NativeValueDropPlan::SnapshotRelease
        }
    );
}
