use std::{
    io::{self, Read},
    path::PathBuf,
    process::ExitCode,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_artifact_identity::{
    package_build_identity, package_local_abi_identity, validate_service_artifact_closure,
    ArtifactIdentityError, PackageUnitArtifactRef, ServiceAssemblyArtifactRef,
    ServiceUnitArtifactRef, ValidatedArtifactContent,
};
use skiff_artifact_model::PackageUnit;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let payload = ErrorEnvelope {
                error: CliErrorBody {
                    code: error.code(),
                    message: error.to_string(),
                },
            };
            let _ = serde_json::to_writer(io::stderr(), &payload);
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), CliError> {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (Some("runtime-program-build-id"), None) => runtime_program_build_id(),
        (Some("package-unit-identities"), None) => package_unit_identities(),
        _ => Err(CliError::SchemaInvalid(
            "usage: skiff-artifact-identity <runtime-program-build-id|package-unit-identities>"
                .to_string(),
        )),
    }
}

fn runtime_program_build_id() -> Result<(), CliError> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| CliError::Internal(format!("failed to read stdin: {error}")))?;
    let request: RuntimeProgramBuildIdRequest =
        serde_json::from_str(&input).map_err(|error| CliError::SchemaInvalid(error.to_string()))?;
    let mut results = Vec::with_capacity(request.services.len());
    for service in request.services {
        if !service.artifact_root.is_absolute() {
            return Err(CliError::SchemaInvalid(format!(
                "services[{}].artifactRoot must be an absolute path",
                service.key
            )));
        }
        let validated = validate_service_artifact_closure(
            &service.artifact_root,
            &service.service_id,
            service.service_version.as_deref(),
            &service.service_assembly.assembly_identity,
            &service.service_assembly.assembly_path,
            &service.service_unit,
            &service.package_units,
        )
        .map_err(CliError::Identity)?;
        results.push(RuntimeProgramBuildIdResult {
            key: service.key,
            dynamic_build_id: validated.dynamic_build_id,
            assembly_identity: validated.assembly_identity,
            service_assembly: validated.service_assembly,
            service_unit: validated.service_unit,
            package_units: validated.package_units,
        });
    }

    serde_json::to_writer(io::stdout(), &RuntimeProgramBuildIdResponse { results })
        .map_err(|error| CliError::Internal(format!("failed to write stdout: {error}")))?;
    Ok(())
}

fn package_unit_identities() -> Result<(), CliError> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| CliError::Internal(format!("failed to read stdin: {error}")))?;
    let request: PackageUnitIdentitiesRequest =
        serde_json::from_str(&input).map_err(|error| CliError::SchemaInvalid(error.to_string()))?;
    let package_unit: PackageUnit = serde_json::from_value(request.package_unit)
        .map_err(|error| CliError::SchemaInvalid(format!("packageUnit is invalid: {error}")))?;
    let response = PackageUnitIdentitiesResponse {
        build_identity: package_build_identity(&package_unit).map_err(CliError::Identity)?,
        abi_identity: package_local_abi_identity(&package_unit).map_err(CliError::Identity)?,
    };

    serde_json::to_writer(io::stdout(), &response)
        .map_err(|error| CliError::Internal(format!("failed to write stdout: {error}")))?;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeProgramBuildIdRequest {
    services: Vec<RuntimeProgramBuildIdService>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeProgramBuildIdService {
    key: String,
    artifact_root: PathBuf,
    service_id: String,
    #[serde(default, deserialize_with = "deserialize_optional_service_version")]
    service_version: Option<String>,
    service_assembly: ServiceAssemblyArtifactRef,
    service_unit: ServiceUnitArtifactRef,
    package_units: Vec<PackageUnitArtifactRef>,
}

fn deserialize_optional_service_version<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    match Value::deserialize(deserializer)? {
        Value::String(version) if !version.is_empty() => Ok(Some(version)),
        Value::String(_) => Err(D::Error::custom(
            "serviceVersion must be a non-empty string",
        )),
        _ => Err(D::Error::custom(
            "serviceVersion must be a non-empty string when present",
        )),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeProgramBuildIdResponse {
    results: Vec<RuntimeProgramBuildIdResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeProgramBuildIdResult {
    key: String,
    dynamic_build_id: String,
    assembly_identity: String,
    service_assembly: ValidatedArtifactContent,
    service_unit: ValidatedArtifactContent,
    package_units: Vec<ValidatedArtifactContent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackageUnitIdentitiesRequest {
    package_unit: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageUnitIdentitiesResponse {
    build_identity: String,
    abi_identity: String,
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: CliErrorBody,
}

#[derive(Debug, Serialize)]
struct CliErrorBody {
    code: &'static str,
    message: String,
}

#[derive(Debug)]
enum CliError {
    SchemaInvalid(String),
    Identity(ArtifactIdentityError),
    Internal(String),
}

impl CliError {
    fn code(&self) -> &'static str {
        match self {
            Self::SchemaInvalid(_) => "schema_invalid",
            Self::Identity(error) => identity_error_code(error),
            Self::Internal(_) => "internal_error",
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SchemaInvalid(message) | Self::Internal(message) => formatter.write_str(message),
            Self::Identity(error) => write!(formatter, "{error}"),
        }
    }
}

fn identity_error_code(error: &ArtifactIdentityError) -> &'static str {
    match error {
        ArtifactIdentityError::InvalidServiceUnit(_)
        | ArtifactIdentityError::InvalidServiceAssembly { .. }
        | ArtifactIdentityError::ServiceAssemblyIdentityMismatch { .. }
        | ArtifactIdentityError::ServiceAssemblyProtocolIdentityMismatch { .. }
        | ArtifactIdentityError::InvalidRuntimeProgramBuildIdentity { .. }
        | ArtifactIdentityError::ServiceUnitPointerMismatch { .. }
        | ArtifactIdentityError::ServiceUnitVersionMismatch { .. }
        | ArtifactIdentityError::InvalidPackageUnit { .. }
        | ArtifactIdentityError::PackageUnitSchemaVersionMismatch { .. }
        | ArtifactIdentityError::InvalidPackageIndex { .. }
        | ArtifactIdentityError::ParseArtifactJson { .. }
        | ArtifactIdentityError::InvalidPublicationId { .. }
        | ArtifactIdentityError::PackageBuildIdentityMismatch { .. }
        | ArtifactIdentityError::PackageAbiIdentityMismatch { .. } => "schema_invalid",
        ArtifactIdentityError::ArtifactNotFound { .. } => "artifact_not_found",
        ArtifactIdentityError::ResolveArtifactPath { source, .. }
        | ArtifactIdentityError::ReadArtifact { source, .. }
            if source.kind() == io::ErrorKind::NotFound =>
        {
            "artifact_not_found"
        }
        ArtifactIdentityError::PackageDependencyCycle { .. } => "dependency_cycle",
        ArtifactIdentityError::PackageDependencyConflict { .. } => "dependency_conflict",
        ArtifactIdentityError::PackageUnitPointerMismatch { .. } => "schema_invalid",
        ArtifactIdentityError::PathEscape { .. }
        | ArtifactIdentityError::ArtifactPathEscapesRoot { .. }
        | ArtifactIdentityError::NonCanonicalArtifactPath { .. }
        | ArtifactIdentityError::InvalidArtifactSegment { .. } => "path_escape",
        _ => "internal_error",
    }
}
