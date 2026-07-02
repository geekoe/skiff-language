mod cache;
pub mod graph;
mod identity;
mod paths;
mod pointer_files;
mod pointer_parse;
mod types;
mod utils;

pub use cache::{
    ArtifactCacheBucketStats, ArtifactCacheEvictionCandidate, ArtifactCacheKind, FileIrCache,
    PackageCache, RemovedArtifactCacheEntry,
};
pub use graph::{
    select_runtime_program_pointer_from_roots, ArtifactGraph, ArtifactGraphCache,
    ArtifactGraphIdentities, ArtifactGraphLoader, RuntimeProgramArtifactSelection,
};
pub use paths::{resolve_index_artifact_path, service_id_artifact_path, ArtifactRootRelativePath};
pub use pointer_files::{
    load_dev_reload_pointers_from_roots, load_service_version_build_pointers_from_roots,
};
pub use types::{
    ArtifactIndexPointer, ArtifactPointerFile, RootedArtifactPointerFile, ServiceAssemblyPointer,
};

const PROTOCOL_IDENTITY_PREFIX: &str = "skiff-protocol-v1";
const SERVICE_VERSION_POINTER_SCHEMA_VERSION: &str = "skiff-service-version-pointer-v1";
const SERVICE_BUILD_SCHEMA_VERSION: &str = "skiff-service-build-v1";
const SERVICE_BUILD_IDENTITY_PREFIX: &str = "skiff-service-build-v1";
