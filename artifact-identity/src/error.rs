use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArtifactIdentityError {
    #[error("service unit must be a JSON object")]
    ServiceUnitMustBeObject,
    #[error("failed to serialize File IR identity payload: {0}")]
    SerializeFileIrIdentity(serde_json::Error),
    #[error("File IR unit declared fileIrIdentity {declared} but content identity is {computed}")]
    FileIrIdentityMismatch { declared: String, computed: String },
    #[error("failed to serialize package build identity payload: {0}")]
    SerializePackageBuildIdentity(serde_json::Error),
    #[error("failed to serialize package ABI identity payload: {0}")]
    SerializePackageAbiIdentity(serde_json::Error),
    #[error("failed to serialize package implementation links identity payload: {0}")]
    SerializePackageImplementationLinksIdentity(serde_json::Error),
    #[error(
        "package unit declared buildIdentity {declared} but content build identity is {computed}"
    )]
    PackageBuildIdentityMismatch { declared: String, computed: String },
    #[error("package unit declared abiIdentity {declared} but content ABI identity is {computed}")]
    PackageAbiIdentityMismatch { declared: String, computed: String },
    #[error(
        "package unit coordinate {package_id}@{package_version} does not match nested publicationAbi coordinate {publication_id}@{publication_version}"
    )]
    PackagePublicationCoordinateMismatch {
        package_id: String,
        package_version: String,
        publication_id: String,
        publication_version: String,
    },
    #[error("package identity input is invalid: {message}")]
    InvalidPackageIdentityInput { message: String },
    #[error("failed to serialize service unit for runtime program identity: {0}")]
    SerializeServiceUnit(serde_json::Error),
    #[error("failed to serialize service unit storage identity payload: {0}")]
    SerializeServiceUnitStorageIdentity(serde_json::Error),
    #[error("failed to serialize operation ABI identity payload: {0}")]
    SerializeOperationAbiIdentity(serde_json::Error),
    #[error("service unit is invalid: {0}")]
    InvalidServiceUnit(serde_json::Error),
    #[error("service assembly is invalid: {message}")]
    InvalidServiceAssembly { message: String },
    #[error("service assembly declared identity {declared} but content identity is {computed}")]
    ServiceAssemblyIdentityMismatch { declared: String, computed: String },
    #[error(
        "service assembly protocolIdentity {assembly} does not match service unit protocolIdentity {service_unit}"
    )]
    ServiceAssemblyProtocolIdentityMismatch {
        assembly: String,
        service_unit: String,
    },
    #[error(
        "runtime program build identity {identity} must use skiff-service-build-v1:sha256:<64 lowercase hex>"
    )]
    InvalidRuntimeProgramBuildIdentity { identity: String },
    #[error(
        "service unit pointer for {path} declared {field} {expected} but service unit has {actual}"
    )]
    ServiceUnitPointerMismatch {
        path: String,
        field: &'static str,
        expected: String,
        actual: String,
    },
    #[error("package unit {path} is invalid: {source}")]
    InvalidPackageUnit {
        path: String,
        source: serde_json::Error,
    },
    #[error("package unit {path} schemaVersion must be {expected}, got {actual}")]
    PackageUnitSchemaVersionMismatch {
        path: String,
        expected: &'static str,
        actual: String,
    },
    #[error("failed to serialize runtime program service unit identity: {0}")]
    SerializeRuntimeProgramServiceUnitIdentity(serde_json::Error),
    #[error("{label} missing publicationAbi")]
    MissingPublicationAbi { label: String },
    #[error("{label} publicationAbi is invalid: {source}")]
    InvalidPublicationAbi {
        label: String,
        source: serde_json::Error,
    },
    #[error("failed to serialize publicationAbi identity payload: {0}")]
    SerializePublicationAbiIdentity(serde_json::Error),
    #[error("publicationAbi surface is invalid: {message}")]
    InvalidPublicationAbiSurface { message: String },
    #[error("publicationAbi declared abiIdentity {declared} but content identity is {computed}")]
    PublicationAbiIdentityMismatch { declared: String, computed: String },
    #[error("failed to serialize package test build identity payload: {0}")]
    SerializePackageTestBuildIdentity(serde_json::Error),
    #[error(
        "package test assembly declared testBuildIdentity {declared} but content identity is {computed}"
    )]
    PackageTestBuildIdentityMismatch { declared: String, computed: String },
    #[error(
        "package test entrypoint {entrypoint_local_id} declared entrypointId {declared} but derived id is {computed}"
    )]
    PackageTestEntrypointIdMismatch {
        entrypoint_local_id: String,
        declared: String,
        computed: String,
    },
    #[error("artifact path {path} for {label} must be relative and stay inside artifacts root")]
    PathEscape { label: String, path: String },
    #[error("artifact {path} was not found")]
    ArtifactNotFound { path: String },
    #[error("failed to read artifact {path}: {source}")]
    ReadArtifact {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse artifact {path}: {source}")]
    ParseArtifactJson {
        path: String,
        source: serde_json::Error,
    },
    #[error("failed to resolve artifact root {path}: {source}")]
    ResolveArtifactRoot {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to resolve artifact {path}: {source}")]
    ResolveArtifactPath {
        path: String,
        source: std::io::Error,
    },
    #[error("{label} path {path} escapes artifacts root {root}")]
    ArtifactPathEscapesRoot {
        label: String,
        path: String,
        root: String,
    },
    #[error("package dependency cycle includes {package_id}")]
    PackageDependencyCycle { package_id: String },
    #[error("package {package_id} is resolved to both {existing_build} and {new_build}")]
    PackageDependencyConflict {
        package_id: String,
        existing_build: String,
        new_build: String,
    },
    #[error(
        "package unit pointer for {path} declared {field} {expected} but package unit has {actual}"
    )]
    PackageUnitPointerMismatch {
        path: String,
        field: &'static str,
        expected: String,
        actual: String,
    },
    #[error("artifact path {path} is not canonical for {label}; expected {expected}")]
    NonCanonicalArtifactPath {
        label: String,
        path: String,
        expected: String,
    },
    #[error("{message}")]
    InvalidPackageIndex { message: String },
    #[error("{label} {value} must be a publication id")]
    InvalidPublicationId { label: String, value: String },
    #[error("{label} {value} is not a safe artifact path segment")]
    InvalidArtifactSegment { label: String, value: String },
}

pub type Result<T> = std::result::Result<T, ArtifactIdentityError>;
