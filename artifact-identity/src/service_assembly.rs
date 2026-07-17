use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_artifact_model::{
    schema::{
        PACKAGE_UNIT_SCHEMA_VERSION, SERVICE_ASSEMBLY_KIND, SERVICE_ASSEMBLY_SCHEMA_VERSION,
        SERVICE_UNIT_SCHEMA_VERSION,
    },
    PackageUnit, ServiceUnit,
};

use crate::{
    artifact_path::ArtifactRelativePath,
    framed_identity,
    framing::sha256_hex,
    package_build_identity, package_local_abi_identity,
    package_resolver::{ordered_pinned_package_closure, publication_storage_segment},
    runtime_program_dynamic_build_id, runtime_program_service_unit_identity_bytes,
    service_unit_hash, service_unit_identity, validate_package_unit_identities,
    ArtifactIdentityError, Result, SERVICE_ASSEMBLY_IDENTITY_PREFIX, SERVICE_BUILD_IDENTITY_PREFIX,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatedArtifactContent {
    pub path: String,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatedServiceArtifactClosure {
    pub assembly_identity: String,
    pub dynamic_build_id: String,
    pub service_assembly: ValidatedArtifactContent,
    pub service_unit: ValidatedArtifactContent,
    pub package_units: Vec<ValidatedArtifactContent>,
}

pub fn service_assembly_identity_projection(assembly: &Value) -> Result<Value> {
    let object = assembly
        .as_object()
        .ok_or_else(|| invalid_assembly("must be an object"))?;
    require_string(object, "schemaVersion", SERVICE_ASSEMBLY_SCHEMA_VERSION)?;
    require_string(object, "kind", SERVICE_ASSEMBLY_KIND)?;
    let service = object
        .get("service")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_assembly("service must be an object"))?;
    for field in ["id", "revisionId", "protocolIdentity"] {
        required_value(service, field)?;
    }
    required_value(service, "api")?;
    required_value(object, "serviceUnit")?;

    let mut projection = object.clone();
    let projected_service = projection
        .get_mut("service")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| invalid_assembly("service must remain an object during projection"))?;
    projected_service.remove("assemblyIdentity");
    Ok(Value::Object(projection))
}

pub fn service_assembly_hash(assembly: &Value) -> Result<String> {
    let projection = service_assembly_identity_projection(assembly)?;
    let bytes = skiff_canonical_json::canonical_json_bytes(&projection).map_err(|error| {
        invalid_assembly(format!("identity projection cannot serialize: {error}"))
    })?;
    Ok(sha256_hex(&bytes))
}

pub fn package_unit_content_hash(unit: &Value) -> Result<String> {
    let bytes = skiff_canonical_json::canonical_json_bytes(unit).map_err(|error| {
        ArtifactIdentityError::InvalidPackageIdentityInput {
            message: error.to_string(),
        }
    })?;
    Ok(sha256_hex(&bytes))
}

