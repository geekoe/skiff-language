use std::{collections::BTreeMap, fs, path::Path};

use serde::Serialize;
use serde_json::Value;
use skiff_artifact_model::{PackageUnit, ServiceUnit};

use crate::{
    artifact_coordinates::validate_service_assembly_artifact_path,
    artifact_path::ArtifactRelativePath,
    artifact_reference::{
        validate_package_ref, validate_service_unit_ref, PackageUnitArtifactRef,
        ServiceUnitArtifactRef,
    },
    package_resolver::ordered_pinned_package_closure,
    runtime_program_dynamic_build_id, runtime_program_service_unit_identity_bytes,
    service_assembly_identity::{invalid_service_assembly, validate_service_assembly_identity},
    ArtifactIdentityError, Result,
};

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

pub fn validate_service_artifact_closure(
    artifact_root: &Path,
    service_id: &str,
    expected_service_version: Option<&str>,
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
            .ok_or_else(|| invalid_service_assembly("serviceUnit is required"))?,
    )
    .map_err(|error| {
        invalid_service_assembly(format!("serviceUnit pointer is invalid: {error}"))
    })?;
    if &assembly_service_ref != service_unit_ref {
        return Err(invalid_service_assembly(
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
    validate_service_version(&service_unit, &service_unit_path, expected_service_version)?;
    validate_protocol_identity_consistency(&assembly_value, &service_unit)?;

    let (packages, package_contents) = load_package_units(artifact_root, package_refs)?;
    let ordered_packages = ordered_pinned_package_closure(&service_unit, &packages)?;
    let package_contents = order_package_contents(&packages, package_contents, &ordered_packages)?;

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

fn load_package_units(
    artifact_root: &Path,
    package_refs: &[PackageUnitArtifactRef],
) -> Result<(Vec<PackageUnit>, Vec<ValidatedArtifactContent>)> {
    let mut packages = Vec::with_capacity(package_refs.len());
    let mut contents = Vec::with_capacity(package_refs.len());
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
        contents.push(ValidatedArtifactContent {
            path: path.to_string(),
            value,
        });
    }
    Ok((packages, contents))
}

fn order_package_contents(
    packages: &[PackageUnit],
    contents: Vec<ValidatedArtifactContent>,
    ordered_packages: &[&PackageUnit],
) -> Result<Vec<ValidatedArtifactContent>> {
    let content_by_package_id = packages
        .iter()
        .zip(contents)
        .map(|(package, content)| (package.package_id.as_str(), content))
        .collect::<BTreeMap<_, _>>();
    ordered_packages
        .iter()
        .map(|package| {
            content_by_package_id
                .get(package.package_id.as_str())
                .cloned()
                .ok_or_else(|| {
                    invalid_service_assembly(format!(
                        "validated package closure omitted {}",
                        package.package_id
                    ))
                })
        })
        .collect()
}

fn validate_service_version(
    unit: &ServiceUnit,
    path: &ArtifactRelativePath,
    expected: Option<&str>,
) -> Result<()> {
    if let Some(expected) = expected {
        if unit.version != expected {
            return Err(ArtifactIdentityError::ServiceUnitVersionMismatch {
                path: path.to_string(),
                expected: expected.to_string(),
                actual: unit.version.clone(),
            });
        }
    }
    Ok(())
}

fn validate_protocol_identity_consistency(assembly: &Value, unit: &ServiceUnit) -> Result<()> {
    let assembly_protocol_identity = assembly
        .pointer("/service/protocolIdentity")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_service_assembly("service.protocolIdentity must be a string"))?;
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

fn require_service_id(assembly: &Value, expected: &str) -> Result<()> {
    let actual = assembly
        .pointer("/service/id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_service_assembly("service.id must be a string"))?;
    if actual != expected {
        return Err(invalid_service_assembly(format!(
            "service.id {actual} does not match pointer serviceId {expected}"
        )));
    }
    Ok(())
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
