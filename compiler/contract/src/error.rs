use std::collections::BTreeMap;

use skiff_artifact_model::BoundaryUnavailableReason;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContractDefinitionError {
    #[error("ServiceContractDefinition operations must contain at least one operation")]
    EmptyOperations,
    #[error("a zero-operation ServiceContractDefinition cannot require package schema types")]
    ZeroOperationTypeRequirements,
    #[error("ServiceContractDefinition {kind} key must be a non-empty string")]
    EmptyStableKey { kind: &'static str },
    #[error("contract diagnostic text references unknown operation stable key {key}")]
    UnknownDiagnosticOperation { key: String },
    #[error("contract diagnostic text references unknown type stable key {key}")]
    UnknownDiagnosticType { key: String },
    #[error("service API projection has no public callable for boundary projection {callable_id}")]
    MissingPublicCallable { callable_id: String },
    #[error("service API projection has no boundary projection for public callable {callable_id}")]
    MissingBoundaryProjection { callable_id: String },
    #[error("serviceCalls contains duplicate public path {path}")]
    DuplicateServiceCallPath { path: String },
    #[error(
        "serviceCalls public path {path} is not present in the Package Local ABI public graph"
    )]
    UnknownServiceCallPath { path: String },
    #[error("serviceCalls public path {path} names a {kind}, not a callable or public instance")]
    NonCallableServiceCallPath { path: String, kind: &'static str },
    #[error(
        "serviceCalls public path {path} selects public-instance method {public_instance}; select the public instance root instead"
    )]
    PublicInstanceMethodSelection {
        path: String,
        public_instance: String,
    },
    #[error(
        "serviceCalls public path {path} aliases public-instance callable {callable_id} at {method_paths:?}"
    )]
    PublicInstanceMethodAlias {
        path: String,
        callable_id: String,
        method_paths: Vec<String>,
    },
    #[error(
        "public instance {public_instance} method {method_path} maps {callable_id}, but its public callable symbol is missing or maps another exact callable"
    )]
    InvalidPublicInstanceMethod {
        public_instance: String,
        method_path: String,
        callable_id: String,
    },
    #[error(
        "service API projection lacks exact provider-free public-instance contract facts for selected roots {public_instances:?}"
    )]
    MissingPublicInstanceContractFacts { public_instances: Vec<String> },
    #[error("public-instance operation facts contain invalid public root `{public_instance}`")]
    InvalidPublicInstanceRoot { public_instance: String },
    #[error("public instance {public_instance} has an empty interface ABI id")]
    EmptyPublicInstanceInterfaceAbi { public_instance: String },
    #[error(
        "public instance {public_instance} interface {canonical_interface} retains an open type parameter"
    )]
    OpenPublicInstanceInterface {
        public_instance: String,
        canonical_interface: String,
    },
    #[error("public-instance operation facts contain an empty method ABI id")]
    EmptyPublicInstanceMethodAbi,
    #[error("public-instance operation facts contain an empty operation stable key")]
    EmptyPublicInstanceOperationStableKey,
    #[error("public instance {public_instance} repeats exact interface {canonical_interface}")]
    DuplicatePublicInstanceInterface {
        public_instance: String,
        canonical_interface: String,
    },
    #[error(
        "public instance {public_instance} interface {canonical_interface} has duplicate or empty method ABI id `{method_abi_id}`"
    )]
    DuplicateOrEmptyPublicInstanceMethodAbi {
        public_instance: String,
        canonical_interface: String,
        method_abi_id: String,
    },
    #[error("public-instance operation facts repeat operation stable key {operation_stable_key}")]
    DuplicatePublicInstanceOperation { operation_stable_key: String },
    #[error("selected public instance {public_instance} has no exact operation facts")]
    MissingSelectedPublicInstanceOperationFacts { public_instance: String },
    #[error(
        "public instance {public_instance} operation facts include unselected operation {operation_stable_key}"
    )]
    UnexpectedPublicInstanceOperation {
        public_instance: String,
        operation_stable_key: String,
    },
    #[error(
        "public instance {public_instance} operation facts omit selected operations {operation_stable_keys:?}"
    )]
    MissingPublicInstanceOperations {
        public_instance: String,
        operation_stable_keys: Vec<String>,
    },
    #[error(
        "public instance {public_instance} references unknown contract operation stable key {operation_stable_key}"
    )]
    UnknownPublicInstanceOperation {
        public_instance: String,
        operation_stable_key: String,
    },
    #[error(
        "public-instance operation tables repeat contract operation id {contract_operation_id}"
    )]
    DuplicatePublicInstanceOperationId { contract_operation_id: String },
    #[error(
        "Package Local ABI public instance {public_path} declares nested instance id {instance_id}"
    )]
    PublicInstanceRootIdentityMismatch {
        public_path: String,
        instance_id: String,
    },
    #[error(
        "public instance {public_instance} exact operation facts do not all-and-only cover its Package Local ABI methods"
    )]
    PublicInstanceOperationCoverage { public_instance: String },
    #[error(
        "public instance {public_instance} exact operation key {operation_stable_key} is not a Package Local ABI callable"
    )]
    InvalidPublicInstanceOperationFact {
        public_instance: String,
        operation_stable_key: String,
    },
    #[error("service API projection repeats operation stable key {operation_stable_key}")]
    DuplicateServiceOperationStableKey { operation_stable_key: String },
    #[error(
        "service API projection maps callable {callable_id} to multiple public paths: {first}, {second}"
    )]
    DuplicatePublicCallable {
        callable_id: String,
        first: String,
        second: String,
    },
    #[error("selected serviceCalls are unavailable: {unavailable:?}")]
    UnavailableServiceCalls {
        unavailable: BTreeMap<String, Vec<BoundaryUnavailableReason>>,
    },
    #[error("service API schema type {public_path} has no implementation type source")]
    MissingPublicTypeSource { public_path: String },
    #[error("service API schema references missing or private package type {symbol}")]
    MissingReachablePackageType { symbol: String },
    #[error("service API schema type {public_path} uses unsupported boundary type {kind}")]
    UnsupportedPackageSchemaType {
        public_path: String,
        kind: &'static str,
    },
    #[error(transparent)]
    Identity(#[from] skiff_artifact_identity::ArtifactIdentityError),
}

pub type Result<T> = std::result::Result<T, ContractDefinitionError>;
