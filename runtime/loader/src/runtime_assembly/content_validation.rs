use std::{collections::BTreeMap, sync::Arc};

use anyhow::Context;
use sha2::{Digest, Sha256};
use skiff_artifact_identity::ArtifactRelativePath;
use skiff_artifact_model::{
    validate_boundary_operation_contract, validate_package_boundary_projections, FileIrRef,
    FileIrUnit, PackageArtifact, PackageArtifactRef, PackageOperationTarget,
    PublicationResourceRef, RuntimeAssembly, ServiceContract, ServiceContractRef,
};

use crate::utils::is_sha256_hash;

pub(super) fn validate_assembly(assembly: &RuntimeAssembly, stage: &str) -> anyhow::Result<()> {
    skiff_artifact_identity::validate_runtime_assembly_identity(assembly)
        .map_err(anyhow::Error::from)
        .with_context(|| format!("typed RuntimeAssembly validation failed {stage}"))
}

pub(super) fn validate_contract_ref(
    reference: &ServiceContractRef,
    contract: &ServiceContract,
) -> anyhow::Result<()> {
    skiff_artifact_identity::validate_service_contract_identities(contract)
        .map_err(anyhow::Error::from)
        .with_context(|| format!("contract content is invalid for ref {reference:?}"))?;
    for (operation_id, descriptor) in &contract.operations {
        validate_boundary_operation_contract(&descriptor.contract).with_context(|| {
            format!(
                "contract operation {operation_id} has an invalid canonical boundary contract for ref {reference:?}"
            )
        })?;
    }
    let actual = ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    };
    if reference != &actual {
        anyhow::bail!("contract content mismatches ref {reference:?}; loaded {actual:?}");
    }
    Ok(())
}

pub(super) fn validate_package_ref(
    reference: &PackageArtifactRef,
    artifact: &PackageArtifact,
) -> anyhow::Result<()> {
    skiff_artifact_identity::validate_package_artifact_identities(artifact)
        .map_err(anyhow::Error::from)
        .with_context(|| format!("package content is invalid for ref {reference:?}"))?;
    validate_package_boundary_projections(artifact).with_context(|| {
        format!(
            "package {} has invalid canonical boundary projections for ref {reference:?}",
            artifact.package_build_id
        )
    })?;
    let actual = PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    };
    if reference != &actual {
        anyhow::bail!("package content mismatches ref {reference:?}; loaded {actual:?}");
    }
    Ok(())
}

pub(super) fn validate_file_ref_path(
    package: &PackageArtifactRef,
    reference: &FileIrRef,
) -> anyhow::Result<()> {
    if reference.module_path.trim().is_empty() {
        anyhow::bail!(
            "package {} contains a File IR ref with an empty module path",
            package.package_build_id
        );
    }
    if let Some(path) = &reference.artifact_path {
        ArtifactRelativePath::parse(
            path,
            format!("package {} File IR artifactPath", package.package_build_id),
        )
        .map_err(anyhow::Error::from)?;
    }
    Ok(())
}

pub(super) fn validate_file_content<V>(
    package: &PackageArtifactRef,
    reference: &FileIrRef,
    file: &FileIrUnit,
    validate_identity: &V,
) -> anyhow::Result<()>
where
    V: Fn(&FileIrUnit) -> anyhow::Result<()> + ?Sized,
{
    validate_identity(file).with_context(|| {
        format!(
            "File IR content is invalid for {} in package {}",
            reference.file_ir_identity, package.package_build_id
        )
    })?;
    validate_file_ref(package, reference, file)
}

pub(super) fn validate_file_ref(
    package: &PackageArtifactRef,
    reference: &FileIrRef,
    file: &FileIrUnit,
) -> anyhow::Result<()> {
    if file.file_ir_identity != reference.file_ir_identity
        || file.module_path != reference.module_path
        || reference
            .source_ast_hash
            .as_ref()
            .is_some_and(|hash| hash != &file.source_ast_hash)
    {
        anyhow::bail!(
            "File IR content mismatches exact ref {}:{} in package {}",
            reference.file_ir_identity,
            reference.module_path,
            package.package_build_id
        );
    }
    Ok(())
}

pub(super) fn validate_resource_ref_path(
    package: &PackageArtifactRef,
    reference: &PublicationResourceRef,
) -> anyhow::Result<()> {
    ArtifactRelativePath::parse(
        &reference.path,
        format!(
            "package {} static resource logical path",
            package.package_build_id
        ),
    )
    .map_err(anyhow::Error::from)?;
    if let Some(path) = &reference.artifact_path {
        ArtifactRelativePath::parse(
            path,
            format!(
                "package {} static resource artifactPath",
                package.package_build_id
            ),
        )
        .map_err(anyhow::Error::from)?;
    }
    if !is_sha256_hash(&reference.sha256) {
        anyhow::bail!(
            "package {} static resource {} sha256 must be 64 lowercase hex characters",
            package.package_build_id,
            reference.path
        );
    }
    Ok(())
}

