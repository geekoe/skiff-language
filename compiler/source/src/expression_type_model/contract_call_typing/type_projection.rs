use skiff_artifact_model::{
    ContractTypeRef, PackageRefIr, PackageSymbolRef, PackageTypeRef, TypeRefIr,
};

pub(super) use crate::contract_type_resolution::package_type_contains_contract;

use crate::{
    contract_type_resolution::package_type_ref_from_source_type, shared::ast::TypeRef,
    ResolvedTypeRef, SourceDependencyAnalysisInput, TypeResolutionContext, TypeResolutionModel,
};

pub(super) struct ContractCallTypeProjection<'a, 'ctx> {
    type_resolution: &'a TypeResolutionModel,
    dependency_analysis: &'a SourceDependencyAnalysisInput,
    type_context: &'a TypeResolutionContext<'ctx>,
}

impl<'a, 'ctx> ContractCallTypeProjection<'a, 'ctx> {
    pub(super) fn new(
        type_resolution: &'a TypeResolutionModel,
        dependency_analysis: &'a SourceDependencyAnalysisInput,
        type_context: &'a TypeResolutionContext<'ctx>,
    ) -> Self {
        Self {
            type_resolution,
            dependency_analysis,
            type_context,
        }
    }

    pub(super) fn try_source_package_type_ref(
        &self,
        ty: &TypeRef,
    ) -> Result<PackageTypeRef, String> {
        package_type_ref_from_source_type(
            ty,
            self.type_context,
            self.type_resolution,
            self.dependency_analysis,
        )
    }

    pub(super) fn try_resolved_package_type(
        &self,
        ty: &ResolvedTypeRef,
    ) -> Result<PackageTypeRef, String> {
        package_type_ref_from_resolved_ir(&ty.ir, self.dependency_analysis)
    }
}

pub(crate) fn contract_source_assignability(
    actual: &ResolvedTypeRef,
    actual_projected: Option<&PackageTypeRef>,
    expected: &ResolvedTypeRef,
    type_resolution: &TypeResolutionModel,
    dependency_analysis: Option<&SourceDependencyAnalysisInput>,
    type_context: &TypeResolutionContext<'_>,
) -> Result<Option<bool>, String> {
    let Some(dependency_analysis) = dependency_analysis else {
        return Ok(None);
    };
    let projection =
        ContractCallTypeProjection::new(type_resolution, dependency_analysis, type_context);
    let actual = match actual_projected {
        Some(actual) => actual.clone(),
        None => projection.try_resolved_package_type(actual)?,
    };
    let expected = projection.try_resolved_package_type(expected)?;
    Ok(
        (package_type_contains_contract(&actual) || package_type_contains_contract(&expected))
            .then(|| package_type_target_assignable(&actual, &expected, dependency_analysis)),
    )
}

pub(crate) fn contract_source_assignability_with_projections(
    actual: &ResolvedTypeRef,
    actual_projected: Option<&PackageTypeRef>,
    expected: &ResolvedTypeRef,
    expected_projected: Option<&PackageTypeRef>,
    type_resolution: &TypeResolutionModel,
    dependency_analysis: Option<&SourceDependencyAnalysisInput>,
    type_context: &TypeResolutionContext<'_>,
) -> Result<Option<bool>, String> {
    let Some(dependency_analysis) = dependency_analysis else {
        return Ok(None);
    };
    let projection =
        ContractCallTypeProjection::new(type_resolution, dependency_analysis, type_context);
    let actual = match actual_projected {
        Some(actual) => actual.clone(),
        None => projection.try_resolved_package_type(actual)?,
    };
    let expected = match expected_projected {
        Some(expected) => expected.clone(),
        None => projection.try_resolved_package_type(expected)?,
    };
    Ok(
        (package_type_contains_contract(&actual) || package_type_contains_contract(&expected))
            .then(|| package_type_target_assignable(&actual, &expected, dependency_analysis)),
    )
}

