use std::collections::BTreeSet;

use skiff_artifact_model::{
    OperationCallableKind, OperationTargetRef, PackageCallableId, PackageExportIndex,
};
use skiff_compiler_projection_input::{
    canonical_package_public_path, ProjectionPackageCallableKey,
    ProjectionPackageCallableSignatureFacts,
};

use crate::{error::ProjectionError, package_artifact::api_exports::PackageExports};

use super::{
    projection_error,
    surface::{CallableTarget, CanonicalCallable},
};

pub(super) fn package_callable_targets(
    package_id: &str,
    exports: &PackageExportIndex,
) -> Vec<CallableTarget> {
    let mut targets = exports
        .functions
        .iter()
        .map(|(public_path, export)| {
            let callable_id = package_callable_id(package_id, public_path);
            CallableTarget {
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
            }
        })
        .collect::<Vec<_>>();
    targets.extend(
        exports
            .public_instances
            .iter()
            .flat_map(|instance| &instance.operations)
            .map(|operation| {
                let target = &operation.receiver_executable.executable_target;
                let public_path = operation.operation.public_path.clone();
                let callable_id = package_callable_id(package_id, &public_path);
                let mut normalized_target = target.clone();
                normalized_target.callable_abi_id = callable_id.to_string();
                CallableTarget {
                    public_path,
                    callable_id,
                    owner_module: target.file_ref.module_path.clone(),
                    executable_index: target.executable_index,
                    target: normalized_target,
                }
            }),
    );
    targets
}

pub(super) fn add_direct_impl_method_targets(
    package_id: &str,
    api_exports: &PackageExports,
    exports: &PackageExportIndex,
    targets: &mut Vec<CallableTarget>,
) {
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
        let callable_id = package_callable_id(package_id, public_path);
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

fn package_callable_id(package_id: &str, public_path: &str) -> PackageCallableId {
    PackageCallableId::new(format!("pkg-callable:{package_id}:{public_path}"))
}
