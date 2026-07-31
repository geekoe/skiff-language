use std::collections::BTreeMap;

use skiff_artifact_identity::type_ref_abi_key;
use skiff_artifact_model::{
    CallableProvenanceSummary, CallableSemanticFacts, ContractTypeRef, FileIrUnit,
    FunctionTypeParamIr, InterfaceInstantiationRef, NamedUnionBranchIr, NominalTypeRefBaseIr,
    PackageCallableSignature, PackageRefIr, PackageSymbolRef, PackageTypeRef, ServiceSymbolRef,
    TypeDescriptorIr, TypeRefIr, ValueProjectionStep, ValueProvenance,
};
use skiff_compiler_projection_input::ResolvedPackageSchema;

use crate::package_artifact::boundary::ordering::escape_lane_rank;

pub(super) fn normalize_semantic_facts(mut facts: CallableSemanticFacts) -> CallableSemanticFacts {
    if let CallableProvenanceSummary::Analyzed {
        return_origins,
        direct_return_origins,
        throw_origins,
        escape_lanes,
    } = &mut facts.provenance
    {
        return_origins.sort_by_key(provenance_sort_key);
        return_origins.dedup();
        direct_return_origins.sort_by_key(provenance_sort_key);
        direct_return_origins.dedup();
        throw_origins.sort_by_key(provenance_sort_key);
        throw_origins.dedup();
        escape_lanes.sort_by_key(|lane| escape_lane_rank(*lane));
        escape_lanes.dedup();
    }
    facts
}

pub(super) fn normalize_public_signature(
    owner_module: &str,
    signature: &mut PackageCallableSignature,
    file_ir_units: &[FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<(), String> {
    for parameter in &mut signature.parameters {
        parameter.ty = normalize_package_type(
            owner_module,
            &parameter.ty,
            file_ir_units,
            public_type_ids,
            resolved_package_schemas,
        )?;
    }
    signature.return_type = normalize_package_type(
        owner_module,
        &signature.return_type,
        file_ir_units,
        public_type_ids,
        resolved_package_schemas,
    )?;
    Ok(())
}

pub(super) fn normalize_implementation_type(
    package_id: &str,
    owner_module: &str,
    ty: &TypeRefIr,
    file_ir_units: &[FileIrUnit],
) -> Result<TypeRefIr, String> {
    let normalize =
        |ty: &TypeRefIr| normalize_implementation_type(package_id, owner_module, ty, file_ir_units);
    match ty {
        TypeRefIr::LocalType { type_index } => {
            implementation_type_symbol(package_id, file_ir_units, owner_module, *type_index)
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => implementation_type_symbol(package_id, file_ir_units, module_path, *type_index),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            let source_path = format!("{}.{}", symbol.module_path, symbol.symbol);
            if implementation_type_location(file_ir_units, &symbol.module_path, &symbol.symbol)
                .is_some()
            {
                Ok(package_symbol_type(package_id, source_path))
            } else {
                Ok(ty.clone())
            }
        }
        TypeRefIr::Builtin { name, args } => Ok(TypeRefIr::Builtin {
            name: name.clone(),
            args: args.iter().map(normalize).collect::<Result<_, _>>()?,
        }),
        TypeRefIr::AppliedNominal { base, arguments } => Ok(TypeRefIr::AppliedNominal {
            base: normalize_implementation_nominal_base(
                package_id,
                owner_module,
                base,
                file_ir_units,
            )?,
            arguments: arguments.iter().map(normalize).collect::<Result<_, _>>()?,
        }),
        TypeRefIr::PackageSymbol { symbol } => {
            let mut symbol = symbol.clone();
            if matches!(
                &symbol.package,
                PackageRefIr::PackageId { package_id: owner } if owner == package_id
            ) {
                symbol.abi_expectation = None;
            }
            Ok(TypeRefIr::PackageSymbol { symbol })
        }
        TypeRefIr::PackageSchema { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => Ok(ty.clone()),
        TypeRefIr::Record { fields } => Ok(TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field)| Ok((name.clone(), normalize(field)?)))
                .collect::<Result<_, String>>()?,
        }),
        TypeRefIr::Union { items } => Ok(TypeRefIr::Union {
            items: items.iter().map(normalize).collect::<Result<_, _>>()?,
        }),
        TypeRefIr::Nullable { inner } => Ok(TypeRefIr::Nullable {
            inner: Box::new(normalize(inner)?),
        }),
        TypeRefIr::AnyInterface { interface } => {
            let identity = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).map_err(
                |error| {
                    format!(
                        "implementation interface identity is not a canonical TypeRefIr: {error}"
                    )
                },
            )?;
            let identity = normalize(&identity)?;
            Ok(TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id: type_ref_abi_key(&identity),
                    canonical_type_args: interface
                        .canonical_type_args
                        .iter()
                        .map(normalize)
                        .collect::<Result<_, _>>()?,
                },
            })
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => Ok(TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| {
                    Ok(FunctionTypeParamIr {
                        name: param.name.clone(),
                        ty: normalize(&param.ty)?,
                    })
                })
                .collect::<Result<_, String>>()?,
            return_type: Box::new(normalize(return_type)?),
        }),
    }
}

