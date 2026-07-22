use skiff_artifact_model::{
    BoundaryUnavailableReason, ContractOperationId, PackageBuildId, PackageCallableId,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("invalid typed {artifact}: {identity_error}")]
    InvalidTypedArtifact {
        artifact: &'static str,
        identity_error: skiff_artifact_identity::ArtifactIdentityError,
    },
    #[error("deployment contract reference {field} mismatch: expected {expected}, got {actual}")]
    ContractReferenceMismatch {
        field: &'static str,
        expected: String,
        actual: String,
    },
    #[error("package closure repeats build {build_id}")]
    DuplicatePackageBuild { build_id: PackageBuildId },
    #[error("implementation package build {build_id} is absent from the artifact closure")]
    MissingImplementation { build_id: PackageBuildId },
    #[error(
        "package build {build_id} reference {field} mismatch: expected {expected}, got {actual}"
    )]
    PackageReferenceMismatch {
        build_id: PackageBuildId,
        field: &'static str,
        expected: String,
        actual: String,
    },
    #[error("package build {build_id} is not reachable from the implementation package")]
    UnreachablePackage { build_id: PackageBuildId },
    #[error("operation {operation_id} is mapped more than once")]
    DuplicateOperationBinding { operation_id: ContractOperationId },
    #[error("contract operation {operation_id} has no deployment mapping")]
    MissingOperationBinding { operation_id: ContractOperationId },
    #[error("deployment maps unknown contract operation {operation_id}")]
    UnknownOperationBinding { operation_id: ContractOperationId },
    #[error("package public path {public_path} does not exist")]
    UnknownPublicPath { public_path: String },
    #[error("package public path {public_path} is not a callable")]
    PublicPathNotCallable { public_path: String },
    #[error(
        "callable {callable_id} selected for operation {operation_id} is boundary unavailable: {reasons:?}"
    )]
    BoundaryUnavailable {
        operation_id: ContractOperationId,
        callable_id: PackageCallableId,
        reasons: Vec<BoundaryUnavailableReason>,
    },
    #[error(
        "callable {callable_id} boundary contract does not match operation {operation_id} descriptor"
    )]
    OperationContractMismatch {
        operation_id: ContractOperationId,
        callable_id: PackageCallableId,
    },
    #[error("operation {operation_id} is not the canonical WebSocket ingress ABI: {message}")]
    InvalidWebSocketIngressContract {
        operation_id: ContractOperationId,
        message: String,
    },
    #[error("callable {callable_id} fails independent boundary eligibility: {reasons:?}")]
    BoundaryEligibilityViolation {
        callable_id: PackageCallableId,
        reasons: Vec<BoundaryUnavailableReason>,
    },
    #[error(
        "callable {callable_id} semantic facts conflict with its boundary projection: {message}"
    )]
    CallableFactsMismatch {
        callable_id: PackageCallableId,
        message: String,
    },
    #[error("missing {kind} binding {key}")]
    MissingRequirementBinding { kind: &'static str, key: String },
    #[error("unexpected {kind} binding {key}")]
    ExtraRequirementBinding { kind: &'static str, key: String },
    #[error("{kind} binding {key} does not satisfy its typed requirement: {message}")]
    RequirementBindingMismatch {
        kind: &'static str,
        key: String,
        message: String,
    },
    #[error("typed {kind} requirement {key} is ambiguous: {message}")]
    ConflictingRequirement {
        kind: &'static str,
        key: String,
        message: String,
    },
    #[error(transparent)]
    ArtifactIdentity(#[from] skiff_artifact_identity::ArtifactIdentityError),
}

pub type ProjectionResult<T> = std::result::Result<T, ProjectionError>;
