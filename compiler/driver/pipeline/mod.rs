pub(crate) mod projection_context;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use skiff_compiler_compiled::{
    compile_parsed_publication_sources as compile_parsed_publication_sources_model,
    compile_source_model,
    projection_input::{
        build_package_projection_inputs as build_compiled_package_projection_inputs,
        build_projection_input as build_compiled_projection_input,
    },
    source_compile_package_facts_from_publications as compiled_package_facts_from_publications,
    CompiledPublication, PackageApiEntryInfo, PackageApiSourceInfo, PackageDependencyInfo,
    PackagePublication, PackagePublicationInfo, PackagePublicationProvenance,
};
use skiff_compiler_lowering::file_ir::FileIrUnit;
use skiff_compiler_projection::{
    project_package_ir_publications, project_package_publications, project_service,
};
use skiff_syntax::ast::SourceFile;
use skiff_syntax::error::{self as error, CompileError};
use skiff_syntax::parser::parse_source;

use crate::{
    emission::{
        emit_service, package_artifacts::build_package_artifacts,
        service_publication::BuiltServicePublication, ServiceEmissionContext,
    },
    input::{
        build_service_job,
        compile_input::{PackagePublicationInput, PublicationInput, ServicePublicationInput},
        discover_package_manifests_with_dependency_dirs, package_alias_bindings, PackageDependency,
        PackageManifest, PackageManifestKey, PackagePublicationJob, PackageResolutionDirs,
        Publication, PublicationCompilePolicy, PublicationManifest, ResolvedServiceDependencies,
        ServiceConfig, ServicePublicationJob, SourceTree,
    },
    shared::publication_error::PublicationError,
    source_compile::{
        self, provider_rules, CompileParsedPublicationSourcesInput, SourceCompilePackageFacts,
    },
};

use projection_context::{
    service_projection_context_from_job, service_projection_context_seed_from_service_job_seeds,
};
use skiff_compiler_projection_input::{PackageProjectionInput, PublicationResourceProjectionInput};

type PackageResourceProjectionTable =
    BTreeMap<(String, String), Vec<PublicationResourceProjectionInput>>;

pub struct ServicePublicationBuildInput<'a> {
    pub config: &'a ServiceConfig,
    pub source_tree: &'a SourceTree,
    pub service_id_override: Option<&'a str>,
    pub package_dirs: PackageResolutionDirs,
    pub service_dependency_artifact_roots: &'a [PathBuf],
}

impl<'a> ServicePublicationBuildInput<'a> {
    pub fn new(config: &'a ServiceConfig, source_tree: &'a SourceTree) -> Self {
        Self {
            config,
            source_tree,
            service_id_override: None,
            package_dirs: PackageResolutionDirs::default(),
            service_dependency_artifact_roots: &[],
        }
    }
}

pub fn build_service_publication(
    input: ServicePublicationBuildInput<'_>,
) -> Result<BuiltServicePublication, PublicationError> {
    let ServicePublicationBuildInput {
        config,
        source_tree,
        service_id_override,
        package_dirs,
        service_dependency_artifact_roots,
    } = input;
    let ServicePublicationJob {
        publication,
        package_aliases,
        service_id,
        service_dependencies,
        service_ingress,
        package_jobs,
        seeds,
    } = build_service_job(
        config,
        source_tree,
        service_id_override,
        service_dependency_artifact_roots,
        |root, dependencies| {
            discover_package_manifests_with_dependency_dirs(root, &package_dirs, dependencies)
        },
    )?;
    let service_projection_seed = service_projection_context_seed_from_service_job_seeds(&seeds);
    let package_resource_table = package_resource_projection_table(&package_jobs);
    let package_publications = compile_package_jobs(package_jobs, &seeds.package_manifests)?;

    let service_input = PublicationInput::Service(ServicePublicationInput::new_with_service_id(
        &publication,
        &package_aliases,
        service_id.as_str(),
        service_dependencies,
        service_ingress,
    ));
    let compiled_service =
        compile_publication_with_package_publications(service_input, &package_publications)?;

    build_service_artifacts_from_compilation(
        &publication,
        &compiled_service,
        &package_publications,
        &package_resource_table,
        service_projection_seed,
    )
}

