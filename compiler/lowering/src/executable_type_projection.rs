use skiff_artifact_model::{PackageTypeRef, TypeRefIr};

/// Projects an exact source type into the representation needed to execute a
/// File IR body. Contract identity deliberately stays outside File IR.
pub(crate) fn execution_type_ref(ty: &PackageTypeRef) -> TypeRefIr {
    match ty {
        PackageTypeRef::Local { local_type } => local_type.clone(),
        PackageTypeRef::PackageSchema { .. } => TypeRefIr::builtin("unknown"),
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
    fn nested_contract_leaf_becomes_only_opaque_unknown() {
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
                    inner: Box::new(TypeRefIr::builtin("unknown")),
                }],
            }
        );
        let wire = serde_json::to_string(&projected).unwrap();
        assert!(!wire.contains(package_schema_type_id.as_str()));
        assert!(!wire.contains("packageSchema"));
        assert!(!wire.contains("example.types"));
        assert!(!wire.contains("serviceSymbol"));
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
