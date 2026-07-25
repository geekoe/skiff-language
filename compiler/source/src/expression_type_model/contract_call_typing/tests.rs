use std::collections::BTreeMap;

use skiff_artifact_model::{
    ContractLiteral, ContractTypeDescriptor, ContractTypeRef, PackageLocalAbiIdentity,
    PackageRefIr, PackageSchemaCanonicalDescriptor, PackageSchemaTypeRecord, PackageTypeRef,
    TypeRefIr,
};

use super::type_projection::{
    package_type_assignable, package_type_target_assignable, resolved_contract_type,
};
use crate::{PackageDependencyAnalysisFacts, SourceDependencyAnalysisInput};

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