fn build_service_artifacts_from_compilation(
    publication: &Publication,
    compiled_service: &CompiledPublication,
    package_publications: &[PackagePublication],
    package_resource_table: &PackageResourceProjectionTable,
    service_projection_seed: projection_context::ServiceProjectionContextSeed,
) -> Result<BuiltServicePublication, PublicationError> {
    let package_projection_inputs =
        build_package_projection_inputs(package_publications, package_resource_table);
    let prelude_projection = crate::shared::prelude_registry::projection_prelude_context();
    let package_projections =
        project_package_publications(&package_projection_inputs, &prelude_projection)?;
    let package_ir_projections = project_package_ir_publications(&package_projections)?;
    let package_artifacts = build_package_artifacts(&package_projections)?;
    let projection_context = service_projection_context_from_job(
        &service_projection_seed,
        &package_projection_inputs,
        &package_ir_projections,
        &publication.manifest.dependencies,
        publication.manifest.api.source.as_ref(),
        &prelude_projection,
    );
    let service_projection_input = build_compiled_projection_input(compiled_service)
        .with_resources(publication_resource_projection_inputs(publication));
    let service_projection = project_service(service_projection_input.view(), projection_context)?;
    let artifacts = emit_service(
        &service_projection,
        ServiceEmissionContext {
            package_artifacts: &package_artifacts,
            package_ir_projections: &package_ir_projections,
        },
    )?;

    Ok(BuiltServicePublication {
        manifest: service_projection.runtime_manifest_projection.manifest,
        artifacts,
    })
}

fn build_package_projection_inputs(
    package_publications: &[PackagePublication],
    resources: &PackageResourceProjectionTable,
) -> Vec<PackageProjectionInput> {
    build_compiled_package_projection_inputs(package_publications)
        .into_iter()
        .map(|input| {
            let key = (input.id().to_string(), input.version().to_string());
            input.with_resources(resources.get(&key).cloned().unwrap_or_default())
        })
        .collect()
}

fn package_resource_projection_table(
    package_jobs: &[PackagePublicationJob],
) -> PackageResourceProjectionTable {
    package_jobs
        .iter()
        .map(|job| {
            (
                (
                    job.publication.manifest.id.to_string(),
                    job.publication.manifest.version.clone(),
                ),
                publication_resource_projection_inputs(&job.publication),
            )
        })
        .collect()
}

fn publication_resource_projection_inputs(
    publication: &Publication,
) -> Vec<PublicationResourceProjectionInput> {
    publication
        .resources
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

pub(crate) fn compile_publication(
    input: PublicationInput<'_>,
) -> Result<CompiledPublication, PublicationError> {
    compile_publication_with_package_facts(input, None)
}

pub(crate) fn compile_publication_with_package_publications<'a>(
    input: PublicationInput<'a>,
    package_publications: &'a [PackagePublication],
) -> Result<CompiledPublication, PublicationError> {
    let package_facts = source_compile_package_facts_from_publications(package_publications);
    let package_facts = (!package_facts.is_empty()).then_some(package_facts);
    compile_publication_with_package_facts(input, package_facts.as_deref())
}

fn compile_publication_with_package_facts<'a, 'facts>(
    input: PublicationInput<'a>,
    package_facts: Option<&'facts [SourceCompilePackageFacts<'a>]>,
) -> Result<CompiledPublication, PublicationError> {
    let model = source_compile::build_with_package_facts(input, package_facts)?;
    Ok(compile_source_model(model)?)
}

pub(crate) fn source_compile_package_facts_from_publications<'a>(
    package_publications: &'a [PackagePublication],
) -> Vec<SourceCompilePackageFacts<'a>> {
    compiled_package_facts_from_publications(package_publications)
}

