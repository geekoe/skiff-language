use skiff_artifact_model::{PackageRefIr, PackageSymbolRef, PackageTypeRef, TypeRefIr};

/// Projects an exact source type into the representation needed to execute a
/// File IR body. Contract identity deliberately stays outside File IR.
pub(crate) fn execution_type_ref(ty: &PackageTypeRef) -> TypeRefIr {
    match ty {
        PackageTypeRef::Local { local_type } => local_type.clone(),
        PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: package_id.clone(),
                },
                symbol_path: stable_schema_key.clone(),
                abi_expectation: None,
            },
        },
        PackageTypeRef::Container { name, arguments } => TypeRefIr::Builtin {
            name: name.clone(),
            args: arguments.iter().map(execution_type_ref).collect(),
        },
        PackageTypeRef::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(execution_type_ref(inner)),
        },
    }
}

#[cfg(test)]
mod tests {
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
}
