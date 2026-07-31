use skiff_artifact_model::PackageSchemaTypeId;

use super::*;

#[test]
fn nested_package_schema_leaf_keeps_its_exact_owner_and_stable_key() {
    let package_schema_type_id = PackageSchemaTypeId::new("package-type:request");
    let source = PackageTypeRef::Container {
        name: "Array".to_string(),
        arguments: vec![PackageTypeRef::Nullable {
            inner: Box::new(PackageTypeRef::PackageSchema {
                package_id: "example.types".to_string(),
                stable_schema_key: "Request".to_string(),
                package_schema_type_id: package_schema_type_id.clone(),
            }),
        }],
    };

    let projected = execution_type_ref(&source);

    assert_eq!(
        projected,
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![TypeRefIr::Nullable {
                inner: Box::new(TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: "example.types".to_string(),
                        },
                        symbol_path: "Request".to_string(),
                        abi_expectation: None,
                    },
                }),
            }],
        }
    );
    let wire = serde_json::to_string(&projected).unwrap();
    assert!(wire.contains("example.types"));
    assert!(wire.contains("Request"));
    assert!(!wire.contains(package_schema_type_id.as_str()));
}

#[test]
fn local_execution_type_is_preserved_without_reinterpretation() {
    let local = TypeRefIr::LocalType { type_index: 7 };
    assert_eq!(
        execution_type_ref(&PackageTypeRef::Local {
            local_type: local.clone(),
        }),
        local
    );
}

#[test]
fn package_owned_any_interface_keeps_exact_executable_target() {
    let projected = execution_type_ref(&PackageTypeRef::Nullable {
        inner: Box::new(PackageTypeRef::AnyInterface {
            interface: Box::new(PackageTypeRef::PackageSchema {
                package_id: "example.interfaces".to_string(),
                stable_schema_key: "Reader".to_string(),
                package_schema_type_id: PackageSchemaTypeId::new("package-type:reader"),
            }),
            arguments: Vec::new(),
        }),
    });

    assert_eq!(
        projected,
        TypeRefIr::Nullable {
            inner: Box::new(TypeRefIr::AnyInterface {
                interface: skiff_artifact_model::InterfaceInstantiationRef {
                    interface_abi_id: type_ref_abi_key(&TypeRefIr::PackageSymbol {
                        symbol: PackageSymbolRef {
                            package: PackageRefIr::PackageId {
                                package_id: "example.interfaces".to_string(),
                            },
                            symbol_path: "Reader".to_string(),
                            abi_expectation: None,
                        },
                    }),
                    canonical_type_args: Vec::new(),
                },
            }),
        }
    );
}
