use std::collections::BTreeMap;

use skiff_artifact_model::{
    validate_file_ir_service_calls, ContractOperationId, PackageRefIr, PackageSymbolRef,
    ServiceProtocolIdentity, TypeDeclIr,
};

use super::*;

#[test]
fn rebuild_reinterns_service_call_tuples_and_rewrites_every_instruction_index() {
    let mut unit = FileIrUnit::empty("consumer.main", "source");
    let later = service_call_ref(1, "operation:zeta", "protocol:zeta");
    let earlier = service_call_ref(0, "operation:alpha", "protocol:alpha");
    unit.external_refs.service_call_refs = vec![later.clone(), earlier.clone()];
    unit.constants.push(skiff_artifact_model::ConstIr {
        name: "calls".to_string(),
        ty: TypeRefIr::builtin("void"),
        body: ExecutableBody {
            expressions: vec![service_call(0), service_call(1), service_call(0)],
            ..ExecutableBody::default()
        },
        source_span: None,
    });

    rebuild_external_refs_for_file_ir_unit(&mut unit).unwrap();

    assert_eq!(unit.external_refs.service_call_refs, vec![earlier, later]);
    assert_eq!(service_call_indices(&unit), vec![1, 0, 1]);
    assert!(unit.constants[0]
            .body
            .expressions
            .iter()
            .all(|expression| matches!(
                expression,
                ExprIr::Call {
                    call: CallIr {
                        site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                            reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                        },
                        ..
                    },
                }
            )));
    validate_file_ir_service_calls(&unit).unwrap();
}

#[test]
fn rebuild_rejects_invalid_service_call_tables_instead_of_dropping_them() {
    let mut unit = FileIrUnit::empty("consumer.main", "source");
    unit.external_refs.service_call_refs =
        vec![service_call_ref(0, "operation:orphan", "protocol:consumer")];
    assert!(matches!(
        rebuild_external_refs_for_file_ir_unit(&mut unit),
        Err(FileIrServiceCallValidationError::OrphanRef { .. })
    ));
}

#[test]
fn rebuild_collects_representation_wrap_target_and_child_external_refs() {
    let package_symbol = |symbol_path: &str| PackageSymbolRef {
        package: PackageRefIr::Dependency {
            dependency_ref: "model".to_string(),
        },
        symbol_path: symbol_path.to_string(),
        abi_expectation: Some("local-abi:model".to_string()),
    };
    let payload = package_symbol("payload");
    let first_argument = package_symbol("First");
    let second_argument = package_symbol("Second");
    let mut unit = FileIrUnit::empty("consumer.main", "source");
    unit.type_table.push(TypeDeclIr {
        name: "Generic".to_string(),
        descriptor: TypeDescriptorIr::Representation {
            representation: TypeRefIr::TypeParam {
                name: "T".to_string(),
            },
        },
        type_params: vec!["T".to_string(), "U".to_string()],
        implements: Vec::new(),
        source_span: None,
    });
    unit.constants.push(skiff_artifact_model::ConstIr {
        name: "wrapped".to_string(),
        ty: TypeRefIr::builtin("void"),
        body: ExecutableBody {
            expressions: vec![
                ExprIr::LoadPackageConst {
                    symbol: payload.clone(),
                },
                ExprIr::RepresentationWrap {
                    value: skiff_artifact_model::ExprRefIr { expression: 0 },
                    type_ref: TypeRefIr::AppliedNominal {
                        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
                        arguments: vec![
                            TypeRefIr::PackageSymbol {
                                symbol: first_argument.clone(),
                            },
                            TypeRefIr::PackageSymbol {
                                symbol: second_argument.clone(),
                            },
                        ],
                    },
                },
            ],
            ..ExecutableBody::default()
        },
        source_span: None,
    });

    rebuild_external_refs_for_file_ir_unit(&mut unit).unwrap();

    assert_eq!(
        unit.external_refs.package_symbols,
        vec![payload, first_argument, second_argument]
    );
}

fn service_call(index: u32) -> ExprIr {
    ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::ServiceCall {
                    service_call_ref_index: ServiceCallRefIndex::new(index),
                },
                concrete_receiver: None,
                site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                    reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                },
                args: Vec::new(),
                inout_args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: BTreeMap::new(),
            },
        }
}

fn service_call_ref(slot: u32, operation: &str, protocol: &str) -> ServiceCallRef {
    ServiceCallRef {
        service_requirement_slot: slot,
        contract_operation_id: ContractOperationId::new(operation),
        expected_protocol_identity: ServiceProtocolIdentity::new(protocol),
    }
}

fn service_call_indices(unit: &FileIrUnit) -> Vec<u32> {
    unit.constants[0]
        .body
        .expressions
        .iter()
        .map(|expression| {
            let ExprIr::Call { call } = expression else {
                panic!("service call expression")
            };
            let CallTargetIr::ServiceCall {
                service_call_ref_index,
            } = call.target
            else {
                panic!("canonical service call target")
            };
            service_call_ref_index.index()
        })
        .collect()
}
