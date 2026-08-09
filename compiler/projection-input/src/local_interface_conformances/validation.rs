use std::collections::BTreeSet;

use skiff_artifact_model::{
    InterfaceInstantiationRef, NominalTypeRefBaseIr, PackageRefIr, PackageSymbolRef, TypeRefIr,
};
use skiff_compiler_core::json_utils::canonical_json_bytes;

use super::{
    ProjectionLocalInterfaceConformance, ProjectionLocalInterfaceConformanceError as Error,
};

pub(super) fn validate_conformance(
    conformance: &ProjectionLocalInterfaceConformance,
) -> Result<(), Error> {
    if conformance.receiver().module_path().is_empty() {
        return Err(Error::EmptyReceiverModulePath);
    }
    if conformance.receiver().symbol().is_empty() {
        return Err(Error::EmptyReceiverSymbol);
    }

    let mut type_parameters = BTreeSet::new();
    for name in conformance.type_parameters() {
        if name.is_empty() {
            return Err(Error::EmptyTypeParameter);
        }
        if !type_parameters.insert(name.as_str()) {
            return Err(Error::DuplicateTypeParameter { name: name.clone() });
        }
    }

    if conformance
        .implementation_executables()
        .iter()
        .any(|executable| executable.module_path().is_empty())
    {
        return Err(Error::EmptyImplementationModulePath);
    }

    validate_exact_interface(conformance.interface())
}

fn validate_exact_interface(interface: &InterfaceInstantiationRef) -> Result<(), Error> {
    let identity =
        serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).map_err(|error| {
            Error::InvalidInterfaceIdentity {
                message: error.to_string(),
            }
        })?;
    let canonical =
        canonical_json_bytes(&identity).map_err(|error| Error::InvalidInterfaceIdentity {
            message: error.to_string(),
        })?;
    if canonical != interface.interface_abi_id.as_bytes() {
        return Err(Error::NonCanonicalInterfaceIdentity);
    }

    validate_owner_stable_type_ref(&identity, "interface identity")?;
    for (index, argument) in interface.canonical_type_args.iter().enumerate() {
        validate_owner_stable_type_ref(argument, &format!("interface type argument {index}"))?;
    }
    Ok(())
}

fn validate_owner_stable_type_ref(ty: &TypeRefIr, location: &str) -> Result<(), Error> {
    match ty {
        TypeRefIr::Builtin { name, args } => {
            require_nonempty(name, "builtin name", location)?;
            validate_type_refs(args, location)
        }
        TypeRefIr::LocalType { .. } => Err(Error::UnstableLocalInterfaceIdentity {
            location: location.to_string(),
        }),
        TypeRefIr::PublicationType { module_path, .. } => {
            require_nonempty(module_path, "publication module path", location)
        }
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            require_nonempty(&symbol.module_path, "service symbol module path", location)?;
            require_nonempty(&symbol.symbol, "service symbol name", location)
        }
        TypeRefIr::PackageSymbol { symbol } => validate_package_symbol(symbol),
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            require_nonempty(package_id, "package schema owner", location)?;
            require_nonempty(stable_schema_key, "package schema stable key", location)?;
            require_nonempty(
                package_schema_type_id.as_str(),
                "package schema type identity",
                location,
            )
        }
        TypeRefIr::AppliedNominal { base, arguments } => {
            validate_owner_stable_nominal_base(base, location)?;
            validate_type_refs(arguments, location)
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                validate_owner_stable_type_ref(field, location)?;
            }
            Ok(())
        }
        TypeRefIr::Union { items } => validate_type_refs(items, location),
        TypeRefIr::Nullable { inner } => validate_owner_stable_type_ref(inner, location),
        TypeRefIr::Literal { .. } => Ok(()),
        TypeRefIr::TypeParam { name } => require_nonempty(name, "type parameter name", location),
        TypeRefIr::AnyInterface { interface } => validate_exact_interface(interface),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for parameter in params {
                validate_owner_stable_type_ref(&parameter.ty, location)?;
            }
            validate_owner_stable_type_ref(return_type, location)
        }
    }
}

fn validate_owner_stable_nominal_base(
    base: &NominalTypeRefBaseIr,
    location: &str,
) -> Result<(), Error> {
    match base {
        NominalTypeRefBaseIr::LocalType { .. } => Err(Error::UnstableLocalInterfaceIdentity {
            location: location.to_string(),
        }),
        NominalTypeRefBaseIr::PublicationType { module_path, .. } => {
            require_nonempty(module_path, "publication module path", location)
        }
        NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
            require_nonempty(&symbol.module_path, "service symbol module path", location)?;
            require_nonempty(&symbol.symbol, "service symbol name", location)
        }
        NominalTypeRefBaseIr::PackageSymbol { symbol } => validate_package_symbol(symbol),
        NominalTypeRefBaseIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            require_nonempty(package_id, "package schema owner", location)?;
            require_nonempty(stable_schema_key, "package schema stable key", location)?;
            require_nonempty(
                package_schema_type_id.as_str(),
                "package schema type identity",
                location,
            )
        }
    }
}

fn validate_package_symbol(symbol: &PackageSymbolRef) -> Result<(), Error> {
    if symbol.symbol_path.is_empty() {
        return Err(Error::EmptyPackageSymbolPath);
    }
    match &symbol.package {
        PackageRefIr::PackageId { package_id } if package_id.is_empty() => {
            return Err(Error::EmptyPackageId {
                symbol_path: symbol.symbol_path.clone(),
            });
        }
        PackageRefIr::PackageId { .. } => {}
        PackageRefIr::Dependency { dependency_ref } => {
            return Err(Error::DependencyAliasInterfaceIdentity {
                dependency_ref: dependency_ref.clone(),
                symbol_path: symbol.symbol_path.clone(),
            });
        }
    }
    if symbol.abi_expectation.as_deref().is_none_or(str::is_empty) {
        return Err(Error::MissingPackageAbiExpectation {
            symbol_path: symbol.symbol_path.clone(),
        });
    }
    Ok(())
}

fn validate_type_refs(types: &[TypeRefIr], location: &str) -> Result<(), Error> {
    for ty in types {
        validate_owner_stable_type_ref(ty, location)?;
    }
    Ok(())
}

fn require_nonempty(value: &str, component: &'static str, location: &str) -> Result<(), Error> {
    if value.is_empty() {
        return Err(Error::EmptyStableIdentityComponent {
            component,
            location: location.to_string(),
        });
    }
    Ok(())
}