pub fn service_build_identity_hash(build_identity: &str) -> Result<&str> {
    build_identity
        .strip_prefix(&format!("{SERVICE_BUILD_IDENTITY_PREFIX}:sha256:"))
        .filter(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or_else(
            || ArtifactIdentityError::InvalidRuntimeProgramBuildIdentity {
                identity: build_identity.to_string(),
            },
        )
}

pub fn service_build_identity_from_assembly_identity(assembly_identity: &str) -> Result<String> {
    let hash = identity_hash(assembly_identity, SERVICE_ASSEMBLY_IDENTITY_PREFIX)?;
    Ok(format!("{SERVICE_BUILD_IDENTITY_PREFIX}:sha256:{hash}"))
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

pub fn service_assembly_identity(assembly: &Value) -> Result<String> {
    Ok(framed_identity(
        SERVICE_ASSEMBLY_IDENTITY_PREFIX,
        &service_assembly_hash(assembly)?,
    ))
}

pub fn validate_service_assembly_identity(assembly: &Value, declared_identity: &str) -> Result<()> {
    let computed = service_assembly_identity(assembly)?;
    if declared_identity != computed {
        return Err(ArtifactIdentityError::ServiceAssemblyIdentityMismatch {
            declared: declared_identity.to_string(),
            computed,
        });
    }
    let embedded = assembly
        .pointer("/service/assemblyIdentity")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_assembly("service.assemblyIdentity is required"))?;
    if embedded != declared_identity {
        return Err(ArtifactIdentityError::ServiceAssemblyIdentityMismatch {
            declared: embedded.to_string(),
            computed: declared_identity.to_string(),
        });
    }
    Ok(())
}

pub fn validate_service_assembly_artifact_path(
    path: &str,
    service_id: &str,
    assembly_identity: &str,
) -> Result<()> {
    let path = ArtifactRelativePath::parse(path, "serviceAssembly.assemblyPath")?;
    validate_canonical_path(
        &path,
        &format!(
            "assemblies/services/{}/{}.json",
            publication_storage_segment(service_id, "service id")?,
            identity_hash(assembly_identity, SERVICE_ASSEMBLY_IDENTITY_PREFIX)?
        ),
        "service assembly",
    )
}

pub fn validate_package_unit_artifact_path(
    path: &str,
    package_id: &str,
    unit_hash: &str,
) -> Result<()> {
    if unit_hash.len() != 64
        || !unit_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ArtifactIdentityError::InvalidPackageIdentityInput {
            message: format!("unit hash {unit_hash} must be 64 lowercase hex characters"),
        });
    }
    let path = ArtifactRelativePath::parse(path, "packageUnit.unitPath")?;
    validate_canonical_path(
        &path,
        &format!(
            "units/packages/{}/{}.json",
            publication_storage_segment(package_id, "package id")?,
            unit_hash
        ),
        "package unit",
    )
}

pub fn validate_service_artifact_closure(
    artifact_root: &Path,
    service_id: &str,
    assembly_identity: &str,
    assembly_path: &str,
    service_unit_ref: &ServiceUnitArtifactRef,
    package_refs: &[PackageUnitArtifactRef],
) -> Result<ValidatedServiceArtifactClosure> {
    let assembly_path = ArtifactRelativePath::parse(assembly_path, "serviceAssembly.assemblyPath")?;
    let assembly_value = read_json(artifact_root, &assembly_path, "service assembly")?;
    validate_service_assembly_identity(&assembly_value, assembly_identity)?;
    require_service_id(&assembly_value, service_id)?;
    validate_service_assembly_artifact_path(assembly_path.as_str(), service_id, assembly_identity)?;

    let assembly_service_ref: ServiceUnitArtifactRef = serde_json::from_value(
        assembly_value
            .get("serviceUnit")
            .cloned()
            .ok_or_else(|| invalid_assembly("serviceUnit is required"))?,
    )
    .map_err(|error| invalid_assembly(format!("serviceUnit pointer is invalid: {error}")))?;
    if &assembly_service_ref != service_unit_ref {
        return Err(invalid_assembly(
            "serviceUnit pointer does not match the selected artifact pointer",
        ));
    }

    let service_unit_path =
        ArtifactRelativePath::parse(&service_unit_ref.unit_path, "serviceUnit.unitPath")?;
    let service_unit_value = read_json(artifact_root, &service_unit_path, "service unit")?;
    let service_unit: ServiceUnit = serde_json::from_value(service_unit_value.clone())
        .map_err(ArtifactIdentityError::InvalidServiceUnit)?;
    validate_service_unit_ref(
        service_id,
        &service_unit,
        service_unit_ref,
        &service_unit_path,
    )?;
    validate_protocol_identity_consistency(&assembly_value, &service_unit)?;

    let mut packages = Vec::with_capacity(package_refs.len());
    let mut package_contents = Vec::with_capacity(package_refs.len());
    for package_ref in package_refs {
        let path = ArtifactRelativePath::parse(
            &package_ref.unit_path,
            format!("package unit {} unitPath", package_ref.package_id),
        )?;
        let value = read_json(artifact_root, &path, "package unit")?;
        let package: PackageUnit = serde_json::from_value(value.clone()).map_err(|source| {
            ArtifactIdentityError::InvalidPackageUnit {
                path: path.to_string(),
                source,
            }
        })?;
        validate_package_ref(&value, &package, package_ref, &path)?;
        packages.push(package);
        package_contents.push(ValidatedArtifactContent {
            path: path.to_string(),
            value,
        });
    }
    let ordered_packages = ordered_pinned_package_closure(&service_unit, &packages)?;
    let content_by_package_id = packages
        .iter()
        .zip(package_contents)
        .map(|(package, content)| (package.package_id.as_str(), content))
        .collect::<std::collections::BTreeMap<_, _>>();
    let package_contents = ordered_packages
        .iter()
        .map(|package| {
            content_by_package_id
                .get(package.package_id.as_str())
                .cloned()
                .ok_or_else(|| {
                    invalid_assembly(format!(
                        "validated package closure omitted {}",
                        package.package_id
                    ))
                })
        })
        .collect::<Result<Vec<_>>>()?;

    let service_identity = runtime_program_service_unit_identity_bytes(&service_unit)?;
    let dynamic_build_id = runtime_program_dynamic_build_id(
        &service_identity,
        ordered_packages
            .iter()
            .map(|package| package.build_identity.as_str()),
    );
    Ok(ValidatedServiceArtifactClosure {
        assembly_identity: assembly_identity.to_string(),
        dynamic_build_id,
        service_assembly: ValidatedArtifactContent {
            path: assembly_path.to_string(),
            value: assembly_value,
        },
        service_unit: ValidatedArtifactContent {
            path: service_unit_path.to_string(),
            value: service_unit_value,
        },
        package_units: package_contents,
    })
}

