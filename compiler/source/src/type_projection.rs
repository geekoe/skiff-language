use skiff_artifact_model::{NominalTypeRefBaseIr, PackageRefIr, PackageTypeRef, TypeRefIr};

use crate::SourceDependencyAnalysisInput;

/// Projects a canonical `TypeRefIr` back into the ABI/wire `PackageTypeRef`
/// subset view. This is a lossy, context-dependent projection:
///
/// - `ServiceSymbol` lookup failure returns `Err`;
/// - `PackageSymbol` lookup failure falls back to `PackageTypeRef::Local`;
/// - `Record`/`Union`/`Function` embedding a contract nominal returns `Err`
///   (they have no exact `PackageTypeRef` representation).
pub fn package_type_ref_from_ir(
    ty: &TypeRefIr,
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> Result<PackageTypeRef, String> {
    match ty {
        TypeRefIr::Builtin { name, args } => Ok(PackageTypeRef::Container {
            name: name.clone(),
            arguments: args
                .iter()
                .map(|argument| package_type_ref_from_ir(argument, dependency_analysis))
                .collect::<Result<Vec<_>, _>>()?,
        }),
        TypeRefIr::Nullable { inner } => Ok(PackageTypeRef::Nullable {
            inner: Box::new(package_type_ref_from_ir(inner, dependency_analysis)?),
        }),
        TypeRefIr::AnyInterface { interface } => Ok(PackageTypeRef::AnyInterface {
            interface: Box::new(package_type_ref_from_ir(
                &serde_json::from_str(&interface.interface_abi_id).map_err(|error| {
                    format!("invalid canonical interface identity: {error}")
                })?,
                dependency_analysis,
            )?),
            arguments: interface
                .canonical_type_args
                .iter()
                .map(|argument| package_type_ref_from_ir(argument, dependency_analysis))
                .collect::<Result<_, _>>()?,
        }),
        TypeRefIr::ServiceSymbol { symbol }
            if dependency_analysis
                .contract_requirement(&symbol.module_path)
                .is_ok() =>
        {
            let record = dependency_analysis
                .public_package_type_by_stable_key(&symbol.module_path, &symbol.symbol)
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
        | TypeRefIr::Function { .. }
            if resolved_ir_contains_contract_symbol(ty, dependency_analysis) =>
        {
            Err(
                "resolved inline type embeds a contract nominal but has no exact PackageTypeRef representation"
                    .to_string(),
            )
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
        TypeRefIr::AppliedNominal { base, arguments } => {
            matches!(
                base,
                NominalTypeRefBaseIr::ServiceSymbol { symbol }
                    if dependency_analysis
                        .contract_requirement(&symbol.module_path)
                        .is_ok()
            ) || arguments
                .iter()
                .any(|argument| resolved_ir_contains_contract_symbol(argument, dependency_analysis))
        }
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
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}

#[cfg(test)]
mod tests;
