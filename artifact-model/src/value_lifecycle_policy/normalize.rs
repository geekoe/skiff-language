use std::collections::BTreeMap;

use crate::{NominalTypeRefBaseIr, PackageRefIr, TypeRefIr};

use super::contract::{
    PositionalTypeEnvironment, ValueLifecyclePolicyBudget, ValueLifecyclePolicyError,
};

pub(super) fn normalize_type(
    ty: &TypeRefIr,
    environment: &PositionalTypeEnvironment,
    budget: &mut ValueLifecyclePolicyBudget,
    depth: u32,
) -> Result<TypeRefIr, ValueLifecyclePolicyError> {
    budget.charge(ty, depth)?;
    let nested = |child: &TypeRefIr, budget: &mut ValueLifecyclePolicyBudget| {
        normalize_type(child, environment, budget, depth + 1)
    };
    Ok(match ty {
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: args
                .iter()
                .map(|argument| nested(argument, budget))
                .collect::<Result<_, _>>()?,
        },
        TypeRefIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol {
            symbol: normalize_package_symbol(symbol)?,
        },
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => TypeRefIr::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        },
        TypeRefIr::AppliedNominal { base, arguments } => {
            let base = match base {
                NominalTypeRefBaseIr::PackageSymbol { symbol } => {
                    NominalTypeRefBaseIr::PackageSymbol {
                        symbol: normalize_package_symbol(symbol)?,
                    }
                }
                NominalTypeRefBaseIr::PackageSchema {
                    package_id,
                    stable_schema_key,
                    package_schema_type_id,
                } => NominalTypeRefBaseIr::PackageSchema {
                    package_id: package_id.clone(),
                    stable_schema_key: stable_schema_key.clone(),
                    package_schema_type_id: package_schema_type_id.clone(),
                },
                NominalTypeRefBaseIr::LocalType { .. } => {
                    return unnormalized("localType");
                }
                NominalTypeRefBaseIr::PublicationType { .. } => {
                    return unnormalized("publicationType");
                }
                NominalTypeRefBaseIr::ServiceSymbol { .. } => {
                    return unnormalized("serviceSymbol");
                }
            };
            TypeRefIr::AppliedNominal {
                base,
                arguments: arguments
                    .iter()
                    .map(|argument| nested(argument, budget))
                    .collect::<Result<_, _>>()?,
            }
        }
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field)| Ok((name.clone(), nested(field, budget)?)))
                .collect::<Result<BTreeMap<_, _>, ValueLifecyclePolicyError>>()?,
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| nested(item, budget))
                .collect::<Result<_, _>>()?,
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(nested(inner, budget)?),
        },
        TypeRefIr::Literal { value } => TypeRefIr::Literal {
            value: value.clone(),
        },
        TypeRefIr::TypeParam { name } => environment
            .resolve(name)
            .ok_or_else(|| ValueLifecyclePolicyError::UnknownTypeParameter { name: name.clone() })
            .and_then(|value| normalize_type(value, environment, budget, depth + 1))?,
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: crate::InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|argument| nested(argument, budget))
                    .collect::<Result<_, _>>()?,
            },
        },
        TypeRefIr::LocalType { .. } => return unnormalized("localType"),
        TypeRefIr::PublicationType { .. } => return unnormalized("publicationType"),
        TypeRefIr::ServiceSymbol { .. } => return unnormalized("serviceSymbol"),
        TypeRefIr::DbObjectSymbol { .. } => return unnormalized("dbObjectSymbol"),
        TypeRefIr::Function { .. } => {
            return Err(ValueLifecyclePolicyError::UnsupportedType { kind: "function" });
        }
    })
}

pub(super) fn normalize_package_symbol(
    symbol: &crate::PackageSymbolRef,
) -> Result<crate::PackageSymbolRef, ValueLifecyclePolicyError> {
    if !matches!(symbol.package, PackageRefIr::PackageId { .. }) {
        return unnormalized("dependencyPackageRef");
    }
    if symbol.abi_expectation.as_deref().is_none_or(str::is_empty) {
        return Err(ValueLifecyclePolicyError::MissingAbiIdentity {
            symbol_path: symbol.symbol_path.clone(),
        });
    }
    Ok(symbol.clone())
}

fn unnormalized<T>(kind: &'static str) -> Result<T, ValueLifecyclePolicyError> {
    Err(ValueLifecyclePolicyError::UnnormalizedOwner { kind })
}