pub(super) fn normalize_implementation_descriptor(
    package_id: &str,
    owner_module: &str,
    descriptor: &TypeDescriptorIr,
    file_ir_units: &[FileIrUnit],
) -> Result<TypeDescriptorIr, String> {
    match descriptor {
        TypeDescriptorIr::Record { fields } => Ok(TypeDescriptorIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    Ok((
                        name.clone(),
                        normalize_implementation_type(package_id, owner_module, ty, file_ir_units)?,
                    ))
                })
                .collect::<Result<_, String>>()?,
        }),
        TypeDescriptorIr::Alias { target } => Ok(TypeDescriptorIr::Alias {
            target: normalize_implementation_type(package_id, owner_module, target, file_ir_units)?,
        }),
        TypeDescriptorIr::Representation { representation } => {
            Ok(TypeDescriptorIr::Representation {
                representation: normalize_implementation_type(
                    package_id,
                    owner_module,
                    representation,
                    file_ir_units,
                )?,
            })
        }
        TypeDescriptorIr::Union { branches } => Ok(TypeDescriptorIr::Union {
            branches: branches
                .iter()
                .map(|branch| match branch {
                    NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                        Ok(NamedUnionBranchIr::ConcreteNominal {
                            nominal_type: normalize_implementation_type(
                                package_id,
                                owner_module,
                                nominal_type,
                                file_ir_units,
                            )?,
                        })
                    }
                    NamedUnionBranchIr::SyntheticDiscriminator {
                        payload_type,
                        discriminator_field,
                        discriminator_value,
                    } => Ok(NamedUnionBranchIr::SyntheticDiscriminator {
                        payload_type: normalize_implementation_type(
                            package_id,
                            owner_module,
                            payload_type,
                            file_ir_units,
                        )?,
                        discriminator_field: discriminator_field.clone(),
                        discriminator_value: discriminator_value.clone(),
                    }),
                    NamedUnionBranchIr::Literal { value } => Ok(NamedUnionBranchIr::Literal {
                        value: value.clone(),
                    }),
                })
                .collect::<Result<Vec<_>, String>>()?,
        }),
        TypeDescriptorIr::Interface => Ok(TypeDescriptorIr::Interface),
    }
}

fn normalize_implementation_nominal_base(
    package_id: &str,
    owner_module: &str,
    base: &NominalTypeRefBaseIr,
    file_ir_units: &[FileIrUnit],
) -> Result<NominalTypeRefBaseIr, String> {
    match base {
        NominalTypeRefBaseIr::LocalType { type_index } => {
            let TypeRefIr::PackageSymbol { symbol } =
                implementation_type_symbol(package_id, file_ir_units, owner_module, *type_index)?
            else {
                unreachable!("implementation local nominal must normalize to a package symbol")
            };
            Ok(NominalTypeRefBaseIr::PackageSymbol { symbol })
        }
        NominalTypeRefBaseIr::PublicationType {
            module_path,
            type_index,
        } => {
            let TypeRefIr::PackageSymbol { symbol } =
                implementation_type_symbol(package_id, file_ir_units, module_path, *type_index)?
            else {
                unreachable!(
                    "implementation publication nominal must normalize to a package symbol"
                )
            };
            Ok(NominalTypeRefBaseIr::PackageSymbol { symbol })
        }
        NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
            let source_path = format!("{}.{}", symbol.module_path, symbol.symbol);
            if implementation_type_location(file_ir_units, &symbol.module_path, &symbol.symbol)
                .is_some()
            {
                let TypeRefIr::PackageSymbol { symbol } =
                    package_symbol_type(package_id, source_path)
                else {
                    unreachable!(
                        "implementation service nominal must normalize to a package symbol"
                    )
                };
                Ok(NominalTypeRefBaseIr::PackageSymbol { symbol })
            } else {
                Ok(base.clone())
            }
        }
        NominalTypeRefBaseIr::PackageSymbol { symbol } => {
            let mut symbol = symbol.clone();
            if matches!(
                &symbol.package,
                PackageRefIr::PackageId { package_id: owner } if owner == package_id
            ) {
                symbol.abi_expectation = None;
            }
            Ok(NominalTypeRefBaseIr::PackageSymbol { symbol })
        }
        NominalTypeRefBaseIr::PackageSchema { .. } => Ok(base.clone()),
    }
}

