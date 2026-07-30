use std::{collections::BTreeMap, path::Path};

use skiff_artifact_identity::validate_package_artifact_identities;
use skiff_artifact_model::{
    ContractRequirement, FileIrUnit, NativeSignatureTypeExpr, NativeTarget, PackageArtifact,
    PackageRefIr, PackageRequirement, ServiceContract, STD_NATIVE_SIGNATURES,
};
use skiff_compiler_contract::{
    compile_service_contract_definition, project_service_api, ContractDefinitionError,
    ServiceApiProjection, ServiceContractDefinition,
};
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;
use skiff_compiler_emission::{
    file_ir_artifacts::publish_file_ir_artifacts,
    package_artifact::{publish_projected_package_artifact, PublishedPackageArtifact},
};
use skiff_compiler_input::ServicePackageRoot;
use skiff_compiler_projection::package_artifact::{
    project_compiled_package_artifact, PackageArtifactProjectionInput,
};
use skiff_compiler_projection_input::PublicationResourceProjectionInput;
use skiff_compiler_projection_input::ResolvedPackageSchema;
use skiff_deployment::storage::CanonicalArtifactStore;

use crate::{
    input::{
        PackageCompileInput, PackageContractCompileDependency, PackageDependency,
        PublicationResourceInput,
    },
    shared::package_compile_error::{
        package_projection_error, projection_input_error, PackageCompileError,
    },
    source_compile,
};

/// Compiles one independent package all the way to its canonical artifact.
///
/// The driver supplies only coordinates, typed dependency requirements and the
/// compiled ProjectionView. Export resolution, exact links, callable
/// coverage validation, runtime requirements, File IR projection and resource
/// projection remain owned by the PackageArtifact projection leaf. Exact
/// callable signatures arrive through the compiled ProjectionView.
pub fn compile_package(
    input: PackageCompileInput<'_>,
) -> Result<PublishedPackageArtifact, PackageCompileError> {
    skiff_compiler_source::prelude_registry::initialize_prelude_registry(input.platform_sources())
        .map_err(|error| PackageCompileError::ContractValidation {
            message: error.to_string(),
        })?;
    let package_id = input.package_id.to_string();
    let package_version = input.package.manifest.version.clone();
    let canonical_artifact_store = open_canonical_artifact_store(input.canonical_artifact_root())?;
    let declared_package_requirements = package_requirements(&input)?;
    let contract_requirements = contract_requirements(&input);
    let pre_source_package_schemas =
        pre_source_contract_package_schemas(&input, canonical_artifact_store.as_ref())?;
    let compiled = source_compile::compile(
        &input,
        &pre_source_package_schemas,
        canonical_artifact_store.as_ref(),
    )?;
    skiff_compiler_source::validate_source_execution_semantics(compiled.compile_model())?;
    let service_requirements = compiled
        .lowered()
        .service_calls()
        .service_requirements()
        .to_vec();
    let service_call_refs = compiled
        .lowered()
        .service_calls()
        .service_call_ref_closure()
        .into_iter()
        .collect::<Vec<_>>();
    let projection = skiff_compiler_compiled::projection_input::build_projection_input(&compiled)
        .map_err(projection_input_error)?
        .with_resources(resource_projection_inputs(&input.package.resources));
    let package_requirements = complete_package_requirement_closure(
        &package_id,
        declared_package_requirements,
        projection.view().file_ir_units(),
        input.available_packages,
    )?;
    let resolved_package_schemas = exact_resolved_package_schemas(
        &package_requirements,
        input.available_packages,
        &pre_source_package_schemas,
        canonical_artifact_store.as_ref(),
    )?;
    let projected = project_compiled_package_artifact(PackageArtifactProjectionInput {
        package_id: &package_id,
        package_version: &package_version,
        projection: projection.view(),
        package_requirements,
        resolved_package_schemas: &resolved_package_schemas,
        contract_requirements,
        service_requirements,
        service_call_refs,
    })
    .map_err(package_projection_error)?;
    let file_ir_units = publish_file_ir_artifacts(projection.view())?;
    Ok(publish_projected_package_artifact(
        &projected,
        &file_ir_units,
    )?)
}

