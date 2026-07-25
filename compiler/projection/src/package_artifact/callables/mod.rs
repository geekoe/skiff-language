mod normalization;
mod signatures;
mod surface;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallableProjection, CallableSemanticFacts, ExecutableKind, FileIrRef, FileIrUnit,
    OperationCallableKind, OperationTargetRef, PackageCallableId, PackageCallableLinkFact,
    PackageCallableParameter, PackageCallableSignature, PackageImplementationLinks,
    PackageLocalAbiSymbol, PackageRuntimeRequirements, PackageTypeRef,
};
use skiff_compiler_projection_input::{
    ProjectionExecutableKey, ProjectionPackageCallableSignatureFacts, ResolvedPackageSchema,
};

use crate::{
    error::ProjectionError,
    package_artifact::{api_exports::PackageExports, export_links::ProjectedPackageExportLinks},
};

use super::boundary::project_boundary_callable_with_package_schemas;

pub(super) struct ProjectedPackageCallableSurface {
    pub public_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    pub implementation_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
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
    package_schema_refs: &BTreeMap<(String, String), skiff_artifact_model::ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<ProjectedPackageCallableSurface, ProjectionError> {
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
        let projection = project_boundary_callable_with_package_schemas(
            &callable.owner_module,
            &callable.signature,
            &facts,
            runtime_requirements,
            file_ir_units,
            package_schema_refs,
            resolved_package_schemas,
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
    let implementation_symbols = project_implementation_callables(
        package_id,
        file_ir_units,
        semantic_facts_by_executable,
        &mut callable_links,
        &mut semantic_facts,
    )?;
    Ok(ProjectedPackageCallableSurface {
        public_symbols: local_surface.public_symbols,
        implementation_symbols,
        implementation_links: local_surface.implementation_links,
        callable_links,
        semantic_facts,
        boundary_projections,
    })
}

fn project_implementation_callables(
    package_id: &str,
    units: &[FileIrUnit],
    facts_by_executable: &BTreeMap<ProjectionExecutableKey, CallableSemanticFacts>,
    callable_links: &mut BTreeMap<PackageCallableId, PackageCallableLinkFact>,
    semantic_facts: &mut BTreeMap<PackageCallableId, CallableSemanticFacts>,
) -> Result<BTreeMap<String, PackageLocalAbiSymbol>, ProjectionError> {
    let mut symbols = BTreeMap::new();
    for unit in units {
        for declaration in unit.declarations.executables.values() {
            let Some(executable) = unit.executables.get(declaration.executable_index as usize)
            else {
                return Err(projection_error(
                    package_id,
                    format!(
                        "implementation symbol {}.{} targets missing executable #{}",
                        unit.module_path, declaration.symbol, declaration.executable_index
                    ),
                ));
            };
            if executable.kind != ExecutableKind::Function {
                continue;
            }
            let top_level_name = declaration
                .symbol
                .strip_prefix(&format!("{}.", unit.module_path))
                .unwrap_or(&declaration.symbol);
            let source_path = format!("{}.{}", unit.module_path, top_level_name);
            let callable_id = PackageCallableId::new(format!(
                "pkg-callable:{package_id}:top-level:{source_path}"
            ));
            let signature = PackageCallableSignature {
                parameters: executable
                    .params
                    .iter()
                    .map(|parameter| PackageCallableParameter {
                        name: parameter.name.clone(),
                        ty: PackageTypeRef::Local {
                            local_type: parameter.ty.clone(),
                        },
                    })
                    .collect(),
                return_type: PackageTypeRef::Local {
                    local_type: executable.return_type.clone(),
                },
                throw_types: Vec::new(),
                may_suspend: executable.may_suspend,
            };
            if symbols
                .insert(
                    source_path.clone(),
                    PackageLocalAbiSymbol::Callable {
                        callable_id: callable_id.clone(),
                        signature,
                    },
                )
                .is_some()
            {
                return Err(projection_error(
                    package_id,
                    format!("duplicate implementation source path {source_path}"),
                ));
            }
            let target = OperationTargetRef {
                file_ref: FileIrRef {
                    file_ir_identity: unit.file_ir_identity.clone(),
                    module_path: unit.module_path.clone(),
                    artifact_path: None,
                    source_ast_hash: Some(unit.source_ast_hash.clone()),
                },
                executable_index: declaration.executable_index,
                callable_abi_id: callable_id.to_string(),
                callable_kind: OperationCallableKind::InternalFunction,
            };
            insert_callable_entry(
                callable_links,
                callable_id.clone(),
                PackageCallableLinkFact {
                    callable_id: callable_id.clone(),
                    target,
                },
                package_id,
                "implementation callable link",
            )?;
            let key = ProjectionExecutableKey::new(
                unit.module_path.clone(),
                declaration.executable_index,
            );
            let facts = facts_by_executable.get(&key).cloned().ok_or_else(|| {
                projection_error(
                    package_id,
                    format!("implementation callable {source_path} has no semantic facts"),
                )
            })?;
            insert_callable_entry(
                semantic_facts,
                callable_id,
                normalization::normalize_semantic_facts(facts),
                package_id,
                "implementation callable semantic facts",
            )?;
        }
    }
    Ok(symbols)
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
