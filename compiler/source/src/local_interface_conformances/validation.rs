use std::collections::BTreeSet;

use skiff_artifact_model::{
    InterfaceInstantiationRef, NominalTypeRefBaseIr, PackageRefIr, PackageSymbolRef,
    ServiceSymbolRef, TypeRefIr,
};
use skiff_compiler_core::json_utils::canonical_json_bytes;

use super::{SourceLocalInterfaceConformance, SourceLocalInterfaceConformanceError as Error};

pub(super) fn validate_conformance(
    conformance: &SourceLocalInterfaceConformance,
) -> Result<(), Error> {
    if conformance.receiver().module_path().is_empty() {
        return Err(Error::EmptyReceiverModulePath);
    }
    if conformance.receiver().symbol().is_empty() {
        return Err(Error::EmptyReceiverSymbol);
    }

    let mut binders = BTreeSet::new();
    for parameter in conformance.receiver_type_parameters() {
        if parameter.is_empty() {
            return Err(Error::EmptyReceiverTypeParameter);
        }
        if !binders.insert(parameter.as_str()) {
            return Err(Error::DuplicateReceiverTypeParameter {
                name: parameter.clone(),
            });
        }
    }

    let expected_receiver = TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: conformance.receiver().module_path().to_string(),
            symbol: conformance.receiver().symbol().to_string(),
        },
    };
    if conformance.receiver_type() != &expected_receiver {
        return Err(Error::ReceiverTypeMismatch);
    }

    for implementation in conformance.implementation_methods() {
        if implementation.module_path().is_empty() {
            return Err(Error::EmptyImplementationModulePath);
        }
        if implementation.symbol().is_empty() {
            return Err(Error::EmptyImplementationSymbol);
        }
    }

    validate_exact_interface(conformance.interface(), &binders)
}

pub(super) fn validate_closed_interface(
    interface: &InterfaceInstantiationRef,
) -> Result<(), Error> {
    validate_exact_interface(interface, &BTreeSet::new())
}

fn validate_exact_interface(
    interface: &InterfaceInstantiationRef,
    binders: &BTreeSet<&str>,
) -> Result<(), Error> {
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
    if !matches!(
        identity,
        TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::PackageSchema { .. }
    ) {
        return Err(Error::InvalidInterfaceRoot);
    }
    validate_owner_stable_type_ref(&identity, "interface identity", binders)?;
    for (index, argument) in interface.canonical_type_args.iter().enumerate() {
        validate_owner_stable_type_ref(
            argument,
            &format!("interface type argument {index}"),
            binders,
        )?;
    }
    Ok(())
}

fn validate_owner_stable_type_ref(
    ty: &TypeRefIr,
    location: &str,
    binders: &BTreeSet<&str>,
) -> Result<(), Error> {
    match ty {
        TypeRefIr::Builtin { name, args } => {
            require_nonempty(name, "builtin name", location)?;
            validate_type_refs(args, location, binders)
        }
        TypeRefIr::LocalType { .. } => Err(Error::OwnerUnstableLocalType {
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
            validate_type_refs(arguments, location, binders)
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                validate_owner_stable_type_ref(field, location, binders)?;
            }
            Ok(())
        }
        TypeRefIr::Union { items } => validate_type_refs(items, location, binders),
        TypeRefIr::Nullable { inner } => validate_owner_stable_type_ref(inner, location, binders),
        TypeRefIr::Literal { .. } => Ok(()),
        TypeRefIr::TypeParam { name } if binders.contains(name.as_str()) => Ok(()),
        TypeRefIr::TypeParam { name } => Err(Error::ResidualTypeParameter {
            name: name.clone(),
            location: location.to_string(),
        }),
        TypeRefIr::AnyInterface { interface } => validate_exact_interface(interface, binders),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for parameter in params {
                validate_owner_stable_type_ref(&parameter.ty, location, binders)?;
            }
            validate_owner_stable_type_ref(return_type, location, binders)
        }
    }
}

fn validate_owner_stable_nominal_base(
    base: &NominalTypeRefBaseIr,
    location: &str,
) -> Result<(), Error> {
    match base {
        NominalTypeRefBaseIr::LocalType { .. } => Err(Error::OwnerUnstableLocalType {
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
            return Err(Error::DependencyAliasIdentity {
                dependency_ref: dependency_ref.clone(),
                symbol_path: symbol.symbol_path.clone(),
            });
        }
    }
    if symbol.abi_expectation.as_deref().is_some_and(str::is_empty) {
        return Err(Error::MissingPackageAbiExpectation {
            symbol_path: symbol.symbol_path.clone(),
        });
    }
    Ok(())
}

fn validate_type_refs(
    types: &[TypeRefIr],
    location: &str,
    binders: &BTreeSet<&str>,
) -> Result<(), Error> {
    for ty in types {
        validate_owner_stable_type_ref(ty, location, binders)?;
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