fn implementation_type_symbol(
    package_id: &str,
    units: &[FileIrUnit],
    module_path: &str,
    type_index: u32,
) -> Result<TypeRefIr, String> {
    let (module_path, symbol) =
        implementation_type_location_by_index(units, module_path, type_index).ok_or_else(|| {
            format!(
                "implementation type {module_path}#{type_index} has no exact top-level declaration"
            )
        })?;
    Ok(package_symbol_type(
        package_id,
        format!("{module_path}.{symbol}"),
    ))
}

fn package_symbol_type(package_id: &str, symbol_path: String) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package_id.to_string(),
            },
            symbol_path,
            abi_expectation: None,
        },
    }
}

fn implementation_type_location<'a>(
    units: &'a [FileIrUnit],
    module_path: &str,
    symbol: &str,
) -> Option<(&'a str, &'a str)> {
    let mut matches = units.iter().filter_map(|unit| {
        if unit.module_path != module_path {
            return None;
        }
        unit.declarations
            .types
            .get_key_value(symbol)
            .map(|(name, _)| (unit.module_path.as_str(), name.as_str()))
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn implementation_type_location_by_index<'a>(
    units: &'a [FileIrUnit],
    module_path: &str,
    type_index: u32,
) -> Option<(&'a str, &'a str)> {
    let mut matches = units.iter().filter_map(|unit| {
        if unit.module_path != module_path {
            return None;
        }
        unit.declarations
            .types
            .iter()
            .find(|(_, declaration)| declaration.type_index == type_index)
            .map(|(symbol, _)| (unit.module_path.as_str(), symbol.as_str()))
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn normalize_package_type(
    owner_module: &str,
    ty: &PackageTypeRef,
    file_ir_units: &[FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<PackageTypeRef, String> {
    Ok(match ty {
        PackageTypeRef::Local { local_type } => {
            let local_type = normalize_local_type(
                owner_module,
                local_type,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )?;
            lift_local_type(local_type)?
        }
        PackageTypeRef::Container { name, arguments } => PackageTypeRef::Container {
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(|argument| {
                    normalize_package_type(
                        owner_module,
                        argument,
                        file_ir_units,
                        public_type_ids,
                        resolved_package_schemas,
                    )
                })
                .collect::<Result<_, _>>()?,
        },
        PackageTypeRef::Nullable { inner } => PackageTypeRef::Nullable {
            inner: Box::new(normalize_package_type(
                owner_module,
                inner,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )?),
        },
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => PackageTypeRef::AnyInterface {
            interface: Box::new(normalize_package_type(
                owner_module,
                interface,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )?),
            arguments: arguments
                .iter()
                .map(|argument| {
                    normalize_package_type(
                        owner_module,
                        argument,
                        file_ir_units,
                        public_type_ids,
                        resolved_package_schemas,
                    )
                })
                .collect::<Result<_, _>>()?,
        },
        exact @ PackageTypeRef::PackageSchema { .. } => exact.clone(),
    })
}

fn normalize_local_type(
    owner_module: &str,
    ty: &TypeRefIr,
    file_ir_units: &[FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<TypeRefIr, String> {
    if let Some((module_path, type_index)) = nominal_source(owner_module, ty) {
        let symbol = exact_type_symbol(file_ir_units, module_path, type_index)?;
        if let Some(ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        }) = public_type_ids.get(&(module_path.to_string(), symbol.symbol.clone()))
        {
            return Ok(TypeRefIr::PackageSchema {
                package_id: package_id.clone(),
                stable_schema_key: stable_schema_key.clone(),
                package_schema_type_id: package_schema_type_id.clone(),
            });
        }
        return Ok(TypeRefIr::ServiceSymbol {
            symbol: exact_public_type_symbol(file_ir_units, module_path, type_index)?,
        });
    }
    let recurse = |ty: &TypeRefIr| {
        normalize_local_type(
            owner_module,
            ty,
            file_ir_units,
            public_type_ids,
            resolved_package_schemas,
        )
    };
    Ok(match ty {
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: args.iter().map(recurse).collect::<Result<_, _>>()?,
        },
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base: normalize_public_nominal_base(owner_module, base, file_ir_units)?,
            arguments: arguments.iter().map(recurse).collect::<Result<_, _>>()?,
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field)| Ok((name.clone(), recurse(field)?)))
                .collect::<Result<_, String>>()?,
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items.iter().map(recurse).collect::<Result<_, _>>()?,
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(recurse(inner)?),
        },
        TypeRefIr::AnyInterface { interface } => {
            let identity = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).map_err(
                |error| format!("public signature any-interface identity is invalid: {error}"),
            )?;
            let identity = recurse(&identity)?;
            TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id: type_ref_abi_key(&identity),
                    canonical_type_args: interface
                        .canonical_type_args
                        .iter()
                        .map(recurse)
                        .collect::<Result<_, _>>()?,
                },
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| {
                    Ok(skiff_artifact_model::FunctionTypeParamIr {
                        name: param.name.clone(),
                        ty: recurse(&param.ty)?,
                    })
                })
                .collect::<Result<_, String>>()?,
            return_type: Box::new(recurse(return_type)?),
        },
        TypeRefIr::ServiceSymbol { symbol } => public_type_ids
            .get(&(symbol.module_path.clone(), symbol.symbol.clone()))
            .and_then(contract_schema_type)
            .unwrap_or_else(|| ty.clone()),
        TypeRefIr::PackageSymbol { symbol } => {
            normalize_package_symbol(symbol, resolved_package_schemas).unwrap_or_else(|| ty.clone())
        }
        TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => ty.clone(),
        TypeRefIr::LocalType { .. } | TypeRefIr::PublicationType { .. } => {
            unreachable!("direct nominal references returned before recursive normalization")
        }
    })
}

fn normalize_public_nominal_base(
    owner_module: &str,
    base: &NominalTypeRefBaseIr,
    file_ir_units: &[FileIrUnit],
) -> Result<NominalTypeRefBaseIr, String> {
    let source = match base {
        NominalTypeRefBaseIr::LocalType { type_index } => Some((owner_module, *type_index)),
        NominalTypeRefBaseIr::PublicationType {
            module_path,
            type_index,
        } => Some((module_path.as_str(), *type_index)),
        _ => None,
    };
    let Some((module_path, type_index)) = source else {
        return Ok(base.clone());
    };
    Ok(NominalTypeRefBaseIr::ServiceSymbol {
        symbol: exact_public_type_symbol(file_ir_units, module_path, type_index)?,
    })
}

fn contract_schema_type(ty: &ContractTypeRef) -> Option<TypeRefIr> {
    let ContractTypeRef::PackageSchema {
        package_id,
        stable_schema_key,
        package_schema_type_id,
    } = ty
    else {
        return None;
    };
    Some(TypeRefIr::PackageSchema {
        package_id: package_id.clone(),
        stable_schema_key: stable_schema_key.clone(),
        package_schema_type_id: package_schema_type_id.clone(),
    })
}

fn normalize_package_symbol(
    symbol: &PackageSymbolRef,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Option<TypeRefIr> {
    let mut matches = resolved_package_schemas
        .iter()
        .filter(|schema| match &symbol.package {
            PackageRefIr::Dependency { dependency_ref } => schema.alias() == dependency_ref,
            PackageRefIr::PackageId { package_id } => schema.package_id() == package_id,
        });
    let schema = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let (package_schema_type_id, record) = schema.public_type(&symbol.symbol_path)?;
    Some(TypeRefIr::PackageSchema {
        package_id: record.package_id.clone(),
        stable_schema_key: record.stable_schema_key.clone(),
        package_schema_type_id: package_schema_type_id.clone(),
    })
}

fn lift_local_type(ty: TypeRefIr) -> Result<PackageTypeRef, String> {
    Ok(match ty {
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        },
        TypeRefIr::Builtin { name, args } if !args.is_empty() => PackageTypeRef::Container {
            name,
            arguments: args
                .into_iter()
                .map(lift_local_type)
                .collect::<Result<_, _>>()?,
        },
        TypeRefIr::Nullable { inner } => PackageTypeRef::Nullable {
            inner: Box::new(lift_local_type(*inner)?),
        },
        TypeRefIr::AnyInterface { interface } => {
            let interface_type = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                .map_err(|error| {
                    format!("public signature any-interface identity is invalid: {error}")
                })?;
            PackageTypeRef::AnyInterface {
                interface: Box::new(lift_local_type(interface_type)?),
                arguments: interface
                    .canonical_type_args
                    .into_iter()
                    .map(lift_local_type)
                    .collect::<Result<_, _>>()?,
            }
        }
        local_type => PackageTypeRef::Local { local_type },
    })
}

fn nominal_source<'a>(owner_module: &'a str, ty: &'a TypeRefIr) -> Option<(&'a str, u32)> {
    match ty {
        TypeRefIr::LocalType { type_index } => Some((owner_module, *type_index)),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => Some((module_path, *type_index)),
        _ => None,
    }
}

