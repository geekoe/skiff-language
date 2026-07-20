mod active_assembly_context;
pub(crate) mod assembly_admission;
mod fingerprint;
pub(crate) mod identity;
pub(crate) mod linker;
mod load;
mod options;
#[allow(dead_code)]
mod program_loader;
pub(crate) mod resolver;
mod runtime_config;
mod service_http;
#[cfg(test)]
mod test_artifacts;
mod utils;

#[allow(unused_imports)]
pub(crate) use fingerprint::artifact_roots_control_fingerprint;
#[allow(unused_imports)]
pub(crate) use load::{
    load_services_from_artifact_index, load_services_from_artifact_index_with_default,
    load_services_from_artifact_root, load_services_from_artifact_root_with_default,
    load_services_from_artifact_roots_with_default,
};
#[allow(unused_imports)]
pub(crate) use options::ArtifactLoadOptions;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use program_loader::load_runtime_program_layers;
#[allow(unused_imports)]
pub use program_loader::{load_runtime_program_parts, LoadOptions, LoadedRuntimeProgramParts};
pub(crate) use runtime_config::load_package_test_local_config;
pub(super) use skiff_artifact_model::schema::SERVICE_VERSION_POINTER_SCHEMA_VERSION;
pub(crate) use skiff_runtime_linker::link_runtime_program_image;
#[allow(unused_imports)]
pub(crate) use skiff_runtime_loader::{
    ArtifactGraph, ArtifactGraphCache, ArtifactGraphIdentities, ArtifactGraphLoader,
};
#[cfg(test)]
pub(crate) use test_artifacts::write_package_unit_value_ref;
#[cfg(test)]
pub(crate) use utils::value_sha256;
#[cfg(test)]
pub(crate) const SERVICE_BUILD_IDENTITY_PREFIX: &str = "skiff-service-build-v1";