pub(crate) fn compile_package_jobs(
    package_jobs: Vec<PackagePublicationJob>,
    available: &BTreeMap<PackageManifestKey, PackageManifest>,
) -> Result<Vec<PackagePublication>, PublicationError> {
    let package_ids = package_jobs
        .iter()
        .map(|job| job.manifest().id.to_string())
        .collect::<BTreeSet<_>>();
    let mut publications_by_id = BTreeMap::<String, PackagePublication>::new();
    let mut package_publications = Vec::with_capacity(package_jobs.len());

    for job in package_jobs {
        let dependency_publications =
            package_publications_for_manifest(job.manifest(), &package_ids, &publications_by_id);
        let compiled = compile_package_publication(
            &job.publication,
            available,
            dependency_publications.as_deref(),
        )?;
        let info = package_publication_info_from_publication(&job.publication);
        let package_publication = PackagePublication::new(info, compiled, job.dependency_config);
        publications_by_id.insert(
            package_publication.id().to_string(),
            package_publication.clone(),
        );
        package_publications.push(package_publication);
    }

    Ok(package_publications)
}

fn compile_package_publication(
    publication: &Publication,
    available: &BTreeMap<PackageManifestKey, PackageManifest>,
    package_publications: Option<&[PackagePublication]>,
) -> Result<CompiledPublication, PublicationError> {
    let manifest = &publication.manifest;
    let package_aliases = package_alias_bindings(&manifest.dependencies, available);
    let input = PublicationInput::Package(PackagePublicationInput::new(
        publication,
        &package_aliases,
        manifest.id.as_str(),
    ));
    if let Some(package_publications) = package_publications {
        compile_publication_with_package_publications(input, package_publications)
    } else {
        compile_publication(input)
    }
}

fn package_publications_for_manifest(
    manifest: &PublicationManifest,
    package_ids: &BTreeSet<String>,
    publications_by_id: &BTreeMap<String, PackagePublication>,
) -> Option<Vec<PackagePublication>> {
    if manifest.dependencies.is_empty() {
        return None;
    }
    if !manifest
        .dependencies
        .iter()
        .all(|dependency| package_ids.contains(&dependency.id))
    {
        return None;
    }
    loaded_dependency_publications(&manifest.dependencies, publications_by_id)
}

fn loaded_dependency_publications(
    dependencies: &[PackageDependency],
    publications_by_id: &BTreeMap<String, PackagePublication>,
) -> Option<Vec<PackagePublication>> {
    let mut seen = BTreeSet::new();
    let mut stack = dependencies
        .iter()
        .map(|dependency| dependency.id.clone())
        .collect::<Vec<_>>();
    while let Some(package_id) = stack.pop() {
        if !seen.insert(package_id.clone()) {
            continue;
        }
        let publication = publications_by_id.get(&package_id)?;
        stack.extend(
            publication
                .dependencies()
                .iter()
                .map(|dependency| dependency.id().to_string()),
        );
    }
    Some(
        seen.into_iter()
            .filter_map(|package_id| publications_by_id.get(&package_id).cloned())
            .collect(),
    )
}

pub(crate) fn package_publication_info_from_publication(
    publication: &Publication,
) -> PackagePublicationInfo {
    PackagePublicationInfo::new(
        publication.manifest.id.to_string(),
        publication.manifest.version.clone(),
        publication
            .manifest
            .dependencies
            .iter()
            .map(package_dependency_info_from_input)
            .collect(),
        publication_api_entries_from_manifest(&publication.manifest),
        publication.manifest.api.source.as_ref().map(|source| {
            PackageApiSourceInfo::new(source.relative_path.clone(), source.content_hash.clone())
        }),
        publication.source_tree.root.clone(),
        PackagePublicationProvenance::new(publication.manifest.provenance.synthetic),
    )
}

fn publication_api_entries_from_manifest(
    manifest: &PublicationManifest,
) -> Vec<PackageApiEntryInfo> {
    manifest
        .api
        .entries()
        .map(|entry| {
            PackageApiEntryInfo::new(
                entry.public_path_string(),
                entry.source_module_hint().to_string(),
            )
        })
        .chain(manifest.api.public_instances().map(|entry| {
            PackageApiEntryInfo::new(
                entry.public_path_string(),
                entry.source_module_hint().to_string(),
            )
        }))
        .collect()
}