fn validate_protocol_identity_consistency(assembly: &Value, unit: &ServiceUnit) -> Result<()> {
    let assembly_protocol_identity = assembly
        .pointer("/service/protocolIdentity")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_assembly("service.protocolIdentity must be a string"))?;
    if assembly_protocol_identity != unit.protocol_identity {
        return Err(
            ArtifactIdentityError::ServiceAssemblyProtocolIdentityMismatch {
                assembly: assembly_protocol_identity.to_string(),
                service_unit: unit.protocol_identity.clone(),
            },
        );
    }
    Ok(())
}

fn validate_service_unit_ref(
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
        return Err(invalid_assembly(format!(
            "serviceUnit.schemaVersion must be {SERVICE_UNIT_SCHEMA_VERSION}"
        )));
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

fn read_json(root: &Path, path: &ArtifactRelativePath, label: &str) -> Result<Value> {
    let resolved = path.resolve_existing(root, label)?;
    let text =
        fs::read_to_string(&resolved).map_err(|source| ArtifactIdentityError::ReadArtifact {
            path: resolved.display().to_string(),
            source,
        })?;
    serde_json::from_str(&text).map_err(|source| ArtifactIdentityError::ParseArtifactJson {
        path: resolved.display().to_string(),
        source,
    })
}

fn validate_canonical_path(path: &ArtifactRelativePath, expected: &str, label: &str) -> Result<()> {
    if path.as_str() != expected {
        return Err(ArtifactIdentityError::NonCanonicalArtifactPath {
            label: label.to_string(),
            path: path.to_string(),
            expected: expected.to_string(),
        });
    }
    Ok(())
}

fn require_service_id(assembly: &Value, expected: &str) -> Result<()> {
    let actual = assembly
        .pointer("/service/id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_assembly("service.id must be a string"))?;
    if actual != expected {
        return Err(invalid_assembly(format!(
            "service.id {actual} does not match pointer serviceId {expected}"
        )));
    }
    Ok(())
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

fn identity_hash<'a>(identity: &'a str, prefix: &str) -> Result<&'a str> {
    identity
        .strip_prefix(&format!("{prefix}:"))
        .filter(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or_else(|| {
            invalid_assembly(format!(
                "identity {identity} must use {prefix}:<64 lowercase hex>"
            ))
        })
}

fn required_value<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a Value> {
    object
        .get(field)
        .ok_or_else(|| invalid_assembly(format!("{field} is required")))
}

fn require_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
    expected: &str,
) -> Result<()> {
    let actual = object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_assembly(format!("{field} must be a string")))?;
    if actual != expected {
        return Err(invalid_assembly(format!("{field} must be {expected}")));
    }
    Ok(())
}

fn invalid_assembly(message: impl Into<String>) -> ArtifactIdentityError {
    ArtifactIdentityError::InvalidServiceAssembly {
        message: message.into(),
    }
}
