use std::collections::BTreeMap;

use crate::context::ProjectedServiceDependencyLockEntry;
use crate::package_unit_artifacts::ProjectedPublicationResource;
use crate::runtime::{GatewayEntry, OperationEntryIr, TimeoutEntry};
use crate::service::service_unit::ServicePackageConfigEntry;
use crate::source_map::PublicationSourceMap;
use crate::typed_artifacts::ServiceUnit;
use skiff_artifact_model::{DbMetadataIr, FileIrUnit, PackageUnit};

/// Owned service projection artifact payload.
///
/// These values are semantic projection products. They are intentionally
/// separate from final published JSON, artifact paths, and hashes so emission
/// can consume a borrow view without constructing the projection bundle itself.
pub struct ServiceArtifactProjection {
    pub package_configs: BTreeMap<String, ServicePackageConfigEntry>,
    pub dependency_lock: Vec<ProjectedServiceDependencyLockEntry>,
    pub db_metadata: Vec<DbMetadataIr>,
    pub operation_entries: Vec<OperationEntryIr>,
    pub gateway: GatewayEntry,
    pub timeout: Option<TimeoutEntry>,
    pub source_map: PublicationSourceMap,
    pub service_unit: ServiceUnit,
    pub file_ir_units: Vec<FileIrUnit>,
    pub resources: Vec<ProjectedPublicationResource>,
    pub package_units_typed: Vec<PackageUnit>,
}
