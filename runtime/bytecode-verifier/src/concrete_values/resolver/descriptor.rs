use skiff_artifact_model::{
    NamedUnionBranchIr, NominalTypeRefBaseIr, PackageRefIr, PackageSymbolRef, TypeDescriptorIr,
    TypeRefIr, ValueLifecycleResolverError,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use super::{resolver_error, PACKAGE_SYMBOL_AUTHORITY};

/// Completes owner-local references in an already-authoritative package ABI
/// descriptor. Implementation descriptors intentionally omit their own ABI
/// identity to avoid a self-referential Local ABI hash; exact hydration is the
/// independent source for that missing identity.
pub(super) fn owner_normalize_descriptor(
    package: &HydratedBytecodePackage,
    descriptor: &TypeDescriptorIr,
) -> Result<TypeDescriptorIr, ValueLifecycleResolverError> {
    Ok(match descriptor.clone() {
        TypeDescriptorIr::Record { fields } => TypeDescriptorIr::Record {
            fields: fields
                .into_iter()
                .map(|(name, ty)| Ok((name, owner_normalize_type(package, ty)?)))
                .collect::<Result<_, ValueLifecycleResolverError>>()?,
        },
        TypeDescriptorIr::Representation { representation } => TypeDescriptorIr::Representation {
            representation: owner_normalize_type(package, representation)?,
        },
        TypeDescriptorIr::Union { branches } => TypeDescriptorIr::Union {
            branches: branches
                .into_iter()
                .map(|branch| owner_normalize_branch(package, branch))
                .collect::<Result<_, _>>()?,
        },
        TypeDescriptorIr::Alias { target } => TypeDescriptorIr::Alias {
            target: owner_normalize_type(package, target)?,
        },
        TypeDescriptorIr::Interface => TypeDescriptorIr::Interface,
    })
}

fn owner_normalize_branch(
    package: &HydratedBytecodePackage,
    branch: NamedUnionBranchIr,
) -> Result<NamedUnionBranchIr, ValueLifecycleResolverError> {
    Ok(match branch {
        NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
            NamedUnionBranchIr::ConcreteNominal {
                nominal_type: owner_normalize_type(package, nominal_type)?,
            }
        }
        NamedUnionBranchIr::SyntheticDiscriminator {
            payload_type,
            discriminator_field,
            discriminator_value,
        } => NamedUnionBranchIr::SyntheticDiscriminator {
            payload_type: owner_normalize_type(package, payload_type)?,
            discriminator_field,
            discriminator_value,
        },
        NamedUnionBranchIr::Literal { value } => NamedUnionBranchIr::Literal { value },
    })
}

fn owner_normalize_type(
    package: &HydratedBytecodePackage,
    ty: TypeRefIr,
) -> Result<TypeRefIr, ValueLifecycleResolverError> {
    Ok(match ty {
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name,
            args: normalize_types(package, args)?,
        },
        TypeRefIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol {
            symbol: owner_normalize_symbol(package, symbol)?,
        },
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base: owner_normalize_base(package, base)?,
            arguments: normalize_types(package, arguments)?,
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .into_iter()
                .map(|(name, ty)| Ok((name, owner_normalize_type(package, ty)?)))
                .collect::<Result<_, ValueLifecycleResolverError>>()?,
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: normalize_types(package, items)?,
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(owner_normalize_type(package, *inner)?),
        },
        TypeRefIr::Function {
            mut params,
            return_type,
        } => {
            for parameter in &mut params {
                parameter.ty = owner_normalize_type(package, parameter.ty.clone())?;
            }
            TypeRefIr::Function {
                params,
                return_type: Box::new(owner_normalize_type(package, *return_type)?),
            }
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. } => ty,
    })
}

fn owner_normalize_base(
    package: &HydratedBytecodePackage,
    base: NominalTypeRefBaseIr,
) -> Result<NominalTypeRefBaseIr, ValueLifecycleResolverError> {
    Ok(match base {
        NominalTypeRefBaseIr::PackageSymbol { symbol } => NominalTypeRefBaseIr::PackageSymbol {
            symbol: owner_normalize_symbol(package, symbol)?,
        },
        NominalTypeRefBaseIr::LocalType { .. }
        | NominalTypeRefBaseIr::PublicationType { .. }
        | NominalTypeRefBaseIr::ServiceSymbol { .. }
        | NominalTypeRefBaseIr::PackageSchema { .. } => base,
    })
}

fn owner_normalize_symbol(
    package: &HydratedBytecodePackage,
    mut symbol: PackageSymbolRef,
) -> Result<PackageSymbolRef, ValueLifecycleResolverError> {
    let is_owner_local = matches!(
        &symbol.package,
        PackageRefIr::PackageId { package_id }
            if package_id == &package.reference().package_id
    );
    if is_owner_local {
        let exact_abi = package.reference().package_local_abi_identity.as_str();
        match symbol
            .abi_expectation
            .as_deref()
            .filter(|expectation| !expectation.is_empty())
        {
            Some(expectation) if expectation != exact_abi => {
                return Err(resolver_error(
                    PACKAGE_SYMBOL_AUTHORITY,
                    format!(
                        "owner-local descriptor type {:?} ABI expectation {expectation:?} disagrees with exact package ABI {exact_abi}",
                        symbol.symbol_path
                    ),
                ));
            }
            Some(_) => {}
            None => symbol.abi_expectation = Some(exact_abi.to_string()),
        }
    }
    Ok(symbol)
}

fn normalize_types(
    package: &HydratedBytecodePackage,
    types: Vec<TypeRefIr>,
) -> Result<Vec<TypeRefIr>, ValueLifecycleResolverError> {
    types
        .into_iter()
        .map(|ty| owner_normalize_type(package, ty))
        .collect()
}
