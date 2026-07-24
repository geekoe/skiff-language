#![allow(dead_code)]

use skiff_artifact_model::{ContractTypeRef, PackageSchemaTypeId};
use skiff_compiler::{PublishedPackageArtifact, ResolvedPackageSchema};

pub fn resolved_package_schema(
    alias: impl Into<String>,
    package: &PublishedPackageArtifact,
) -> Result<ResolvedPackageSchema, skiff_compiler::ResolvedPackageSchemaError> {
    ResolvedPackageSchema::new(
        alias.into(),
        package.artifact.package_id.clone(),
        package.artifact.package_version.clone(),
        package.artifact.package_build_id.clone(),
        package
            .artifact
            .package_local_abi
            .local_abi_identity
            .clone(),
        package.package_schema_index.clone(),
        package.package_schema_type_records.clone(),
    )
}

pub fn public_contract_type(
    package: &PublishedPackageArtifact,
    stable_schema_key: &str,
) -> (ContractTypeRef, PackageSchemaTypeId) {
    let entry = package
        .package_schema_index
        .types
        .get(stable_schema_key)
        .unwrap_or_else(|| panic!("missing public package schema type `{stable_schema_key}`"));
    (
        ContractTypeRef::package_schema(
            package.artifact.package_id.clone(),
            stable_schema_key,
            entry.package_schema_type_id.clone(),
        ),
        entry.package_schema_type_id.clone(),
    )
}
