use super::*;
use skiff_artifact_model::LiteralIr;

#[test]
fn canonical_named_union_branches_keep_context_and_applied_arguments() {
    let descriptor = TypeDescriptorIr::Union {
        branches: vec![
            NamedUnionBranchIr::ConcreteNominal {
                nominal_type: TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
                    arguments: vec![TypeRefIr::TypeParam {
                        name: "T".to_string(),
                    }],
                },
            },
            NamedUnionBranchIr::SyntheticDiscriminator {
                payload_type: TypeRefIr::Record {
                    fields: BTreeMap::from([(
                        "kind".to_string(),
                        TypeRefIr::Literal {
                            value: LiteralIr::String {
                                value: "retry".to_string(),
                            },
                        },
                    )]),
                },
                discriminator_field: "kind".to_string(),
                discriminator_value: "retry".to_string(),
            },
            NamedUnionBranchIr::Literal {
                value: LiteralIr::String {
                    value: "cancelled".to_string(),
                },
            },
        ],
    };
    let names = BTreeMap::from([(("api".to_string(), 0), "Ok".to_string())]);

    let projected = projection_visible_type_descriptor("api", &descriptor, &names);

    let TypeDescriptorIr::Union { branches } = projected else {
        panic!("named union must remain a named union");
    };
    assert_eq!(branches.len(), 3);
    assert_eq!(
        branches[0],
        NamedUnionBranchIr::ConcreteNominal {
            nominal_type: TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "api".to_string(),
                        symbol: "Ok".to_string(),
                    },
                },
                arguments: vec![TypeRefIr::TypeParam {
                    name: "T".to_string(),
                }],
            },
        }
    );
    assert!(matches!(
        branches[1],
        NamedUnionBranchIr::SyntheticDiscriminator { .. }
    ));
    assert!(matches!(branches[2], NamedUnionBranchIr::Literal { .. }));
}

#[test]
fn record_representation_alias_and_interface_kinds_are_not_flattened() {
    let names = BTreeMap::new();
    for descriptor in [
        TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "value".to_string(),
                TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
            )]),
        },
        TypeDescriptorIr::Representation {
            representation: TypeRefIr::TypeParam {
                name: "T".to_string(),
            },
        },
        TypeDescriptorIr::Alias {
            target: TypeRefIr::builtin("string"),
        },
        TypeDescriptorIr::Interface,
    ] {
        let projected = projection_visible_type_descriptor("api", &descriptor, &names);
        assert_eq!(
            std::mem::discriminant(&projected),
            std::mem::discriminant(&descriptor)
        );
    }
}
