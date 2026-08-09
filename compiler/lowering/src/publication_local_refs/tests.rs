use super::*;
use skiff_artifact_model::{
    CallIr, ContractOperationId, ExecutableBody, ServiceCallRef, ServiceCallRefIndex,
    ServiceProtocolIdentity,
};
use skiff_artifact_model::{LiteralIr, TypeDeclIr, TypeDeclarationIr};

fn current_package_duration_ref(package_id: &str) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package_id.to_string(),
            },
            symbol_path: "std.time.Duration".to_string(),
            abi_expectation: None,
        },
    }
}

#[test]
fn current_package_symbol_type_refs_become_publication_local() {
    let index = PublicationLocalRefIndex {
        current_package_id: Some("skiff.run/std".to_string()),
        package_dependency_abi_expectations: BTreeMap::new(),
        package_dependency_abi_expectations_by_package_id: BTreeMap::new(),
        types_by_module_symbol: BTreeMap::from([(
            ("std.time".to_string(), "Duration".to_string()),
            PublicationTypeRefLocation {
                module_path: "std.time".to_string(),
                type_index: 3,
            },
        )]),
        type_resolution: None,
        alias_expansion_error: RefCell::new(None),
    };

    let mut local = current_package_duration_ref("skiff.run/std");
    assert!(rewrite_type_ref(&index, "std.time", &mut local));
    assert_eq!(local, TypeRefIr::LocalType { type_index: 3 });

    let mut cross_module = current_package_duration_ref("skiff.run/std");
    assert!(rewrite_type_ref(
        &index,
        "std.time.__test",
        &mut cross_module
    ));
    assert_eq!(
        cross_module,
        TypeRefIr::PublicationType {
            module_path: "std.time".to_string(),
            type_index: 3,
        }
    );

    let mut dependency = current_package_duration_ref("example.com/time");
    assert!(!rewrite_type_ref(
        &index,
        "std.time.__test",
        &mut dependency
    ));
    assert!(matches!(dependency, TypeRefIr::PackageSymbol { .. }));
}

#[test]
fn package_id_interface_identity_receives_exact_dependency_abi() {
    let package_id = "example.com/interfaces";
    let index = PublicationLocalRefIndex {
        current_package_id: Some("example.com/consumer".to_string()),
        package_dependency_abi_expectations: BTreeMap::new(),
        package_dependency_abi_expectations_by_package_id: BTreeMap::from([(
            package_id.to_string(),
            "local-abi:interfaces".to_string(),
        )]),
        types_by_module_symbol: BTreeMap::new(),
        type_resolution: None,
        alias_expansion_error: RefCell::new(None),
    };
    let interface_identity = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package_id.to_string(),
            },
            symbol_path: "tools.ToolProvider".to_string(),
            abi_expectation: None,
        },
    };
    let mut ty = TypeRefIr::AnyInterface {
        interface: InterfaceInstantiationRef {
            interface_abi_id: serde_json::to_string(&interface_identity).unwrap(),
            canonical_type_args: Vec::new(),
        },
    };

    assert!(rewrite_type_ref(&index, "consumer.main", &mut ty));
    let TypeRefIr::AnyInterface { interface } = ty else {
        panic!("any interface")
    };
    let TypeRefIr::PackageSymbol { symbol } =
        serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).unwrap()
    else {
        panic!("package interface identity")
    };
    assert_eq!(
        symbol.abi_expectation.as_deref(),
        Some("local-abi:interfaces")
    );
}

#[test]
fn local_interface_identity_uses_one_publication_coordinate_in_every_file() {
    let index = PublicationLocalRefIndex {
        current_package_id: Some("example.com/consumer".to_string()),
        package_dependency_abi_expectations: BTreeMap::new(),
        package_dependency_abi_expectations_by_package_id: BTreeMap::new(),
        types_by_module_symbol: BTreeMap::new(),
        type_resolution: None,
        alias_expansion_error: RefCell::new(None),
    };
    let mut owner = InterfaceInstantiationRef {
        interface_abi_id: type_ref_abi_key(&TypeRefIr::LocalType { type_index: 17 }),
        canonical_type_args: Vec::new(),
    };
    let mut sibling = InterfaceInstantiationRef {
        interface_abi_id: type_ref_abi_key(&TypeRefIr::PublicationType {
            module_path: "internal.drain".to_string(),
            type_index: 17,
        }),
        canonical_type_args: Vec::new(),
    };

    assert!(rewrite_interface_instantiation_ref(
        &index,
        "internal.drain",
        &mut owner
    ));
    assert!(!rewrite_interface_instantiation_ref(
        &index,
        "internal.worker",
        &mut sibling
    ));
    assert_eq!(owner.interface_abi_id, sibling.interface_abi_id);
    assert_eq!(
        serde_json::from_str::<TypeRefIr>(&owner.interface_abi_id).unwrap(),
        TypeRefIr::PublicationType {
            module_path: "internal.drain".to_string(),
            type_index: 17,
        }
    );
}

