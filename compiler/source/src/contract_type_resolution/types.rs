use skiff_artifact_model::{ContractTypeRef, PackageTypeRef, TypeRefIr};

use crate::{
    shared::ast::TypeRef, SourceDependencyAnalysisInput, TypeResolutionContext, TypeResolutionModel,
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
        let resolved = self
            .type_resolution
            .resolve_type_ref(ty, context)
            .map_err(|error| format!("cannot resolve source type `{}`: {error}", ty.name))?;
        self.resolve_expanded_ir(&resolved.ir)
    }

    fn resolve_expanded_ir(&self, ty: &TypeRefIr) -> Result<PackageTypeRef, String> {
        match ty {
            TypeRefIr::Builtin { name, args } => Ok(PackageTypeRef::Container {
                name: name.clone(),
                arguments: args
                    .iter()
                    .map(|arg| self.resolve_expanded_ir(arg))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            TypeRefIr::Nullable { inner } => Ok(PackageTypeRef::Nullable {
                inner: Box::new(self.resolve_expanded_ir(inner)?),
            }),
            TypeRefIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => Ok(PackageTypeRef::PackageSchema {
                package_id: package_id.clone(),
                stable_schema_key: stable_schema_key.clone(),
                package_schema_type_id: package_schema_type_id.clone(),
            }),
            TypeRefIr::PackageSymbol { symbol } => {
                let record = match &symbol.package {
                    skiff_artifact_model::PackageRefIr::Dependency { dependency_ref } => self
                        .dependency_analysis
                        .direct_package_type(dependency_ref, &symbol.symbol_path),
                    skiff_artifact_model::PackageRefIr::PackageId { package_id } => self
                        .dependency_analysis
                        .package_type_by_owner_and_stable_key(package_id, &symbol.symbol_path),
                };
                match record {
                    Some(record) => Ok(PackageTypeRef::PackageSchema {
                        package_id: record.package_id.clone(),
                        stable_schema_key: record.stable_schema_key.clone(),
                        package_schema_type_id: record.package_schema_type_id.clone(),
                    }),
                    None => Ok(PackageTypeRef::Local {
                        local_type: ty.clone(),
                    }),
                }
            }
            TypeRefIr::AnyInterface { interface } => {
                let identity: TypeRefIr = serde_json::from_str(&interface.interface_abi_id)
                    .map_err(|error| {
                        format!(
                            "invalid interface ABI identity {}: {error}",
                            interface.interface_abi_id
                        )
                    })?;
                Ok(PackageTypeRef::AnyInterface {
                    interface: Box::new(self.resolve_expanded_ir(&identity)?),
                    arguments: interface
                        .canonical_type_args
                        .iter()
                        .map(|arg| self.resolve_expanded_ir(arg))
                        .collect::<Result<Vec<_>, _>>()?,
                })
            }
            TypeRefIr::AppliedNominal { .. } => Ok(PackageTypeRef::Local {
                local_type: self.normalize_local_ir(ty)?,
            }),
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Record { .. }
            | TypeRefIr::Union { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. }
            | TypeRefIr::Function { .. } => Ok(PackageTypeRef::Local {
                local_type: self.normalize_local_ir(ty)?,
            }),
        }
    }

    fn normalize_local_ir(&self, ty: &TypeRefIr) -> Result<TypeRefIr, String> {
        match ty {
            TypeRefIr::Builtin { name, args } => Ok(TypeRefIr::Builtin {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| self.normalize_local_ir(arg))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            TypeRefIr::PackageSymbol { symbol } => {
                let record = match &symbol.package {
                    skiff_artifact_model::PackageRefIr::Dependency { dependency_ref } => self
                        .dependency_analysis
                        .direct_package_type(dependency_ref, &symbol.symbol_path),
                    skiff_artifact_model::PackageRefIr::PackageId { package_id } => self
                        .dependency_analysis
                        .package_type_by_owner_and_stable_key(package_id, &symbol.symbol_path),
                };
                Ok(match record {
                    Some(record) => TypeRefIr::PackageSchema {
                        package_id: record.package_id.clone(),
                        stable_schema_key: record.stable_schema_key.clone(),
                        package_schema_type_id: record.package_schema_type_id.clone(),
                    },
                    None => ty.clone(),
                })
            }
            TypeRefIr::AppliedNominal { base, arguments } => Ok(TypeRefIr::AppliedNominal {
                base: base.clone(),
                arguments: arguments
                    .iter()
                    .map(|argument| self.normalize_local_ir(argument))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            TypeRefIr::Record { fields } => Ok(TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, field)| Ok((name.clone(), self.normalize_local_ir(field)?)))
                    .collect::<Result<_, String>>()?,
            }),
            TypeRefIr::Union { items } => Ok(TypeRefIr::Union {
                items: items
                    .iter()
                    .map(|item| self.normalize_local_ir(item))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            TypeRefIr::Nullable { inner } => Ok(TypeRefIr::Nullable {
                inner: Box::new(self.normalize_local_ir(inner)?),
            }),
            TypeRefIr::AnyInterface { interface } => {
                let identity: TypeRefIr = serde_json::from_str(&interface.interface_abi_id)
                    .map_err(|error| {
                        format!(
                            "invalid interface ABI identity {}: {error}",
                            interface.interface_abi_id
                        )
                    })?;
                let identity = self.normalize_local_ir(&identity)?;
                let args = interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| self.normalize_local_ir(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(TypeRefIr::AnyInterface {
                    interface: skiff_artifact_identity::interface_instantiation_ref(identity, args),
                })
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => Ok(TypeRefIr::Function {
                params: params
                    .iter()
                    .map(|param| {
                        Ok(skiff_artifact_model::FunctionTypeParamIr {
                            name: param.name.clone(),
                            ty: self.normalize_local_ir(&param.ty)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                return_type: Box::new(self.normalize_local_ir(return_type)?),
            }),
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSchema { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. } => Ok(ty.clone()),
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
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            package_type_contains_contract(interface)
                || arguments.iter().any(package_type_contains_contract)
        }
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
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => Ok(PackageTypeRef::AnyInterface {
            interface: Box::new(package_type_ref_from_validated_contract_ref(interface)?),
            arguments: arguments
                .iter()
                .map(package_type_ref_from_validated_contract_ref)
                .collect::<Result<_, _>>()?,
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