fn package_type_ref_from_resolved_ir(
    ty: &TypeRefIr,
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> Result<PackageTypeRef, String> {
    match ty {
        TypeRefIr::Builtin { name, args } => Ok(PackageTypeRef::Container {
            name: name.clone(),
            arguments: args
                .iter()
                .map(|argument| {
                    package_type_ref_from_resolved_ir(argument, dependency_analysis)
                })
                .collect::<Result<Vec<_>, _>>()?,
        }),
        TypeRefIr::Nullable { inner } => Ok(PackageTypeRef::Nullable {
            inner: Box::new(package_type_ref_from_resolved_ir(
                inner,
                dependency_analysis,
            )?),
        }),
        TypeRefIr::ServiceSymbol { symbol } if dependency_analysis
            .contract_requirement(&symbol.module_path)
            .is_ok() => {
            let record = dependency_analysis
                .public_package_type_by_stable_key(
                    &symbol.module_path,
                    &symbol.symbol,
                )
                .map_err(|error| error.to_string())?;
            Ok(PackageTypeRef::PackageSchema {
                package_id: record.package_id.clone(),
                stable_schema_key: record.stable_schema_key.clone(),
                package_schema_type_id: record.package_schema_type_id.clone(),
            })
        }
        TypeRefIr::PackageSymbol { symbol } => {
            let record = match &symbol.package {
                PackageRefIr::PackageId { package_id } => dependency_analysis
                    .package_type_by_owner_and_stable_key(package_id, &symbol.symbol_path),
                PackageRefIr::Dependency { dependency_ref } => dependency_analysis
                    .direct_package_type(dependency_ref, &symbol.symbol_path),
            };
            let Some(record) = record else {
                return Ok(PackageTypeRef::Local {
                    local_type: ty.clone(),
                });
            };
            Ok(PackageTypeRef::PackageSchema {
                package_id: record.package_id.clone(),
                stable_schema_key: record.stable_schema_key.clone(),
                package_schema_type_id: record.package_schema_type_id.clone(),
            })
        }
        TypeRefIr::Record { .. }
        | TypeRefIr::Union { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. }
            if resolved_ir_contains_contract_symbol(ty, dependency_analysis) =>
        {
            Err("resolved inline type embeds a contract nominal but has no exact PackageTypeRef representation"
                .to_string())
        }
        _ => Ok(PackageTypeRef::Local {
            local_type: ty.clone(),
        }),
    }
}

pub(crate) fn package_type_ref_from_contract_type(ty: &ContractTypeRef) -> PackageTypeRef {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => PackageTypeRef::Container {
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(package_type_ref_from_contract_type)
                .collect(),
        },
        ContractTypeRef::Nullable { inner } => PackageTypeRef::Nullable {
            inner: Box::new(package_type_ref_from_contract_type(inner)),
        },
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => PackageTypeRef::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        },
        ContractTypeRef::TypeParam { name } => PackageTypeRef::Local {
            local_type: TypeRefIr::TypeParam { name: name.clone() },
        },
        ContractTypeRef::Record { fields } => PackageTypeRef::Local {
            local_type: TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), contract_type_ref_to_ir(ty)))
                    .collect(),
            },
        },
        ContractTypeRef::StructuralUnion { variants } => PackageTypeRef::Local {
            local_type: TypeRefIr::Union {
                items: variants.iter().map(contract_type_ref_to_ir).collect(),
            },
        },
        ContractTypeRef::Literal { value } => PackageTypeRef::Local {
            local_type: TypeRefIr::Literal {
                value: match value {
                    skiff_artifact_model::ContractLiteral::String { value } => {
                        skiff_artifact_model::LiteralIr::String {
                            value: value.clone(),
                        }
                    }
                },
            },
        },
    }
}

