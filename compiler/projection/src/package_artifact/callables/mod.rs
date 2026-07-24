mod normalization;
mod signatures;
mod surface;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallableProjection, CallableSemanticFacts, FileIrUnit, PackageCallableId,
    PackageCallableLinkFact, PackageImplementationLinks, PackageLocalAbiSymbol,
    PackageRuntimeRequirements,
};
use skiff_compiler_projection_input::{
    ProjectionExecutableKey, ProjectionPackageCallableSignatureFacts,
};

use crate::{
    error::ProjectionError,
    package_artifact::{api_exports::PackageExports, export_links::ProjectedPackageExportLinks},
};

use super::boundary::project_boundary_callable;

pub(super) struct ProjectedPackageCallableSurface {
    pub public_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    pub implementation_links: PackageImplementationLinks,
    pub callable_links: BTreeMap<PackageCallableId, PackageCallableLinkFact>,
    pub semantic_facts: BTreeMap<PackageCallableId, CallableSemanticFacts>,
    pub boundary_projections: BTreeMap<PackageCallableId, BoundaryCallableProjection>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn project_package_callable_surface(
    package_id: &str,
    api_exports: &PackageExports,
    exports: &ProjectedPackageExportLinks,
    file_ir_units: &[FileIrUnit],
    semantic_facts_by_executable: &BTreeMap<ProjectionExecutableKey, CallableSemanticFacts>,
    signatures: &ProjectionPackageCallableSignatureFacts,
    runtime_requirements: &PackageRuntimeRequirements,
) -> Result<ProjectedPackageCallableSurface, ProjectionError> {
    let public_type_ids = exports
        .exports
        .types
        .iter()
        .map(|(qualified_path, export)| {
            let public_path = qualified_path
                .strip_prefix(&format!("{package_id}/"))
                .unwrap_or(qualified_path);
            (
                (export.file.module_path.clone(), export.symbol.clone()),
                format!("type:{public_path}"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut local_surface =
        surface::project_local_surface(package_id, api_exports, exports, signatures)?;
    let mut callable_links = BTreeMap::new();
    let mut semantic_facts = BTreeMap::new();
    let mut boundary_projections = BTreeMap::new();
    for callable in local_surface.callables {
        surface::insert_public_symbol(
            &mut local_surface.public_symbols,
            callable.public_path.clone(),
            PackageLocalAbiSymbol::Callable {
                callable_id: callable.callable_id.clone(),
                signature: callable.signature.clone(),
            },
        )?;
        let executable_key =
            ProjectionExecutableKey::new(callable.owner_module.clone(), callable.executable_index);
        let facts = semantic_facts_by_executable
            .get(&executable_key)
            .cloned()
            .ok_or_else(|| {
                projection_error(
                    package_id,
                    format!(
                        "public callable {} target {}#{} has no typed semantic facts",
                        callable.public_path, callable.owner_module, callable.executable_index
                    ),
                )
            })?;
        let facts = normalization::normalize_semantic_facts(facts);
        let projection = project_boundary_callable(
            &callable.owner_module,
            &callable.signature,
            &facts,
            runtime_requirements,
            file_ir_units,
            &public_type_ids,
        )?;
        insert_callable_entry(
            &mut callable_links,
            callable.callable_id.clone(),
            PackageCallableLinkFact {
                callable_id: callable.callable_id.clone(),
                target: callable.target,
            },
            package_id,
            "callable link",
        )?;
        insert_callable_entry(
            &mut semantic_facts,
            callable.callable_id.clone(),
            facts,
            package_id,
            "callable semantic facts",
        )?;
        insert_callable_entry(
            &mut boundary_projections,
            callable.callable_id,
            projection,
            package_id,
            "boundary projection",
        )?;
    }
    Ok(ProjectedPackageCallableSurface {
        public_symbols: local_surface.public_symbols,
        implementation_links: local_surface.implementation_links,
        callable_links,
        semantic_facts,
        boundary_projections,
    })
}

fn insert_callable_entry<T>(
    map: &mut BTreeMap<PackageCallableId, T>,
    callable_id: PackageCallableId,
    value: T,
    package_id: &str,
    label: &str,
) -> Result<(), ProjectionError> {
    if map.insert(callable_id.clone(), value).is_some() {
        return Err(projection_error(
            package_id,
            format!("duplicate {label} id {callable_id}"),
        ));
    }
    Ok(())
}

pub(super) fn projection_error(package_id: &str, message: impl Into<String>) -> ProjectionError {
    ProjectionError::InvalidPackageArtifact {
        message: format!(
            "package {package_id} artifact projection: {}",
            message.into()
        ),
    }
}
