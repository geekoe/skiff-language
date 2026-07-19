use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    ExecutableExport, ExecutableKind, FileIrRef, FileIrUnit, PackageExportIndex,
};
use skiff_compiler_core::naming::impl_method_declaration_name;

use crate::{
    error::ProjectionError,
    package_artifact::{
        model::PackageExportLinkProjectionInput,
        visible_types::{projection_visible_executable_signature, PackageVisibleTypeNames},
    },
};

use super::{
    public_instance_error, PackagePublicInstanceInterface,
    PackagePublicInstanceMethodExecutionLink, PackagePublicInstanceReceiver,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn project_operations(
    package: &PackageExportLinkProjectionInput<'_>,
    files_by_module: &BTreeMap<String, FileIrRef>,
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    public_path: &str,
    receiver: &PackagePublicInstanceReceiver,
    interfaces: &[PackagePublicInstanceInterface],
    package_type_names: &PackageVisibleTypeNames,
    exports: &mut PackageExportIndex,
) -> Result<Vec<PackagePublicInstanceMethodExecutionLink>, ProjectionError> {
    let mut operations = Vec::new();
    let mut method_names = BTreeSet::new();
    for interface in interfaces {
        for method in &interface.methods {
            if !method_names.insert(method.name.clone()) {
                return Err(public_instance_error(
                    package,
                    public_path,
                    format!(
                        "derives conflicting method `{}` from multiple interfaces",
                        method.name
                    ),
                ));
            }
            let target_symbol = impl_method_declaration_name(&receiver.symbol.symbol, &method.name);
            if method.executable_module != receiver.symbol.module_path
                || method.executable_symbol != target_symbol
            {
                return Err(public_instance_error(
                    package,
                    public_path,
                    format!(
                        "source-validated method {} target {}.{} does not match receiver target {}.{}",
                        method.name,
                        method.executable_module,
                        method.executable_symbol,
                        receiver.symbol.module_path,
                        target_symbol
                    ),
                ));
            }
            let target_unit = file_units_by_module
                .get(method.executable_module.as_str())
                .copied()
                .ok_or_else(|| {
                    public_instance_error(
                        package,
                        public_path,
                        format!(
                            "method {} target module {} has no File IR unit",
                            method.name, method.executable_module
                        ),
                    )
                })?;
            let executable_index = impl_method_executable_index(target_unit, &target_symbol)
                .ok_or_else(|| {
                    public_instance_error(
                        package,
                        public_path,
                        format!(
                            "receiver {}.{} is missing implementation method {}",
                            receiver.symbol.module_path, receiver.symbol.symbol, method.name
                        ),
                    )
                })?;
            let executable = target_unit
                .executables
                .get(executable_index as usize)
                .ok_or_else(|| {
                    public_instance_error(
                        package,
                        public_path,
                        format!(
                            "receiver {}.{} method {} points to missing executable index {}",
                            receiver.symbol.module_path,
                            receiver.symbol.symbol,
                            method.name,
                            executable_index
                        ),
                    )
                })?;
            let executable_symbol_matches = executable.symbol == target_symbol
                || executable
                    .symbol
                    .strip_prefix(&format!("{}.", method.executable_module))
                    .is_some_and(|symbol| symbol == target_symbol);
            if executable.kind != ExecutableKind::ImplMethod || !executable_symbol_matches {
                return Err(public_instance_error(
                    package,
                    public_path,
                    format!(
                        "receiver {}.{} method {} target index {} resolved to {:?} `{}` instead of impl method {}",
                        receiver.symbol.module_path,
                        receiver.symbol.symbol,
                        method.name,
                        executable_index,
                        executable.kind,
                        executable.symbol,
                        target_symbol
                    ),
                ));
            }
            let target_file = files_by_module
                .get(&method.executable_module)
                .cloned()
                .ok_or_else(|| {
                    public_instance_error(
                        package,
                        public_path,
                        format!(
                            "method {} target module {} has no File IR ref",
                            method.name, method.executable_module
                        ),
                    )
                })?;
            let executable = ExecutableExport {
                file: target_file,
                executable_index,
                symbol: target_symbol.clone(),
                signature: projection_visible_executable_signature(
                    &method.executable_module,
                    executable,
                    package_type_names,
                ),
            };
            exports
                .impl_methods
                .entry(target_symbol)
                .or_insert_with(|| executable.clone());
            operations.push(PackagePublicInstanceMethodExecutionLink {
                name: method.name.clone(),
                public_path: format!("{public_path}.{}", method.name),
                executable,
            });
        }
    }
    Ok(operations)
}

fn impl_method_executable_index(unit: &FileIrUnit, target_symbol: &str) -> Option<u32> {
    unit.link_targets
        .executables
        .get(target_symbol)
        .map(|target| target.executable_index)
        .or_else(|| {
            unit.declarations
                .executables
                .get(target_symbol)
                .map(|target| target.executable_index)
        })
}
