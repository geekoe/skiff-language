use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArtifactIdentityError {
    #[error("failed to serialize actor ABI identity input: {0}")]
    SerializeActorAbiIdentity(serde_json::Error),
    #[error("failed to serialize actor method identity input: {0}")]
    SerializeActorMethodIdentity(serde_json::Error),
    #[error("failed to serialize actor implementation identity input: {0}")]
    SerializeActorImplementationIdentity(serde_json::Error),
    #[error("actor implementation identity input is invalid: {message}")]
    InvalidActorImplementationIdentityInput { message: String },
    #[error("failed to serialize package schema type identity payload: {0}")]
    SerializePackageSchemaTypeIdentity(serde_json::Error),
    #[error("failed to serialize package schema index identity payload: {0}")]
    SerializePackageSchemaIndexIdentity(serde_json::Error),
    #[error("failed to serialize contract operation identity payload: {0}")]
    SerializeContractOperationIdentity(serde_json::Error),
    #[error("failed to serialize gateway entry identity payload: {0}")]
    SerializeGatewayEntryIdentity(serde_json::Error),
    #[error("gateway entry protocol surface is invalid: {message}")]
    InvalidGatewayEntryProtocolSurface { message: String },
    #[error(
        "gateway entry identity {identity} must use {prefix}:<64 lowercase hex>",
        prefix = crate::GATEWAY_ENTRY_IDENTITY_PREFIX
    )]
    InvalidGatewayEntryIdentity { identity: String },
    #[error("failed to serialize service protocol identity payload: {0}")]
    SerializeServiceProtocolIdentity(serde_json::Error),
    #[error(
        "service protocol identity {identity} must use skiff-service-protocol-v5:sha256:<64 lowercase hex>"
    )]
    InvalidServiceProtocolIdentity { identity: String },
    #[error("service contract is invalid: {message}")]
    InvalidServiceContract { message: String },
    #[error(
        "service contract declared protocol identity {declared} but content identity is {computed}"
    )]
    ServiceProtocolIdentityMismatch { declared: String, computed: String },
    #[error("failed to serialize deployment artifact identity payload: {0}")]
    SerializeDeploymentArtifactIdentity(serde_json::Error),
    #[error("service deployment is invalid: {message}")]
    InvalidServiceDeployment { message: String },
    #[error(
        "service deployment declared artifact identity {declared} but content identity is {computed}"
    )]
    DeploymentArtifactIdentityMismatch { declared: String, computed: String },
    #[error("service deployment reference is invalid: {message}")]
    ServiceDeploymentRefMismatch { message: String },
    #[error("failed to serialize runtime assembly identity payload: {0}")]
    SerializeAssemblyIdentity(serde_json::Error),
    #[error("runtime assembly is invalid: {message}")]
    InvalidRuntimeAssembly { message: String },
    #[error("runtime assembly declared identity {declared} but content identity is {computed}")]
    AssemblyIdentityMismatch { declared: String, computed: String },
    #[error("failed to serialize PackageArtifact local ABI identity payload: {0}")]
    SerializePackageArtifactLocalAbiIdentity(serde_json::Error),
    #[error("failed to serialize PackageArtifact build identity payload: {0}")]
    SerializePackageArtifactBuildIdentity(serde_json::Error),
    #[error("PackageArtifact is invalid: {message}")]
    InvalidPackageArtifact { message: String },
    #[error(
        "PackageArtifact declared local ABI identity {declared} but content identity is {computed}"
    )]
    PackageArtifactLocalAbiIdentityMismatch { declared: String, computed: String },
    #[error(
        "PackageArtifact declared build identity {declared} but content identity is {computed}"
    )]
    PackageArtifactBuildIdentityMismatch { declared: String, computed: String },
    #[error("failed to serialize File IR identity payload: {0}")]
    SerializeFileIrIdentity(serde_json::Error),
    #[error("File IR {field} must be {expected}, got {actual}")]
    FileIrGenerationMismatch {
        field: &'static str,
        expected: &'static str,
        actual: String,
    },
    #[error("File IR service call refs are invalid: {0}")]
    InvalidFileIrServiceCalls(#[from] skiff_artifact_model::FileIrServiceCallValidationError),
    #[error("File IR package call refs are invalid: {0}")]
    InvalidFileIrPackageCalls(#[from] skiff_artifact_model::FileIrPackageCallValidationError),
    #[error("File IR unit declared fileIrIdentity {declared} but content identity is {computed}")]
    FileIrIdentityMismatch { declared: String, computed: String },
    #[error("artifact path {path} for {label} must be relative and stay inside artifacts root")]
    PathEscape { label: String, path: String },
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
    #[error("artifact path {path} is not canonical for {label}; expected {expected}")]
    NonCanonicalArtifactPath {
        label: String,
        path: String,
        expected: String,
    },
    #[error("{label} {value} must be a publication id")]
    InvalidPublicationId { label: String, value: String },
    #[error("{label} {value} is not a safe artifact path segment")]
    InvalidArtifactSegment { label: String, value: String },
}

pub type Result<T> = std::result::Result<T, ArtifactIdentityError>;
