use std::collections::BTreeMap;

use skiff_artifact_model::{FileIrUnit, NominalTypeRefBaseIr, ServiceSymbolRef, TypeRefIr};
use skiff_compiler_projection_input::{
    canonical_package_public_path, DuplicateProjectionPackageCallableSignature,
    ProjectionPackageCallableKey, ProjectionPackageCallableSignatureFacts,
};
use skiff_compiler_source::{PackageSourceModel, SourceInterfaceConformanceKey, SourceSymbolKey};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProjectionInputBuildError {
    #[error("source callable signature `{public_path}` has no package API binding")]
    MissingApiBinding { public_path: String },
    #[error(
        "public-instance callable signature `{public_path}` cannot resolve receiver {source_module}.{source_symbol}"
    )]
    MissingPublicInstanceReceiver {
        public_path: String,
        source_module: String,
        source_symbol: String,
    },
    #[error(
        "public-instance callable signature `{public_path}` has no unique validated implementation for {receiver_module}.{receiver_symbol}.{method}"
    )]
    MissingValidatedPublicInstanceMethod {
        public_path: String,
        receiver_module: String,
        receiver_symbol: String,
        method: String,
    },
    #[error(
        "public instance `{public_path}` receiver {receiver_module}.{receiver_symbol} has no validated conformance to {interface_module}.{interface_symbol}"
    )]
    MissingValidatedPublicInstanceConformance {
        public_path: String,
        receiver_module: String,
        receiver_symbol: String,
        interface_module: String,
        interface_symbol: String,
    },
    #[error(
        "source callable signature `{public_path}` targets missing File IR module `{module_path}`"
    )]
    MissingModule {
        public_path: String,
        module_path: String,
    },
    #[error(
        "source callable signature `{public_path}` targets missing File IR executable `{module_path}.{source_symbol}`"
    )]
    MissingExecutable {
        public_path: String,
        module_path: String,
        source_symbol: String,
    },
    #[error(transparent)]
    DuplicateSignature(#[from] DuplicateProjectionPackageCallableSignature),
}