fn exact_public_type_symbol(
    file_ir_units: &[FileIrUnit],
    module_path: &str,
    type_index: u32,
) -> Result<ServiceSymbolRef, String> {
    let symbol = exact_type_symbol(file_ir_units, module_path, type_index)?;
    let unit = exact_module_unit(file_ir_units, module_path, type_index)?;
    let link = unit.link_targets.types.get(&symbol.symbol).ok_or_else(|| {
        format!(
            "public signature type {module_path}#{type_index} `{}` is private or nonexported",
            symbol.symbol
        )
    })?;
    if link.type_index != type_index {
        return Err(format!(
            "public signature type {module_path}#{type_index} `{}` has wrong exported owner slot {}",
            symbol.symbol, link.type_index
        ));
    }
    Ok(symbol)
}

fn exact_type_symbol(
    file_ir_units: &[FileIrUnit],
    module_path: &str,
    type_index: u32,
) -> Result<ServiceSymbolRef, String> {
    let unit = exact_module_unit(file_ir_units, module_path, type_index)?;
    let declaration = unit.type_table.get(type_index as usize).ok_or_else(|| {
        format!("public signature type {module_path}#{type_index} has no type-table entry")
    })?;
    Ok(ServiceSymbolRef {
        module_path: module_path.to_string(),
        symbol: declaration.name.clone(),
    })
}

