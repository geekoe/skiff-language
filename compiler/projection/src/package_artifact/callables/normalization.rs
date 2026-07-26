use std::collections::BTreeMap;

use skiff_artifact_identity::type_ref_abi_key;
use skiff_artifact_model::{
    CallableProvenanceSummary, CallableSemanticFacts, ContractTypeRef, FileIrUnit,
    FunctionTypeParamIr, InterfaceInstantiationRef, NamedUnionBranchIr, NominalTypeRefBaseIr,
    PackageCallableSignature, PackageRefIr, PackageSymbolRef, PackageTypeRef, ServiceSymbolRef,
    TypeDescriptorIr, TypeRefIr, ValueProjectionStep, ValueProvenance,
};

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
) -> Result<(), String> {
    for parameter in &mut signature.parameters {
        parameter.ty =
            normalize_package_type(owner_module, &parameter.ty, file_ir_units, public_type_ids)?;
    }
    signature.return_type = normalize_package_type(
        owner_module,
        &signature.return_type,
        file_ir_units,
        public_type_ids,
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
                    interface_abi_id: serde_json::to_string(&identity)
                        .map_err(|error| error.to_string())?,
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
) -> Result<PackageTypeRef, String> {
    Ok(match ty {
        PackageTypeRef::Local { local_type } => {
            let local_type =
                normalize_local_type(owner_module, local_type, file_ir_units, public_type_ids)?;
            lift_local_type(local_type)
        }
        PackageTypeRef::Container { name, arguments } => PackageTypeRef::Container {
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(|argument| {
                    normalize_package_type(owner_module, argument, file_ir_units, public_type_ids)
                })
                .collect::<Result<_, _>>()?,
        },
        PackageTypeRef::Nullable { inner } => PackageTypeRef::Nullable {
            inner: Box::new(normalize_package_type(
                owner_module,
                inner,
                file_ir_units,
                public_type_ids,
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
            )?),
            arguments: arguments
                .iter()
                .map(|argument| {
                    normalize_package_type(owner_module, argument, file_ir_units, public_type_ids)
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
    let recurse =
        |ty: &TypeRefIr| normalize_local_type(owner_module, ty, file_ir_units, public_type_ids);
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
        TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
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

fn lift_local_type(ty: TypeRefIr) -> PackageTypeRef {
    match ty {
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
            arguments: args.into_iter().map(lift_local_type).collect(),
        },
        TypeRefIr::Nullable { inner } => PackageTypeRef::Nullable {
            inner: Box::new(lift_local_type(*inner)),
        },
        local_type => PackageTypeRef::Local { local_type },
    }
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
mod tests {
    use super::*;
    use skiff_artifact_model::{
        CallableEffectSummary, PackageCallableParameter, PackageSchemaTypeId, TypeDeclIr,
        TypeDeclarationIr, TypeLinkTargetIr,
    };

    fn fixture() -> (Vec<FileIrUnit>, BTreeMap<(String, String), ContractTypeRef>) {
        let mut unit = FileIrUnit::empty("api", "source-hash");
        unit.type_table
            .extend(
                ["PublicError", "LocalHandle", "PrivateDetail"].map(|name| TypeDeclIr {
                    name: name.into(),
                    descriptor: TypeDescriptorIr::Record {
                        fields: BTreeMap::new(),
                    },
                    type_params: Vec::new(),
                    implements: Vec::new(),
                    source_span: None,
                }),
            );
        unit.declarations.types.insert(
            "PublicError".into(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "PublicError".into(),
                source_span: None,
            },
        );
        unit.declarations.types.insert(
            "LocalHandle".into(),
            TypeDeclarationIr {
                type_index: 1,
                symbol: "LocalHandle".into(),
                source_span: None,
            },
        );
        unit.declarations.types.insert(
            "PrivateDetail".into(),
            TypeDeclarationIr {
                type_index: 2,
                symbol: "PrivateDetail".into(),
                source_span: None,
            },
        );
        unit.link_targets
            .types
            .insert("PublicError".into(), TypeLinkTargetIr { type_index: 0 });
        unit.link_targets
            .types
            .insert("LocalHandle".into(), TypeLinkTargetIr { type_index: 1 });
        let exact = ContractTypeRef::package_schema(
            "example.pkg",
            "errors.PublicError",
            PackageSchemaTypeId::new("schema:public-error"),
        );
        (
            vec![unit],
            BTreeMap::from([(("api".into(), "PublicError".into()), exact)]),
        )
    }

    #[test]
    fn public_nominals_are_exact_through_parameters_and_return() {
        let (units, refs) = fixture();
        let nested = PackageTypeRef::Local {
            local_type: TypeRefIr::Builtin {
                name: "Array".into(),
                args: vec![TypeRefIr::Nullable {
                    inner: Box::new(TypeRefIr::LocalType { type_index: 0 }),
                }],
            },
        };
        let mut signature = PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![PackageCallableParameter {
                name: "values".into(),
                ty: nested,
            }],
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::PublicationType {
                    module_path: "api".into(),
                    type_index: 0,
                },
            },
            may_suspend: false,
        };

        normalize_public_signature("api", &mut signature, &units, &refs).unwrap();

        let exact = PackageTypeRef::PackageSchema {
            package_id: "example.pkg".into(),
            stable_schema_key: "errors.PublicError".into(),
            package_schema_type_id: PackageSchemaTypeId::new("schema:public-error"),
        };
        assert_eq!(signature.return_type, exact);
        assert_eq!(
            signature.parameters[0].ty,
            PackageTypeRef::Container {
                name: "Array".into(),
                arguments: vec![PackageTypeRef::Nullable {
                    inner: Box::new(exact),
                }],
            }
        );
    }

    #[test]
    fn private_or_unresolved_local_nominal_is_rejected() {
        let (units, refs) = fixture();
        let private = PackageTypeRef::Local {
            local_type: TypeRefIr::LocalType { type_index: 2 },
        };
        let error = normalize_package_type("api", &private, &units, &refs).unwrap_err();
        assert!(
            error.contains("PrivateDetail") && error.contains("private or nonexported"),
            "{error}"
        );
    }

    #[test]
    fn package_schema_promotion_precedes_service_symbol_export_validation() {
        let (mut units, refs) = fixture();
        units[0].link_targets.types.remove("PublicError");
        assert_eq!(
            normalize_package_type(
                "api",
                &PackageTypeRef::Local {
                    local_type: TypeRefIr::LocalType { type_index: 0 },
                },
                &units,
                &refs,
            )
            .unwrap(),
            PackageTypeRef::PackageSchema {
                package_id: "example.pkg".into(),
                stable_schema_key: "errors.PublicError".into(),
                package_schema_type_id: PackageSchemaTypeId::new("schema:public-error"),
            }
        );
    }

    #[test]
    fn public_signature_normalization_preserves_applied_wrapper_and_normalizes_arguments() {
        let (units, refs) = fixture();
        let applied = PackageTypeRef::Local {
            local_type: TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
                arguments: vec![TypeRefIr::LocalType { type_index: 0 }],
            },
        };

        assert_eq!(
            normalize_package_type("api", &applied, &units, &refs).unwrap(),
            PackageTypeRef::Local {
                local_type: TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: "api".to_string(),
                            symbol: "LocalHandle".to_string(),
                        },
                    },
                    arguments: vec![TypeRefIr::PackageSchema {
                        package_id: "example.pkg".into(),
                        stable_schema_key: "errors.PublicError".into(),
                        package_schema_type_id: PackageSchemaTypeId::new("schema:public-error"),
                    }],
                },
            }
        );
    }

    #[test]
    fn public_signature_normalization_covers_every_nested_package_and_local_shape() {
        let (units, refs) = fixture();
        let local_handle = TypeRefIr::LocalType { type_index: 1 };
        let schema_type = TypeRefIr::LocalType { type_index: 0 };
        let inner_any_interface = TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: type_ref_abi_key(&local_handle),
                canonical_type_args: vec![schema_type.clone()],
            },
        };
        let nested_function = TypeRefIr::Function {
            params: vec![
                FunctionTypeParamIr {
                    name: "builtin".into(),
                    ty: TypeRefIr::Builtin {
                        name: "Array".into(),
                        args: vec![local_handle.clone()],
                    },
                },
                FunctionTypeParamIr {
                    name: "record".into(),
                    ty: TypeRefIr::Record {
                        fields: BTreeMap::from([(
                            "choice".into(),
                            TypeRefIr::Union {
                                items: vec![
                                    TypeRefIr::PublicationType {
                                        module_path: "api".into(),
                                        type_index: 1,
                                    },
                                    TypeRefIr::Nullable {
                                        inner: Box::new(schema_type.clone()),
                                    },
                                ],
                            },
                        )]),
                    },
                },
                FunctionTypeParamIr {
                    name: "applied".into(),
                    ty: TypeRefIr::AppliedNominal {
                        base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
                        arguments: vec![schema_type],
                    },
                },
                FunctionTypeParamIr {
                    name: "existential".into(),
                    ty: inner_any_interface,
                },
            ],
            return_type: Box::new(local_handle.clone()),
        };
        let mut signature = PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![
                PackageCallableParameter {
                    name: "direct".into(),
                    ty: PackageTypeRef::Local {
                        local_type: local_handle.clone(),
                    },
                },
                PackageCallableParameter {
                    name: "nested".into(),
                    ty: PackageTypeRef::Container {
                        name: "Envelope".into(),
                        arguments: vec![PackageTypeRef::Nullable {
                            inner: Box::new(PackageTypeRef::AnyInterface {
                                interface: Box::new(PackageTypeRef::Local {
                                    local_type: local_handle.clone(),
                                }),
                                arguments: vec![PackageTypeRef::Local {
                                    local_type: nested_function,
                                }],
                            }),
                        }],
                    },
                },
            ],
            return_type: PackageTypeRef::Local {
                local_type: local_handle,
            },
            may_suspend: false,
        };

        normalize_public_signature("api", &mut signature, &units, &refs).unwrap();

        let value = serde_json::to_value(&signature).unwrap();
        assert_eq!(count_json_kind(&value, "localType"), 0);
        assert_eq!(count_json_kind(&value, "publicationType"), 0);
        for required in [
            "container",
            "nullable",
            "anyInterface",
            "builtin",
            "record",
            "union",
            "function",
            "appliedNominal",
            "serviceSymbol",
            "packageSchema",
        ] {
            assert!(
                count_json_kind(&value, required) > 0,
                "normalized signature lost required `{required}` shape: {value}"
            );
        }
        let exact_handle = PackageTypeRef::Local {
            local_type: TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "api".into(),
                    symbol: "LocalHandle".into(),
                },
            },
        };
        assert_eq!(signature.parameters[0].ty, exact_handle);
        assert_eq!(signature.return_type, exact_handle);
    }

    #[test]
    fn public_signature_uses_source_module_slots_not_public_display_paths() {
        let (mut units, mut refs) = fixture();
        let mut display = FileIrUnit::empty("public.api", "display-source-hash");
        display.type_table.extend([
            TypeDeclIr {
                name: "Unused".into(),
                descriptor: TypeDescriptorIr::Record {
                    fields: BTreeMap::new(),
                },
                type_params: Vec::new(),
                implements: Vec::new(),
                source_span: None,
            },
            TypeDeclIr {
                name: "DisplayHandle".into(),
                descriptor: TypeDescriptorIr::Record {
                    fields: BTreeMap::new(),
                },
                type_params: Vec::new(),
                implements: Vec::new(),
                source_span: None,
            },
        ]);
        display
            .link_targets
            .types
            .insert("DisplayHandle".into(), TypeLinkTargetIr { type_index: 1 });
        units.push(display);
        refs.insert(
            ("public.api".into(), "DisplayHandle".into()),
            ContractTypeRef::package_schema(
                "wrong.pkg",
                "DisplayHandle",
                PackageSchemaTypeId::new("schema:wrong-display"),
            ),
        );

        assert_eq!(
            normalize_package_type(
                "api",
                &PackageTypeRef::Local {
                    local_type: TypeRefIr::LocalType { type_index: 1 },
                },
                &units,
                &refs,
            )
            .unwrap(),
            PackageTypeRef::Local {
                local_type: TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "api".into(),
                        symbol: "LocalHandle".into(),
                    },
                },
            }
        );

        refs.remove(&("public.api".into(), "DisplayHandle".into()));
        assert_eq!(
            normalize_package_type(
                "api",
                &PackageTypeRef::Local {
                    local_type: TypeRefIr::PublicationType {
                        module_path: "public.api".into(),
                        type_index: 1,
                    },
                },
                &units,
                &refs,
            )
            .unwrap(),
            PackageTypeRef::Local {
                local_type: TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "public.api".into(),
                        symbol: "DisplayHandle".into(),
                    },
                },
            }
        );
    }

    #[test]
    fn public_signature_owner_resolution_failures_are_closed() {
        let (units, refs) = fixture();
        let normalize = |owner_module: &str, type_index: u32, units: &[FileIrUnit]| {
            normalize_package_type(
                owner_module,
                &PackageTypeRef::Local {
                    local_type: TypeRefIr::LocalType { type_index },
                },
                units,
                &refs,
            )
            .unwrap_err()
        };

        let missing_module = normalize("wrong.owner", 1, &units);
        assert!(
            missing_module.contains("has no source module"),
            "{missing_module}"
        );
        let missing_slot = normalize("api", 99, &units);
        assert!(
            missing_slot.contains("has no type-table entry"),
            "{missing_slot}"
        );

        let mut missing_symbol = units.clone();
        missing_symbol[0].type_table[1].name = "MissingSymbol".into();
        let missing_symbol = normalize("api", 1, &missing_symbol);
        assert!(
            missing_symbol.contains("MissingSymbol")
                && missing_symbol.contains("private or nonexported"),
            "{missing_symbol}"
        );

        let mut wrong_slot = units.clone();
        wrong_slot[0]
            .link_targets
            .types
            .get_mut("LocalHandle")
            .unwrap()
            .type_index = 0;
        let wrong_slot = normalize("api", 1, &wrong_slot);
        assert!(
            wrong_slot.contains("wrong exported owner slot"),
            "{wrong_slot}"
        );

        let mut ambiguous = units.clone();
        ambiguous.push(units[0].clone());
        let ambiguous = normalize("api", 1, &ambiguous);
        assert!(
            ambiguous.contains("ambiguous source modules"),
            "{ambiguous}"
        );
    }

    #[test]
    fn implementation_normalization_preserves_applied_owner_and_ordered_arguments() {
        let mut unit = FileIrUnit::empty("api", "source-hash");
        unit.declarations.types.insert(
            "Box".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "api.Box".to_string(),
                source_span: None,
            },
        );
        let units = vec![unit];
        let applied = |argument| TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
            arguments: vec![argument],
        };

        let string_box = normalize_implementation_type(
            "example.pkg",
            "api",
            &applied(TypeRefIr::builtin("string")),
            &units,
        )
        .unwrap();
        let number_box = normalize_implementation_type(
            "example.pkg",
            "api",
            &applied(TypeRefIr::builtin("number")),
            &units,
        )
        .unwrap();

        assert_ne!(string_box, number_box);
        assert_ne!(
            skiff_artifact_identity::type_ref_abi_key(&string_box),
            skiff_artifact_identity::type_ref_abi_key(&number_box)
        );
        assert_eq!(
            string_box,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: "example.pkg".to_string(),
                        },
                        symbol_path: "api.Box".to_string(),
                        abi_expectation: None,
                    },
                },
                arguments: vec![TypeRefIr::builtin("string")],
            }
        );
    }

    #[test]
    fn same_symbol_path_from_distinct_package_owners_does_not_merge() {
        let applied = |package_id: &str| TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: package_id.to_string(),
                    },
                    symbol_path: "models.Box".to_string(),
                    abi_expectation: Some("abi:shared".to_string()),
                },
            },
            arguments: vec![TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: format!("{package_id}/nested"),
                        },
                        symbol_path: "models.Value".to_string(),
                        abi_expectation: Some("abi:nested-shared".to_string()),
                    },
                },
                arguments: vec![TypeRefIr::builtin("string")],
            }],
        };

        let first =
            normalize_implementation_type("consumer", "api", &applied("example.one"), &[]).unwrap();
        let second =
            normalize_implementation_type("consumer", "api", &applied("example.two"), &[]).unwrap();

        assert_ne!(first, second);
        assert_eq!(first, applied("example.one"));
        assert_eq!(second, applied("example.two"));
    }

    #[test]
    fn reachable_and_direct_return_origins_are_normalized_independently() {
        let field = ValueProvenance::CallerParameterProjection {
            index: 1,
            path: skiff_artifact_model::ValueProjectionPath::field("payload").unwrap(),
        };
        let element = ValueProvenance::CallerParameterProjection {
            index: 1,
            path: skiff_artifact_model::ValueProjectionPath::container_element(),
        };
        let mut facts = CallableSemanticFacts {
            effects: CallableEffectSummary::analysis_pending(),
            provenance: CallableProvenanceSummary::Analyzed {
                return_origins: vec![
                    field.clone(),
                    ValueProvenance::Fresh,
                    field.clone(),
                    element.clone(),
                ],
                direct_return_origins: vec![
                    ValueProvenance::DependencyReturn {
                        callable_id: "pkg-callable:z".into(),
                    },
                    ValueProvenance::Constant,
                    element.clone(),
                    ValueProvenance::Fresh,
                    ValueProvenance::Constant,
                ],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            },
            resolved_call_targets: BTreeMap::new(),
        };

        facts = normalize_semantic_facts(facts);
        let CallableProvenanceSummary::Analyzed {
            return_origins,
            direct_return_origins,
            ..
        } = facts.provenance
        else {
            panic!("fixture provenance must remain analyzed")
        };
        assert_eq!(
            return_origins,
            vec![ValueProvenance::Fresh, element.clone(), field]
        );
        assert_eq!(
            direct_return_origins,
            vec![
                ValueProvenance::Fresh,
                ValueProvenance::Constant,
                element,
                ValueProvenance::DependencyReturn {
                    callable_id: "pkg-callable:z".into(),
                },
            ]
        );
    }

    fn count_json_kind(value: &serde_json::Value, expected: &str) -> usize {
        match value {
            serde_json::Value::Array(items) => items
                .iter()
                .map(|item| count_json_kind(item, expected))
                .sum(),
            serde_json::Value::Object(fields) => {
                usize::from(
                    fields.get("kind").and_then(serde_json::Value::as_str) == Some(expected),
                ) + fields
                    .values()
                    .map(|field| count_json_kind(field, expected))
                    .sum::<usize>()
            }
            _ => 0,
        }
    }
}
