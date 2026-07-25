use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryErrorContract, BoundaryFeatureUnavailableReason, ContractLiteral,
    ContractTypeDescriptor, ContractTypeRef, PackageLocalAbiIdentity, PackageRefIr,
    PackageSchemaCanonicalDescriptor, PackageSchemaTypeRecord, PackageTypeRef, TypeRefIr,
};

use super::operation_shape_diagnostics;
use super::type_projection::{
    local_ir_json_compatible, package_type_assignable, package_type_target_assignable,
    resolved_contract_type,
};
use crate::{PackageDependencyAnalysisFacts, SourceDependencyAnalysisInput};

#[test]
fn typed_error_contract_is_supported_by_source_calls() {
    let (mut contract, _) = crate::contract_dependency_test_fixture::contract_and_schema(
        "example.echo",
        "1.0.0",
        "echo",
        "Failure",
        "Response",
    );
    let operation = contract.operations.values_mut().next().unwrap();
    operation.contract.errors = BoundaryErrorContract::Typed {
        payload_type: operation.contract.parameters[0].ty.clone(),
        value_plan: operation.contract.parameters[0].value_plan.clone(),
    };

    assert!(
        operation_shape_diagnostics("echo/echo", &operation.contract).is_empty(),
        "a declared typed error is source-callable and can be handled by catch"
    );

    operation.contract.errors = BoundaryErrorContract::Unsupported {
        reason: BoundaryFeatureUnavailableReason::LanguageUnsupported,
    };
    assert!(
        operation_shape_diagnostics("echo/echo", &operation.contract)
            .iter()
            .any(|diagnostic| diagnostic.contains("error contract unsupported by source calls")),
        "an explicitly unsupported error contract must remain rejected"
    );
}

#[test]
fn package_schema_assignability_requires_exact_owner_key_and_identity() {
    let exact = package_type("example.types", "Payload", "type:payload");
    assert!(package_type_assignable(&exact, &exact));
    assert!(!package_type_assignable(
        &exact,
        &package_type("other.types", "Payload", "type:payload")
    ));
    assert!(!package_type_assignable(
        &exact,
        &package_type("example.types", "Renamed", "type:payload")
    ));
    assert!(!package_type_assignable(
        &exact,
        &package_type("example.types", "Payload", "type:other")
    ));
}

#[test]
fn contract_ref_materializes_as_owner_package_symbol_not_service_symbol() {
    let resolved = resolved_contract_type(
        &ContractTypeRef::package_schema("example.types", "Payload", "type:payload".into()),
        "renamedService",
    )
    .unwrap();
    assert_eq!(resolved.source_text, "renamedService.Payload");
    assert!(matches!(
        resolved.ir,
        TypeRefIr::PackageSymbol { symbol }
            if symbol.package == PackageRefIr::PackageId {
                package_id: "example.types".to_string()
            } && symbol.symbol_path == "Payload"
    ));
}

#[test]
fn stream_and_container_refs_preserve_package_schema_identity() {
    let nested = PackageTypeRef::Container {
        name: "Stream".to_string(),
        arguments: vec![package_type("example.types", "Event", "type:event")],
    };
    assert!(package_type_assignable(&nested, &nested));
}

#[test]
fn package_nominal_target_typing_accepts_only_its_exact_representation() {
    let expected = package_type("example.types", "Role", "type:role");
    let dependency_analysis = dependency_analysis_with_alias(
        "example.types",
        "Role",
        "type:role",
        ContractTypeRef::StructuralUnion {
            variants: vec![
                ContractTypeRef::Literal {
                    value: ContractLiteral::String {
                        value: "user".to_string(),
                    },
                },
                ContractTypeRef::Literal {
                    value: ContractLiteral::String {
                        value: "assistant".to_string(),
                    },
                },
            ],
        },
    );
    let user = literal("user");
    assert!(package_type_target_assignable(
        &user,
        &expected,
        &dependency_analysis
    ));
    assert!(!package_type_target_assignable(
        &literal("system"),
        &expected,
        &dependency_analysis
    ));
    assert!(!package_type_target_assignable(
        &package_type("other.types", "Role", "type:role"),
        &expected,
        &dependency_analysis
    ));
}

#[test]
fn package_nominal_target_typing_projects_local_cross_package_symbols() {
    let exact = package_type("example.types", "Role", "type:role");
    let dependency_analysis = dependency_analysis_with_alias(
        "example.types",
        "Role",
        "type:role",
        ContractTypeRef::StructuralUnion {
            variants: vec![ContractTypeRef::Literal {
                value: ContractLiteral::String {
                    value: "user".to_string(),
                },
            }],
        },
    );
    let local_symbol = PackageTypeRef::Local {
        local_type: TypeRefIr::PackageSymbol {
            symbol: skiff_artifact_model::PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "example.types".to_string(),
                },
                symbol_path: "Role".to_string(),
                abi_expectation: None,
            },
        },
    };
    assert!(package_type_target_assignable(
        &exact,
        &local_symbol,
        &dependency_analysis
    ));
    assert!(package_type_target_assignable(
        &PackageTypeRef::Container {
            name: "Array".to_string(),
            arguments: vec![exact],
        },
        &PackageTypeRef::Local {
            local_type: TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![match local_symbol {
                    PackageTypeRef::Local { local_type } => local_type,
                    _ => unreachable!(),
                }],
            },
        },
        &dependency_analysis,
    ));
}

