use skiff_artifact_identity::{assign_package_artifact_identities, package_schema_index_identity};
use skiff_artifact_model::{
    PackageBuildId, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef,
    PackageSymbolRef, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use super::*;

#[test]
fn compiler_owned_std_selection_is_exact_and_fail_closed() {
    let std = canonical_artifact(SKIFF_STD_PUBLICATION_ID);
    let other = canonical_artifact("example.other");
    let available = [other.clone(), std.clone()];
    let selected = compiler_owned_std_artifact("example.consumer", &available)
        .unwrap()
        .expect("one exact std artifact should be selected");
    assert_eq!(selected.package_build_id, std.package_build_id);
    assert!(
        compiler_owned_std_artifact("example.consumer", std::slice::from_ref(&other))
            .unwrap()
            .is_none(),
        "an undeclared non-std available artifact must not become source-visible"
    );
    assert!(
        compiler_owned_std_artifact("example.consumer", &[])
            .unwrap()
            .is_none(),
        "absence must not fabricate a std owner"
    );

    let error = compiler_owned_std_artifact("example.consumer", &[std.clone(), std.clone()])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("duplicate exact canonical artifacts"),
        "{error}"
    );

    let mut wrong_identity = std;
    wrong_identity.package_build_id = PackageBuildId::new("forged");
    let error =
        compiler_owned_std_artifact("example.consumer", std::slice::from_ref(&wrong_identity))
            .unwrap_err()
            .to_string();
    assert!(error.contains("identity validation failed"), "{error}");
}

#[test]
fn applied_base_and_nested_arguments_bind_only_the_exact_package_abi() {
    let dependency = package_artifact("example.dep", "abi:dep");
    let package_symbol = |package_id: &str, symbol_path: &str, abi_expectation| PackageSymbolRef {
        package: PackageRefIr::PackageId {
            package_id: package_id.to_string(),
        },
        symbol_path: symbol_path.to_string(),
        abi_expectation,
    };
    let input = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::PackageSymbol {
            symbol: package_symbol("example.dep", "Box", Some("abi:stale-base".to_string())),
        },
        arguments: vec![
            TypeRefIr::PackageSymbol {
                symbol: package_symbol("example.dep", "Value", None),
            },
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PackageSymbol {
                    symbol: package_symbol("example.dep", "Nested", None),
                },
                arguments: vec![TypeRefIr::PackageSymbol {
                    symbol: package_symbol("example.other", "Value", Some("abi:other".to_string())),
                }],
            },
        ],
    };

    assert_eq!(
        bind_type_identity(&input, &dependency),
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol {
                symbol: package_symbol("example.dep", "Box", Some("abi:dep".to_string())),
            },
            arguments: vec![
                TypeRefIr::PackageSymbol {
                    symbol: package_symbol("example.dep", "Value", Some("abi:dep".to_string()),),
                },
                TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::PackageSymbol {
                        symbol: package_symbol(
                            "example.dep",
                            "Nested",
                            Some("abi:dep".to_string()),
                        ),
                    },
                    arguments: vec![TypeRefIr::PackageSymbol {
                        symbol: package_symbol(
                            "example.other",
                            "Value",
                            Some("abi:other".to_string()),
                        ),
                    }],
                },
            ],
        }
    );
}

#[test]
fn dependency_signature_identity_binding_preserves_ordered_callable_type_parameters() {
    let dependency = package_artifact("example.dep", "abi:dep");
    let signature = PackageCallableSignature {
        type_params: vec!["T".to_string(), "Id".to_string()],
        parameters: vec![skiff_artifact_model::PackageCallableParameter {
            name: "id".to_string(),
            ty: PackageTypeRef::Local {
                local_type: TypeRefIr::TypeParam {
                    name: "Id".to_string(),
                },
            },
        }],
        return_type: PackageTypeRef::Nullable {
            inner: Box::new(PackageTypeRef::Local {
                local_type: TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
            }),
        },
        may_suspend: false,
    };

    let bound = bind_callable_signature_identity(&signature, &dependency);
    assert_eq!(bound.type_params, ["T", "Id"]);
    assert_eq!(bound.parameters, signature.parameters);
    assert_eq!(bound.return_type, signature.return_type);
}

fn package_artifact(package_id: &str, local_abi: &str) -> PackageArtifact {
    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("build"),
        files: Vec::new(),
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new(local_abi),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: PackageSchemaIndexIdentity::new("schema"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    }
}

fn canonical_artifact(package_id: &str) -> PackageArtifact {
    let mut artifact = package_artifact(package_id, "unassigned");
    artifact.schema_version = PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string();
    artifact.package_build_id = PackageBuildId::new("unassigned");
    artifact.package_local_abi.local_abi_identity = PackageLocalAbiIdentity::new("unassigned");
    artifact.package_schema_index.package_schema_index_identity =
        package_schema_index_identity(package_id, &BTreeMap::new()).unwrap();
    assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}
