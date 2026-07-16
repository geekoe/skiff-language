use std::collections::BTreeMap;

use skiff_artifact_identity::package_implementation_links_identity;
use skiff_compiler_core::artifact::{
    PackageDependencyPublicLinkScope, PackageTestPackageUnitRef, PackageUnit,
};
use skiff_compiler_core::json_utils::value_sha256;

use super::{
    PackageTestArtifactBuildError, PublishedFileIrArtifact,
    PublishedPackageTestPackageUnitArtifact, PublishedResourceArtifact,
};
use crate::emission::package_unit_artifacts::materialize_package_unit_artifact;

pub(super) fn materialize_projected_package_unit_for_test(
    package_id: &str,
    version: &str,
    files: Vec<PublishedFileIrArtifact>,
    resource_blobs: Vec<PublishedResourceArtifact>,
    unit: PackageUnit,
) -> Result<PublishedPackageTestPackageUnitArtifact, PackageTestArtifactBuildError> {
    if unit.package_id != package_id {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: format!(
                "dependency package unit id {} does not match input package id {package_id}",
                unit.package_id
            ),
        });
    }
    if unit.version != version {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: format!(
                "dependency package unit version {} does not match input package version {version}",
                unit.version
            ),
        });
    }
    let materialized =
        materialize_package_unit_artifact(&unit, &files, &resource_blobs).map_err(|error| {
            PackageTestArtifactBuildError::InvalidInput {
                message: format!(
                    "failed to materialize production package unit {package_id}@{version}: {error}"
                ),
            }
        })?;
    let unit = materialized.unit;
    let value = materialized.artifact.value;
    let unit_path = materialized.artifact.path;
    let resource_blobs = materialized.resource_blobs;
    let reference = PackageTestPackageUnitRef {
        package_id: package_id.to_string(),
        version: version.to_string(),
        build_identity: unit.build_identity.clone(),
        unit_path: unit_path.clone(),
        public_abi_identity: unit.abi_identity.clone(),
        implementation_links_identity: package_implementation_links_identity(
            &unit.implementation_links,
        )?,
    };
    Ok(PublishedPackageTestPackageUnitArtifact {
        files,
        resource_blobs,
        unit,
        value,
        unit_path,
        reference,
    })
}

#[derive(Clone)]
pub(super) struct DependencySlotRecord {
    pub(super) unit: PublishedPackageTestPackageUnitArtifact,
    pub(super) public_scope: PackageDependencyPublicLinkScope,
}

pub(super) fn normalize_dependency_slot_records(
    dependency_units: Vec<PublishedPackageTestPackageUnitArtifact>,
) -> Result<Vec<DependencySlotRecord>, PackageTestArtifactBuildError> {
    let mut records = dependency_units
        .into_iter()
        .map(|unit| {
            Ok(DependencySlotRecord {
                public_scope: dependency_public_link_scope(&unit.unit)?,
                unit,
            })
        })
        .collect::<Result<Vec<_>, PackageTestArtifactBuildError>>()?;
    records.sort_by(|left, right| {
        dependency_slot_sort_key(&left.unit.reference)
            .cmp(&dependency_slot_sort_key(&right.unit.reference))
    });

    let mut package_slots = BTreeMap::<String, (String, String)>::new();
    let mut slot_records = BTreeMap::<
        (String, String, String),
        (PackageTestPackageUnitRef, PackageDependencyPublicLinkScope),
    >::new();
    let mut normalized = Vec::new();
    for record in records {
        let reference = record.unit.reference.clone();
        let slot_key = dependency_slot_sort_key(&reference);
        if let Some((seen_version, seen_build_identity)) =
            package_slots.get(reference.package_id.as_str())
        {
            if seen_version != &reference.version
                || seen_build_identity != &reference.build_identity
            {
                return Err(PackageTestArtifactBuildError::InvalidInput {
                    message: format!(
                        "dependency package {} resolves to multiple package slots: {}@{} and {}@{}",
                        reference.package_id,
                        reference.version,
                        reference.build_identity,
                        seen_version,
                        seen_build_identity
                    ),
                });
            }
        } else {
            package_slots.insert(
                reference.package_id.clone(),
                (reference.version.clone(), reference.build_identity.clone()),
            );
        }

        if let Some((seen_ref, seen_scope)) = slot_records.get(&slot_key) {
            if seen_ref == &reference && seen_scope == &record.public_scope {
                continue;
            }
            if seen_ref == &reference {
                return Err(PackageTestArtifactBuildError::InvalidInput {
                    message: format!(
                        "dependency package {}@{} build {} has conflicting public scopes",
                        reference.package_id, reference.version, reference.build_identity
                    ),
                });
            }
            return Err(PackageTestArtifactBuildError::InvalidInput {
                message: format!(
                    "dependency package {}@{} build {} has conflicting package unit refs",
                    reference.package_id, reference.version, reference.build_identity
                ),
            });
        }
        slot_records.insert(slot_key, (reference, record.public_scope.clone()));
        normalized.push(record);
    }
    Ok(normalized)
}

fn dependency_slot_sort_key(reference: &PackageTestPackageUnitRef) -> (String, String, String) {
    (
        reference.package_id.clone(),
        reference.version.clone(),
        reference.build_identity.clone(),
    )
}

fn dependency_public_link_scope(
    unit: &PackageUnit,
) -> Result<PackageDependencyPublicLinkScope, PackageTestArtifactBuildError> {
    Ok(PackageDependencyPublicLinkScope {
        package_id: unit.package_id.clone(),
        version: unit.version.clone(),
        build_identity: unit.build_identity.clone(),
        public_abi_identity: unit.abi_identity.clone(),
        public_export_digest: value_sha256(
            &serde_json::to_value(&unit.publication_abi).expect("publication ABI must serialize"),
        ),
        implementation_links_digest: package_implementation_links_identity(
            &unit.implementation_links,
        )?,
        allow_private: false,
    })
}

pub(super) fn validate_unique_dependency_aliases(
    package_unit: &PackageUnit,
) -> Result<(), PackageTestArtifactBuildError> {
    let mut aliases = BTreeMap::<String, ()>::new();
    for dependency in &package_unit.dependencies {
        if aliases.insert(dependency.alias.clone(), ()).is_some() {
            return Err(PackageTestArtifactBuildError::InvalidInput {
                message: format!(
                    "package {} dependency alias {} is declared more than once",
                    package_unit.package_id, dependency.alias
                ),
            });
        }
    }
    Ok(())
}