#[test]
fn service_call_target_and_ref_survive_publication_local_rewrite() {
    let call_ref = ServiceCallRef {
        service_requirement_slot: 2,
        contract_operation_id: ContractOperationId::new("operation:echo"),
        expected_protocol_identity: ServiceProtocolIdentity::new("protocol:echo"),
    };
    let mut unit = FileIrUnit::empty("consumer.main", "source");
    unit.external_refs.service_call_refs = vec![call_ref.clone()];
    unit.constants.push(skiff_artifact_model::ConstIr {
            name: "call".to_string(),
            ty: TypeRefIr::builtin("void"),
            body: ExecutableBody {
                expressions: vec![ExprIr::Call {
                    call: CallIr {
                        target: CallTargetIr::ServiceCall {
                            service_call_ref_index: ServiceCallRefIndex::new(0),
                        },
                        site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                            reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                        },
                        args: Vec::new(),
                        inout_args: Vec::new(),
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                }],
                ..ExecutableBody::default()
            },
            source_span: None,
        });

    rewrite_publication_local_refs(
        std::slice::from_mut(&mut unit),
        None,
        None,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();

    assert_eq!(unit.external_refs.service_call_refs, vec![call_ref]);
    let ExprIr::Call { call } = &unit.constants[0].body.expressions[0] else {
        panic!("service call expression")
    };
    assert!(matches!(
        call.target,
        CallTargetIr::ServiceCall {
            service_call_ref_index
        } if service_call_ref_index.index() == 0
    ));
    assert!(matches!(
        call.site,
        skiff_artifact_model::InstructionSourceSite::Synthetic {
            reason:
                skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
        }
    ));
}

#[test]
fn representation_wrap_targets_and_nested_child_become_publication_local() {
    let package_id = "example.com/model";
    let package_symbol = |symbol_path: &str| TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package_id.to_string(),
            },
            symbol_path: symbol_path.to_string(),
            abi_expectation: None,
        },
    };
    let mut model = FileIrUnit::empty("model.types", "source");
    model.type_table.push(TypeDeclIr {
        name: "Payload".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    model.declarations.types.insert(
        "Payload".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "Payload".to_string(),
            source_span: None,
        },
    );

    let mut consumer = FileIrUnit::empty("consumer.main", "source");
    consumer.type_table = vec![
        TypeDeclIr {
            name: "Inner".to_string(),
            descriptor: TypeDescriptorIr::Representation {
                representation: TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
            },
            type_params: vec!["T".to_string()],
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: "Outer".to_string(),
            descriptor: TypeDescriptorIr::Representation {
                representation: TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
            },
            type_params: vec!["T".to_string()],
            implements: Vec::new(),
            source_span: None,
        },
    ];
    consumer.constants.push(skiff_artifact_model::ConstIr {
        name: "nested".to_string(),
        ty: TypeRefIr::builtin("void"),
        body: ExecutableBody {
            expressions: vec![
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "payload".to_string(),
                    },
                },
                ExprIr::RepresentationWrap {
                    value: skiff_artifact_model::ExprRefIr { expression: 0 },
                    type_ref: TypeRefIr::AppliedNominal {
                        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
                        arguments: vec![package_symbol("model.types.Payload")],
                    },
                },
                ExprIr::RepresentationWrap {
                    value: skiff_artifact_model::ExprRefIr { expression: 1 },
                    type_ref: TypeRefIr::AppliedNominal {
                        base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
                        arguments: vec![package_symbol("model.types.Payload")],
                    },
                },
            ],
            ..ExecutableBody::default()
        },
        source_span: None,
    });
    let mut units = vec![model, consumer];

    rewrite_publication_local_refs(
        &mut units,
        Some(package_id),
        None,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();

    let expressions = &units[1].constants[0].body.expressions;
    assert!(matches!(
        &expressions[1],
        ExprIr::RepresentationWrap {
            value,
            type_ref:
                TypeRefIr::AppliedNominal {
                    base:
                        NominalTypeRefBaseIr::LocalType { type_index: 0 },
                    arguments,
                },
        } if value.expression == 0
            && arguments == &vec![TypeRefIr::PublicationType {
                module_path: "model.types".to_string(),
                type_index: 0,
            }]
    ));
    assert!(matches!(
        &expressions[2],
        ExprIr::RepresentationWrap {
            value,
            type_ref:
                TypeRefIr::AppliedNominal {
                    base:
                        NominalTypeRefBaseIr::LocalType { type_index: 1 },
                    arguments,
                },
        } if value.expression == 1
            && arguments == &vec![TypeRefIr::PublicationType {
                module_path: "model.types".to_string(),
                type_index: 0,
            }]
    ));
    assert!(units[1].external_refs.package_symbols.is_empty());
}
