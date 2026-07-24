use skiff_artifact_model::{ContractTypeRef, PackageTypeRef, TypeRefIr};

use crate::{
    shared::{ast::TypeRef, type_expr::TypeExpr},
    SourceDependencyAnalysisInput, TypeResolutionContext, TypeResolutionModel,
};

pub(super) struct ContractAwareTypeResolver<'a> {
    type_resolution: &'a TypeResolutionModel,
    dependency_analysis: &'a SourceDependencyAnalysisInput,
}

impl ContractAwareTypeResolver<'_> {
    pub(super) fn new<'a>(
        type_resolution: &'a TypeResolutionModel,
        dependency_analysis: &'a SourceDependencyAnalysisInput,
    ) -> ContractAwareTypeResolver<'a> {
        ContractAwareTypeResolver {
            type_resolution,
            dependency_analysis,
        }
    }

    pub(super) fn resolve_source_type_ref(
        &self,
        ty: &TypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<PackageTypeRef, String> {
        // Existing source alias expansion remains authoritative for local and
        // package aliases. The expanded syntax is then resolved again here so
        // a contract nominal can never disappear into TypeRefIr.
        let expanded = self
            .type_resolution
            .resolve_type_ref(ty, context)
            .map_err(|error| format!("cannot resolve source type `{}`: {error}", ty.name))?
            .source_text;
        self.resolve_expanded_expr(&TypeExpr::parse(&expanded), context)
    }

    fn resolve_expanded_expr(
        &self,
        expr: &TypeExpr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<PackageTypeRef, String> {
        match expr {
            TypeExpr::Named { name, args } => {
                if let Some((alias, stable_key)) = name.split_once('.') {
                    if self.dependency_analysis.contract_requirement(alias).is_ok() {
                        if stable_key.is_empty() {
                            return Err(format!(
                                "contract dependency type `{name}` has no stable type key"
                            ));
                        }
                        let record = self
                            .dependency_analysis
                            .public_package_type_by_stable_key(alias, stable_key)
                            .map_err(|error| error.to_string())?;
                        return Ok(PackageTypeRef::PackageSchema {
                            package_id: record.package_id.clone(),
                            stable_schema_key: record.stable_schema_key.clone(),
                            package_schema_type_id: record.package_schema_type_id.clone(),
                        });
                    }
                }

                let text = expr.to_type_string();
                let resolved = self
                    .type_resolution
                    .resolve_type_text(&text, context)
                    .map_err(|error| format!("cannot resolve source type `{text}`: {error}"))?;
                if let TypeRefIr::PackageSymbol { symbol } = &resolved.ir {
                    if let skiff_artifact_model::PackageRefIr::Dependency { dependency_ref } =
                        &symbol.package
                    {
                        if let Some(record) = self
                            .dependency_analysis
                            .direct_package_type(dependency_ref, &symbol.symbol_path)
                        {
                            return Ok(PackageTypeRef::PackageSchema {
                                package_id: record.package_id.clone(),
                                stable_schema_key: record.stable_schema_key.clone(),
                                package_schema_type_id: record.package_schema_type_id.clone(),
                            });
                        }
                    }
                }
                if let TypeRefIr::Builtin { name, .. } = resolved.ir {
                    let arguments = args
                        .iter()
                        .map(|arg| self.resolve_expanded_expr(arg, context))
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(PackageTypeRef::Container { name, arguments });
                }

                let arguments = args
                    .iter()
                    .map(|arg| self.resolve_expanded_expr(arg, context))
                    .collect::<Result<Vec<_>, _>>()?;
                if arguments.iter().any(package_type_contains_contract) {
                    return Err(format!(
                        "source type `{text}` embeds a contract nominal in a non-container generic type with no exact PackageTypeRef representation"
                    ));
                }
                Ok(PackageTypeRef::Local {
                    local_type: resolved.ir,
                })
            }
            TypeExpr::Nullable(inner) => Ok(PackageTypeRef::Nullable {
                inner: Box::new(self.resolve_expanded_expr(inner, context)?),
            }),
            TypeExpr::EmptyRecord
            | TypeExpr::StringLiteral(_)
            | TypeExpr::AnyInterface { .. }
            | TypeExpr::Union(_)
            | TypeExpr::Record(_)
            | TypeExpr::Function { .. } => {
                if self.inline_expr_contains_contract(expr, context)? {
                    return Err(format!(
                        "inline source type `{}` embeds a contract nominal but has no exact PackageTypeRef representation",
                        expr.to_type_string()
                    ));
                }
                let text = expr.to_type_string();
                let local_type = self
                    .type_resolution
                    .resolve_type_text(&text, context)
                    .map_err(|error| format!("cannot resolve source type `{text}`: {error}"))?
                    .ir;
                Ok(PackageTypeRef::Local { local_type })
            }
        }
    }

    fn inline_expr_contains_contract(
        &self,
        expr: &TypeExpr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<bool, String> {
        match expr {
            TypeExpr::Named { .. } | TypeExpr::Nullable(_) => self
                .resolve_expanded_expr(expr, context)
                .map(|resolved| package_type_contains_contract(&resolved)),
            TypeExpr::AnyInterface { interface } => {
                self.inline_expr_contains_contract(interface, context)
            }
            TypeExpr::Union(items) => {
                for item in items {
                    if self.inline_expr_contains_contract(item, context)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            TypeExpr::Record(fields) => {
                for field in fields {
                    if self.inline_expr_contains_contract(&field.ty, context)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            TypeExpr::Function {
                params,
                return_type,
            } => {
                for parameter in params {
                    if self.inline_expr_contains_contract(&parameter.ty, context)? {
                        return Ok(true);
                    }
                }
                self.inline_expr_contains_contract(return_type, context)
            }
            TypeExpr::EmptyRecord | TypeExpr::StringLiteral(_) => Ok(false),
        }
    }
}

pub(crate) fn package_type_contains_contract(ty: &PackageTypeRef) -> bool {
    match ty {
        PackageTypeRef::PackageSchema { .. } => true,
        PackageTypeRef::Container { arguments, .. } => {
            arguments.iter().any(package_type_contains_contract)
        }
        PackageTypeRef::Nullable { inner } => package_type_contains_contract(inner),
        PackageTypeRef::Local { .. } => false,
    }
}

/// Converts only type refs taken from an already validated contract. Inline
/// contract shapes have no source name and no lossless PackageTypeRef terminal
/// in this phase, so they fail closed instead of becoming a local/display type.
pub(super) fn package_type_ref_from_validated_contract_ref(
    ty: &ContractTypeRef,
) -> Result<PackageTypeRef, String> {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => Ok(PackageTypeRef::Container {
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(package_type_ref_from_validated_contract_ref)
                .collect::<Result<Vec<_>, _>>()?,
        }),
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(PackageTypeRef::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        }),
        ContractTypeRef::TypeParam { name } => Ok(PackageTypeRef::Local {
            local_type: TypeRefIr::TypeParam { name: name.clone() },
        }),
        ContractTypeRef::Nullable { inner } => Ok(PackageTypeRef::Nullable {
            inner: Box::new(package_type_ref_from_validated_contract_ref(inner)?),
        }),
        ContractTypeRef::Record { .. } => {
            Err("inline contract record has no exact PackageTypeRef representation".to_string())
        }
        ContractTypeRef::StructuralUnion { .. } => Err(
            "inline contract structural union has no exact PackageTypeRef representation"
                .to_string(),
        ),
        ContractTypeRef::Literal { .. } => {
            Err("inline contract literal has no exact PackageTypeRef representation".to_string())
        }
    }
}
