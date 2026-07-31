use std::collections::BTreeMap;

use skiff_artifact_model::{
    ContractTypeDescriptor, ContractTypeRef, PackageBuildId, PackageLocalAbiIdentity, PackageRefIr,
    PackageSchemaCanonicalDescriptor, PackageSchemaTypeRecord, PackageSymbolRef, PackageTypeRef,
    ServiceSymbolRef, TypeRefIr,
};

use super::package_type_ref_from_ir;
use crate::{
    contract_dependency_test_fixture::resolved_contract_fixture, PackageDependencyAnalysisFacts,
    SourceDependencyAnalysisInput,
};

fn package_dependency_analysis() -> SourceDependencyAnalysisInput {
    let record = PackageSchemaTypeRecord {
        package_id: "example.types".to_string(),
        stable_schema_key: "Payload".to_string(),
        package_schema_type_id: "type:payload".into(),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Alias {
                target: ContractTypeRef::builtin("string"),
            },
        },
    };
    SourceDependencyAnalysisInput::new(
        [(
            "types".to_string(),
            PackageDependencyAnalysisFacts::new(
                PackageBuildId::new("build:types"),
                PackageLocalAbiIdentity::new("abi"),
                BTreeMap::new(),
            )
            .with_schema_bindings([("Payload".to_string(), record)]),
        )],
        [],
    )
    .unwrap()
}

fn contract_dependency_analysis() -> SourceDependencyAnalysisInput {
    SourceDependencyAnalysisInput::new(
        [],
        [resolved_contract_fixture(
            "svc",
            "example.echo",
            "echo",
            "Failure",
            "Response",
        )],
    )
    .unwrap()
}

fn package_symbol(package: PackageRefIr, symbol_path: &str) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package,
            symbol_path: symbol_path.to_string(),
            abi_expectation: None,
        },
    }
}

fn service_symbol(module_path: &str, symbol: &str) -> TypeRefIr {
    TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
        },
    }
}

#[test]
fn package_symbol_resolves_to_exact_package_schema() {
    let dependency_analysis = package_dependency_analysis();
    let ty = package_symbol(
        PackageRefIr::PackageId {
            package_id: "example.types".to_string(),
        },
        "Payload",
    );
    assert_eq!(
        package_type_ref_from_ir(&ty, &dependency_analysis).unwrap(),
        PackageTypeRef::PackageSchema {
            package_id: "example.types".to_string(),
            stable_schema_key: "Payload".to_string(),
            package_schema_type_id: "type:payload".into(),
        }
    );
}

#[test]
fn package_symbol_lookup_failure_falls_back_to_local() {
    let dependency_analysis = package_dependency_analysis();
    let ty = package_symbol(
        PackageRefIr::PackageId {
            package_id: "example.types".to_string(),
        },
        "Missing",
    );
    assert_eq!(
        package_type_ref_from_ir(&ty, &dependency_analysis).unwrap(),
        PackageTypeRef::Local {
            local_type: ty.clone(),
        }
    );
}

#[test]
fn package_symbol_dependency_ref_resolves_and_missing_dependency_falls_back_to_local() {
    let dependency_analysis = package_dependency_analysis();
    let ty = package_symbol(
        PackageRefIr::Dependency {
            dependency_ref: "types".to_string(),
        },
        "Payload",
    );
    assert_eq!(
        package_type_ref_from_ir(&ty, &dependency_analysis).unwrap(),
        PackageTypeRef::PackageSchema {
            package_id: "example.types".to_string(),
            stable_schema_key: "Payload".to_string(),
            package_schema_type_id: "type:payload".into(),
        }
    );

    let unbound = package_symbol(
        PackageRefIr::Dependency {
            dependency_ref: "missing".to_string(),
        },
        "Payload",
    );
    assert_eq!(
        package_type_ref_from_ir(&unbound, &dependency_analysis).unwrap(),
        PackageTypeRef::Local {
            local_type: unbound.clone(),
        }
    );
}

#[test]
fn service_symbol_resolves_to_contract_package_schema() {
    let dependency_analysis = contract_dependency_analysis();
    let ty = service_symbol("svc", "Failure");
    match package_type_ref_from_ir(&ty, &dependency_analysis).unwrap() {
        PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => {
            assert_eq!(package_id, "example.echo.package");
            assert_eq!(stable_schema_key, "Failure");
        }
        other => panic!("expected PackageSchema, got {other:?}"),
    }
}

#[test]
fn service_symbol_lookup_failure_is_err() {
    let dependency_analysis = contract_dependency_analysis();
    let ty = service_symbol("svc", "Missing");
    assert!(package_type_ref_from_ir(&ty, &dependency_analysis).is_err());
}

#[test]
fn service_symbol_without_contract_requirement_falls_back_to_local() {
    let dependency_analysis = contract_dependency_analysis();
    let ty = service_symbol("other", "Failure");
    assert_eq!(
        package_type_ref_from_ir(&ty, &dependency_analysis).unwrap(),
        PackageTypeRef::Local {
            local_type: ty.clone(),
        }
    );
}

#[test]
fn record_union_and_function_embedding_contract_symbols_are_err() {
    let dependency_analysis = contract_dependency_analysis();
    let embedded = service_symbol("svc", "Failure");
    let record = TypeRefIr::Record {
        fields: BTreeMap::from([("value".to_string(), embedded.clone())]),
    };
    assert!(package_type_ref_from_ir(&record, &dependency_analysis).is_err());

    let union = TypeRefIr::Union {
        items: vec![embedded.clone()],
    };
    assert!(package_type_ref_from_ir(&union, &dependency_analysis).is_err());

    let function = TypeRefIr::Function {
        params: vec![skiff_artifact_model::FunctionTypeParamIr {
            name: "input".to_string(),
            ty: embedded,
        }],
        return_type: Box::new(TypeRefIr::builtin("string")),
    };
    assert!(package_type_ref_from_ir(&function, &dependency_analysis).is_err());
}

#[test]
fn local_types_without_contract_symbols_pass_through_verbatim() {
    let dependency_analysis = contract_dependency_analysis();
    let type_param = TypeRefIr::TypeParam {
        name: "T".to_string(),
    };
    assert_eq!(
        package_type_ref_from_ir(&type_param, &dependency_analysis).unwrap(),
        PackageTypeRef::Local {
            local_type: type_param.clone(),
        }
    );
    let record = TypeRefIr::Record {
        fields: BTreeMap::from([("value".to_string(), TypeRefIr::builtin("string"))]),
    };
    assert_eq!(
        package_type_ref_from_ir(&record, &dependency_analysis).unwrap(),
        PackageTypeRef::Local {
            local_type: record.clone(),
        }
    );
}

#[test]
fn any_interface_round_trips_through_canonical_identity() {
    let dependency_analysis = package_dependency_analysis();
    let ty = TypeRefIr::AnyInterface {
        interface: skiff_artifact_model::InterfaceInstantiationRef {
            interface_abi_id: serde_json::to_string(&TypeRefIr::builtin("Reader")).unwrap(),
            canonical_type_args: vec![TypeRefIr::builtin("string")],
        },
    };
    assert_eq!(
        package_type_ref_from_ir(&ty, &dependency_analysis).unwrap(),
        PackageTypeRef::AnyInterface {
            interface: Box::new(PackageTypeRef::Container {
                name: "Reader".to_string(),
                arguments: Vec::new(),
            }),
            arguments: vec![PackageTypeRef::Container {
                name: "string".to_string(),
                arguments: Vec::new(),
            }],
        }
    );
}
