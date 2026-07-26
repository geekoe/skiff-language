use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    ExecutableExport, ExecutableKind, FileIrRef, FileIrUnit, PackageExportIndex,
};

use crate::{
    error::ProjectionError,
    package_artifact::{
        model::PackageExportLinkProjectionInput,
        visible_types::{
            projection_visible_executable_signature, projection_visible_type_ref,
            PackageVisibleTypeNames,
        },
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
            let target_symbol = method.executable_symbol.clone();
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
            if executable.type_params != receiver.type_params {
                return Err(public_instance_error(
                    package,
                    public_path,
                    format!(
                        "source-validated method {} target {}.{} has type parameters {:?}, expected {:?}",
                        method.name,
                        method.executable_module,
                        method.executable_symbol,
                        executable.type_params,
                        receiver.type_params
                    ),
                ));
            }
            let explicit_self = executable
                .params
                .first()
                .filter(|parameter| parameter.name == "self");
            let implementation_receiver = match (executable.self_type.as_ref(), explicit_self) {
                (Some(_), Some(_)) => {
                    return Err(public_instance_error(
                        package,
                        public_path,
                        format!(
                            "source-validated method {} target {}.{} declares two receivers",
                            method.name, method.executable_module, method.executable_symbol
                        ),
                    ));
                }
                (Some(self_type), None) => self_type,
                (None, Some(self_parameter)) => &self_parameter.ty,
                (None, None) => {
                    return Err(public_instance_error(
                        package,
                        public_path,
                        format!(
                            "source-validated method {} target {}.{} has no receiver",
                            method.name, method.executable_module, method.executable_symbol
                        ),
                    ));
                }
            };
            if executable
                .params
                .iter()
                .skip(usize::from(explicit_self.is_some()))
                .any(|parameter| parameter.name == "self")
            {
                return Err(public_instance_error(
                    package,
                    public_path,
                    format!(
                        "source-validated method {} target {}.{} has a non-leading receiver",
                        method.name, method.executable_module, method.executable_symbol
                    ),
                ));
            }
            let implementation_receiver = projection_visible_type_ref(
                &method.executable_module,
                implementation_receiver,
                package_type_names,
            );
            if implementation_receiver != receiver.definition_type() {
                return Err(public_instance_error(
                    package,
                    public_path,
                    format!(
                        "source-validated method {} target {}.{} has the wrong receiver",
                        method.name, method.executable_module, method.executable_symbol
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