/// Resolves the exact canonical schema owners needed while validating service
/// contracts, before source compilation starts.
///
/// Non-platform owners are deliberately limited to exact direct manifest
/// dependencies. `std` is the sole compiler-owned owner and is selected from
/// the exact canonical artifact already supplied by the authoring boundary;
/// this function never opens a latest-by-package-id pointer.
fn pre_source_contract_package_schemas(
    input: &PackageCompileInput<'_>,
    store: Option<&CanonicalArtifactStore>,
) -> Result<Vec<ResolvedPackageSchema>, PackageCompileError> {
    let mut schemas = Vec::<ResolvedPackageSchema>::new();
    let owners = input
        .contract_dependencies
        .iter()
        .flat_map(|dependency| &dependency.contract.package_type_requirements)
        .map(|requirement| requirement.package_id.as_str())
        .chain(
            input
                .package_dependencies
                .iter()
                .map(|dependency| dependency.id.as_str()),
        )
        .chain((input.package_id != SKIFF_STD_PUBLICATION_ID).then_some(SKIFF_STD_PUBLICATION_ID))
        .collect::<std::collections::BTreeSet<_>>();

    for owner in owners.iter().copied() {
        let binding = pre_source_schema_binding(
            owner,
            input.package_dependencies,
            input.contract_dependencies,
            input.dependency_packages,
            input.available_packages,
        )?;
        let Some((alias, artifact)) = binding else {
            // Preserve MissingPackageSchema at the contract boundary for an
            // undeclared owner or absent compiler-owned std.
            continue;
        };

        let matching = schemas
            .iter()
            .filter(|schema| schema.package_id() == owner)
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(package_schema_input_error(format!(
                "package schema owner {owner} has duplicate resolved schema bindings"
            )));
        }
        if let Some(schema) = matching.first() {
            validate_pre_source_schema(schema, &alias, artifact)?;
            continue;
        }

        let Some(store) = store else {
            continue;
        };
        validate_package_artifact_identities(artifact).map_err(|error| {
            package_schema_input_error(format!(
                "pre-source package schema {alias}={owner}@{} PackageArtifact identity validation failed: {error}",
                artifact.package_version
            ))
        })?;
        let resolved = store
            .resolve_package_artifact_schema(artifact)
            .map_err(|error| {
                package_schema_input_error(format!(
                    "pre-source package schema {alias}={owner}@{} resolution failed: {error}",
                    artifact.package_version
                ))
            })?;
        let records = resolved
            .records
            .iter()
            .map(|(type_id, record)| (type_id.clone(), record.as_ref().clone()))
            .collect();
        let schema = ResolvedPackageSchema::new(
            alias,
            artifact.package_id.clone(),
            artifact.package_version.clone(),
            artifact.package_build_id.clone(),
            artifact.package_local_abi.local_abi_identity.clone(),
            resolved.index.as_ref().clone(),
            records,
        )
        .map_err(|error| package_schema_input_error(error.to_string()))?;
        schemas.push(schema);
    }
    Ok(schemas)
}

fn open_canonical_artifact_store(
    root: Option<&Path>,
) -> Result<Option<CanonicalArtifactStore>, PackageCompileError> {
    root.map(|root| {
        CanonicalArtifactStore::open(root).map_err(|error| {
            package_schema_input_error(format!(
                "canonical artifact root {} could not be opened: {error}",
                root.display()
            ))
        })
    })
    .transpose()
}

fn pre_source_schema_binding<'a>(
    owner: &str,
    package_dependencies: &[PackageDependency],
    contract_dependencies: &[PackageContractCompileDependency],
    dependency_packages: &'a [PackageArtifact],
    available_packages: &'a [PackageArtifact],
) -> Result<Option<(String, &'a PackageArtifact)>, PackageCompileError> {
    if owner == SKIFF_STD_PUBLICATION_ID {
        let matches = available_packages
            .iter()
            .filter(|artifact| artifact.package_id == owner)
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(package_schema_input_error(format!(
                "compiler-owned package schema owner {owner} has duplicate exact canonical artifacts"
            )));
        }
        return Ok(matches
            .first()
            .map(|artifact| ("std".to_string(), *artifact)));
    }

    let declarations = package_dependencies
        .iter()
        .filter(|dependency| dependency.id == owner)
        .collect::<Vec<_>>();
    if declarations.len() > 1 {
        return Err(package_schema_input_error(format!(
            "package schema owner {owner} has duplicate direct dependency declarations"
        )));
    }
    let Some(dependency) = declarations.first() else {
        let contract_declarations = contract_dependencies
            .iter()
            .filter(|dependency| dependency.requirement.service_id == owner)
            .collect::<Vec<_>>();
        if contract_declarations.len() > 1 {
            return Err(package_schema_input_error(format!(
                "package schema owner {owner} has duplicate direct service dependency declarations"
            )));
        }
        let Some(dependency) = contract_declarations.first() else {
            return Ok(None);
        };
        let artifacts = available_packages
            .iter()
            .filter(|artifact| {
                artifact.package_id == dependency.requirement.service_id
                    && artifact.package_version == dependency.requirement.contract_version
            })
            .collect::<Vec<_>>();
        if artifacts.len() > 1 {
            return Err(package_schema_input_error(format!(
                "service package schema owner {owner}@{} has duplicate exact canonical artifacts",
                dependency.requirement.contract_version
            )));
        }
        return Ok(artifacts
            .first()
            .map(|artifact| (dependency.requirement.alias.clone(), *artifact)));
    };
    let artifacts = dependency_packages
        .iter()
        .filter(|artifact| {
            artifact.package_id == dependency.id && artifact.package_version == dependency.version
        })
        .collect::<Vec<_>>();
    if artifacts.len() > 1 {
        return Err(package_schema_input_error(format!(
            "package schema owner {owner}@{} has duplicate exact canonical artifacts",
            dependency.version
        )));
    }
    Ok(artifacts
        .first()
        .map(|artifact| (dependency.effective_alias().to_string(), *artifact)))
}

