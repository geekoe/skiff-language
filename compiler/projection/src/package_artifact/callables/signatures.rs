use std::collections::BTreeSet;

use skiff_artifact_model::{
    OperationCallableKind, OperationTargetRef, PackageCallableId, PackageCallableSignature,
    PackageExportIndex, PackageTypeRef,
};
use skiff_compiler_projection_input::{
    ProjectionPackageCallableKey, ProjectionPackageCallableSignatureFacts,
};

use crate::{error::ProjectionError, package_exports::PackageExports};

use super::{projection_error, surface::CallableSeed};

pub(super) fn publication_callable_seeds(
    package_id: &str,
    exports: &PackageExportIndex,
) -> Vec<CallableSeed> {
    let mut seeds = exports
        .functions
        .iter()
        .map(|(public_path, export)| {
            let callable_id = package_callable_id(package_id, public_path);
            CallableSeed {
                public_path: public_path.clone(),
                callable_id: callable_id.clone(),
                owner_module: export.file.module_path.clone(),
                executable_index: export.executable_index,
                signature: unassigned_signature(),
                target: OperationTargetRef {
                    file_ref: export.file.clone(),
                    executable_index: export.executable_index,
                    callable_abi_id: callable_id.to_string(),
                    callable_kind: OperationCallableKind::PublicFunction,
                },
            }
        })
        .collect::<Vec<_>>();
    seeds.extend(
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
                CallableSeed {
                    public_path,
                    callable_id,
                    owner_module: target.file_ref.module_path.clone(),
                    executable_index: target.executable_index,
                    signature: unassigned_signature(),
                    target: normalized_target,
                }
            }),
    );
    seeds
}

pub(super) fn add_direct_impl_method_seeds(
    package_id: &str,
    api_exports: &PackageExports,
    exports: &PackageExportIndex,
    seeds: &mut Vec<CallableSeed>,
) {
    let explicit_paths = api_exports
        .symbols
        .keys()
        .map(|path| scoped_public_path(package_id, path))
        .collect::<BTreeSet<_>>();
    let seeded_paths = seeds
        .iter()
        .map(|seed| seed.public_path.clone())
        .collect::<BTreeSet<_>>();
    for (public_path, export) in &exports.impl_methods {
        if !explicit_paths.contains(public_path) || seeded_paths.contains(public_path) {
            continue;
        }
        let callable_id = package_callable_id(package_id, public_path);
        seeds.push(CallableSeed {
            public_path: public_path.clone(),
            callable_id: callable_id.clone(),
            owner_module: export.file.module_path.clone(),
            executable_index: export.executable_index,
            signature: unassigned_signature(),
            target: OperationTargetRef {
                file_ref: export.file.clone(),
                executable_index: export.executable_index,
                callable_abi_id: callable_id.to_string(),
                callable_kind: OperationCallableKind::ImplMethod,
            },
        });
    }
    seeds.sort_by(|left, right| left.public_path.cmp(&right.public_path));
}

pub(super) fn attach_canonical_signatures(
    package_id: &str,
    signatures: &ProjectionPackageCallableSignatureFacts,
    seeds: &mut [CallableSeed],
) -> Result<(), ProjectionError> {
    let expected = seeds
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
    for seed in seeds {
        seed.signature = signatures
            .signature(&callable_signature_key(seed))
            .expect("exact signature coverage checked")
            .clone();
    }
    Ok(())
}

fn callable_signature_key(seed: &CallableSeed) -> ProjectionPackageCallableKey {
    ProjectionPackageCallableKey::new(
        seed.public_path.clone(),
        seed.owner_module.clone(),
        seed.executable_index,
    )
}

fn unassigned_signature() -> PackageCallableSignature {
    PackageCallableSignature {
        parameters: Vec::new(),
        return_type: PackageTypeRef::Local {
            local_type: skiff_artifact_model::TypeRefIr::native("void"),
        },
        throw_types: Vec::new(),
        may_suspend: false,
    }
}

fn package_callable_id(package_id: &str, public_path: &str) -> PackageCallableId {
    PackageCallableId::new(format!("pkg-callable:{package_id}:{public_path}"))
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
