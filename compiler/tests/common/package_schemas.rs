#![allow(dead_code)]

use skiff_artifact_model::{ContractTypeRef, PackageSchemaTypeId};
use skiff_compiler::PublishedPackageArtifact;

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