fn validate_pre_source_schema(
    schema: &ResolvedPackageSchema,
    alias: &str,
    artifact: &PackageArtifact,
) -> Result<(), PackageCompileError> {
    let requirement = PackageRequirement {
        alias: alias.to_string(),
        package_id: artifact.package_id.clone(),
        exact_version: artifact.package_version.clone(),
        expected_local_abi: artifact.package_local_abi.local_abi_identity.clone(),
        expected_package_build: None,
    };
    validate_package_artifact_identities(artifact).map_err(|error| {
        package_schema_input_error(format!(
            "pre-source package schema {alias}={}@{} PackageArtifact identity validation failed: {error}",
            artifact.package_id, artifact.package_version
        ))
    })?;
    schema
        .validate_exact_binding(&requirement, artifact)
        .map_err(|error| package_schema_input_error(error.to_string()))
}

fn exact_resolved_package_schemas(
    requirements: &[PackageRequirement],
    available_artifacts: &[PackageArtifact],
    available_schemas: &[ResolvedPackageSchema],
    store: Option<&CanonicalArtifactStore>,
) -> Result<Vec<ResolvedPackageSchema>, PackageCompileError> {
    requirements
        .iter()
        .map(|requirement| {
            let matches = available_schemas
                .iter()
                .filter(|schema| {
                    schema.alias() == requirement.alias
                        && schema.package_id() == requirement.package_id
                        && schema.exact_version() == requirement.exact_version
                })
                .collect::<Vec<_>>();
            if matches.len() > 1 {
                return Err(package_schema_input_error(format!(
                    "exact package requirement {}={}@{} has duplicate resolved schemas",
                    requirement.alias, requirement.package_id, requirement.exact_version
                )));
            }
            let artifact = available_artifacts
                .iter()
                .find(|artifact| {
                    artifact.package_id == requirement.package_id
                        && artifact.package_version == requirement.exact_version
                        && artifact.package_local_abi.local_abi_identity
                            == requirement.expected_local_abi
                })
                .ok_or_else(|| {
                    package_schema_input_error(format!(
                        "resolved schema {}={}@{} has no exact canonical PackageArtifact binding",
                        requirement.alias, requirement.package_id, requirement.exact_version
                    ))
                })?;
            validate_package_artifact_identities(artifact).map_err(|error| {
                package_schema_input_error(format!(
                    "resolved schema {}={}@{} PackageArtifact identity validation failed: {error}",
                    requirement.alias, requirement.package_id, requirement.exact_version
                ))
            })?;
            let resolved_from_store;
            let schema = if let Some(schema) = matches.first() {
                *schema
            } else {
                let store = store.ok_or_else(|| {
                    package_schema_input_error(format!(
                        "exact package requirement {}={}@{} has no resolved schema or canonical store resolver",
                        requirement.alias, requirement.package_id, requirement.exact_version
                    ))
                })?;
                let resolved = store
                    .resolve_package_artifact_schema(artifact)
                    .map_err(|error| {
                        package_schema_input_error(format!(
                            "exact package requirement {}={}@{} schema resolution failed: {error}",
                            requirement.alias,
                            requirement.package_id,
                            requirement.exact_version
                        ))
                    })?;
                let records = resolved
                    .records
                    .iter()
                    .map(|(type_id, record)| (type_id.clone(), record.as_ref().clone()))
                    .collect();
                resolved_from_store = ResolvedPackageSchema::new(
                    requirement.alias.clone(),
                    artifact.package_id.clone(),
                    artifact.package_version.clone(),
                    artifact.package_build_id.clone(),
                    artifact.package_local_abi.local_abi_identity.clone(),
                    resolved.index.as_ref().clone(),
                    records,
                )
                .map_err(|error| package_schema_input_error(error.to_string()))?;
                &resolved_from_store
            };
            schema
                .validate_exact_binding(requirement, artifact)
                .map_err(|error| package_schema_input_error(error.to_string()))?;
            Ok(schema.clone())
        })
        .collect()
}

