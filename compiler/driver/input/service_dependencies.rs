use std::path::PathBuf;

use crate::{
    input::{ResolvedServiceDependencies, ServiceDependency},
    shared::publication_error::PublicationError,
};

pub(crate) fn resolve_service_dependencies(
    dependencies: &[ServiceDependency],
    artifact_roots: &[PathBuf],
) -> Result<ResolvedServiceDependencies, PublicationError> {
    skiff_compiler_input::service_dependencies::resolve_service_dependencies(
        dependencies,
        artifact_roots,
    )
    .map_err(PublicationError::from)
}