fn contract_type_ref_to_ir(ty: &ContractTypeRef) -> TypeRefIr {
    match package_type_ref_from_contract_type(ty) {
        PackageTypeRef::Local { local_type } => local_type,
        PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId { package_id },
                symbol_path: stable_schema_key,
                abi_expectation: None,
            },
        },
        PackageTypeRef::Container { name, arguments } => TypeRefIr::Builtin {
            name,
            args: arguments
                .iter()
                .map(|argument| contract_type_ref_to_ir_from_package(argument.clone()))
                .collect(),
        },
        PackageTypeRef::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(contract_type_ref_to_ir_from_package(*inner)),
        },
    }
}

fn contract_type_ref_to_ir_from_package(ty: PackageTypeRef) -> TypeRefIr {
    match ty {
        PackageTypeRef::Local { local_type } => local_type,
        PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId { package_id },
                symbol_path: stable_schema_key,
                abi_expectation: None,
            },
        },
        PackageTypeRef::Container { name, arguments } => TypeRefIr::Builtin {
            name,
            args: arguments
                .into_iter()
                .map(contract_type_ref_to_ir_from_package)
                .collect(),
        },
        PackageTypeRef::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(contract_type_ref_to_ir_from_package(*inner)),
        },
    }
}

fn resolved_ir_contains_contract_symbol(
    ty: &TypeRefIr,
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> bool {
    match ty {
        TypeRefIr::ServiceSymbol { symbol } => dependency_analysis
            .contract_requirement(&symbol.module_path)
            .is_ok(),
        TypeRefIr::Builtin { args, .. } | TypeRefIr::Union { items: args } => args
            .iter()
            .any(|argument| resolved_ir_contains_contract_symbol(argument, dependency_analysis)),
        TypeRefIr::Record { fields } => fields
            .values()
            .any(|field| resolved_ir_contains_contract_symbol(field, dependency_analysis)),
        TypeRefIr::Nullable { inner } => {
            resolved_ir_contains_contract_symbol(inner, dependency_analysis)
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params.iter().any(|parameter| {
                resolved_ir_contains_contract_symbol(&parameter.ty, dependency_analysis)
            }) || resolved_ir_contains_contract_symbol(return_type, dependency_analysis)
        }
        TypeRefIr::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .any(|argument| resolved_ir_contains_contract_symbol(argument, dependency_analysis)),
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}

pub(super) fn package_type_assignable(actual: &PackageTypeRef, expected: &PackageTypeRef) -> bool {
    match (actual, expected) {
        (
            PackageTypeRef::PackageSchema {
                package_id: actual_package,
                stable_schema_key: actual_key,
                package_schema_type_id: actual_id,
            },
            PackageTypeRef::PackageSchema {
                package_id: expected_package,
                stable_schema_key: expected_key,
                package_schema_type_id: expected_id,
            },
        ) => {
            actual_package == expected_package
                && actual_key == expected_key
                && actual_id == expected_id
        }
        (
            PackageTypeRef::Container {
                name: actual_name,
                arguments: actual_arguments,
            },
            PackageTypeRef::Container {
                name: expected_name,
                arguments: expected_arguments,
            },
        ) => {
            actual_name == expected_name
                && actual_arguments.len() == expected_arguments.len()
                && actual_arguments
                    .iter()
                    .zip(expected_arguments)
                    .all(|(actual, expected)| package_type_assignable(actual, expected))
        }
        (
            PackageTypeRef::Nullable { inner: actual },
            PackageTypeRef::Nullable { inner: expected },
        ) => package_type_assignable(actual, expected),
        (actual, PackageTypeRef::Nullable { inner: expected }) => {
            package_type_is_null(actual) || package_type_assignable(actual, expected)
        }
        (
            PackageTypeRef::Local { local_type: actual },
            PackageTypeRef::Local {
                local_type: expected,
            },
        ) => actual == expected,
        _ => false,
    }
}

pub(super) fn package_type_target_assignable(
    actual: &PackageTypeRef,
    expected: &PackageTypeRef,
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> bool {
    if package_type_assignable(actual, expected) {
        return true;
    }
    match (actual, expected) {
        (PackageTypeRef::PackageSchema { .. }, PackageTypeRef::PackageSchema { .. }) => false,
        (
            _,
            PackageTypeRef::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            },
        ) => dependency_analysis
            .exact_package_type(package_id, stable_schema_key, package_schema_type_id)
            .and_then(package_schema_representation)
            .is_some_and(|representation| {
                package_type_target_assignable(actual, &representation, dependency_analysis)
            }),
        (
            PackageTypeRef::Container {
                name: actual_name,
                arguments: actual_arguments,
            },
            PackageTypeRef::Container {
                name: expected_name,
                arguments: expected_arguments,
            },
        ) => {
            actual_name == expected_name
                && actual_arguments.len() == expected_arguments.len()
                && actual_arguments
                    .iter()
                    .zip(expected_arguments)
                    .all(|(actual, expected)| {
                        package_type_target_assignable(actual, expected, dependency_analysis)
                    })
        }
        (
            PackageTypeRef::Nullable { inner: actual },
            PackageTypeRef::Nullable { inner: expected },
        ) => package_type_target_assignable(actual, expected, dependency_analysis),
        (actual, PackageTypeRef::Nullable { inner: expected }) => {
            package_type_is_null(actual)
                || package_type_target_assignable(actual, expected, dependency_analysis)
        }
        (
            PackageTypeRef::Local { local_type: actual },
            PackageTypeRef::Local {
                local_type: expected,
            },
        ) => local_ir_target_assignable(actual, expected),
        (
            PackageTypeRef::Local { local_type: actual },
            PackageTypeRef::Container { name, arguments },
        ) => local_ir_target_assignable(
            actual,
            &TypeRefIr::Builtin {
                name: name.clone(),
                args: arguments
                    .iter()
                    .cloned()
                    .map(contract_type_ref_to_ir_from_package)
                    .collect(),
            },
        ),
        _ => false,
    }
}

fn local_ir_target_assignable(actual: &TypeRefIr, expected: &TypeRefIr) -> bool {
    if actual == expected {
        return true;
    }
    match (actual, expected) {
        (_, TypeRefIr::Union { items }) => items
            .iter()
            .any(|candidate| local_ir_target_assignable(actual, candidate)),
        (TypeRefIr::Union { items }, _) => items
            .iter()
            .all(|candidate| local_ir_target_assignable(candidate, expected)),
        (
            TypeRefIr::Record {
                fields: actual_fields,
            },
            TypeRefIr::Record {
                fields: expected_fields,
            },
        ) => {
            actual_fields.len() == expected_fields.len()
                && expected_fields.iter().all(|(name, expected)| {
                    actual_fields
                        .get(name)
                        .is_some_and(|actual| local_ir_target_assignable(actual, expected))
                })
        }
        (
            TypeRefIr::Builtin {
                name: actual_name,
                args: actual_args,
            },
            TypeRefIr::Builtin {
                name: expected_name,
                args: expected_args,
            },
        ) => {
            actual_name == expected_name
                && actual_args.len() == expected_args.len()
                && actual_args
                    .iter()
                    .zip(expected_args)
                    .all(|(actual, expected)| local_ir_target_assignable(actual, expected))
        }
        (actual, TypeRefIr::Nullable { inner }) => {
            matches!(
                actual,
                TypeRefIr::Literal {
                    value: skiff_artifact_model::LiteralIr::Null
                }
            ) || local_ir_target_assignable(actual, inner)
        }
        (
            TypeRefIr::Literal {
                value: skiff_artifact_model::LiteralIr::String { .. },
            },
            TypeRefIr::Builtin { name, args },
        ) => name == "string" && args.is_empty(),
        (
            TypeRefIr::Literal {
                value: skiff_artifact_model::LiteralIr::Bool { .. },
            },
            TypeRefIr::Builtin { name, args },
        ) => name == "bool" && args.is_empty(),
        (
            TypeRefIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number { .. },
            },
            TypeRefIr::Builtin { name, args },
        ) => name == "number" && args.is_empty(),
        _ => false,
    }
}

fn package_schema_representation(
    record: &skiff_artifact_model::PackageSchemaTypeRecord,
) -> Option<PackageTypeRef> {
    use skiff_artifact_model::ContractTypeDescriptor;
    match &record.canonical_descriptor.descriptor {
        ContractTypeDescriptor::Alias { target }
        | ContractTypeDescriptor::Representation { target } => {
            Some(package_type_ref_from_contract_type(target))
        }
        ContractTypeDescriptor::Enumeration { variants } => Some(PackageTypeRef::Local {
            local_type: TypeRefIr::Union {
                items: variants
                    .iter()
                    .map(|value| TypeRefIr::Literal {
                        value: skiff_artifact_model::LiteralIr::String {
                            value: value.clone(),
                        },
                    })
                    .collect(),
            },
        }),
        ContractTypeDescriptor::StructuralUnion { variants } => Some(PackageTypeRef::Local {
            local_type: TypeRefIr::Union {
                items: variants.iter().map(contract_type_ref_to_ir).collect(),
            },
        }),
        ContractTypeDescriptor::Record { fields } => Some(PackageTypeRef::Local {
            local_type: TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), contract_type_ref_to_ir(ty)))
                    .collect(),
            },
        }),
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => {
            Some(PackageTypeRef::Local {
                local_type: TypeRefIr::Union {
                    items: branches
                        .iter()
                        .map(|branch| contract_type_ref_to_ir(&branch.branch_type))
                        .collect(),
                },
            })
        }
        ContractTypeDescriptor::CallbackInterface { .. } => None,
    }
}

