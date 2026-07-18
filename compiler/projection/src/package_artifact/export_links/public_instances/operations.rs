use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    ExecutableExport, FileIrRef, LocalReceiverExecutableRef, OperationCallableKind,
    OperationConstReceiverRef, OperationTargetRef, PackageExportIndex, PublicInstanceOperation,
    ReceiverCallAbi,
};
use skiff_compiler_core::naming::impl_method_declaration_name;
use skiff_compiler_publication_abi::{
    package_public_instance_method_operation, public_signature_from_interface_method_signature,
    public_signature_from_receiver_executable_signature,
};

use crate::{
    error::ProjectionError,
    package_artifact::{
        model::PackageExportLinkProjectionInput,
        visible_types::{projection_visible_executable_signature, PackageVisibleTypeNames},
    },
};

use super::{public_instance_error, PackagePublicInstanceInterface, PackagePublicInstanceReceiver};

#[allow(clippy::too_many_arguments)]
pub(super) fn project_operations(
    package: &PackageExportLinkProjectionInput<'_>,
    files_by_module: &BTreeMap<String, FileIrRef>,
    public_path: &str,
    receiver: &PackagePublicInstanceReceiver<'_>,
    receiver_const: &OperationConstReceiverRef,
    interfaces: &[PackagePublicInstanceInterface],
    package_type_names: &PackageVisibleTypeNames,
    exports: &mut PackageExportIndex,
) -> Result<Vec<PublicInstanceOperation>, ProjectionError> {
    let mut operations = Vec::new();
    let mut method_names = BTreeSet::new();
    for interface in interfaces {
        for method in &interface.methods {
            if !method_names.insert(method.name.clone()) {
                return Err(public_instance_error(
                    package,
                    public_path,
                    format!(
                        "derives conflicting operation `{}` from multiple interfaces",
                        method.name
                    ),
                ));
            }
            let target_symbol = impl_method_declaration_name(&receiver.symbol.symbol, &method.name);
            let executable_index = impl_method_executable_index(receiver.unit, &target_symbol)
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
            let executable = receiver
                .unit
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
            let executable_signature = projection_visible_executable_signature(
                &receiver.symbol.module_path,
                executable,
                package_type_names,
            );
            let public_signature =
                public_signature_from_receiver_executable_signature(executable_signature.clone());
            let interface_signature = public_signature_from_interface_method_signature(method);
            if public_signature != interface_signature {
                return Err(public_instance_error(
                    package,
                    public_path,
                    format!(
                        "receiver {}.{} method {} signature does not match listed interface method",
                        receiver.symbol.module_path, receiver.symbol.symbol, method.name
                    ),
                ));
            }
            let operation = package_public_instance_method_operation(
                public_path,
                &interface.instantiation,
                &method.name,
                &interface_signature,
            );
            let target_file = files_by_module
                .get(&receiver.unit.module_path)
                .cloned()
                .ok_or_else(|| {
                    public_instance_error(
                        package,
                        public_path,
                        format!(
                            "receiver module {} has no File IR ref",
                            receiver.unit.module_path
                        ),
                    )
                })?;
            exports
                .impl_methods
                .entry(target_symbol.clone())
                .or_insert(ExecutableExport {
                    file: target_file.clone(),
                    executable_index,
                    symbol: target_symbol.clone(),
                    signature: executable_signature,
                });
            let method_abi_id = operation
                .method_abi_id
                .clone()
                .unwrap_or_else(|| operation.operation_abi_id.clone());
            operations.push(PublicInstanceOperation {
                operation,
                receiver_executable: LocalReceiverExecutableRef {
                    receiver: receiver_const.clone(),
                    executable_target: OperationTargetRef {
                        file_ref: target_file,
                        executable_index,
                        callable_abi_id: format!(
                            "callable:{}.{}",
                            receiver.unit.module_path, target_symbol
                        ),
                        callable_kind: OperationCallableKind::ImplMethod,
                    },
                    method_abi_id,
                    receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
                },
            });
        }
    }
    Ok(operations)
}

fn impl_method_executable_index(
    unit: &skiff_artifact_model::FileIrUnit,
    target_symbol: &str,
) -> Option<u32> {
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
