mod public_instances;

#[cfg(test)]
pub(super) use public_instances::package_public_instance_method_operation;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    ConstExport, ConstIr, ExecutableExport, ExecutableIr, FileIrUnit, PackageExportIndex,
    PackageRequirement, TypeExport,
};
use skiff_compiler_core::{
    id::SKIFF_STD_PUBLICATION_ID,
    package_interface_methods::{package_interface_method_signatures, PackageTypeSymbolIndex},
};

use crate::{
    error::ProjectionError,
    package_artifact::visible_types::{
        package_type_names_from_file_units, projection_visible_executable_signature,
        projection_visible_interface_method_signature, projection_visible_type_descriptor,
        projection_visible_type_ref,
    },
};

use super::{assets::file_ir_refs_from_units, model::PackageExportLinkProjectionInput};

pub(super) fn project_package_export_index(
    package: &PackageExportLinkProjectionInput<'_>,
    dependencies: &[PackageRequirement],
) -> Result<PackageExportIndex, ProjectionError> {
    let files_by_module = file_ir_refs_from_units(package.file_ir_units)
        .into_iter()
        .map(|file_ref| (file_ref.module_path.clone(), file_ref))
        .collect::<BTreeMap<_, _>>();
    let file_units_by_module = package
        .file_ir_units
        .iter()
        .map(|unit| (unit.module_path.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let package_type_names = package_type_names_from_file_units(
        package
            .file_ir_units
            .iter()
            .map(|unit| (unit.module_path.as_str(), unit)),
    );
    let type_symbols = package_type_symbol_index(package, &file_units_by_module, dependencies)?;
    let mut exports = PackageExportIndex::default();

    for (public_symbol, export) in &package.exports.symbols {
        let package_symbol = package_scoped_export_symbol(package, public_symbol);
        let module = export.module.as_str();
        let symbol = export.symbol.as_str();
        let file_ref = files_by_module.get(module).cloned().ok_or_else(|| {
            package_export_error(
                package,
                public_symbol,
                format!("API export points to missing module {module}"),
            )
        })?;
        let file_unit = file_units_by_module.get(module).copied().ok_or_else(|| {
            package_export_error(
                package,
                public_symbol,
                format!("API export points to missing File IR unit for module {module}"),
            )
        })?;
        if let Some(type_index) = type_link_target_index(file_unit, symbol) {
            let ty = type_export_decl(package, public_symbol, module, file_unit, type_index)?;
            let interface_methods = file_unit
                .declarations
                .interfaces
                .get(&ty.name)
                .map(|interface| {
                    package_interface_method_signatures(
                        package.package_id,
                        &type_symbols,
                        module,
                        interface,
                    )
                    .map_err(|message| package_export_error(package, public_symbol, message))
                })
                .transpose()?
                .unwrap_or_default()
                .into_iter()
                .map(|method| {
                    projection_visible_interface_method_signature(
                        module,
                        &method,
                        &package_type_names,
                    )
                })
                .collect();
            exports.types.insert(
                package_symbol.clone(),
                TypeExport {
                    file: file_ref,
                    type_index,
                    symbol: ty.name.clone(),
                    descriptor: Some(projection_visible_type_descriptor(
                        module,
                        &ty.descriptor,
                        &package_type_names,
                    )),
                    type_params: ty.type_params.clone(),
                    interface_methods,
                },
            );
            continue;
        }
        if let Some(const_index) = const_link_target_index(file_unit, symbol) {
            let constant =
                const_export_decl(package, public_symbol, module, file_unit, const_index)?;
            exports.constants.insert(
                package_symbol.clone(),
                ConstExport {
                    file: file_ref,
                    const_index,
                    symbol: constant.name.clone(),
                    ty: projection_visible_type_ref(module, &constant.ty, &package_type_names),
                },
            );
            continue;
        }
        if let Some(executable_index) = executable_link_target_index(file_unit, symbol) {
            let executable = executable_export_decl(
                package,
                public_symbol,
                module,
                file_unit,
                executable_index,
            )?;
            let export = ExecutableExport {
                file: file_ref,
                executable_index,
                symbol: executable.symbol.clone(),
                signature: projection_visible_executable_signature(
                    module,
                    executable,
                    &package_type_names,
                ),
            };
            if executable_has_self_receiver(executable) {
                exports.impl_methods.insert(package_symbol.clone(), export);
            } else {
                exports.functions.insert(package_symbol.clone(), export);
            }
            continue;
        }

        return Err(package_export_error(
            package,
            public_symbol,
            format!("API export points to missing symbol {symbol} in module {module}"),
        ));
    }

    public_instances::project_package_public_instances(
        package,
        &files_by_module,
        &file_units_by_module,
        &type_symbols,
        &package_type_names,
        &mut exports,
    )?;

    Ok(exports)
}

pub(super) fn package_scoped_export_symbol(
    package: &PackageExportLinkProjectionInput<'_>,
    public_symbol: &str,
) -> String {
    if package.package_id == SKIFF_STD_PUBLICATION_ID && !public_symbol.starts_with("std.") {
        format!("std.{public_symbol}")
    } else {
        public_symbol.to_string()
    }
}

fn package_type_symbol_index(
    package: &PackageExportLinkProjectionInput<'_>,
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    dependencies: &[PackageRequirement],
) -> Result<PackageTypeSymbolIndex, ProjectionError> {
    let mut index = PackageTypeSymbolIndex::default();
    for dependency in dependencies {
        index.insert_dependency(&dependency.alias, &dependency.package_id);
        index.insert_dependency(&dependency.package_id, &dependency.package_id);
    }
    for (public_symbol, export) in &package.exports.symbols {
        let module = export.module.as_str();
        let symbol = export.symbol.as_str();
        let Some(file_unit) = file_units_by_module.get(module).copied() else {
            continue;
        };
        let Some(type_index) = type_link_target_index(file_unit, symbol) else {
            continue;
        };
        let Some(type_decl) = file_unit.type_table.get(type_index as usize) else {
            return Err(package_export_error(
                package,
                public_symbol,
                format!(
                    "type export index {type_index} is out of bounds for module {module} type table"
                ),
            ));
        };
        index.insert_type(
            module.to_string(),
            type_index,
            type_decl.name.clone(),
            package_scoped_export_symbol(package, public_symbol),
        );
    }
    Ok(index)
}

fn package_export_error(
    package: &PackageExportLinkProjectionInput<'_>,
    public_symbol: &str,
    message: impl Into<String>,
) -> ProjectionError {
    ProjectionError::InvalidPackageArtifact {
        message: format!(
            "package {} export {}: {}",
            package.package_id,
            public_symbol,
            message.into()
        ),
    }
}

fn type_link_target_index(unit: &FileIrUnit, symbol: &str) -> Option<u32> {
    unit.link_targets
        .types
        .get(symbol)
        .map(|target| target.type_index)
}

fn executable_link_target_index(unit: &FileIrUnit, symbol: &str) -> Option<u32> {
    unit.link_targets
        .executables
        .get(symbol)
        .map(|target| target.executable_index)
}

fn const_link_target_index(unit: &FileIrUnit, symbol: &str) -> Option<u32> {
    unit.link_targets
        .constants
        .get(symbol)
        .map(|target| target.const_index)
}

fn type_export_decl<'a>(
    package: &PackageExportLinkProjectionInput<'_>,
    public_symbol: &str,
    module: &str,
    file_unit: &'a FileIrUnit,
    type_index: u32,
) -> Result<&'a skiff_artifact_model::TypeDeclIr, ProjectionError> {
    file_unit.type_table.get(type_index as usize).ok_or_else(|| {
        package_export_error(
            package,
            public_symbol,
            format!("type export index {type_index} is out of bounds for module {module} type table"),
        )
    })
}

fn const_export_decl<'a>(
    package: &PackageExportLinkProjectionInput<'_>,
    public_symbol: &str,
    module: &str,
    file_unit: &'a FileIrUnit,
    const_index: u32,
) -> Result<&'a ConstIr, ProjectionError> {
    file_unit
        .constants
        .get(const_index as usize)
        .ok_or_else(|| {
            package_export_error(
                package,
                public_symbol,
                format!("const export index {const_index} is out of bounds for module {module}"),
            )
        })
}

fn executable_export_decl<'a>(
    package: &PackageExportLinkProjectionInput<'_>,
    public_symbol: &str,
    module: &str,
    file_unit: &'a FileIrUnit,
    executable_index: u32,
) -> Result<&'a ExecutableIr, ProjectionError> {
    file_unit
        .executables
        .get(executable_index as usize)
        .ok_or_else(|| {
            package_export_error(
                package,
                public_symbol,
                format!(
                    "executable export index {executable_index} is out of bounds for module {module}"
                ),
            )
        })
}

fn executable_has_self_receiver(executable: &ExecutableIr) -> bool {
    executable.self_type.is_some()
        || executable
            .params
            .first()
            .is_some_and(|param| param.name == "self")
}