fn exact_module_unit<'a>(
    file_ir_units: &'a [FileIrUnit],
    module_path: &str,
    type_index: u32,
) -> Result<&'a FileIrUnit, String> {
    let mut units = file_ir_units
        .iter()
        .filter(|unit| unit.module_path == module_path);
    let unit = units.next().ok_or_else(|| {
        format!("public signature type {module_path}#{type_index} has no source module")
    })?;
    if units.next().is_some() {
        return Err(format!(
            "public signature type {module_path}#{type_index} has ambiguous source modules"
        ));
    }
    Ok(unit)
}

fn provenance_sort_key(origin: &ValueProvenance) -> (u8, String) {
    match origin {
        ValueProvenance::Fresh => (0, String::new()),
        ValueProvenance::Constant => (1, String::new()),
        ValueProvenance::CallerParameter { index } => (2, format!("{index:010}")),
        ValueProvenance::CallerParameterProjection { index, path } => {
            let mut key = format!("{index:010}:");
            for step in path.steps() {
                match step {
                    ValueProjectionStep::Field { name } => {
                        key.push_str(&format!("f{}:{name};", name.len()));
                    }
                    ValueProjectionStep::ContainerElement {} => key.push_str("e;"),
                }
            }
            (3, key)
        }
        ValueProvenance::DependencyReturn { callable_id } => (4, callable_id.clone()),
    }
}

#[cfg(test)]
mod tests;
