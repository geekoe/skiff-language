use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use skiff_artifact_model::PackageUnit;
#[cfg(test)]
use skiff_compiler::test_support::compile_package_ast_config_and_effect_metadata_with_compiled_dependency_publications_for_test;
use skiff_compiler::{
    read_service_config_with_profile,
    test_support::{
        compile_ast_file_ir_for_test,
        compile_package_ast_file_ir_artifacts_with_compiled_dependency_publications_unit_and_metadata_for_test,
        compile_package_dependency_publications_for_test,
        compile_parsed_only_package_ast_file_ir_artifacts_for_test,
        compile_parsed_only_package_ast_file_ir_artifacts_with_compiled_dependency_publications_entrypoint_metadata_for_test,
        discover_package_manifests_with_dependency_dirs, is_complex_package_id,
        package_alias_bindings, package_public_path,
        package_unit_artifacts_from_dependency_publications_for_test, resolve_package_imports,
        FileIrCompileOptions, TestCompilerSourceFile, TestPackageDependencyPublications,
        TestPackageManifest as PackageManifest, TestPackageTestDependencyPackageInput,
        TestResolvedPackage,
    },
    PackageDependency, PackageManifestKey, PackageResolutionDirs, PublishedFileIrArtifact,
};
use skiff_syntax::ast::SourceFile as AstSourceFile;

use super::{
    root_paths::{export_source_paths, resolve_official_package_private_root_paths},
    sources::read_package_production_sources,
    visibility::{
        merge_function_return_types, merge_production_exports, production_function_exports,
        production_function_return_types,
    },
    PackageDependencyArtifacts, PackageTestSource, ParsedSource, RuntimeTestArtifact,
    SkiffTestError, SkiffTestOptions,
};

pub(super) struct PackageSourceCompiledArtifacts {
    pub(super) artifacts: Vec<PublishedFileIrArtifact>,
    pub(super) config_and_effect_metadata: skiff_artifact_model::ConfigAndEffectMetadata,
    pub(super) package_unit: PackageUnit,
}

pub(super) struct PackageTestCompiledArtifact {
    pub(super) artifact: RuntimeTestArtifact,
    pub(super) config_and_effect_metadata: skiff_artifact_model::ConfigAndEffectMetadata,
    pub(super) entrypoint_config_and_effect_metadata:
        Vec<skiff_artifact_model::ConfigAndEffectMetadata>,
}

