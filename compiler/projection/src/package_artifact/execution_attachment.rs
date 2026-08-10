use skiff_artifact_identity::{
    assign_package_artifact_identities, package_artifact_local_abi_identity_projection,
    validate_package_artifact_identities,
};
use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity,
    validate_bytecode_statement_manifest_identity_lexical, BytecodeArtifactRef,
    BytecodeStatementManifestIdentity,
};

use crate::error::ProjectionError;

use super::ProjectedPackageArtifact;

/// Execution-owned facts attached to an otherwise complete package
/// projection. Callers may construct this DTO directly; attachment validates
/// its path and identity boundaries before accepting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageExecutionAttachment {
    pub bytecode: BytecodeArtifactRef,
    pub statement_manifest_identity: BytecodeStatementManifestIdentity,
}

/// Returns a new package projection with bytecode execution facts attached.
///
/// The source must be the canonical unattached projection state: no bytecode
/// reference and the exact package-owned empty statement manifest. The
/// attachment's bytecode reference must also be path-free. All writes,
/// identity assignment, and validation occur on a clone, so every failure
/// leaves the source projection unchanged.
pub fn attach_package_execution(
    projected: &ProjectedPackageArtifact,
    attachment: PackageExecutionAttachment,
) -> Result<ProjectedPackageArtifact, ProjectionError> {
    let package_id = projected.artifact.package_id.as_str();
    let empty_manifest = derive_bytecode_statement_manifest_identity(package_id, &[]).map_err(
        |error| {
            invalid_attachment(format!(
                "failed to derive the canonical empty statement manifest for package {package_id}: {error}"
            ))
        },
    )?;
    if projected.artifact.bytecode.is_some()
        || projected.artifact.bytecode_statement_manifest_identity != empty_manifest
    {
        return Err(invalid_attachment(format!(
            "package {}@{} must have bytecode absent and the exact canonical empty statement manifest before execution attachment",
            projected.artifact.package_id, projected.artifact.package_version
        )));
    }
    if let Some(path) = attachment.bytecode.artifact_path.as_deref() {
        return Err(invalid_attachment(format!(
            "execution attachment bytecode reference must be path-free, got artifactPath {path:?}"
        )));
    }
    validate_bytecode_statement_manifest_identity_lexical(&attachment.statement_manifest_identity)
        .map_err(|error| {
            invalid_attachment(format!(
                "execution attachment has an invalid statement manifest identity: {error}"
            ))
        })?;
    let mut attached = projected.clone();
    validate_package_artifact_identities(&attached.artifact).map_err(|error| {
        invalid_attachment(format!(
            "source PackageArtifact identities are invalid before execution attachment: {error}"
        ))
    })?;

    let source_local_identity = attached
        .artifact
        .package_local_abi
        .local_abi_identity
        .clone();
    let source_local_projection =
        package_artifact_local_abi_identity_projection(&attached.artifact).map_err(|error| {
            invalid_attachment(format!(
                "failed to project source PackageArtifact Local ABI: {error}"
            ))
        })?;

    // Install both execution-owned facts before the first fallible identity
    // operation; a failed assignment can only discard this private clone.
    attached.artifact.bytecode = Some(attachment.bytecode);
    attached.artifact.bytecode_statement_manifest_identity = attachment.statement_manifest_identity;
    assign_package_artifact_identities(&mut attached.artifact).map_err(|error| {
        invalid_attachment(format!(
            "failed to assign PackageArtifact identities after execution attachment: {error}"
        ))
    })?;
    validate_package_artifact_identities(&attached.artifact).map_err(|error| {
        invalid_attachment(format!(
            "attached PackageArtifact identities failed validation: {error}"
        ))
    })?;

    let attached_local_projection =
        package_artifact_local_abi_identity_projection(&attached.artifact).map_err(|error| {
            invalid_attachment(format!(
                "failed to project attached PackageArtifact Local ABI: {error}"
            ))
        })?;
    if attached.artifact.package_local_abi.local_abi_identity != source_local_identity
        || attached_local_projection != source_local_projection
    {
        return Err(invalid_attachment(
            "execution attachment must not change PackageArtifact Local ABI identity or projection",
        ));
    }

    Ok(attached)
}

fn invalid_attachment(message: impl Into<String>) -> ProjectionError {
    ProjectionError::InvalidPackageArtifact {
        message: message.into(),
    }
}