pub(super) fn validate_resource_content(
    package: &PackageArtifactRef,
    reference: &PublicationResourceRef,
    bytes: &[u8],
) -> anyhow::Result<()> {
    let actual_len = u64::try_from(bytes.len())
        .map_err(|_| anyhow::anyhow!("static resource size does not fit u64"))?;
    if actual_len != reference.byte_len {
        anyhow::bail!(
            "package {} static resource {} size mismatch: declared {}, loaded {}",
            package.package_build_id,
            reference.path,
            reference.byte_len,
            actual_len
        );
    }
    let actual_hash = hex::encode(Sha256::digest(bytes));
    if actual_hash != reference.sha256 {
        anyhow::bail!(
            "package {} static resource {} hash mismatch: declared {}, loaded {}",
            package.package_build_id,
            reference.path,
            reference.sha256,
            actual_hash
        );
    }
    Ok(())
}

pub(super) fn validate_package_file_targets(
    package_ref: &PackageArtifactRef,
    artifact: &PackageArtifact,
    files: &[Arc<FileIrUnit>],
    file_slots: &BTreeMap<String, usize>,
) -> anyhow::Result<()> {
    let resolve = |reference: &FileIrRef, label: &str| -> anyhow::Result<&FileIrUnit> {
        validate_file_ref_path(package_ref, reference)?;
        let Some(slot) = file_slots.get(&reference.file_ir_identity) else {
            anyhow::bail!(
                "package {} {label} targets missing File IR {}",
                package_ref.package_build_id,
                reference.file_ir_identity
            );
        };
        let file = files
            .get(*slot)
            .expect("file slot map must point inside hydrated files");
        if file.module_path != reference.module_path {
            anyhow::bail!(
                "package {} {label} File IR module mismatch for {}",
                package_ref.package_build_id,
                reference.file_ir_identity
            );
        }
        validate_file_ref(package_ref, reference, file)?;
        Ok(file)
    };

    for (name, export) in &artifact.implementation_links.types {
        let file = resolve(&export.file, &format!("type link {name}"))?;
        if file.type_table.get(export.type_index as usize).is_none() {
            anyhow::bail!(
                "package {} type link {name} targets missing type index {}",
                package_ref.package_build_id,
                export.type_index
            );
        }
    }
    for (name, export) in &artifact.implementation_links.constants {
        let file = resolve(&export.file, &format!("constant link {name}"))?;
        if file.constants.get(export.const_index as usize).is_none() {
            anyhow::bail!(
                "package {} constant link {name} targets missing const index {}",
                package_ref.package_build_id,
                export.const_index
            );
        }
    }
    for (name, export) in artifact
        .implementation_links
        .functions
        .iter()
        .chain(&artifact.implementation_links.impl_methods)
    {
        validate_executable_target(
            package_ref,
            &export.file,
            export.executable_index,
            &format!("executable link {name}"),
            &resolve,
        )?;
    }
    for (name, target) in &artifact.implementation_links.operation_targets {
        match target {
            PackageOperationTarget::LocalExecutable { target, .. } => {
                validate_executable_target(
                    package_ref,
                    &target.file_ref,
                    target.executable_index,
                    &format!("operation target {name}"),
                    &resolve,
                )?;
            }
            PackageOperationTarget::LocalConstReceiverExecutable { target, .. } => {
                let receiver_file = resolve(
                    &target.receiver.file_ref,
                    &format!("operation receiver {name}"),
                )?;
                if receiver_file
                    .constants
                    .get(target.receiver.const_index as usize)
                    .is_none()
                {
                    anyhow::bail!(
                        "package {} operation receiver {name} targets missing const index {}",
                        package_ref.package_build_id,
                        target.receiver.const_index
                    );
                }
                validate_executable_target(
                    package_ref,
                    &target.executable_target.file_ref,
                    target.executable_target.executable_index,
                    &format!("operation executable {name}"),
                    &resolve,
                )?;
            }
        }
    }
    for (callable, link) in &artifact.callable_links {
        validate_executable_target(
            package_ref,
            &link.target.file_ref,
            link.target.executable_index,
            &format!("callable {callable}"),
            &resolve,
        )?;
    }
    Ok(())
}

fn validate_executable_target<'a>(
    package: &PackageArtifactRef,
    reference: &FileIrRef,
    executable_index: u32,
    label: &str,
    resolve: &impl Fn(&FileIrRef, &str) -> anyhow::Result<&'a FileIrUnit>,
) -> anyhow::Result<()> {
    let file = resolve(reference, label)?;
    if file.executables.get(executable_index as usize).is_none() {
        anyhow::bail!(
            "package {} {label} targets missing executable index {executable_index}",
            package.package_build_id
        );
    }
    Ok(())
}
