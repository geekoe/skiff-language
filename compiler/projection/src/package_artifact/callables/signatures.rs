use std::collections::BTreeSet;

use skiff_artifact_model::{OperationCallableKind, OperationTargetRef, PackageExportIndex};
use skiff_compiler_core::public_package_callable_id;
use skiff_compiler_projection_input::{
    canonical_package_public_path, ProjectionPackageCallableKey,
    ProjectionPackageCallableSignatureFacts,
};

use crate::{
    error::ProjectionError,
    package_artifact::{api_exports::PackageExports, export_links::ProjectedPackageExportLinks},
};

use super::{
    projection_error,
    surface::{CallableTarget, CanonicalCallable},
};

pub(super) fn package_callable_targets(
    package_id: &str,
    exports: &ProjectedPackageExportLinks,
) -> Result<Vec<CallableTarget>, ProjectionError> {
    let mut targets = exports
        .exports
        .functions
        .iter()
        .map(|(public_path, export)| {
            let callable_id = project_public_callable_id(package_id, public_path)?;
            Ok(CallableTarget {
                public_path: public_path.clone(),
                callable_id: callable_id.clone(),
                owner_module: export.file.module_path.clone(),
                executable_index: export.executable_index,
                target: OperationTargetRef {
                    file_ref: export.file.clone(),
                    executable_index: export.executable_index,
                    callable_abi_id: callable_id.to_string(),
                    callable_kind: OperationCallableKind::PublicFunction,
                },
            })
        })
        .collect::<Result<Vec<_>, ProjectionError>>()?;
    let instance_targets = exports
        .public_instances
        .iter()
        .flat_map(|instance| &instance.methods)
        .map(|method| {
            let public_path = method.public_path.clone();
            let callable_id = project_public_callable_id(package_id, &public_path)?;
            Ok(CallableTarget {
                public_path,
                callable_id: callable_id.clone(),
                owner_module: method.executable.file.module_path.clone(),
                executable_index: method.executable.executable_index,
                target: method.executable.operation_target_ref(
                    callable_id.to_string(),
                    OperationCallableKind::ImplMethod,
                ),
            })
        })
        .collect::<Result<Vec<_>, ProjectionError>>()?;
    targets.extend(instance_targets);
    Ok(targets)
}

pub(super) fn add_direct_impl_method_targets(
    package_id: &str,
    api_exports: &PackageExports,
    exports: &PackageExportIndex,
    targets: &mut Vec<CallableTarget>,
) -> Result<(), ProjectionError> {
    let explicit_paths = api_exports
        .symbols
        .keys()
        .map(|path| canonical_package_public_path(package_id, path))
        .collect::<BTreeSet<_>>();
    let target_paths = targets
        .iter()
        .map(|target| target.public_path.clone())
        .collect::<BTreeSet<_>>();
    for (public_path, export) in &exports.impl_methods {
        if !explicit_paths.contains(public_path) || target_paths.contains(public_path) {
            continue;
        }
        let callable_id = project_public_callable_id(package_id, public_path)?;
        targets.push(CallableTarget {
            public_path: public_path.clone(),
            callable_id: callable_id.clone(),
            owner_module: export.file.module_path.clone(),
            executable_index: export.executable_index,
            target: OperationTargetRef {
                file_ref: export.file.clone(),
                executable_index: export.executable_index,
                callable_abi_id: callable_id.to_string(),
                callable_kind: OperationCallableKind::ImplMethod,
            },
        });
    }
    targets.sort_by(|left, right| left.public_path.cmp(&right.public_path));
    Ok(())
}

pub(super) fn attach_canonical_signatures(
    package_id: &str,
    signatures: &ProjectionPackageCallableSignatureFacts,
    targets: Vec<CallableTarget>,
) -> Result<Vec<CanonicalCallable>, ProjectionError> {
    let expected = targets
        .iter()
        .map(callable_signature_key)
        .collect::<BTreeSet<_>>();
    let actual = signatures.keys().cloned().collect::<BTreeSet<_>>();
    if expected != actual {
        let missing = expected
            .difference(&actual)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let extra = actual
            .difference(&expected)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        return Err(projection_error(
            package_id,
            format!(
                "canonical callable signatures must exactly cover package API; missing={missing:?}, extra={extra:?}"
            ),
        ));
    }
    targets
        .into_iter()
        .map(|target| {
            let key = callable_signature_key(&target);
            let signature = signatures.signature(&key).cloned().ok_or_else(|| {
                projection_error(
                    package_id,
                    format!("canonical callable signature `{key}` is missing after coverage check"),
                )
            })?;
            Ok(CanonicalCallable {
                signature,
                public_path: target.public_path,
                callable_id: target.callable_id,
                owner_module: target.owner_module,
                executable_index: target.executable_index,
                target: target.target,
            })
        })
        .collect()
}

fn callable_signature_key(target: &CallableTarget) -> ProjectionPackageCallableKey {
    ProjectionPackageCallableKey::new(
        target.public_path.clone(),
        target.owner_module.clone(),
        target.executable_index,
    )
}

pub(super) fn project_public_callable_id(
    package_id: &str,
    public_path: &str,
) -> Result<skiff_artifact_model::PackageCallableId, ProjectionError> {
    public_package_callable_id(package_id, public_path)
        .map_err(|error| projection_error(package_id, error.to_string()))
}
