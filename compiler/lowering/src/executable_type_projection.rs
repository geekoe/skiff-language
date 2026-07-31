use skiff_artifact_identity::type_ref_abi_key;
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
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: type_ref_abi_key(&execution_type_ref(interface)),
                canonical_type_args: arguments.iter().map(execution_type_ref).collect(),
            },
        },
    }
}

#[cfg(test)]
mod tests;
