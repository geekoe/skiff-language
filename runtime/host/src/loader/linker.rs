//! Runtime-local test-support fixture helpers for linked programs.

#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;

#[cfg(any(test, feature = "test-support"))]
use crate::program::{
    ArtifactFileIrUnit as FileIrUnit, PackageUnit, RuntimeProgramLayers, ServiceUnit,
};
#[cfg(any(test, feature = "test-support"))]
use skiff_runtime_activation::build_runtime_activation_for_image;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use skiff_runtime_linker::package_handler_target;
#[cfg(any(test, feature = "test-support"))]
use skiff_runtime_linker::ProgramResult;

#[cfg(any(test, feature = "test-support"))]
#[allow(dead_code)]
pub fn link_runtime_program_layers(
    service: Arc<ServiceUnit>,
    service_files: Vec<Arc<FileIrUnit>>,
    packages: Vec<Arc<PackageUnit>>,
    package_files: Vec<Vec<Arc<FileIrUnit>>>,
) -> ProgramResult<RuntimeProgramLayers> {
    let build = skiff_runtime_linker::link_runtime_program_image_from_parts(
        service,
        service_files,
        packages,
        package_files,
    )?;
    let activation =
        build_runtime_activation_for_image(&build.image, build.activation_facts.clone())?;
    Ok(RuntimeProgramLayers::from_owned(
        build.identity,
        build.image,
        activation,
    ))
}