fn package_type_is_null(ty: &PackageTypeRef) -> bool {
    matches!(
        ty,
        PackageTypeRef::Container { name, arguments }
            if name == "null" && arguments.is_empty()
    ) || matches!(
        ty,
        PackageTypeRef::Local {
            local_type: TypeRefIr::Literal {
                value: skiff_artifact_model::LiteralIr::Null,
            },
        }
    )
}

pub(super) fn resolved_contract_type(
    ty: &ContractTypeRef,
    alias: &str,
) -> Result<ResolvedTypeRef, String> {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => {
            let arguments = arguments
                .iter()
                .map(|argument| resolved_contract_type(argument, alias))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ResolvedTypeRef {
                source_text: if arguments.is_empty() {
                    name.clone()
                } else {
                    format!(
                        "{name}<{}>",
                        arguments
                            .iter()
                            .map(|argument| argument.source_text.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
                ir: TypeRefIr::Builtin {
                    name: name.clone(),
                    args: arguments.into_iter().map(|argument| argument.ir).collect(),
                },
            })
        }
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => Ok(ResolvedTypeRef {
            source_text: format!("{alias}.{stable_schema_key}"),
            ir: TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: package_id.clone(),
                    },
                    symbol_path: stable_schema_key.clone(),
                    abi_expectation: None,
                },
            },
        }),
        ContractTypeRef::TypeParam { name } => Ok(ResolvedTypeRef {
            source_text: name.clone(),
            ir: TypeRefIr::TypeParam { name: name.clone() },
        }),
        ContractTypeRef::Nullable { inner } => {
            let inner = resolved_contract_type(inner, alias)?;
            Ok(ResolvedTypeRef {
                source_text: format!("{}?", inner.source_text),
                ir: TypeRefIr::Nullable {
                    inner: Box::new(inner.ir),
                },
            })
        }
        ContractTypeRef::Record { .. } => {
            Err("inline contract record has no exact source type representation".to_string())
        }
        ContractTypeRef::StructuralUnion { .. } => Err(
            "inline contract structural union has no exact source type representation".to_string(),
        ),
        ContractTypeRef::Literal { .. } => {
            Err("inline contract literal has no exact source type representation".to_string())
        }
    }
}
