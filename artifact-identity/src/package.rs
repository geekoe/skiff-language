mod projection;

pub use projection::{PackageBuildIdentityProjection, PackageLocalAbiIdentityProjection};

use serde_json::{Map, Value};
use skiff_artifact_model::{
    PackageImplementationLinks, PackageOperationTarget, PackageUnit, PublicationAbiUnit,
};

use crate::framing::{canonical_ir_bytes, identity, sha256_hex};
use crate::publication::assign_publication_abi_identity;
use crate::publication_validation::{
    validate_publication_abi_identity, validate_publication_operation_ref,
};
use crate::{
    ArtifactIdentityError, Result, PACKAGE_BUILD_IDENTITY_PREFIX,
    PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX, PACKAGE_LOCAL_ABI_IDENTITY_PREFIX,
};

/// Returns the explicit semantic projection used by the package local ABI identity.
///
/// The nested publication surface is validated before its declared identity is
/// admitted into this outer preimage.
pub fn package_local_abi_identity_projection(
    unit: &PackageUnit,
) -> Result<PackageLocalAbiIdentityProjection> {
    validate_package_identity_inputs(unit)?;
    Ok(PackageLocalAbiIdentityProjection::from_unit(unit))
}

/// Returns the explicit semantic projection used by the package build identity.
///
/// Storage locations and ref-level provenance are removed by the typed
/// projection; File IR content remains represented by its opaque identity.
pub fn package_build_identity_projection(
    unit: &PackageUnit,
) -> Result<PackageBuildIdentityProjection> {
    validate_package_identity_inputs(unit)?;
    let local_abi_identity = package_local_abi_identity_from_validated(unit)?;
    PackageBuildIdentityProjection::from_unit(unit, local_abi_identity)
}

pub fn package_build_hash(unit: &PackageUnit) -> Result<String> {
    let projection = package_build_identity_projection(unit)?;
    package_build_hash_from_projection(&projection)
}

pub fn package_build_identity(unit: &PackageUnit) -> Result<String> {
    Ok(identity(
        PACKAGE_BUILD_IDENTITY_PREFIX,
        &package_build_hash(unit)?,
    ))
}

pub fn package_local_abi_hash(unit: &PackageUnit) -> Result<String> {
    let projection = package_local_abi_identity_projection(unit)?;
    package_local_abi_hash_from_projection(&projection)
}

pub fn package_local_abi_identity(unit: &PackageUnit) -> Result<String> {
    Ok(identity(
        PACKAGE_LOCAL_ABI_IDENTITY_PREFIX,
        &package_local_abi_hash(unit)?,
    ))
}

/// Returns the content identity used by package-test link-policy references.
///
/// This intentionally preserves the v1 wire: canonical JSON of the complete
/// `PackageImplementationLinks` DTO, hashed directly without the package ABI
/// projection's storage-field exclusions.
pub fn package_implementation_links_identity(links: &PackageImplementationLinks) -> Result<String> {
    let bytes = canonical_ir_bytes(
        links,
        ArtifactIdentityError::SerializePackageImplementationLinksIdentity,
    )?;
    Ok(identity(
        PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    ))
}

/// Temporary call-site migration alias. T06/T07 must adopt
/// [`package_local_abi_hash`] and delete this old conceptual name.
pub fn package_abi_hash(unit: &PackageUnit) -> Result<String> {
    package_local_abi_hash(unit)
}

/// Temporary call-site migration alias. T06/T07 must adopt
/// [`package_local_abi_identity`] and delete this old conceptual name.
pub fn package_abi_identity(unit: &PackageUnit) -> Result<String> {
    package_local_abi_identity(unit)
}

/// Assigns the nested publication identity, package local ABI identity and
/// package build identity in dependency order using the same projections as
/// validation.
pub fn assign_package_unit_identities(unit: &mut PackageUnit) -> Result<(String, String)> {
    unit.publication_abi.publication_id = unit.package_id.clone();
    unit.publication_abi.version = unit.version.clone();
    normalize_package_dependency_configs(unit);

    // This validates every operation descriptor before assigning the nested
    // publication identity, so no untrusted operation id enters an outer hash.
    assign_publication_abi_identity(&mut unit.publication_abi)?;
    let abi_identity = package_local_abi_identity_from_validated(unit)?;
    unit.abi_identity = abi_identity.clone();
    let build_identity = package_build_identity_from_validated(unit, abi_identity.clone())?;
    unit.build_identity = build_identity.clone();

    // Keep assign and validate mechanically tied to the same owner and catch
    // future projection/assignment drift at the producer boundary.
    validate_package_unit_identities(unit)?;
    Ok((build_identity, abi_identity))
}

