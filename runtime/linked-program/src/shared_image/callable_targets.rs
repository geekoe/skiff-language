use std::collections::BTreeMap;

use skiff_artifact_model::{
    ConstExport, OperationCallableKind, PackageCallableId, PackageLocalAbiSymbol,
};

use crate::{ConstAddr, FileAddr, UnitAddr};

use super::{
    semantic_file_ref_matches_loaded, LinkedPackageCallableTarget, SharedPackageCode,
    SharedPackageImageError, SharedPackageImageResult,
};

pub(super) fn link_callable_targets(
    code: &SharedPackageCode,
) -> SharedPackageImageResult<BTreeMap<PackageCallableId, LinkedPackageCallableTarget>> {
    let mut receivers = BTreeMap::<PackageCallableId, (String, ConstAddr)>::new();
    for (public_path, symbol) in &code.artifact.package_local_abi.public_symbols {
        let PackageLocalAbiSymbol::PublicInstance { methods, .. } = symbol else {
            continue;
        };
        let receiver_link = code
            .artifact
            .implementation_links
            .constants
            .get(public_path)
            .ok_or_else(
                || SharedPackageImageError::MissingPublicInstanceReceiverLink {
                    package_build_id: code.package_build_id().clone(),
                    public_path: public_path.clone(),
                },
            )?;
        let receiver = const_addr(code, receiver_link)?;
        for callable_id in methods.values() {
            let callable = code
                .artifact
                .callable_links
                .get(callable_id)
                .ok_or_else(
                    || SharedPackageImageError::MissingPublicInstanceCallableLink {
                        package_build_id: code.package_build_id().clone(),
                        public_path: public_path.clone(),
                        package_callable_id: callable_id.clone(),
                    },
                )?;
            if callable.target.callable_kind != OperationCallableKind::ImplMethod {
                return Err(
                    SharedPackageImageError::PublicInstanceCallableKindMismatch {
                        package_build_id: code.package_build_id().clone(),
                        public_path: public_path.clone(),
                        package_callable_id: callable_id.clone(),
                        actual: callable.target.callable_kind,
                    },
                );
            }
            match receivers.get(callable_id) {
                Some((first_path, first_receiver)) if first_receiver == &receiver => {
                    return Err(
                        SharedPackageImageError::DuplicatePublicInstanceCallableReceiver {
                            package_build_id: code.package_build_id().clone(),
                            package_callable_id: callable_id.clone(),
                            first_public_path: first_path.clone(),
                            duplicate_public_path: public_path.clone(),
                        },
                    );
                }
                Some((first_path, first_receiver)) => {
                    return Err(
                        SharedPackageImageError::ConflictingPublicInstanceCallableReceiver {
                            package_build_id: code.package_build_id().clone(),
                            package_callable_id: callable_id.clone(),
                            first_public_path: first_path.clone(),
                            first_receiver: Box::new(first_receiver.clone()),
                            conflicting_public_path: public_path.clone(),
                            conflicting_receiver: Box::new(receiver),
                        },
                    );
                }
                None => {
                    receivers.insert(callable_id.clone(), (public_path.clone(), receiver.clone()));
                }
            }
        }
    }

    code.artifact
        .callable_links
        .iter()
        .map(|(callable_id, fact)| {
            let executable_addr = code.executable_addr(&fact.target)?;
            Ok((
                callable_id.clone(),
                LinkedPackageCallableTarget {
                    executable_addr,
                    receiver_const: receivers
                        .get(callable_id)
                        .map(|(_, receiver)| receiver.clone()),
                },
            ))
        })
        .collect()
}

fn const_addr(
    code: &SharedPackageCode,
    receiver: &ConstExport,
) -> SharedPackageImageResult<ConstAddr> {
    let file_index = code
        .files_by_identity
        .get(&receiver.file.file_ir_identity)
        .copied()
        .ok_or_else(
            || SharedPackageImageError::PublicInstanceReceiverFileNotLoaded {
                package_build_id: code.package_build_id().clone(),
                file_ir_identity: receiver.file.file_ir_identity.clone(),
            },
        )?;
    let expected_file_ref = code
        .artifact
        .files
        .get(file_index)
        .expect("hydrated files preserve artifact file order");
    let file = code
        .files
        .get(file_index)
        .expect("hydrated files preserve artifact file order");
    if !semantic_file_ref_matches_loaded(&receiver.file, file) {
        return Err(
            SharedPackageImageError::PublicInstanceReceiverFileRefMismatch {
                package_build_id: code.package_build_id().clone(),
                expected: Box::new(expected_file_ref.clone()),
                actual: Box::new(receiver.file.clone()),
            },
        );
    }
    let const_index = receiver.const_index as usize;
    if const_index >= file.constants.len() {
        return Err(
            SharedPackageImageError::PublicInstanceReceiverConstOutOfBounds {
                package_build_id: code.package_build_id().clone(),
                file_ir_identity: file.file_ir_identity.clone(),
                const_index: receiver.const_index,
                const_count: file.constants.len(),
            },
        );
    }
    Ok(ConstAddr {
        unit: UnitAddr::Package(code.code_slot.index()),
        file: FileAddr::LoadedFileIndex(file_index),
        const_index,
    })
}
