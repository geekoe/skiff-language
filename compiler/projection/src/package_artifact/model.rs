use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallableSemanticFacts, ContractRequirement, PackageArtifact, PackageExportIndex,
    PackageRequirement, PackageRuntimeRequirements, ServiceCallRef, ServiceRequirement,
};
use skiff_compiler_projection_input::{
    ProjectionExecutableKey, ProjectionPackageCallableSignatureFacts,
};

use crate::{
    package_exports::PackageExports,
    package_unit_artifacts::{PackageFileIrProjection, ProjectedPublicationResource},
};

/// Fully typed inputs consumed by the canonical PackageArtifact projector.
///
/// The export index and executable-keyed facts are produced before this
/// boundary. No source text, deployment config, provider artifact, or legacy
/// aggregate is accepted here.
pub struct PackageArtifactProjectionInput<'a> {
    pub package_id: &'a str,
    pub package_version: &'a str,
    pub api_exports: &'a PackageExports,
    pub export_index: PackageExportIndex,
    pub file_ir_units: Vec<PackageFileIrProjection>,
    pub resources: Vec<ProjectedPublicationResource>,
    pub package_requirements: Vec<PackageRequirement>,
    pub contract_requirements: Vec<ContractRequirement>,
    pub service_requirements: Vec<ServiceRequirement>,
    pub runtime_requirements: PackageRuntimeRequirements,
    pub callable_semantic_facts: BTreeMap<ProjectionExecutableKey, CallableSemanticFacts>,
    /// Exact canonical signature set. File IR is an implementation/link leaf,
    /// never a fallback signature source at this boundary.
    pub callable_signatures: ProjectionPackageCallableSignatureFacts,
    pub service_call_refs: Vec<ServiceCallRef>,
}

#[derive(Debug, Clone)]
pub struct ProjectedPackageArtifact {
    pub artifact: PackageArtifact,
    pub file_ir_units: Vec<PackageFileIrProjection>,
    pub resources: Vec<ProjectedPublicationResource>,
}
