use std::{
    io::{self, Read},
    path::PathBuf,
    process::ExitCode,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_artifact_identity::{
    package_abi_identity, package_build_identity,
    runtime_program_dynamic_build_id_from_artifact_refs,
    runtime_program_dynamic_build_id_from_artifact_root, ArtifactIdentityError,
    PackageUnitArtifactRef,
};
use skiff_artifact_model::{PackageUnit, ServiceUnit};

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
    if !request.artifact_root.is_absolute() {
        return Err(CliError::SchemaInvalid(
            "artifactRoot must be an absolute path".to_string(),
        ));
    }

    let mut results = Vec::with_capacity(request.services.len());
    for service in request.services {
        let service_unit: ServiceUnit = serde_json::from_value(service.service_unit)
            .map_err(|error| CliError::SchemaInvalid(format!("serviceUnit is invalid: {error}")))?;
        let dynamic_build_id = if let Some(package_units) = service.package_units {
            runtime_program_dynamic_build_id_from_artifact_refs(
                &request.artifact_root,
                &service_unit,
                &package_units
                    .into_iter()
                    .map(Into::into)
                    .collect::<Vec<_>>(),
            )
        } else {
            runtime_program_dynamic_build_id_from_artifact_root(
                &request.artifact_root,
                &service_unit,
            )
        }
        .map_err(CliError::Identity)?;
        results.push(RuntimeProgramBuildIdResult {
            key: service.key,
            dynamic_build_id,
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
        abi_identity: package_abi_identity(&package_unit).map_err(CliError::Identity)?,
    };

    serde_json::to_writer(io::stdout(), &response)
        .map_err(|error| CliError::Internal(format!("failed to write stdout: {error}")))?;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeProgramBuildIdRequest {
    artifact_root: PathBuf,
    services: Vec<RuntimeProgramBuildIdService>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeProgramBuildIdService {
    key: String,
    service_unit: Value,
    #[serde(default)]
    package_units: Option<Vec<RuntimeProgramPackageUnitRef>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeProgramPackageUnitRef {
    package_id: String,
    version: String,
    build_identity: String,
    abi_identity: String,
    #[serde(default)]
    unit_hash: Option<String>,
    unit_path: PathBuf,
}

impl From<RuntimeProgramPackageUnitRef> for PackageUnitArtifactRef {
    fn from(value: RuntimeProgramPackageUnitRef) -> Self {
        Self {
            package_id: value.package_id,
            version: value.version,
            build_identity: value.build_identity,
            abi_identity: value.abi_identity,
            unit_hash: value.unit_hash,
            unit_path: value.unit_path,
        }
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
        | ArtifactIdentityError::InvalidArtifactSegment { .. } => "path_escape",
        _ => "internal_error",
    }
}