pub(super) fn package_dependency_artifacts(
    current_manifest: &PackageManifest,
    package_root: &Path,
    production_sources: &[PackageTestSource],
    test_sources: &[PackageTestSource],
    options: &SkiffTestOptions,
) -> Result<PackageDependencyArtifacts, SkiffTestError> {
    let package_dirs = options.package_resolution_dirs_for(package_root);
    let mut available = discover_package_manifests_with_dependency_dirs(
        package_root,
        &package_dirs,
        &current_manifest.dependencies,
    )?;
    available
        .entry((
            current_manifest.id.clone(),
            current_manifest.version.clone(),
        ))
        .or_insert_with(|| current_manifest.clone());
    let imports = package_import_paths(
        &current_manifest.dependencies,
        production_sources,
        test_sources,
    );
    let resolved = resolve_package_imports(&imports, &current_manifest.dependencies, &available)?;
    let dependency_publications =
        compile_package_dependency_publications_for_test(current_manifest, &resolved, &available)?;
    let mut package_units_by_id =
        package_unit_artifacts_from_dependency_publications_for_test(&dependency_publications)?
            .into_iter()
            .map(|artifact| (artifact.package_id.clone(), artifact))
            .collect::<BTreeMap<_, _>>();
    let mut production_exports = BTreeMap::new();
    let mut function_return_types = BTreeMap::new();
    let mut package_ids = BTreeSet::new();
    let mut package_test_dependency_packages = Vec::new();
    for package in resolved
        .into_iter()
        .filter(|package| package.manifest.id != current_manifest.id)
    {
        package_ids.insert(package.manifest.id.clone());
        let package_root =
            package
                .manifest
                .path
                .parent()
                .ok_or_else(|| SkiffTestError::RuntimeSetup {
                    message: format!(
                        "package {} manifest path has no parent",
                        package.manifest.id
                    ),
                })?;
        let sources = read_dependency_package_production_sources(&package.manifest, package_root)?;
        merge_production_exports(
            &mut production_exports,
            production_function_exports(&package.manifest, &sources, true),
        );
        merge_function_return_types(
            &mut function_return_types,
            production_function_return_types(&sources),
        );
        let Some(unit_artifact) = package_units_by_id.remove(&package.manifest.id) else {
            if package.manifest.synthetic {
                continue;
            }
            return Err(SkiffTestError::RuntimeSetup {
                message: format!(
                    "package {} resolved for tests but no graph PackageUnit artifact was produced",
                    package.manifest.id
                ),
            });
        };
        package_test_dependency_packages.push(TestPackageTestDependencyPackageInput {
            package_id: unit_artifact.package_id.clone(),
            package_version: unit_artifact.package_version.clone(),
            package_dependencies: unit_artifact.package_dependencies.clone(),
            production_files: unit_artifact.production_files.clone(),
            package_unit: Some(unit_artifact.unit.clone()),
        });
    }
    let package_aliases = package_alias_bindings(&current_manifest.dependencies, &available);
    Ok(PackageDependencyArtifacts {
        package_test_dependency_packages,
        dependency_publications,
        production_exports,
        function_return_types,
        package_ids,
        package_aliases,
    })
}
pub(super) fn service_dependency_artifacts(
    service_root: &Path,
    profile: Option<&str>,
    production_sources: &[ParsedSource],
    options: &SkiffTestOptions,
) -> Result<PackageDependencyArtifacts, SkiffTestError> {
    let config = read_service_config_with_profile(service_root, profile)?;
    let package_dirs = options.package_resolution_dirs_for(service_root);
    let available = discover_package_manifests_with_dependency_dirs(
        service_root,
        &package_dirs,
        &config.publication.dependencies,
    )?;
    let imports = production_sources
        .iter()
        .flat_map(|source| source.ast.imports.iter().map(|import| import.path.clone()))
        .collect::<Vec<_>>();
    let resolved = resolve_package_imports(&imports, &config.publication.dependencies, &available)?;
    dependency_artifacts_from_resolved(resolved, &available, &config.publication.dependencies)
}
pub(super) fn dependency_artifacts_from_resolved(
    resolved: Vec<TestResolvedPackage>,
    available: &BTreeMap<PackageManifestKey, PackageManifest>,
    dependencies: &[PackageDependency],
) -> Result<PackageDependencyArtifacts, SkiffTestError> {
    let mut production_exports = BTreeMap::new();
    let mut function_return_types = BTreeMap::new();
    let mut package_ids = BTreeSet::new();
    for package in resolved {
        package_ids.insert(package.manifest.id.clone());
        if package.manifest.synthetic {
            continue;
        }
        let package_root =
            package
                .manifest
                .path
                .parent()
                .ok_or_else(|| SkiffTestError::RuntimeSetup {
                    message: format!(
                        "package {} manifest path has no parent",
                        package.manifest.id
                    ),
                })?;
        let sources = read_dependency_package_production_sources(&package.manifest, package_root)?;
        merge_production_exports(
            &mut production_exports,
            production_function_exports(&package.manifest, &sources, true),
        );
        merge_function_return_types(
            &mut function_return_types,
            production_function_return_types(&sources),
        );
    }
    Ok(PackageDependencyArtifacts {
        package_test_dependency_packages: Vec::new(),
        dependency_publications: TestPackageDependencyPublications::default(),
        production_exports,
        function_return_types,
        package_ids,
        package_aliases: package_alias_bindings(dependencies, available),
    })
}

