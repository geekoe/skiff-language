use skiff_artifact_model::{
    ContractTypeRef, PackageTypeRef, ServiceContract, ServiceSymbolRef, TypeRefIr,
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
            .then(|| package_type_assignable(&actual, &expected)),
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
            .then(|| package_type_assignable(&actual, &expected)),
    )
}

fn package_type_ref_from_resolved_ir(
    ty: &TypeRefIr,
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> Result<PackageTypeRef, String> {
    match ty {
        TypeRefIr::Native { name, args } => Ok(PackageTypeRef::Container {
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
        TypeRefIr::ServiceSymbol { symbol }
            if dependency_analysis
                .contract_requirement(&symbol.module_path)
                .is_ok() =>
        {
            Err(format!(
                "resolved contract symbol `{}.{}` has no source-origin exact projection",
                symbol.module_path, symbol.symbol
            ))
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

fn resolved_ir_contains_contract_symbol(
    ty: &TypeRefIr,
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> bool {
    match ty {
        TypeRefIr::ServiceSymbol { symbol } => dependency_analysis
            .contract_requirement(&symbol.module_path)
            .is_ok(),
        TypeRefIr::Native { args, .. } | TypeRefIr::Union { items: args } => args
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
            PackageTypeRef::Contract {
                contract_type_id: actual,
            },
            PackageTypeRef::Contract {
                contract_type_id: expected,
            },
        ) => actual == expected,
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
    contract: &ServiceContract,
) -> Result<ResolvedTypeRef, String> {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => {
            let arguments = arguments
                .iter()
                .map(|argument| resolved_contract_type(argument, alias, contract))
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
                ir: TypeRefIr::Native {
                    name: name.clone(),
                    args: arguments.into_iter().map(|argument| argument.ir).collect(),
                },
            })
        }
        ContractTypeRef::Contract { contract_type_id } => {
            let schema_type = contract
                .boundary_schema
                .get(contract_type_id)
                .ok_or_else(|| {
                    format!("validated contract is missing boundary type `{contract_type_id}`")
                })?;
            Ok(ResolvedTypeRef {
                source_text: format!("{alias}.{}", schema_type.stable_key),
                ir: TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: alias.to_string(),
                        symbol: schema_type.stable_key.clone(),
                    },
                },
            })
        }
        ContractTypeRef::TypeParam { name } => Ok(ResolvedTypeRef {
            source_text: name.clone(),
            ir: TypeRefIr::TypeParam { name: name.clone() },
        }),
        ContractTypeRef::Nullable { inner } => {
            let inner = resolved_contract_type(inner, alias, contract)?;
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
