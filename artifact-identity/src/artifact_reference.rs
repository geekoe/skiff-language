use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_artifact_model::{
    schema::{PACKAGE_UNIT_SCHEMA_VERSION, SERVICE_UNIT_SCHEMA_VERSION},
    PackageUnit, ServiceUnit,
};

use crate::{
    artifact_coordinates::{
        package_unit_content_hash, publication_storage_segment, validate_canonical_path,
        validate_package_unit_artifact_path,
    },
    artifact_path::ArtifactRelativePath,
    package_build_identity, package_local_abi_identity, service_unit_hash, service_unit_identity,
    validate_package_unit_identities, ArtifactIdentityError, Result,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceUnitArtifactRef {
    pub schema_version: String,
    pub unit_identity: String,
    pub unit_hash: String,
    pub unit_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceAssemblyArtifactRef {
    pub assembly_identity: String,
    pub assembly_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageUnitArtifactRef {
    pub schema_version: String,
    pub package_id: String,
    pub version: String,
    pub build_identity: String,
    pub abi_identity: String,
    pub unit_hash: String,
    pub unit_path: String,
}

pub fn service_unit_artifact_ref(
    service_id: &str,
    unit_path: impl Into<String>,
    unit: &ServiceUnit,
) -> Result<ServiceUnitArtifactRef> {
    let reference = ServiceUnitArtifactRef {
        schema_version: unit.schema_version.clone(),
        unit_identity: service_unit_identity(unit)?,
        unit_hash: service_unit_hash(unit)?,
        unit_path: unit_path.into(),
    };
    let path = ArtifactRelativePath::parse(&reference.unit_path, "serviceUnit.unitPath")?;
    validate_service_unit_ref(service_id, unit, &reference, &path)?;
    Ok(reference)
}

pub fn package_unit_artifact_ref(
    unit_path: impl Into<String>,
    raw: &Value,
) -> Result<PackageUnitArtifactRef> {
    let unit_path = unit_path.into();
    let unit: PackageUnit = serde_json::from_value(raw.clone()).map_err(|source| {
        ArtifactIdentityError::InvalidPackageUnit {
            path: unit_path.clone(),
            source,
        }
    })?;
    let reference = PackageUnitArtifactRef {
        schema_version: unit.schema_version.clone(),
        package_id: unit.package_id.clone(),
        version: unit.version.clone(),
        build_identity: unit.build_identity.clone(),
        abi_identity: unit.abi_identity.clone(),
        unit_hash: package_unit_content_hash(raw)?,
        unit_path,
    };
    let path = ArtifactRelativePath::parse(
        &reference.unit_path,
        format!("package unit {} unitPath", reference.package_id),
    )?;
    validate_package_ref(raw, &unit, &reference, &path)?;
    Ok(reference)
}

pub(crate) fn validate_service_unit_ref(
    service_id: &str,
    unit: &ServiceUnit,
    reference: &ServiceUnitArtifactRef,
    path: &ArtifactRelativePath,
) -> Result<()> {
    compare_service_ref(
        path,
        "schemaVersion",
        &reference.schema_version,
        &unit.schema_version,
    )?;
    compare_service_ref(path, "serviceId", service_id, &unit.service.id)?;
    let hash = service_unit_hash(unit)?;
    let identity = service_unit_identity(unit)?;
    compare_service_ref(path, "unitHash", &reference.unit_hash, &hash)?;
    compare_service_ref(path, "unitIdentity", &reference.unit_identity, &identity)?;
    if reference.schema_version != SERVICE_UNIT_SCHEMA_VERSION {
        return Err(ArtifactIdentityError::InvalidServiceAssembly {
            message: format!("serviceUnit.schemaVersion must be {SERVICE_UNIT_SCHEMA_VERSION}"),
        });
    }
    validate_canonical_path(
        path,
        &format!(
            "units/services/{}/{}.json",
            publication_storage_segment(service_id, "service id")?,
            hash
        ),
        "service unit",
    )
}

pub(crate) fn validate_package_ref(
    raw: &Value,
    unit: &PackageUnit,
    reference: &PackageUnitArtifactRef,
    path: &ArtifactRelativePath,
) -> Result<()> {
    if reference.schema_version != PACKAGE_UNIT_SCHEMA_VERSION
        || unit.schema_version != PACKAGE_UNIT_SCHEMA_VERSION
    {
        return Err(ArtifactIdentityError::PackageUnitSchemaVersionMismatch {
            path: path.to_string(),
            expected: PACKAGE_UNIT_SCHEMA_VERSION,
            actual: unit.schema_version.clone(),
        });
    }
    validate_package_unit_identities(unit)?;
    compare_package_ref(path, "packageId", &reference.package_id, &unit.package_id)?;
    compare_package_ref(path, "version", &reference.version, &unit.version)?;
    compare_package_ref(
        path,
        "buildIdentity",
        &reference.build_identity,
        &package_build_identity(unit)?,
    )?;
    compare_package_ref(
        path,
        "abiIdentity",
        &reference.abi_identity,
        &package_local_abi_identity(unit)?,
    )?;
    let unit_hash = package_unit_content_hash(raw)?;
    compare_package_ref(path, "unitHash", &reference.unit_hash, &unit_hash)?;
    validate_package_unit_artifact_path(path.as_str(), &unit.package_id, &unit_hash)
}

fn compare_service_ref(
    path: &ArtifactRelativePath,
    field: &'static str,
    expected: &str,
    actual: &str,
) -> Result<()> {
    if expected != actual {
        return Err(ArtifactIdentityError::ServiceUnitPointerMismatch {
            path: path.to_string(),
            field,
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

fn compare_package_ref(
    path: &ArtifactRelativePath,
    field: &'static str,
    expected: &str,
    actual: &str,
) -> Result<()> {
    if expected != actual {
        return Err(ArtifactIdentityError::PackageUnitPointerMismatch {
            path: path.to_string(),
            field,
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}