fn package_import_paths(
    _dependencies: &[PackageDependency],
    production_sources: &[PackageTestSource],
    test_sources: &[PackageTestSource],
) -> Vec<Vec<String>> {
    let mut imports = production_sources
        .iter()
        .chain(test_sources.iter())
        .flat_map(|source| source.ast.imports.iter().map(|import| import.path.clone()))
        .collect::<Vec<_>>();
    imports.sort();
    imports.dedup();
    imports
}

fn read_dependency_package_production_sources(
    manifest: &PackageManifest,
    package_root: &Path,
) -> Result<Vec<PackageTestSource>, SkiffTestError> {
    let export_sources = export_source_paths(manifest, package_root)?;
    let sources = read_package_production_sources(manifest, package_root, &export_sources)?;
    resolve_official_package_private_root_paths(manifest, sources)
}

pub(super) fn package_source_artifacts(
    manifest: &PackageManifest,
    package_root: &Path,
    sources: &[PackageTestSource],
    package_aliases: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<RuntimeTestArtifact>, SkiffTestError> {
    package_source_file_ir_artifacts(manifest, package_root, sources, package_aliases)?
        .into_iter()
        .map(|artifact| runtime_test_artifact(artifact, Some(manifest.id.as_str())))
        .collect()
}

pub(super) fn package_source_file_ir_artifacts(
    manifest: &PackageManifest,
    package_root: &Path,
    sources: &[PackageTestSource],
    package_aliases: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<PublishedFileIrArtifact>, SkiffTestError> {
    let compiler_sources = sources
        .iter()
        .map(|source| {
            (
                TestCompilerSourceFile {
                    relative_path: source.relative_path.clone(),
                    module_path: source.module_path.clone(),
                    is_api: false,
                    is_test_file: false,
                    text: source.text.clone(),
                },
                source.ast.clone(),
            )
        })
        .collect::<Vec<_>>();
    let artifacts = compile_parsed_only_package_ast_file_ir_artifacts_for_test(
        &manifest.id,
        &manifest.dependencies,
        package_root,
        &compiler_sources,
        package_aliases,
    )?
    .into_iter()
    .map(|mut artifact| {
        artifact.role = "package".to_string();
        artifact
    })
    .collect::<Vec<_>>();
    Ok(artifacts)
}

pub(super) fn package_source_file_ir_artifacts_with_dependency_publications(
    manifest: &PackageManifest,
    package_root: &Path,
    sources: &[PackageTestSource],
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_publications: &TestPackageDependencyPublications,
) -> Result<PackageSourceCompiledArtifacts, SkiffTestError> {
    let compiler_sources = sources
        .iter()
        .map(|source| {
            (
                TestCompilerSourceFile {
                    relative_path: source.relative_path.clone(),
                    module_path: source.module_path.clone(),
                    is_api: false,
                    is_test_file: source.is_test_file,
                    text: source.text.clone(),
                },
                source.ast.clone(),
            )
        })
        .collect::<Vec<_>>();
    let compiled =
        compile_package_ast_file_ir_artifacts_with_compiled_dependency_publications_unit_and_metadata_for_test(
            manifest,
            package_root,
            &compiler_sources,
            package_aliases,
            dependency_publications,
        )?;
    let package_unit =
        compiled
            .package_unit
            .clone()
            .ok_or_else(|| SkiffTestError::RuntimeSetup {
                message: format!("package {} did not produce a PackageUnit", manifest.id),
            })?;
    let artifacts = compiled
        .file_ir_artifacts
        .into_iter()
        .map(|mut artifact| {
            artifact.role = "package".to_string();
            artifact
        })
        .collect::<Vec<_>>();
    Ok(PackageSourceCompiledArtifacts {
        artifacts,
        config_and_effect_metadata: compiled.config_and_effect_metadata,
        package_unit,
    })
}

pub(super) fn runtime_test_artifact_for_test_function(
    source_text: &str,
    source_path: impl Into<String>,
    module_path: impl Into<String>,
    role: impl Into<String>,
    package_id: Option<&str>,
    _test_entry_function_name: &str,
    ast: AstSourceFile,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &BTreeMap<String, String>,
) -> Result<RuntimeTestArtifact, SkiffTestError> {
    let source_path = source_path.into();
    let module_path = module_path.into();
    let role = role.into();
    let file_ir = compile_ast_file_ir_for_test(
        source_text,
        source_path,
        module_path.clone(),
        role,
        ast,
        FileIrCompileOptions {
            package_aliases,
            external_type_symbols,
            ..FileIrCompileOptions::default()
        },
    )
    .map_err(|source_error| SkiffTestError::Compile {
        name: module_path,
        source: source_error,
    })?;
    runtime_test_artifact(file_ir, package_id)
}

pub(super) fn package_test_file_ir_artifact_for_test_functions(
    manifest: &PackageManifest,
    package_root: &Path,
    production_sources: &[PackageTestSource],
    test_source: &PackageTestSource,
    test_ast: AstSourceFile,
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_publications: &TestPackageDependencyPublications,
    entrypoint_function_names: &[String],
) -> Result<PackageTestCompiledArtifact, SkiffTestError> {
    let mut compiler_sources = production_sources
        .iter()
        .map(|source| {
            (
                TestCompilerSourceFile {
                    relative_path: source.relative_path.clone(),
                    module_path: source.module_path.clone(),
                    is_api: false,
                    is_test_file: false,
                    text: source.text.clone(),
                },
                source.ast.clone(),
            )
        })
        .collect::<Vec<_>>();
    compiler_sources.push((
        TestCompilerSourceFile {
            relative_path: test_source.relative_path.clone(),
            module_path: test_source.module_path.clone(),
            is_api: false,
            is_test_file: true,
            text: test_source.text.clone(),
        },
        test_ast,
    ));
    let test_source_path = test_source.relative_path.to_string_lossy().to_string();
    let test_module_path = test_source.module_path.clone();
    let compiled =
        compile_parsed_only_package_ast_file_ir_artifacts_with_compiled_dependency_publications_entrypoint_metadata_for_test(
            manifest,
            package_root,
            &compiler_sources,
            package_aliases,
            dependency_publications,
            entrypoint_function_names,
        )?;
    let mut artifact = compiled
        .compiled
        .file_ir_artifacts
        .into_iter()
        .find(|artifact| {
            artifact.source_path == test_source_path && artifact.module_path == test_module_path
        })
        .ok_or_else(|| SkiffTestError::RuntimeSetup {
            message: format!(
                "package test file {} ({}) was not emitted",
                test_source_path, test_module_path
            ),
        })?;
    artifact.role = "package-test".to_string();
    Ok(PackageTestCompiledArtifact {
        artifact: runtime_test_artifact(artifact, Some(manifest.id.as_str()))?,
        config_and_effect_metadata: compiled.compiled.config_and_effect_metadata,
        entrypoint_config_and_effect_metadata: compiled.entrypoint_config_and_effect_metadata,
    })
}

#[cfg(test)]
pub(super) fn package_test_config_and_effect_metadata_for_test_function(
    manifest: &PackageManifest,
    package_root: &Path,
    production_sources: &[PackageTestSource],
    test_source: &PackageTestSource,
    test_ast: AstSourceFile,
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_publications: &TestPackageDependencyPublications,
) -> Result<skiff_artifact_model::ConfigAndEffectMetadata, SkiffTestError> {
    let mut compiler_sources = production_sources
        .iter()
        .map(|source| {
            (
                TestCompilerSourceFile {
                    relative_path: source.relative_path.clone(),
                    module_path: source.module_path.clone(),
                    is_api: false,
                    is_test_file: false,
                    text: source.text.clone(),
                },
                source.ast.clone(),
            )
        })
        .collect::<Vec<_>>();
    compiler_sources.push((
        TestCompilerSourceFile {
            relative_path: test_source.relative_path.clone(),
            module_path: test_source.module_path.clone(),
            is_api: false,
            is_test_file: true,
            text: test_source.text.clone(),
        },
        test_ast,
    ));
    compile_package_ast_config_and_effect_metadata_with_compiled_dependency_publications_for_test(
        manifest,
        package_root,
        &compiler_sources,
        package_aliases,
        dependency_publications,
    )
    .map_err(Into::into)
}

fn runtime_test_artifact(
    artifact: PublishedFileIrArtifact,
    package_id: Option<&str>,
) -> Result<RuntimeTestArtifact, SkiffTestError> {
    Ok(RuntimeTestArtifact {
        source_path: artifact.source_path,
        module_path: artifact.module_path,
        role: artifact.role,
        package_id: package_id.map(str::to_string),
        file_ir: artifact.unit,
    })
}

pub(super) fn package_test_source_type_symbols(
    sources: &[PackageTestSource],
) -> BTreeMap<String, String> {
    let mut symbols = BTreeMap::new();
    for source in sources {
        insert_ast_type_symbols(&mut symbols, &source.module_path, &source.ast);
    }
    symbols
}

fn insert_ast_type_symbols(
    symbols: &mut BTreeMap<String, String>,
    module_path: &str,
    ast: &AstSourceFile,
) {
    for name in ast
        .types
        .iter()
        .map(|ty| ty.name.as_str())
        .chain(ast.aliases.iter().map(|alias| alias.name.as_str()))
        .chain(ast.dbs.iter().map(|db| db.name.as_str()))
        .chain(
            ast.interfaces
                .iter()
                .map(|interface| interface.name.as_str()),
        )
    {
        symbols
            .entry(name.to_string())
            .or_insert_with(|| format!("{module_path}.{name}"));
    }
}

pub(super) fn self_package_alias_bindings(
    manifest: &PackageManifest,
) -> BTreeMap<String, Vec<String>> {
    let mut roots = manifest
        .api
        .iter()
        .filter_map(|entry| package_export_root(manifest, &entry.path))
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    if is_complex_package_id(&manifest.id) {
        roots
            .iter()
            .map(|root| (root.clone(), vec![root.clone()]))
            .collect()
    } else {
        BTreeMap::from([(manifest.id.clone(), roots)])
    }
}
pub(super) fn package_export_root(manifest: &PackageManifest, path: &str) -> Option<String> {
    let path = path.trim();
    if is_complex_package_id(&manifest.id) {
        if path.is_empty() {
            return None;
        }
    }

    let public_path = if is_complex_package_id(&manifest.id) {
        path.to_string()
    } else {
        package_public_path(&manifest.id, path)
    };
    let mut parts = public_path.split('.');
    let first = parts.next()?;
    Some(match parts.next() {
        Some(second) => format!("{first}.{second}"),
        None => first.to_string(),
    })
}
pub(super) fn package_test_alias_bindings(
    manifest: &PackageManifest,
    package_root: &Path,
    dependency_aliases: &BTreeMap<String, Vec<String>>,
    package_dirs: &PackageResolutionDirs,
) -> Result<BTreeMap<String, Vec<String>>, SkiffTestError> {
    let mut aliases = dependency_aliases.clone();
    let available = discover_package_manifests_with_dependency_dirs(
        package_root,
        package_dirs,
        &manifest.dependencies,
    )?;
    for dependency in manifest
        .dependencies
        .iter()
        .filter(|dependency| dependency.alias.is_none())
    {
        if let Some(dependency_manifest) =
            available.get(&(dependency.id.clone(), dependency.version.clone()))
        {
            aliases.extend(self_package_alias_bindings(dependency_manifest));
        }
    }
    Ok(aliases)
}
