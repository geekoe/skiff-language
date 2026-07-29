#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    package_schema_descriptor_refs, ContractTypeRef, PackageSchemaTypeId, PackageSchemaTypeRecord,
};
use skiff_compiler::{PublishedPackageArtifact, ResolvedPackageSchema};

pub fn resolved_package_schema(
    alias: impl Into<String>,
    package: &PublishedPackageArtifact,
) -> Result<ResolvedPackageSchema, skiff_compiler::ResolvedPackageSchemaError> {
    let mut pending = package
        .package_schema_index
        .types
        .values()
        .map(|entry| entry.package_schema_type_id.clone())
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    let mut records = BTreeMap::<PackageSchemaTypeId, PackageSchemaTypeRecord>::new();
    while let Some(type_id) = pending.pop() {
        if !visited.insert(type_id.clone()) {
            continue;
        }
        let Some(record) = package
            .resolved_package_schema_type_records
            .get(&type_id)
            .cloned()
        else {
            continue;
        };
        pending.extend(
            package_schema_descriptor_refs(&record.canonical_descriptor.descriptor)
                .into_iter()
                .map(|reference| reference.package_schema_type_id),
        );
        records.insert(type_id, record);
    }
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
        records,
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