fn package_schema_input_error(message: String) -> PackageCompileError {
    PackageCompileError::PackageSchemaInput { message }
}

#[derive(Debug)]
pub struct CompiledServicePackage {
    pub package: PublishedPackageArtifact,
    pub service_api: ServiceApiProjection,
}

#[derive(Debug, thiserror::Error)]
pub enum ServicePackageCompileError {
    #[error(transparent)]
    Package(#[from] PackageCompileError),
    #[error(transparent)]
    ServiceApi(#[from] ContractDefinitionError),
}

/// Compiles a service exactly once as a package, then deterministically
/// projects its service API from that canonical package result.
pub fn compile_service_package(
    input: PackageCompileInput<'_>,
    service_root: &ServicePackageRoot,
) -> Result<CompiledServicePackage, ServicePackageCompileError> {
    if service_root.package.publication != input.package.manifest {
        return Err(PackageCompileError::ContractValidation {
            message: "validated service package root does not match the package compilation input"
                .to_string(),
        }
        .into());
    }
    let package = compile_package(input)?;
    let service_api = project_service_api(
        &service_root.service.id,
        &service_root.service.service_calls,
        &package.artifact,
        &package.resolved_package_schema_type_records,
    )?;
    Ok(CompiledServicePackage {
        package,
        service_api,
    })
}

/// The code-free contract pipeline. No package/provider source is accepted.
pub fn compile_contract(
    definition: ServiceContractDefinition,
) -> Result<ServiceContract, ContractDefinitionError> {
    compile_service_contract_definition(definition)
}

fn package_requirements(
    input: &PackageCompileInput<'_>,
) -> Result<Vec<PackageRequirement>, PackageCompileError> {
    package_requirements_for_dependencies(
        input.package_id,
        input.package_dependencies,
        input.dependency_packages,
    )
}

fn package_requirements_for_dependencies(
    package_id: &str,
    dependencies: &[PackageDependency],
    dependency_packages: &[PackageArtifact],
) -> Result<Vec<PackageRequirement>, PackageCompileError> {
    dependencies
        .iter()
        .map(|dependency| package_requirement(package_id, dependency, dependency_packages))
        .collect()
}

fn complete_package_requirement_closure(
    owner_package_id: &str,
    mut requirements: Vec<PackageRequirement>,
    file_ir_units: &[FileIrUnit],
    available_artifacts: &[PackageArtifact],
) -> Result<Vec<PackageRequirement>, PackageCompileError> {
    if owner_package_id == SKIFF_STD_PUBLICATION_ID
        || !file_ir_units
            .iter()
            .any(file_ir_unit_references_platform_std)
    {
        return Ok(requirements);
    }

    if requirements.iter().any(|requirement| {
        requirement.alias == "std" || requirement.package_id == SKIFF_STD_PUBLICATION_ID
    }) {
        return Err(validation_error(format!(
            "package {owner_package_id} declares platform std as a package dependency; std requirements are compiler-owned"
        )));
    }

    let mut std_artifact = None;
    for artifact in available_artifacts {
        if artifact.package_id != SKIFF_STD_PUBLICATION_ID {
            continue;
        }
        if std_artifact.replace(artifact).is_some() {
            return Err(validation_error(format!(
                "canonical package graph contains duplicate artifact id {SKIFF_STD_PUBLICATION_ID}"
            )));
        }
    }
    let std_artifact = std_artifact.ok_or_else(|| {
        validation_error(format!(
            "package {owner_package_id} references platform std, but the same compile graph has no canonical PackageArtifact for {SKIFF_STD_PUBLICATION_ID}"
        ))
    })?;
    validate_package_artifact_identities(std_artifact).map_err(|error| {
        validation_error(format!(
            "canonical platform std artifact {}@{} identity validation failed: {error}",
            std_artifact.package_id, std_artifact.package_version
        ))
    })?;

    requirements.push(PackageRequirement {
        alias: "std".to_string(),
        package_id: std_artifact.package_id.clone(),
        exact_version: std_artifact.package_version.clone(),
        expected_local_abi: std_artifact.package_local_abi.local_abi_identity.clone(),
        expected_package_build: None,
    });
    Ok(requirements)
}

fn file_ir_unit_references_platform_std(file: &FileIrUnit) -> bool {
    file.external_refs
        .package_symbols
        .iter()
        .map(|symbol| &symbol.package)
        .chain(
            file.external_refs
                .package_callables
                .iter()
                .map(|callable| &callable.package_ref),
        )
        .any(package_ref_references_platform_std)
        || file
            .external_refs
            .native_targets
            .iter()
            .any(native_target_signature_references_platform_std)
}

fn package_ref_references_platform_std(package_ref: &PackageRefIr) -> bool {
    match package_ref {
        PackageRefIr::PackageId { package_id } => package_id == SKIFF_STD_PUBLICATION_ID,
        PackageRefIr::Dependency { dependency_ref } => dependency_ref == "std",
    }
}

fn native_target_signature_references_platform_std(target: &NativeTarget) -> bool {
    let target_path = if target.namespace.is_empty() {
        target.symbol.clone()
    } else {
        format!("{}.{}", target.namespace, target.symbol)
    };
    STD_NATIVE_SIGNATURES
        .iter()
        .filter(|signature| {
            target
                .binding_key
                .as_deref()
                .is_some_and(|binding_key| binding_key == signature.binding_key)
                || target_path == signature.target
                || signature.aliases.contains(&target_path.as_str())
        })
        .any(|signature| {
            signature
                .params
                .iter()
                .chain(std::iter::once(&signature.return_type))
                .any(native_signature_type_references_platform_std)
        })
}

fn native_signature_type_references_platform_std(ty: &NativeSignatureTypeExpr) -> bool {
    match ty {
        NativeSignatureTypeExpr::TypeParam(_) | NativeSignatureTypeExpr::Builtin(_) => false,
        NativeSignatureTypeExpr::Package { package_id, .. } => {
            *package_id == SKIFF_STD_PUBLICATION_ID
        }
        NativeSignatureTypeExpr::Array(inner)
        | NativeSignatureTypeExpr::Nullable(inner)
        | NativeSignatureTypeExpr::Stream(inner) => {
            native_signature_type_references_platform_std(inner)
        }
        NativeSignatureTypeExpr::Map(key, value) => {
            native_signature_type_references_platform_std(key)
                || native_signature_type_references_platform_std(value)
        }
    }
}

fn package_requirement(
    package_id: &str,
    dependency: &PackageDependency,
    dependency_packages: &[PackageArtifact],
) -> Result<PackageRequirement, PackageCompileError> {
    let artifact = dependency_packages
        .iter()
        .find(|artifact| {
            artifact.package_id == dependency.id && artifact.package_version == dependency.version
        })
        .ok_or_else(|| PackageCompileError::ContractValidation {
            message: format!(
                "package {package_id} dependency {}@{} has no canonical PackageArtifact",
                dependency.id, dependency.version
            ),
        })?;
    Ok(PackageRequirement {
        alias: dependency.effective_alias().to_string(),
        package_id: dependency.id.clone(),
        exact_version: dependency.version.clone(),
        expected_local_abi: artifact.package_local_abi.local_abi_identity.clone(),
        expected_package_build: dependency
            .top_level_alias
            .as_ref()
            .map(|_| artifact.package_build_id.clone()),
    })
}

fn contract_requirements(input: &PackageCompileInput<'_>) -> Vec<ContractRequirement> {
    input
        .contract_dependencies
        .iter()
        .map(|dependency| dependency.requirement.clone())
        .collect()
}

fn resource_projection_inputs(
    resources: &[PublicationResourceInput],
) -> Vec<PublicationResourceProjectionInput> {
    resources
        .iter()
        .map(|resource| {
            PublicationResourceProjectionInput::new(
                resource.path.clone(),
                resource.absolute_path.clone(),
                resource.byte_len,
                resource.sha256.clone(),
                resource.content_type.clone(),
            )
        })
        .collect()
}

fn validation_error(message: String) -> PackageCompileError {
    PackageCompileError::ContractValidation { message }
}

#[cfg(test)]
mod tests;
