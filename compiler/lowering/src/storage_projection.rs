use std::collections::BTreeMap;

use skiff_artifact_model::{
    ActorMetadataIr, DbDeclarationIr, DbIndexIr, DbMetadataIndexIr, DbMetadataIr, TaskTargetIr,
};
pub use skiff_compiler_core::dispatch_targets::PackageTaskTargetSource;

use crate::file_ir::FileIrUnit;
use skiff_compiler_source::{
    parsed_sources::ParsedCompilerSource, PackageSourceModel,
    SourceCompileError as PublicationError,
};

use super::{CompiledPackageSource, LoweredPackage};

#[derive(Clone, Debug, Default)]
pub struct CompiledPackageStorageProjection {
    pub db: Vec<DbMetadataIr>,
    pub actors: Vec<ActorMetadataIr>,
}

pub fn project_service_storage_projection(
    source_model: &PackageSourceModel,
    lowered: &LoweredPackage,
) -> Result<CompiledPackageStorageProjection, PublicationError> {
    service_storage_projection(
        source_model.sources().parsed_sources(),
        lowered.file_ir_units(),
        lowered.sources(),
    )
}

pub fn service_storage_projection(
    parsed_sources: &[ParsedCompilerSource],
    file_ir_units: &[FileIrUnit],
    sources: &[CompiledPackageSource],
) -> Result<CompiledPackageStorageProjection, PublicationError> {
    Ok(CompiledPackageStorageProjection {
        db: service_db_metadata(parsed_sources, file_ir_units, sources),
        // Explicit actor declarations are preserved as independent compiler/artifact
        // facts. Runtime execution metadata is intentionally not synthesized by this
        // compiler-only checkpoint.
        actors: Vec::new(),
    })
}

#[cfg(test)]
pub fn service_task_targets(
    file_ir_units: &[FileIrUnit],
    service_protocol_identity: &str,
) -> Result<Vec<TaskTargetIr>, PublicationError> {
    service_task_targets_with_packages(file_ir_units, &[], service_protocol_identity)
}

pub fn service_task_targets_with_packages(
    service_file_ir_units: &[FileIrUnit],
    package_sources: &[PackageTaskTargetSource],
    service_protocol_identity: &str,
) -> Result<Vec<TaskTargetIr>, PublicationError> {
    skiff_compiler_core::dispatch_targets::service_task_targets_with_packages(
        service_file_ir_units,
        package_sources,
        service_protocol_identity,
    )
    .map_err(|error| PublicationError::ContractValidation {
        message: error.message,
    })
}

fn service_db_metadata(
    parsed_sources: &[ParsedCompilerSource],
    file_ir_units: &[FileIrUnit],
    sources: &[CompiledPackageSource],
) -> Vec<DbMetadataIr> {
    let units_by_module = file_ir_units
        .iter()
        .zip(sources)
        .map(|(unit, source)| {
            (
                unit.module_path.as_str(),
                (unit, service_storage_role_for_source_role(source.role)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    parsed_sources
        .iter()
        .flat_map(|parsed| {
            let Some((unit, source_role)) = units_by_module.get(parsed.module_path()).copied()
            else {
                return Vec::new();
            };
            parsed
                .ast()
                .dbs
                .iter()
                .filter_map(|db| {
                    unit.declarations
                        .db
                        .get(&db.name)
                        .map(|db| service_db_entry(source_role, unit, db))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn service_db_entry(source_role: &str, unit: &FileIrUnit, db: &DbDeclarationIr) -> DbMetadataIr {
    DbMetadataIr {
        module_path: unit.module_path.clone(),
        source_role: source_role.to_string(),
        package_id: None,
        package_version: None,
        file_ir_identity: None,
        kind: db.kind,
        ty: db.type_ref.clone(),
        type_name: db.type_name.clone(),
        collection_name: db.collection_name.clone(),
        key: Some(db.key.clone()),
        fields: db.fields.clone(),
        retention: db.retention.clone(),
        leases: db.leases.clone(),
        indexes: db.indexes.iter().map(db_metadata_index).collect(),
    }
}

fn service_storage_role_for_source_role(
    role: skiff_compiler_core::source_role::PublicationSourceRole,
) -> &'static str {
    match role {
        skiff_compiler_core::source_role::PublicationSourceRole::Contract => "contract",
        skiff_compiler_core::source_role::PublicationSourceRole::Implementation
        | skiff_compiler_core::source_role::PublicationSourceRole::Package => "internal",
    }
}

fn db_metadata_index(index: &DbIndexIr) -> DbMetadataIndexIr {
    DbMetadataIndexIr {
        name: index.name.clone(),
        unique: index.unique,
        fields: index.fields.clone(),
    }
}

#[cfg(test)]
mod task_tests {
    use super::*;

    #[test]
    fn task_wrapper_matches_shared_core_for_empty_projection() {
        let wrapper_targets = service_task_targets_with_packages(&[], &[], "proto")
            .expect("wrapper should accept empty input");
        let core_targets =
            skiff_compiler_core::dispatch_targets::service_task_targets_with_packages(
                &[],
                &[],
                "proto",
            )
            .expect("core should accept empty input");

        assert_eq!(wrapper_targets, core_targets);
    }
}
