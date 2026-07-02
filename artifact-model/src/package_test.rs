use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::package_unit::ConfigAndEffectMetadata;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageTestAssembly {
    pub schema_version: String,
    pub kind: PackageTestAssemblyKind,
    pub package_id: String,
    pub package_version: String,
    pub test_build_identity: String,
    pub production_package_unit: PackageTestPackageUnitRef,
    pub test_files: Vec<PackageTestFileIrRef>,
    pub dependency_package_units: Vec<PackageTestPackageUnitRef>,
    pub test_entrypoints: Vec<PackageTestEntrypoint>,
    pub link_policy: PackageTestLinkPolicy,
    pub config_and_effect_metadata: ConfigAndEffectMetadata,
    pub source_map: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PackageTestAssemblyKind {
    PackageTest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageTestEntrypoint {
    pub kind: PackageTestEntrypointKind,
    pub entrypoint_local_id: String,
    pub entrypoint_id: String,
    pub display_name: String,
    pub source_path: String,
    pub module_path: String,
    pub owner_test_file: PackageTestFileIrRef,
    pub executable_ref: PackageTestExecutableRef,
    pub default_run: bool,
    pub config_and_effect_metadata: ConfigAndEffectMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_expected_error: Option<PackageTestRuntimeExpectedError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PackageTestEntrypointKind {
    TestOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageTestRuntimeExpectedError {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_contains: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageTestExecutableRef {
    pub file_ir_identity: String,
    pub executable_index: u32,
    pub executable_local_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageTestFileIrRef {
    pub file_ir_identity: String,
    pub file_ir_path: String,
    pub source_path: String,
    pub module_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageTestPackageUnitRef {
    pub package_id: String,
    pub version: String,
    pub build_identity: String,
    pub unit_path: String,
    pub public_abi_identity: String,
    pub implementation_links_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageTestLinkPolicy {
    pub current_package_production: PackageProductionLinkScope,
    pub test_file_scopes: Vec<PackageTestFileLinkScope>,
    pub dependency_public_scopes: Vec<PackageDependencyPublicLinkScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageProductionLinkScope {
    pub package_id: String,
    pub version: String,
    pub build_identity: String,
    pub files_digest: String,
    pub implementation_links_digest: String,
    pub allow_private: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageTestFileLinkScope {
    pub owner_test_file_identity: String,
    pub source_path: String,
    pub module_path: String,
    pub allowed_local_link_digest: String,
    pub entrypoint_local_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageDependencyPublicLinkScope {
    pub package_id: String,
    pub version: String,
    pub build_identity: String,
    pub public_abi_identity: String,
    pub public_export_digest: String,
    pub implementation_links_digest: String,
    pub allow_private: bool,
}
