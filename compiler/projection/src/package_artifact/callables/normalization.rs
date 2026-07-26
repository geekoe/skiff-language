use std::collections::BTreeMap;

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
) {
    for parameter in &mut signature.parameters {
        parameter.ty =
            normalize_package_type(owner_module, &parameter.ty, file_ir_units, public_type_ids);
    }
    signature.return_type = normalize_package_type(
        owner_module,
        &signature.return_type,
        file_ir_units,
        public_type_ids,
    );
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
) -> PackageTypeRef {
    match ty {
        PackageTypeRef::Local { local_type } => {
            let local_type =
                normalize_local_type(owner_module, local_type, file_ir_units, public_type_ids);
            lift_local_type(local_type)
        }
        PackageTypeRef::Container { name, arguments } => PackageTypeRef::Container {
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(|argument| {
                    normalize_package_type(owner_module, argument, file_ir_units, public_type_ids)
                })
                .collect(),
        },
        PackageTypeRef::Nullable { inner } => PackageTypeRef::Nullable {
            inner: Box::new(normalize_package_type(
                owner_module,
                inner,
                file_ir_units,
                public_type_ids,
            )),
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
            )),
            arguments: arguments
                .iter()
                .map(|argument| {
                    normalize_package_type(owner_module, argument, file_ir_units, public_type_ids)
                })
                .collect(),
        },
        exact @ PackageTypeRef::PackageSchema { .. } => exact.clone(),
    }
}

fn normalize_local_type(
    owner_module: &str,
    ty: &TypeRefIr,
    file_ir_units: &[FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
) -> TypeRefIr {
    if let Some((module_path, type_index)) = nominal_source(owner_module, ty) {
        if let Some(binding_name) = local_type_binding(file_ir_units, module_path, type_index) {
            if let Some(exact) =
                public_type_ids.get(&(module_path.to_string(), binding_name.to_string()))
            {
                if let ContractTypeRef::PackageSchema {
                    package_id,
                    stable_schema_key,
                    package_schema_type_id,
                } = exact
                {
                    return TypeRefIr::PackageSchema {
                        package_id: package_id.clone(),
                        stable_schema_key: stable_schema_key.clone(),
                        package_schema_type_id: package_schema_type_id.clone(),
                    };
                }
            }
        }
        return ty.clone();
    }
    match ty {
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| normalize_local_type(owner_module, arg, file_ir_units, public_type_ids))
                .collect(),
        },
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base: normalize_public_nominal_base(owner_module, base, file_ir_units),
            arguments: arguments
                .iter()
                .map(|argument| {
                    normalize_local_type(owner_module, argument, file_ir_units, public_type_ids)
                })
                .collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field)| {
                    (
                        name.clone(),
                        normalize_local_type(owner_module, field, file_ir_units, public_type_ids),
                    )
                })
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| {
                    normalize_local_type(owner_module, item, file_ir_units, public_type_ids)
                })
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(normalize_local_type(
                owner_module,
                inner,
                file_ir_units,
                public_type_ids,
            )),
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| skiff_artifact_model::FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: normalize_local_type(
                        owner_module,
                        &param.ty,
                        file_ir_units,
                        public_type_ids,
                    ),
                })
                .collect(),
            return_type: Box::new(normalize_local_type(
                owner_module,
                return_type,
                file_ir_units,
                public_type_ids,
            )),
        },
        _ => ty.clone(),
    }
}

fn normalize_public_nominal_base(
    owner_module: &str,
    base: &NominalTypeRefBaseIr,
    file_ir_units: &[FileIrUnit],
) -> NominalTypeRefBaseIr {
    let source = match base {
        NominalTypeRefBaseIr::LocalType { type_index } => Some((owner_module, *type_index)),
        NominalTypeRefBaseIr::PublicationType {
            module_path,
            type_index,
        } => Some((module_path.as_str(), *type_index)),
        _ => None,
    };
    let Some((module_path, type_index)) = source else {
        return base.clone();
    };
    let Some(symbol) = local_type_binding(file_ir_units, module_path, type_index) else {
        return base.clone();
    };
    NominalTypeRefBaseIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
        },
    }
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

fn local_type_binding<'a>(
    file_ir_units: &'a [FileIrUnit],
    module_path: &str,
    type_index: u32,
) -> Option<&'a str> {
    let mut units = file_ir_units
        .iter()
        .filter(|unit| unit.module_path == module_path);
    let unit = units.next()?;
    if units.next().is_some() {
        return None;
    }
    let mut declarations = unit
        .declarations
        .types
        .iter()
        .filter(|(_, declaration)| declaration.type_index == type_index);
    let (binding_name, _) = declarations.next()?;
    if declarations.next().is_some() {
        return None;
    }
    Some(binding_name)
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
        CallableEffectSummary, PackageCallableParameter, PackageSchemaTypeId, TypeDeclarationIr,
    };

    fn fixture() -> (Vec<FileIrUnit>, BTreeMap<(String, String), ContractTypeRef>) {
        let mut unit = FileIrUnit::empty("api", "source-hash");
        unit.declarations.types.insert(
            "PublicError".into(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "PublicError".into(),
                source_span: None,
            },
        );
        unit.declarations.types.insert(
            "PrivateDetail".into(),
            TypeDeclarationIr {
                type_index: 1,
                symbol: "PrivateDetail".into(),
                source_span: None,
            },
        );
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

        normalize_public_signature("api", &mut signature, &units, &refs);

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
    fn private_or_unresolved_local_nominal_remains_local_only() {
        let (units, refs) = fixture();
        let private = PackageTypeRef::Local {
            local_type: TypeRefIr::LocalType { type_index: 1 },
        };
        assert_eq!(
            normalize_package_type("api", &private, &units, &refs),
            private
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
            normalize_package_type("api", &applied, &units, &refs),
            PackageTypeRef::Local {
                local_type: TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: "api".to_string(),
                            symbol: "PrivateDetail".to_string(),
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
}
