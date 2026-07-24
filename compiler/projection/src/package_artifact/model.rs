use std::path::PathBuf;

use skiff_artifact_model::{
    ContractRequirement, FileIrUnit, PackageArtifact, PackageRequirement, ServiceCallRef,
    ServiceRequirement,
};
use skiff_compiler_projection_input::{ProjectionView, ResolvedPackageSchema};

use super::api_exports::PackageExports;

/// Complete typed input for terminal PackageArtifact projection.
///
/// Projection owns exports, executable signatures, semantic facts, runtime
/// requirements, File IR references, and resource projection. The driver only
/// supplies coordinates plus already-resolved dependency/call facts.
pub struct PackageArtifactProjectionInput<'a> {
    pub package_id: &'a str,
    pub package_version: &'a str,
    pub projection: ProjectionView<'a>,
    pub package_requirements: Vec<PackageRequirement>,
    pub resolved_package_schemas: &'a [ResolvedPackageSchema],
    pub contract_requirements: Vec<ContractRequirement>,
    pub service_requirements: Vec<ServiceRequirement>,
    pub service_call_refs: Vec<ServiceCallRef>,
}

pub(super) struct PackageExportLinkProjectionInput<'a> {
    pub package_id: &'a str,
    pub exports: &'a PackageExports,
    pub file_ir_units: &'a [FileIrUnit],
}

#[derive(Debug, Clone)]
pub struct ProjectedPackageArtifact {
    pub artifact: PackageArtifact,
    pub file_ir_units: Vec<FileIrUnit>,
    pub resources: Vec<ProjectedPackageResource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedPackageResource {
    pub path: String,
    pub absolute_path: PathBuf,
    pub byte_len: u64,
    pub sha256: String,
    pub content_type: Option<String>,
}