#[test]
fn package_nominal_target_typing_recurses_through_records_and_iterables() {
    let expected = package_type("example.types", "Payload", "type:payload");
    let dependency_analysis = dependency_analysis_with_alias(
        "example.types",
        "Payload",
        "type:payload",
        ContractTypeRef::Record {
            fields: BTreeMap::from([(
                "items".to_string(),
                ContractTypeRef::Builtin {
                    name: "Array".to_string(),
                    arguments: vec![ContractTypeRef::Builtin {
                        name: "string".to_string(),
                        arguments: Vec::new(),
                    }],
                },
            )]),
        },
    );
    let actual = PackageTypeRef::Local {
        local_type: TypeRefIr::Record {
            fields: BTreeMap::from([(
                "items".to_string(),
                TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![TypeRefIr::Builtin {
                        name: "string".to_string(),
                        args: Vec::new(),
                    }],
                },
            )]),
        },
    };
    assert!(package_type_target_assignable(
        &actual,
        &expected,
        &dependency_analysis
    ));
}

#[test]
fn json_target_accepts_only_json_compatible_package_nominal_representations() {
    let json = PackageTypeRef::Container {
        name: "Json".to_string(),
        arguments: Vec::new(),
    };
    for (stable_key, type_id, target) in [
        ("Scalar", "type:scalar", ContractTypeRef::builtin("string")),
        (
            "Union",
            "type:union",
            ContractTypeRef::StructuralUnion {
                variants: vec![
                    ContractTypeRef::builtin("string"),
                    ContractTypeRef::builtin("null"),
                ],
            },
        ),
        (
            "Record",
            "type:record",
            ContractTypeRef::Record {
                fields: BTreeMap::from([("enabled".to_string(), ContractTypeRef::builtin("bool"))]),
            },
        ),
        (
            "Container",
            "type:container",
            ContractTypeRef::Builtin {
                name: "Array".to_string(),
                arguments: vec![ContractTypeRef::builtin("number")],
            },
        ),
    ] {
        let dependency_analysis =
            dependency_analysis_with_alias("example.types", stable_key, type_id, target);
        assert!(
            package_type_target_assignable(
                &package_type("example.types", stable_key, type_id),
                &json,
                &dependency_analysis,
            ),
            "{stable_key} should cross only the explicit JSON target"
        );
    }

    let non_json = dependency_analysis_with_alias(
        "example.types",
        "Binary",
        "type:binary",
        ContractTypeRef::builtin("bytes"),
    );
    assert!(!package_type_target_assignable(
        &package_type("example.types", "Binary", "type:binary"),
        &json,
        &non_json,
    ));
    assert!(!package_type_target_assignable(
        &package_type("example.types", "Missing", "type:missing"),
        &json,
        &non_json,
    ));

    let exact_scalar = package_type("example.types", "Scalar", "type:scalar");
    let scalar = dependency_analysis_with_alias(
        "example.types",
        "Scalar",
        "type:scalar",
        ContractTypeRef::builtin("string"),
    );
    assert!(!package_type_target_assignable(
        &exact_scalar,
        &package_type("other.types", "Scalar", "type:scalar"),
        &scalar,
    ));

    assert!(local_ir_json_compatible(
        &TypeRefIr::Builtin {
            name: "Map".to_string(),
            args: vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("Json"),],
        },
        &scalar,
        false,
    ));
}

fn dependency_analysis_with_alias(
    package_id: &str,
    stable_key: &str,
    type_id: &str,
    target: ContractTypeRef,
) -> SourceDependencyAnalysisInput {
    let record = PackageSchemaTypeRecord {
        package_id: package_id.to_string(),
        stable_schema_key: stable_key.to_string(),
        package_schema_type_id: type_id.into(),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Alias { target },
        },
    };
    SourceDependencyAnalysisInput::new(
        [(
            "types".to_string(),
            PackageDependencyAnalysisFacts::new(
                skiff_artifact_model::PackageBuildId::new("build:types"),
                PackageLocalAbiIdentity::new("abi"),
                BTreeMap::new(),
            )
            .with_schema_bindings([(stable_key.to_string(), record)]),
        )],
        [],
    )
    .unwrap()
}

fn literal(value: &str) -> PackageTypeRef {
    PackageTypeRef::Local {
        local_type: TypeRefIr::Literal {
            value: skiff_artifact_model::LiteralIr::String {
                value: value.to_string(),
            },
        },
    }
}

fn package_type(package_id: &str, stable_key: &str, type_id: &str) -> PackageTypeRef {
    PackageTypeRef::PackageSchema {
        package_id: package_id.to_string(),
        stable_schema_key: stable_key.to_string(),
        package_schema_type_id: type_id.into(),
    }
}
