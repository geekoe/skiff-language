use serde::Serialize;
use serde_json::{Map, Value};
use skiff_artifact_model::{
    ConfigAndEffectMetadata, FileIrRef, PackageDependencyConstraint, PackageImplementationLinks,
    PackageUnit, PublicationAbiUnit,
};

use crate::framing::{canonical_ir_bytes, identity, sha256_hex};
use crate::publication::{assign_publication_abi_identity, publication_abi_hash};
use crate::{
    ArtifactIdentityError, Result, PACKAGE_ABI_IDENTITY_PREFIX, PACKAGE_BUILD_IDENTITY_PREFIX,
};

pub fn package_build_hash(unit: &PackageUnit) -> Result<String> {
    Ok(sha256_hex(&canonical_ir_bytes(
        &PackageBuildIdentityPayload {
            schema_version: &unit.schema_version,
            package_id: &unit.package_id,
            version: &unit.version,
            publication_abi: &unit.publication_abi,
            files: &unit.files,
            resources: &unit.resources,
            dependencies: &unit.dependencies,
            implementation_links: &unit.implementation_links,
            config_and_effect_metadata: &unit.config_and_effect_metadata,
        },
        ArtifactIdentityError::SerializePackageBuildIdentity,
    )?))
}

pub fn package_build_identity(unit: &PackageUnit) -> Result<String> {
    Ok(identity(
        PACKAGE_BUILD_IDENTITY_PREFIX,
        &package_build_hash(unit)?,
    ))
}

pub fn package_abi_hash(unit: &PackageUnit) -> Result<String> {
    publication_abi_hash(&unit.publication_abi)
}

pub fn package_abi_identity(unit: &PackageUnit) -> Result<String> {
    Ok(identity(
        PACKAGE_ABI_IDENTITY_PREFIX,
        &package_abi_hash(unit)?,
    ))
}

pub fn validate_package_unit_identities(unit: &PackageUnit) -> Result<()> {
    crate::validate_publication_abi_identity(&unit.publication_abi)?;
    let computed_build = package_build_identity(unit)?;
    if unit.build_identity != computed_build {
        return Err(ArtifactIdentityError::PackageBuildIdentityMismatch {
            declared: unit.build_identity.clone(),
            computed: computed_build,
        });
    }

    let computed_abi = package_abi_identity(unit)?;
    if unit.abi_identity != computed_abi {
        return Err(ArtifactIdentityError::PackageAbiIdentityMismatch {
            declared: unit.abi_identity.clone(),
            computed: computed_abi,
        });
    }

    Ok(())
}

pub fn assign_package_unit_identities(unit: &mut PackageUnit) -> Result<(String, String)> {
    unit.publication_abi.publication_id = unit.package_id.clone();
    unit.publication_abi.version = unit.version.clone();
    assign_publication_abi_identity(&mut unit.publication_abi)?;
    let abi_identity = package_abi_identity(unit)?;
    unit.abi_identity = abi_identity.clone();
    normalize_package_dependency_configs(unit);
    let build_identity = package_build_identity(unit)?;
    unit.build_identity = build_identity.clone();
    Ok((build_identity, abi_identity))
}

fn normalize_package_dependency_configs(unit: &mut PackageUnit) {
    for dependency in &mut unit.dependencies {
        if dependency.config.is_null() {
            dependency.config = Value::Object(Map::new());
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageBuildIdentityPayload<'a> {
    schema_version: &'a str,
    package_id: &'a str,
    version: &'a str,
    publication_abi: &'a PublicationAbiUnit,
    files: &'a [FileIrRef],
    resources: &'a [skiff_artifact_model::PublicationResourceRef],
    dependencies: &'a [PackageDependencyConstraint],
    implementation_links: &'a PackageImplementationLinks,
    config_and_effect_metadata: &'a ConfigAndEffectMetadata,
}