pub(crate) fn build_package_callable_signatures(
    model: &PackageSourceModel,
    file_ir_units: &[FileIrUnit],
    package_id: &str,
) -> Result<ProjectionPackageCallableSignatureFacts, ProjectionInputBuildError> {
    let units_by_module = file_ir_units
        .iter()
        .map(|unit| (unit.module_path.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let entries = model
        .callable_signatures()
        .iter()
        .map(|(public_path, signature)| {
            let (module_path, executable_index) =
                package_callable_target(model, &units_by_module, public_path)?;
            Ok((
                ProjectionPackageCallableKey::new(
                    canonical_package_public_path(package_id, public_path),
                    module_path,
                    executable_index,
                ),
                signature.clone(),
            ))
        })
        .collect::<Result<Vec<_>, ProjectionInputBuildError>>()?;
    Ok(ProjectionPackageCallableSignatureFacts::try_from_entries(
        entries,
    )?)
}

fn package_callable_target(
    model: &PackageSourceModel,
    units_by_module: &BTreeMap<&str, &FileIrUnit>,
    public_path: &str,
) -> Result<(String, u32), ProjectionInputBuildError> {
    if let Some(binding) = model.export_bindings().public_callables().get(public_path) {
        return executable_target(
            units_by_module,
            public_path,
            &binding.source_module,
            &binding.source_symbol,
        );
    }

    let (instance, method_name) = model
        .export_bindings()
        .public_instances()
        .values()
        .find_map(|instance| {
            public_path
                .strip_prefix(&format!("{}.", instance.public_path))
                .filter(|method_name| !method_name.contains('.'))
                .map(|method_name| (instance, method_name))
        })
        .ok_or_else(|| ProjectionInputBuildError::MissingApiBinding {
            public_path: public_path.to_string(),
        })?;
    let receiver = resolve_public_instance_receiver_symbol(
        units_by_module,
        &instance.source_module,
        &instance.source_symbol,
    )
    .ok_or_else(
        || ProjectionInputBuildError::MissingPublicInstanceReceiver {
            public_path: public_path.to_string(),
            source_module: instance.source_module.clone(),
            source_symbol: instance.source_symbol.clone(),
        },
    )?;
    let receiver_key = SourceSymbolKey::new(&receiver.module_path, &receiver.symbol);
    let validated_methods = instance
        .interfaces
        .iter()
        .filter_map(|interface| {
            model.interface_signatures().validated_method(
                &SourceInterfaceConformanceKey {
                    receiver: receiver_key.clone(),
                    interface: SourceSymbolKey::new(
                        &interface.source_module,
                        &interface.source_symbol,
                    ),
                },
                method_name,
            )
        })
        .collect::<Vec<_>>();
    let [validated_method] = validated_methods.as_slice() else {
        return Err(
            ProjectionInputBuildError::MissingValidatedPublicInstanceMethod {
                public_path: public_path.to_string(),
                receiver_module: receiver.module_path,
                receiver_symbol: receiver.symbol,
                method: method_name.to_string(),
            },
        );
    };
    executable_target(
        units_by_module,
        public_path,
        validated_method.executable.module_path(),
        validated_method.executable.symbol(),
    )
}

fn executable_target(
    units_by_module: &BTreeMap<&str, &FileIrUnit>,
    public_path: &str,
    module_path: &str,
    source_symbol: &str,
) -> Result<(String, u32), ProjectionInputBuildError> {
    let unit = units_by_module.get(module_path).ok_or_else(|| {
        ProjectionInputBuildError::MissingModule {
            public_path: public_path.to_string(),
            module_path: module_path.to_string(),
        }
    })?;
    let declaration = unit
        .declarations
        .executables
        .get(source_symbol)
        .ok_or_else(|| ProjectionInputBuildError::MissingExecutable {
            public_path: public_path.to_string(),
            module_path: module_path.to_string(),
            source_symbol: source_symbol.to_string(),
        })?;
    Ok((module_path.to_string(), declaration.executable_index))
}

pub(crate) fn resolve_public_instance_receiver_symbol(
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    const_module: &str,
    const_symbol: &str,
) -> Option<ServiceSymbolRef> {
    let unit = file_units_by_module.get(const_module).copied()?;
    let const_decl = unit.declarations.constants.get(const_symbol)?;
    let constant = unit.constants.get(const_decl.const_index as usize)?;
    package_nominal_service_symbol(file_units_by_module, const_module, &constant.ty)
}

fn package_nominal_service_symbol(
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    module_path: &str,
    ty: &TypeRefIr,
) -> Option<ServiceSymbolRef> {
    match ty {
        TypeRefIr::LocalType { type_index } => {
            let unit = file_units_by_module.get(module_path).copied()?;
            let decl = unit.type_table.get(*type_index as usize)?;
            Some(ServiceSymbolRef {
                module_path: module_path.to_string(),
                symbol: decl.name.clone(),
            })
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => {
            let unit = file_units_by_module.get(module_path.as_str()).copied()?;
            let decl = unit.type_table.get(*type_index as usize)?;
            Some(ServiceSymbolRef {
                module_path: module_path.clone(),
                symbol: decl.name.clone(),
            })
        }
        TypeRefIr::ServiceSymbol { symbol } => {
            let unit = file_units_by_module
                .get(symbol.module_path.as_str())
                .copied()?;
            unit.declarations.types.get(&symbol.symbol)?;
            Some(symbol.clone())
        }
        TypeRefIr::AppliedNominal { base, .. } => match base {
            NominalTypeRefBaseIr::LocalType { type_index } => {
                let unit = file_units_by_module.get(module_path).copied()?;
                let decl = unit.type_table.get(*type_index as usize)?;
                Some(ServiceSymbolRef {
                    module_path: module_path.to_string(),
                    symbol: decl.name.clone(),
                })
            }
            NominalTypeRefBaseIr::PublicationType {
                module_path,
                type_index,
            } => {
                let unit = file_units_by_module.get(module_path.as_str()).copied()?;
                let decl = unit.type_table.get(*type_index as usize)?;
                Some(ServiceSymbolRef {
                    module_path: module_path.clone(),
                    symbol: decl.name.clone(),
                })
            }
            NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
                let unit = file_units_by_module
                    .get(symbol.module_path.as_str())
                    .copied()?;
                unit.declarations.types.get(&symbol.symbol)?;
                Some(symbol.clone())
            }
            NominalTypeRefBaseIr::PackageSymbol { .. }
            | NominalTypeRefBaseIr::PackageSchema { .. } => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests;