pub(crate) fn package_dependency_info_from_input(
    dependency: &PackageDependency,
) -> PackageDependencyInfo {
    PackageDependencyInfo::new(
        dependency.id.clone(),
        dependency.version.clone(),
        dependency.alias.clone(),
        dependency.config.clone(),
        dependency.collection_name_mapping.clone(),
    )
}

pub(crate) fn compile_parsed_publication_sources(
    input: CompileParsedPublicationSourcesInput<'_, '_>,
) -> Result<CompiledPublication, PublicationError> {
    Ok(compile_parsed_publication_sources_model(input)?)
}

pub(crate) fn compile_source_file_ir_unit(
    source: &str,
    source_path: impl Into<String>,
    module_path: impl Into<String>,
    role: impl Into<String>,
) -> error::Result<FileIrUnit> {
    let ast = parse_source(source)?;
    compile_parsed_source_file_ir_unit(ast, source, source_path, module_path, role)
}

pub(crate) fn compile_parsed_source_file_ir_unit(
    mut ast: SourceFile,
    source: &str,
    source_path: impl Into<String>,
    module_path: impl Into<String>,
    role: impl Into<String>,
) -> error::Result<FileIrUnit> {
    let source_path = source_path.into();
    let module_path = module_path.into();
    let role = role.into();
    let is_test_file = source_path.ends_with(".test.skiff");
    if !is_test_file {
        crate::input::test_rules::validate_no_test_declarations_in_production_source(
            &source_path,
            &ast,
        )?;
    }
    validate_single_file_source(&source_path, &module_path, &mut ast, &role)?;
    let package_aliases = BTreeMap::new();
    compile_parsed_source_file_ir_unit_with_additional_sources(
        ast,
        source,
        source_path,
        module_path,
        role,
        &package_aliases,
        Vec::new(),
    )
}

pub(crate) fn compile_parsed_source_file_ir_unit_with_additional_sources(
    ast: SourceFile,
    source: &str,
    source_path: impl Into<String>,
    module_path: impl Into<String>,
    role: impl Into<String>,
    package_aliases: &BTreeMap<String, Vec<String>>,
    additional_sources: Vec<source_compile::source_graph::CompilerSourceFile>,
) -> error::Result<FileIrUnit> {
    let source_path = source_path.into();
    let module_path = module_path.into();
    let role = role.into();
    let root = PathBuf::from("/");
    let main_source = source_compile::source_graph::CompilerSourceFile::from_parsed_ast(
        PathBuf::from(&source_path),
        module_path.clone(),
        role == "contract",
        source_path.ends_with(".test.skiff"),
        source.to_string(),
        ast,
    );
    let mut sources = Vec::with_capacity(1 + additional_sources.len());
    sources.push(main_source);
    sources.extend(additional_sources);
    let production_sources = sources
        .iter()
        .filter(|source| !source.is_test_file)
        .cloned()
        .collect::<Vec<_>>();
    let parsed_sources = source_compile::parsed_sources::parse_publication_sources(&root, &sources)
        .map_err(compile_error_from_source_compile_error)?;
    let package_dependencies = Vec::<crate::input::PackageDependency>::new();
    let compiled = compile_parsed_publication_sources(CompileParsedPublicationSourcesInput {
        parsed_sources,
        production_sources,
        diagnostic_root: &root,
        publication_api: None,
        package_aliases,
        package_dependencies: &package_dependencies,
        package_facts: None,
        service_dependencies: ResolvedServiceDependencies::default(),
        service_ingress: None,
        policy: single_file_publication_policy(&role),
    })
    .map_err(compile_error_from_publication_error)?;
    compiled
        .file_ir_units()
        .iter()
        .find(|unit| unit.module_path.as_str() == module_path.as_str())
        .cloned()
        .ok_or_else(|| {
            CompileError::Semantic(format!(
                "single-file compile did not emit File IR for module `{module_path}`"
            ))
        })
}

fn single_file_publication_policy(role: &str) -> PublicationCompilePolicy<'static> {
    if is_single_file_package_role(role) {
        PublicationCompilePolicy::Package {
            package_id: "skiff.run/single-file-helper",
        }
    } else {
        PublicationCompilePolicy::Service {
            service_id: "skiff.run/single-file-helper",
        }
    }
}

