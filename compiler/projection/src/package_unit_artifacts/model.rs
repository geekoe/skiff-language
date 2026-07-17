use std::path::PathBuf;

use skiff_artifact_model::{AbiIdentityFacts, FileIrUnit, PackageUnit};
use skiff_compiler_projection_input::{
    ProjectionCallableEffectFacts, PublicationResourceProjectionInput,
};

use crate::{package_exports::PackageExports, ConfigProjection};

#[derive(Debug, Clone)]
pub struct PackageFileIrProjection {
    pub unit: FileIrUnit,
    pub identity: String,
    pub source_ast_hash: String,
    pub module_path: String,
}

impl PackageFileIrProjection {
    pub fn from_unit(unit: FileIrUnit) -> Self {
        Self {
            identity: unit.file_ir_identity.clone(),
            source_ast_hash: unit.source_ast_hash.clone(),
            module_path: unit.module_path.clone(),
            unit,
        }
    }
}

pub struct PackageIrProjectionSource<'a> {
    pub package_id: &'a str,
    pub version: &'a str,
    pub exports: &'a PackageExports,
    pub abi_identity_projection: &'a AbiIdentityFacts,
    pub config_projection: &'a ConfigProjection,
    pub callable_effects: &'a ProjectionCallableEffectFacts,
    pub resources: &'a [PublicationResourceProjectionInput],
    pub file_ir_units: Vec<PackageFileIrProjection>,
}

#[derive(Debug, Clone)]
pub struct ProjectedPackageIrArtifacts {
    pub unit: PackageUnit,
    pub config_projection: ConfigProjection,
    pub file_ir_units: Vec<PackageFileIrProjection>,
    pub resources: Vec<ProjectedPublicationResource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedPublicationResource {
    pub path: String,
    pub absolute_path: PathBuf,
    pub byte_len: u64,
    pub sha256: String,
    pub content_type: Option<String>,
}