/// Validates nested semantic identities before comparing either declared
/// package identity. The build preimage always receives a recomputed local ABI
/// identity rather than trusting `unit.abi_identity`.
pub fn validate_package_unit_identities(unit: &PackageUnit) -> Result<()> {
    validate_package_identity_inputs(unit)?;

    let computed_abi = package_local_abi_identity_from_validated(unit)?;
    if unit.abi_identity != computed_abi {
        return Err(ArtifactIdentityError::PackageAbiIdentityMismatch {
            declared: unit.abi_identity.clone(),
            computed: computed_abi,
        });
    }

    let computed_build = package_build_identity_from_validated(unit, unit.abi_identity.clone())?;
    if unit.build_identity != computed_build {
        return Err(ArtifactIdentityError::PackageBuildIdentityMismatch {
            declared: unit.build_identity.clone(),
            computed: computed_build,
        });
    }

    Ok(())
}

fn validate_package_identity_inputs(unit: &PackageUnit) -> Result<()> {
    validate_package_publication_coordinate(unit, &unit.publication_abi)?;
    validate_publication_abi_identity(&unit.publication_abi)?;
    validate_package_operation_targets(unit)
}

fn validate_package_operation_targets(unit: &PackageUnit) -> Result<()> {
    for (target_key, target) in &unit.implementation_links.operation_targets {
        let operation = match target {
            PackageOperationTarget::LocalExecutable { operation, .. }
            | PackageOperationTarget::LocalConstReceiverExecutable { operation, .. } => operation,
        };
        if target_key != &operation.operation_abi_id {
            return Err(ArtifactIdentityError::InvalidPackageIdentityInput {
                message: format!(
                    "implementationLinks operation target key {target_key} does not match nested operationAbiId {}",
                    operation.operation_abi_id
                ),
            });
        }
        validate_publication_operation_ref(
            &unit.publication_abi,
            operation,
            &format!("implementationLinks operation target {target_key}"),
        )?;
    }
    Ok(())
}

fn validate_package_publication_coordinate(
    unit: &PackageUnit,
    publication: &PublicationAbiUnit,
) -> Result<()> {
    if publication.publication_id == unit.package_id && publication.version == unit.version {
        return Ok(());
    }
    Err(
        ArtifactIdentityError::PackagePublicationCoordinateMismatch {
            package_id: unit.package_id.clone(),
            package_version: unit.version.clone(),
            publication_id: publication.publication_id.clone(),
            publication_version: publication.version.clone(),
        },
    )
}

fn package_local_abi_identity_from_validated(unit: &PackageUnit) -> Result<String> {
    let projection = PackageLocalAbiIdentityProjection::from_unit(unit);
    Ok(identity(
        PACKAGE_LOCAL_ABI_IDENTITY_PREFIX,
        &package_local_abi_hash_from_projection(&projection)?,
    ))
}

fn package_local_abi_hash_from_projection(
    projection: &PackageLocalAbiIdentityProjection,
) -> Result<String> {
    Ok(sha256_hex(&canonical_ir_bytes(
        projection,
        ArtifactIdentityError::SerializePackageAbiIdentity,
    )?))
}

fn package_build_identity_from_validated(
    unit: &PackageUnit,
    local_abi_identity: String,
) -> Result<String> {
    let projection = PackageBuildIdentityProjection::from_unit(unit, local_abi_identity)?;
    Ok(identity(
        PACKAGE_BUILD_IDENTITY_PREFIX,
        &package_build_hash_from_projection(&projection)?,
    ))
}

fn package_build_hash_from_projection(
    projection: &PackageBuildIdentityProjection,
) -> Result<String> {
    Ok(sha256_hex(&canonical_ir_bytes(
        projection,
        ArtifactIdentityError::SerializePackageBuildIdentity,
    )?))
}

fn normalize_package_dependency_configs(unit: &mut PackageUnit) {
    for dependency in &mut unit.dependencies {
        if dependency.config.is_null() {
            dependency.config = Value::Object(Map::new());
        }
    }
}