fn is_single_file_package_role(role: &str) -> bool {
    role == "package" || role.starts_with("package-")
}

fn compile_error_from_publication_error(error: PublicationError) -> CompileError {
    CompileError::Semantic(error.to_string())
}

fn compile_error_from_source_compile_error(
    error: source_compile::SourceCompileError,
) -> CompileError {
    CompileError::Semantic(error.to_string())
}

#[cfg(test)]
pub(crate) fn compile_parsed_source_file_ir_artifact(
    ast: SourceFile,
    source: &str,
    source_path: impl Into<String>,
    module_path: impl Into<String>,
    role: impl Into<String>,
) -> error::Result<crate::emission::artifact::PublishedFileIrArtifact> {
    let source_path = source_path.into();
    let module_path = module_path.into();
    let role = role.into();
    let unit = compile_parsed_source_file_ir_unit(
        ast,
        source,
        source_path.clone(),
        module_path.clone(),
        role.clone(),
    )?;
    Ok(
        crate::emission::file_ir_artifacts::published_file_ir_artifact_from_unit(
            &unit,
            source_path,
            module_path,
            role,
        ),
    )
}

fn validate_single_file_source(
    source_path: &str,
    module_path: &str,
    ast: &mut SourceFile,
    role: &str,
) -> error::Result<()> {
    let mut violations = Vec::new();
    if !is_single_file_package_role(role) {
        crate::source_compile::reserved_names::validate_reserved_names(
            source_path,
            ast,
            &mut violations,
        );
    }
    {
        let empty: BTreeSet<String> = BTreeSet::new();
        crate::source_compile::source_name_resolution::collect_unresolved_dotted_root_violations(
            source_path,
            ast,
            &single_source_module_roots(module_path),
            &empty,
            &empty,
            &mut violations,
        );
    }
    if is_single_file_package_role(role) {
        let implicit_roots =
            crate::source_compile::package_rules::implicit_std_module_roots(module_path);
        crate::source_compile::root_projection_validation::collect_std_root_projection_violations_with_implicit_roots(
            source_path,
            ast,
            &implicit_roots,
            &mut violations,
        );
    } else {
        crate::source_compile::root_projection_validation::collect_std_root_projection_violations(
            source_path,
            ast,
            &mut violations,
        );
    }
    provider_rules::collect_removed_connect_provider_violations(source_path, ast, &mut violations);
    crate::source_compile::alias_resolution::validate_and_expand_source_aliases(
        source_path,
        ast,
        &mut violations,
    );
    if role == "package" {
        if !source_path_allows_native_functions(source_path, role) {
            let inferred_package_id = package_id_from_source_path(source_path).unwrap_or("");
            provider_rules::collect_non_std_package_native_function_violations(
                inferred_package_id,
                source_path,
                ast,
                &mut violations,
            );
            provider_rules::collect_non_std_package_native_type_violations(
                inferred_package_id,
                source_path,
                ast,
                &mut violations,
            );
        }
    } else {
        provider_rules::collect_service_native_function_violations(
            source_path,
            ast,
            &mut violations,
        );
        provider_rules::collect_service_native_type_violations(source_path, ast, &mut violations);
    }
    if !violations.is_empty() {
        return Err(CompileError::Semantic(
            violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    Ok(())
}

fn source_path_allows_native_functions(source_path: &str, role: &str) -> bool {
    if role != "package" {
        return false;
    }
    Path::new(source_path)
        .components()
        .next()
        .is_some_and(|component| {
            component.as_os_str() == "std" || component.as_os_str() == "prelude"
        })
}

fn package_id_from_source_path(source_path: &str) -> Option<&str> {
    Path::new(source_path)
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str())
}

fn single_source_module_roots(module_path: &str) -> BTreeSet<String> {
    module_path
        .split('.')
        .next()
        .filter(|root| !root.is_empty())
        .map(|root| BTreeSet::from([root.to_string()]))
        .unwrap_or_default()
}
