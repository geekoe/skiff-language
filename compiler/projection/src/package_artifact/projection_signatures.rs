use std::collections::BTreeSet;

use skiff_artifact_model::{
    ExecutableExport, ExecutableSignatureIr, PackageCallableParameter, PackageCallableSignature,
    PackageExportIndex, PackageTypeRef,
};
use skiff_compiler_projection_input::{
    ProjectionPackageCallableKey, ProjectionPackageCallableSignatureFacts,
};

use crate::error::ProjectionError;

use super::api_exports::PackageExports;

pub(super) fn project_callable_signatures(
    package_id: &str,
    api_exports: &PackageExports,
    exports: &PackageExportIndex,
) -> Result<ProjectionPackageCallableSignatureFacts, ProjectionError> {
    let mut entries = Vec::new();
    let mut seeded_paths = BTreeSet::new();
    for (public_path, export) in &exports.functions {
        push_signature(&mut entries, public_path, export);
        seeded_paths.insert(public_path.clone());
    }
    for instance in &exports.public_instances {
        for operation in &instance.operations {
            let target = &operation.receiver_executable.executable_target;
            let executable = executable_by_target(exports, target).ok_or_else(|| {
                projection_error(
                    package_id,
                    format!(
                        "public instance operation {} target {}#{} has no executable signature",
                        operation.operation.public_path,
                        target.file_ref.module_path,
                        target.executable_index
                    ),
                )
            })?;
            push_signature(&mut entries, &operation.operation.public_path, executable);
            seeded_paths.insert(operation.operation.public_path.clone());
        }
    }
    let explicit_paths = api_exports
        .symbols
        .keys()
        .map(|path| scoped_public_path(package_id, path))
        .collect::<BTreeSet<_>>();
    for (public_path, export) in &exports.impl_methods {
        if explicit_paths.contains(public_path) && !seeded_paths.contains(public_path) {
            push_signature(&mut entries, public_path, export);
        }
    }
    ProjectionPackageCallableSignatureFacts::try_from_entries(entries).map_err(|error| {
        projection_error(
            package_id,
            format!("canonical callable signature projection: {error}"),
        )
    })
}

fn push_signature(
    entries: &mut Vec<(ProjectionPackageCallableKey, PackageCallableSignature)>,
    public_path: &str,
    export: &ExecutableExport,
) {
    entries.push((
        ProjectionPackageCallableKey::new(
            public_path,
            export.file.module_path.clone(),
            export.executable_index,
        ),
        package_callable_signature(&export.signature),
    ));
}

fn package_callable_signature(signature: &ExecutableSignatureIr) -> PackageCallableSignature {
    let receiver_offset = usize::from(
        signature.self_type.is_some()
            || signature
                .params
                .first()
                .is_some_and(|parameter| parameter.name == "self"),
    );
    PackageCallableSignature {
        parameters: signature.params[receiver_offset..]
            .iter()
            .map(|parameter| PackageCallableParameter {
                name: parameter.name.clone(),
                ty: PackageTypeRef::Local {
                    local_type: parameter.ty.clone(),
                },
            })
            .collect(),
        return_type: PackageTypeRef::Local {
            local_type: signature.return_type.clone(),
        },
        throw_types: Vec::new(),
        may_suspend: signature.may_suspend,
    }
}

fn executable_by_target<'a>(
    exports: &'a PackageExportIndex,
    target: &skiff_artifact_model::OperationTargetRef,
) -> Option<&'a ExecutableExport> {
    exports
        .functions
        .values()
        .chain(exports.impl_methods.values())
        .find(|export| {
            export.file.file_ir_identity == target.file_ref.file_ir_identity
                && export.executable_index == target.executable_index
        })
}

fn scoped_public_path(package_id: &str, public_path: &str) -> String {
    if package_id == skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID
        && !public_path.starts_with("std.")
    {
        format!("std.{public_path}")
    } else {
        public_path.to_string()
    }
}

fn projection_error(package_id: &str, message: String) -> ProjectionError {
    ProjectionError::ContractValidation {
        message: format!("package {package_id} callable signature projection: {message}"),
    }
}
