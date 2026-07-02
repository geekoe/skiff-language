use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env, fs,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};

use serde_json::Value as JsonValue;
use skiff_artifact_identity::package_test_entrypoint_local_id;
use skiff_artifact_model::{ConfigAndEffectMetadata, FileIrUnit, PackageUnit};
use skiff_compiler::test_support::{
    discover_package_manifests, package_test_artifacts::TestPackageTestEntrypointSummary,
    read_user_package_manifest, TestPackageManifest as PackageManifest,
    TestPackageTestArtifactInput, TestPackageTestEntrypointInput, TestPackageTestFileIrArtifact,
    PACKAGE_CONFIG_FILE,
};
use skiff_compiler::PublishedFileIrArtifact;
use skiff_syntax::ast::SourceFile as AstSourceFile;

use super::{
    artifacts::{
        package_dependency_artifacts,
        package_source_file_ir_artifacts_with_dependency_publications, package_test_alias_bindings,
        package_test_file_ir_artifact_for_test_functions,
    },
    doubles::{
        config_for_runtime_test_modules, doubles_for_runtime_test_modules,
        live_missing_config_skip_message, read_runtime_test_inputs,
        service_db_mongo_url_for_runtime_test,
    },
    root_paths::{
        export_source_paths, resolve_official_package_private_root_paths,
        resolve_package_test_root_paths,
    },
    runtime_process::{
        drop_test_service_databases, execute_dev_synced_package_test_entrypoint_with_context,
        prepare_dev_synced_package_test, PreparedPackageTestDispatchContext, RuntimeExpectedError,
    },
    service::{runtime_live_expected_error, runtime_live_request_payload},
    sources::{
        collect_package_test_cases, package_test_ast, package_test_ast_for_cases,
        read_package_production_sources, read_package_test_sources,
    },
    types::TestEffectDouble,
    visibility::{merge_production_exports, production_function_exports},
    PackageDependencyArtifacts, PackageTestCase, PackageTestSource, PrivateVisibilityScope,
    SkiffTestError, SkiffTestOptions, SkiffTestResult, SkiffTestSummary,
};

const PACKAGE_TEST_CONCURRENCY_ENV: &str = "SKIFF_PACKAGE_TEST_CONCURRENCY";

pub(super) fn run_package_tests(
    input: &Path,
    package_root: &Path,
    input_is_file: bool,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, SkiffTestError> {
    if options.live {
        debug_assert!(input_is_file);
        debug_assert!(options.allow_network);
        debug_assert!(options.config_path.is_some());
    }
    let dispatch_concurrency_override = package_test_dispatch_concurrency_override(options)
        .map_err(|message| SkiffTestError::RuntimeSetup { message })?;
    let current_manifest = current_package_manifest(package_root)?;
    let export_sources = export_source_paths(&current_manifest, package_root)?;
    let test_sources = read_package_test_sources(
        input,
        package_root,
        input_is_file,
        &current_manifest,
        &export_sources,
    )?;
    let test_sources = if input_is_file {
        test_sources
    } else {
        test_sources
            .into_iter()
            .filter(|source| source.ast.test_default_run.unwrap_or(true))
            .collect()
    };
    let test_doubles = read_runtime_test_inputs(input, input_is_file, options)?;
    let production_sources =
        read_package_production_sources(&current_manifest, package_root, &export_sources)?;
    let production_sources =
        resolve_official_package_private_root_paths(&current_manifest, production_sources)?;
    let current_package_private_modules = production_sources
        .iter()
        .map(|source| source.module_path.clone())
        .collect::<BTreeSet<_>>();
    let mut production_exports =
        production_function_exports(&current_manifest, &production_sources, false);
    let dependency_artifacts = package_dependency_artifacts(
        &current_manifest,
        package_root,
        &production_sources,
        &test_sources,
        options,
    )?;
    merge_production_exports(
        &mut production_exports,
        dependency_artifacts.production_exports.clone(),
    );
    let mut package_ids = dependency_artifacts.package_ids.clone();
    package_ids.insert(current_manifest.id.clone());
    let test_package_aliases = package_test_alias_bindings(
        &current_manifest,
        package_root,
        &dependency_artifacts.package_aliases,
        &options.package_resolution_dirs_for(package_root),
    )?;
    let test_sources = resolve_package_test_root_paths(test_sources, &production_sources)?;
    let disallowed_import_modules = production_sources
        .iter()
        .map(|source| source.module_path.clone())
        .collect::<BTreeSet<_>>();
    let production_compiled = package_source_file_ir_artifacts_with_dependency_publications(
        &current_manifest,
        package_root,
        &production_sources,
        &test_package_aliases,
        &dependency_artifacts.dependency_publications,
    )?;
    let production_config_and_effect_metadata =
        production_compiled.config_and_effect_metadata.clone();
    let production_package_unit = production_compiled.package_unit.clone();
    let production_artifacts = production_compiled.artifacts;
    let package_dependencies = production_package_unit.dependencies.clone();
    let tests = collect_package_test_cases(&test_sources);
    let test_count = tests.len();
    let compile_context = PackageTestBatchCompileContext {
        current_manifest: &current_manifest,
        package_root,
        production_sources: &production_sources,
        test_package_aliases: &test_package_aliases,
        dependency_artifacts: &dependency_artifacts,
        package_dependencies: &package_dependencies,
        production_package_unit: &production_package_unit,
        production_config_and_effect_metadata: &production_config_and_effect_metadata,
        production_artifacts: &production_artifacts,
    };
    let mut results = (0..test_count).map(|_| None).collect::<Vec<_>>();
    let mut ready_tests = Vec::new();
    let mut databases_to_drop: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (result_index, test) in tests.into_iter().enumerate() {
        let private_visibility_scope =
            PrivateVisibilityScope::Modules(current_package_private_modules.clone());
        if let Some(message) = super::visibility::private_visibility_error_in_ast(
            &test.source.ast,
            &test.source.synthetic_imports,
            test.test_index,
            &production_exports,
            &private_visibility_scope,
            &package_ids,
            &test_package_aliases,
            &disallowed_import_modules,
        ) {
            results[result_index] = Some(SkiffTestResult {
                module_path: test.module_path,
                name: test.name,
                passed: false,
                skipped: false,
                message: Some(message),
            });
            continue;
        }

        let test_module_paths = [test.module_path.as_str()];
        let test_config = config_for_runtime_test_modules(
            &test_doubles.config,
            &test_doubles.configs,
            &test_module_paths,
            &test.name,
        );
        let request_payload = if options.live {
            runtime_live_request_payload(&test_config).map_err(|message| {
                SkiffTestError::InvalidTestDouble {
                    path: input.display().to_string(),
                    message,
                }
            })?
        } else {
            None
        };
        let expected_error = if options.live {
            runtime_live_expected_error(&test_config).map_err(|message| {
                SkiffTestError::InvalidTestDouble {
                    path: input.display().to_string(),
                    message,
                }
            })?
        } else {
            None
        };
        let service_db_mongo_url = service_db_mongo_url_for_runtime_test(
            test_doubles.service_db_mongo_url.as_deref(),
            &test_config,
        )
        .map_err(|message| SkiffTestError::InvalidTestDouble {
            path: input.display().to_string(),
            message,
        })?;
        let doubles = if options.live {
            None
        } else {
            Some(doubles_for_runtime_test_modules(
                &test_doubles.tests,
                &test_module_paths,
                &test.name,
            ))
        };
        ready_tests.push(ReadyPackageTest {
            result_index,
            test,
            test_config,
            service_db_mongo_url,
            doubles,
            request_payload,
            expected_error,
        });
    }

    run_ready_package_tests(
        &ready_tests,
        &compile_context,
        options,
        dispatch_concurrency_override,
        &mut results,
        &mut databases_to_drop,
    );
    let results = results
        .into_iter()
        .map(|result| result.expect("package test result should be populated before collection"))
        .collect::<Vec<_>>();

    for (mongo_url, service_ids) in &databases_to_drop {
        let _ = drop_test_service_databases(mongo_url, service_ids);
    }

    let passed = results
        .iter()
        .filter(|result| result.passed && !result.skipped)
        .count();
    let skipped = results.iter().filter(|result| result.skipped).count();
    let failed = results.len() - passed - skipped;
    Ok(SkiffTestSummary {
        passed,
        skipped,
        failed,
        results,
    })
}

struct ReadyPackageTest {
    result_index: usize,
    test: PackageTestCase,
    test_config: JsonValue,
    service_db_mongo_url: Option<String>,
    doubles: Option<HashMap<String, Vec<TestEffectDouble>>>,
    request_payload: Option<String>,
    expected_error: Option<RuntimeExpectedError>,
}

struct PackageTestBatchCompileContext<'a> {
    current_manifest: &'a PackageManifest,
    package_root: &'a Path,
    production_sources: &'a [PackageTestSource],
    test_package_aliases: &'a BTreeMap<String, Vec<String>>,
    dependency_artifacts: &'a PackageDependencyArtifacts,
    package_dependencies: &'a [skiff_artifact_model::PackageDependencyConstraint],
    production_package_unit: &'a PackageUnit,
    production_config_and_effect_metadata: &'a ConfigAndEffectMetadata,
    production_artifacts: &'a [PublishedFileIrArtifact],
}

struct CompiledPackageTestOwnerArtifactInput {
    test_file: TestPackageTestFileIrArtifact,
    entrypoints: Vec<(usize, TestPackageTestEntrypointInput)>,
    owner_config_and_effect_metadata: ConfigAndEffectMetadata,
}

fn same_package_test_owner(left: &PackageTestCase, right: &PackageTestCase) -> bool {
    left.source.relative_path == right.source.relative_path
        && left.source.module_path == right.source.module_path
}

fn run_ready_package_tests(
    ready_tests: &[ReadyPackageTest],
    compile_context: &PackageTestBatchCompileContext<'_>,
    options: &SkiffTestOptions,
    dispatch_concurrency_override: Option<usize>,
    results: &mut [Option<SkiffTestResult>],
    databases_to_drop: &mut BTreeMap<String, Vec<String>>,
) {
    if ready_tests.is_empty() {
        return;
    }
    match package_test_artifact_input_for_ready_tests(
        compile_context,
        ready_tests,
        dispatch_concurrency_override,
    ) {
        Ok(artifact_input) => dispatch_package_test_artifact(
            artifact_input,
            ready_tests,
            options,
            dispatch_concurrency_override,
            results,
            databases_to_drop,
        ),
        Err(message) => {
            for ready_test in ready_tests {
                record_package_test_failure(ready_test, message.clone(), results);
            }
        }
    }
}

fn package_test_artifact_input_for_ready_tests(
    context: &PackageTestBatchCompileContext<'_>,
    ready_tests: &[ReadyPackageTest],
    concurrency_override: Option<usize>,
) -> Result<TestPackageTestArtifactInput, String> {
    if ready_tests.is_empty() {
        return Err("package test batch must contain at least one test".to_string());
    }
    let mut test_files = Vec::new();
    let mut package_test_config_and_effect_metadata = ConfigAndEffectMetadata::default();
    let mut entrypoints_by_ready_index = (0..ready_tests.len()).map(|_| None).collect::<Vec<_>>();
    let owner_batches = ready_package_test_owner_batches(ready_tests);
    let compiled_owners = compile_package_test_owner_artifact_inputs(
        context,
        ready_tests,
        &owner_batches,
        concurrency_override,
    )?;
    for compiled_owner in compiled_owners {
        if ready_tests.len() == 1 {
            package_test_config_and_effect_metadata =
                compiled_owner.owner_config_and_effect_metadata.clone();
        }
        test_files.push(compiled_owner.test_file);
        for (ready_index, entrypoint) in compiled_owner.entrypoints {
            entrypoints_by_ready_index[ready_index] = Some(entrypoint);
        }
    }
    Ok(TestPackageTestArtifactInput {
        artifact_root: PathBuf::new(),
        package_id: context.current_manifest.id.clone(),
        package_version: context.current_manifest.version.clone(),
        package_dependencies: context.package_dependencies.to_vec(),
        production_package_unit: Some(context.production_package_unit.clone()),
        production_config_and_effect_metadata: context
            .production_config_and_effect_metadata
            .clone(),
        package_test_config_and_effect_metadata,
        production_files: context.production_artifacts.to_vec(),
        dependency_packages: context
            .dependency_artifacts
            .package_test_dependency_packages
            .clone(),
        test_files,
        entrypoints: entrypoints_by_ready_index
            .into_iter()
            .map(|entrypoint| {
                entrypoint.expect("package test artifact input should build every ready entrypoint")
            })
            .collect(),
    })
}

fn compile_package_test_owner_artifact_inputs(
    context: &PackageTestBatchCompileContext<'_>,
    ready_tests: &[ReadyPackageTest],
    owner_batches: &[Vec<usize>],
    concurrency_override: Option<usize>,
) -> Result<Vec<CompiledPackageTestOwnerArtifactInput>, String> {
    if owner_batches.is_empty() {
        return Ok(Vec::new());
    }
    let concurrency = package_test_artifact_input_compile_concurrency(
        owner_batches.len(),
        concurrency_override,
        available_package_test_parallelism(),
    );
    let next_owner_index = AtomicUsize::new(0);
    let mut compiled = thread::scope(|scope| {
        let handles = (0..concurrency)
            .map(|_| {
                scope.spawn(|| {
                    let mut owner_results = Vec::new();
                    loop {
                        let owner_index = next_owner_index.fetch_add(1, Ordering::Relaxed);
                        if owner_index >= owner_batches.len() {
                            break;
                        }
                        owner_results.push((
                            owner_index,
                            compile_package_test_owner_artifact_input(
                                context,
                                ready_tests,
                                &owner_batches[owner_index],
                            ),
                        ));
                    }
                    owner_results
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .flat_map(|handle| {
                handle
                    .join()
                    .expect("package test owner compile worker should not panic")
            })
            .collect::<Vec<_>>()
    });
    compiled.sort_by_key(|(owner_index, _)| *owner_index);
    compiled
        .into_iter()
        .map(|(_, result)| result)
        .collect::<Result<Vec<_>, _>>()
}

fn compile_package_test_owner_artifact_input(
    context: &PackageTestBatchCompileContext<'_>,
    ready_tests: &[ReadyPackageTest],
    owner_ready_indices: &[usize],
) -> Result<CompiledPackageTestOwnerArtifactInput, String> {
    let first = ready_tests
        .get(owner_ready_indices[0])
        .expect("ready test owner batch index should be valid");
    let source = &first.test.source;
    let test_ast = package_test_ast_for_ready_test_indices(ready_tests, owner_ready_indices);
    let explicit_const_type_annotations = explicit_const_type_annotations(&test_ast);
    let entrypoint_function_names = owner_ready_indices
        .iter()
        .map(|ready_index| ready_tests[*ready_index].test.function_name.clone())
        .collect::<Vec<_>>();
    let test_compiled = package_test_file_ir_artifact_for_test_functions(
        context.current_manifest,
        context.package_root,
        context.production_sources,
        source,
        test_ast.clone(),
        context.test_package_aliases,
        &context.dependency_artifacts.dependency_publications,
        &entrypoint_function_names,
    )
    .map_err(|error| error.to_string())?;
    let test_artifact = test_compiled.artifact;
    let source_path = source.relative_path.to_string_lossy().to_string();
    if test_compiled.entrypoint_config_and_effect_metadata.len() != owner_ready_indices.len() {
        return Err(format!(
            "package test owner {} returned {} entrypoint metadata item(s) for {} selected test(s)",
            source.relative_path.display(),
            test_compiled.entrypoint_config_and_effect_metadata.len(),
            owner_ready_indices.len()
        ));
    }
    let entrypoints = owner_ready_indices
        .iter()
        .copied()
        .zip(test_compiled.entrypoint_config_and_effect_metadata)
        .map(|(ready_index, config_and_effect_metadata)| {
            let ready = &ready_tests[ready_index];
            let executable_ref =
                test_executable_ref(&test_artifact.file_ir, &ready.test.function_name)?;
            Ok((
                ready_index,
                TestPackageTestEntrypointInput {
                    display_name: ready.test.name.clone(),
                    source_path: source_path.clone(),
                    module_path: source.module_path.clone(),
                    test_ordinal: ready.test.test_index as u32,
                    executable_index: executable_ref.index,
                    executable_local_id: ready.test.function_name.clone(),
                    symbol: Some(executable_ref.symbol),
                    default_run: true,
                    config_and_effect_metadata,
                },
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(CompiledPackageTestOwnerArtifactInput {
        test_file: TestPackageTestFileIrArtifact {
            source_path,
            module_path: source.module_path.clone(),
            file_ir: test_artifact.file_ir,
            explicit_const_type_annotations,
        },
        entrypoints,
        owner_config_and_effect_metadata: test_compiled.config_and_effect_metadata,
    })
}

fn package_test_artifact_input_compile_concurrency(
    owner_count: usize,
    override_value: Option<usize>,
    available_parallelism: usize,
) -> usize {
    if owner_count == 0 {
        return 0;
    }
    override_value
        .unwrap_or_else(|| available_parallelism.max(1))
        .max(1)
        .min(owner_count)
}

fn package_test_ast_for_ready_test_indices(
    ready_tests: &[ReadyPackageTest],
    ready_indices: &[usize],
) -> AstSourceFile {
    let first = ready_tests
        .get(ready_indices[0])
        .expect("package test source batch index should be valid");
    if ready_indices.len() == 1 {
        package_test_ast(
            &first.test.source.ast,
            first.test.test_index,
            &first.test.function_name,
        )
    } else {
        package_test_ast_for_cases(
            &first.test.source.ast,
            ready_indices.iter().map(|ready_index| {
                let ready = &ready_tests[*ready_index];
                (ready.test.test_index, ready.test.function_name.as_str())
            }),
        )
    }
}

fn ready_package_test_owner_batches(ready_tests: &[ReadyPackageTest]) -> Vec<Vec<usize>> {
    let mut batches: Vec<Vec<usize>> = Vec::new();
    for (ready_index, ready) in ready_tests.iter().enumerate() {
        if let Some(batch) = batches
            .iter_mut()
            .find(|batch| same_package_test_owner(&ready_tests[batch[0]].test, &ready.test))
        {
            batch.push(ready_index);
        } else {
            batches.push(vec![ready_index]);
        }
    }
    batches
}

fn dispatch_package_test_artifact(
    artifact_input: TestPackageTestArtifactInput,
    ready_tests: &[ReadyPackageTest],
    options: &SkiffTestOptions,
    dispatch_concurrency_override: Option<usize>,
    results: &mut [Option<SkiffTestResult>],
    databases_to_drop: &mut BTreeMap<String, Vec<String>>,
) {
    let prepared = match prepare_dev_synced_package_test(artifact_input, options) {
        Ok(prepared) => prepared,
        Err(message) => {
            for ready in ready_tests {
                record_package_test_failure(ready, message.clone(), results);
            }
            return;
        }
    };
    if let Err(message) = validate_prepared_package_test_entrypoints(
        ready_tests,
        &prepared.entrypoints,
        &prepared.package_id,
        &prepared.package_version,
    ) {
        for ready in ready_tests {
            record_package_test_failure(ready, message.clone(), results);
        }
        return;
    }
    let dispatch_context = prepared.dispatch_context();
    let concurrency = package_test_dispatch_concurrency(
        options.live,
        ready_tests.len(),
        dispatch_concurrency_override,
        available_package_test_parallelism(),
    );
    let reports = dispatch_prepared_package_test_entrypoints(
        &dispatch_context,
        &prepared.entrypoints,
        ready_tests,
        options,
        concurrency,
    );
    merge_package_test_dispatch_reports(ready_tests, reports, options, results, databases_to_drop);
}

fn validate_prepared_package_test_entrypoints(
    ready_tests: &[ReadyPackageTest],
    entrypoints: &[TestPackageTestEntrypointSummary],
    package_id: &str,
    package_version: &str,
) -> Result<(), String> {
    if entrypoints.len() != ready_tests.len() {
        return Err(format!(
            "package test artifact writer returned {} entrypoint(s) for {} selected test(s)",
            entrypoints.len(),
            ready_tests.len()
        ));
    }
    let mut local_ids = BTreeSet::new();
    for (index, (ready, entrypoint)) in ready_tests.iter().zip(entrypoints).enumerate() {
        if entrypoint.display_name != ready.test.name {
            return Err(format!(
                "package test artifact entrypoint {index} display name mismatch: expected `{}`, got `{}`",
                ready.test.name, entrypoint.display_name
            ));
        }
        if entrypoint.entrypoint_local_id.trim().is_empty() {
            return Err(format!(
                "package test artifact entrypoint {index} for `{}` returned an empty local id",
                ready.test.name
            ));
        }
        let expected_local_id =
            expected_package_test_entrypoint_local_id(package_id, package_version, ready)?;
        if entrypoint.entrypoint_local_id != expected_local_id {
            return Err(format!(
                "package test artifact entrypoint {index} local id mismatch for `{}`: expected `{}`, got `{}`",
                ready.test.name, expected_local_id, entrypoint.entrypoint_local_id
            ));
        }
        if !local_ids.insert(entrypoint.entrypoint_local_id.as_str()) {
            return Err(format!(
                "package test artifact entrypoint {index} for `{}` returned duplicate local id `{}`",
                ready.test.name, entrypoint.entrypoint_local_id
            ));
        }
    }
    Ok(())
}

fn expected_package_test_entrypoint_local_id(
    package_id: &str,
    package_version: &str,
    ready: &ReadyPackageTest,
) -> Result<String, String> {
    package_test_entrypoint_local_id(
        package_id,
        package_version,
        &ready.test.source.relative_path.to_string_lossy(),
        ready.test.test_index as u32,
        &ready.test.name,
    )
    .map_err(|error| format!("failed to compute package test entrypoint local id: {error}"))
}

struct PackageTestDispatchReport {
    ready_index: usize,
    result_index: usize,
    runtime_result: Result<(), String>,
    service_db_cleanup: Option<(String, String)>,
}

fn dispatch_prepared_package_test_entrypoints(
    prepared: &PreparedPackageTestDispatchContext<'_>,
    entrypoints: &[TestPackageTestEntrypointSummary],
    ready_tests: &[ReadyPackageTest],
    options: &SkiffTestOptions,
    concurrency: usize,
) -> Vec<PackageTestDispatchReport> {
    if ready_tests.is_empty() {
        return Vec::new();
    }
    if concurrency <= 1 {
        return (0..ready_tests.len())
            .map(|ready_index| {
                dispatch_one_prepared_package_test_entrypoint(
                    prepared,
                    &entrypoints[ready_index],
                    ready_tests,
                    ready_index,
                    options,
                )
            })
            .collect();
    }

    let next_ready_index = AtomicUsize::new(0);
    thread::scope(|scope| {
        let handles = (0..concurrency)
            .map(|_| {
                scope.spawn(|| {
                    let mut reports = Vec::new();
                    loop {
                        let ready_index = next_ready_index.fetch_add(1, Ordering::Relaxed);
                        if ready_index >= ready_tests.len() {
                            break;
                        }
                        reports.push(dispatch_one_prepared_package_test_entrypoint(
                            prepared,
                            &entrypoints[ready_index],
                            ready_tests,
                            ready_index,
                            options,
                        ));
                    }
                    reports
                })
            })
            .collect::<Vec<_>>();
        let mut reports = Vec::with_capacity(ready_tests.len());
        for handle in handles {
            if let Ok(worker_reports) = handle.join() {
                reports.extend(worker_reports);
            }
        }
        reports
    })
}

fn dispatch_one_prepared_package_test_entrypoint(
    prepared: &PreparedPackageTestDispatchContext<'_>,
    entrypoint: &TestPackageTestEntrypointSummary,
    ready_tests: &[ReadyPackageTest],
    ready_index: usize,
    options: &SkiffTestOptions,
) -> PackageTestDispatchReport {
    let ready = &ready_tests[ready_index];
    let result_index = ready.result_index;
    let dispatch = catch_unwind(AssertUnwindSafe(|| {
        execute_dev_synced_package_test_entrypoint_with_context(
            prepared,
            entrypoint,
            ready.test_config.clone(),
            ready.service_db_mongo_url.as_deref(),
            ready.doubles.clone(),
            ready.request_payload.as_deref(),
            ready.expected_error.as_ref(),
            options,
        )
    }));
    match dispatch {
        Ok(report) => {
            let service_db_cleanup = ready
                .service_db_mongo_url
                .as_ref()
                .zip(report.service_db_service_id)
                .map(|(mongo_url, service_id)| (mongo_url.clone(), service_id));
            PackageTestDispatchReport {
                ready_index,
                result_index,
                runtime_result: report.result,
                service_db_cleanup,
            }
        }
        Err(_) => PackageTestDispatchReport {
            ready_index,
            result_index,
            runtime_result: Err("package test dispatch worker panicked".to_string()),
            service_db_cleanup: None,
        },
    }
}

fn merge_package_test_dispatch_reports(
    ready_tests: &[ReadyPackageTest],
    mut reports: Vec<PackageTestDispatchReport>,
    options: &SkiffTestOptions,
    results: &mut [Option<SkiffTestResult>],
    databases_to_drop: &mut BTreeMap<String, Vec<String>>,
) {
    reports.sort_by_key(|report| report.ready_index);
    let mut reported = vec![false; ready_tests.len()];
    for report in reports {
        let Some(ready) = ready_tests.get(report.ready_index) else {
            continue;
        };
        if reported[report.ready_index] {
            continue;
        }
        reported[report.ready_index] = true;
        if ready.result_index != report.result_index {
            record_package_test_failure(
                ready,
                format!(
                    "package test dispatch report index mismatch: ready result index {}, report result index {}",
                    ready.result_index, report.result_index
                ),
                results,
            );
            continue;
        }
        if let Some((mongo_url, service_id)) = report.service_db_cleanup {
            databases_to_drop
                .entry(mongo_url)
                .or_default()
                .push(service_id);
        }
        record_package_test_runtime_result(ready, report.runtime_result, options, results);
    }
    for (ready_index, ready) in ready_tests.iter().enumerate() {
        if !reported[ready_index] {
            record_package_test_failure(
                ready,
                "package test dispatch worker did not return a report".to_string(),
                results,
            );
        }
    }
}

fn package_test_dispatch_concurrency_override(
    options: &SkiffTestOptions,
) -> Result<Option<usize>, String> {
    if options.package_test_concurrency.is_some() {
        return package_test_dispatch_concurrency_override_for_values(
            options.package_test_concurrency,
            None,
        );
    }
    let env_value = match env::var(PACKAGE_TEST_CONCURRENCY_ENV) {
        Ok(value) => Some(value),
        Err(env::VarError::NotPresent) => None,
        Err(env::VarError::NotUnicode(_)) => {
            return Err(format!(
                "{PACKAGE_TEST_CONCURRENCY_ENV} must be a positive integer"
            ));
        }
    };
    package_test_dispatch_concurrency_override_for_values(
        options.package_test_concurrency,
        env_value.as_deref(),
    )
}

fn package_test_dispatch_concurrency_override_for_values(
    option_value: Option<usize>,
    env_value: Option<&str>,
) -> Result<Option<usize>, String> {
    if let Some(value) = option_value {
        if value == 0 {
            return Err("package_test_concurrency must be a positive integer".to_string());
        }
        return Ok(Some(value));
    }
    env_value
        .map(|value| parse_package_test_dispatch_concurrency(value, PACKAGE_TEST_CONCURRENCY_ENV))
        .transpose()
}

fn parse_package_test_dispatch_concurrency(value: &str, source: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{source} must be a positive integer, got {value}"))?;
    if parsed == 0 {
        return Err(format!("{source} must be a positive integer, got {value}"));
    }
    Ok(parsed)
}

fn package_test_dispatch_concurrency(
    live: bool,
    ready_test_count: usize,
    override_value: Option<usize>,
    available_parallelism: usize,
) -> usize {
    if ready_test_count == 0 {
        return 0;
    }
    let limit = match override_value {
        Some(value) => value,
        None if live => 1,
        None => available_parallelism.max(1),
    };
    limit.max(1).min(ready_test_count)
}

fn available_package_test_parallelism() -> usize {
    thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1)
}

fn record_package_test_runtime_result(
    ready: &ReadyPackageTest,
    result: Result<(), String>,
    options: &SkiffTestOptions,
    results: &mut [Option<SkiffTestResult>],
) {
    match result {
        Ok(()) => {
            results[ready.result_index] = Some(SkiffTestResult {
                module_path: ready.test.module_path.clone(),
                name: ready.test.name.clone(),
                passed: true,
                skipped: false,
                message: None,
            });
        }
        Err(message) => {
            if options.live {
                if let Some(message) = live_missing_config_skip_message(&message) {
                    results[ready.result_index] = Some(SkiffTestResult {
                        module_path: ready.test.module_path.clone(),
                        name: ready.test.name.clone(),
                        passed: false,
                        skipped: true,
                        message: Some(message),
                    });
                    return;
                }
            }
            record_package_test_failure(ready, message, results);
        }
    }
}

fn record_package_test_failure(
    ready: &ReadyPackageTest,
    message: String,
    results: &mut [Option<SkiffTestResult>],
) {
    results[ready.result_index] = Some(SkiffTestResult {
        module_path: ready.test.module_path.clone(),
        name: ready.test.name.clone(),
        passed: false,
        skipped: false,
        message: Some(message),
    });
}

fn explicit_const_type_annotations(ast: &AstSourceFile) -> BTreeSet<String> {
    ast.consts
        .iter()
        .filter(|decl| decl.ty.is_some())
        .map(|decl| decl.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::artifacts::package_test_config_and_effect_metadata_for_test_function;

    use super::*;
    use serde_json::json;
    use skiff_compiler::{
        test_support::{compile_package_dependency_publications_for_test, TestResolvedPackage},
        PackageDependency,
    };
    use skiff_syntax::parser::parse_source;

    #[test]
    fn explicit_const_type_annotations_only_records_annotated_consts() {
        let ast = parse_source(
            r#"
                type Provider {}
                const annotated: Provider = Provider {}
                const inferred = Provider {}
            "#,
        )
        .expect("test source should parse");

        let annotations = explicit_const_type_annotations(&ast);

        assert!(annotations.contains("annotated"));
        assert!(!annotations.contains("inferred"));
    }

    #[test]
    fn package_test_metadata_only_entrypoints_keep_single_case_config() {
        let package_root = Path::new("/tmp/skiff-test-runner-package-test-metadata");
        let manifest = PackageManifest {
            id: "example.com/meta".to_string(),
            version: "1.0.0".to_string(),
            api: Vec::new(),
            dependencies: Vec::new(),
            path: package_root.join("package.yml"),
            synthetic: false,
        };
        let production_source = package_source(
            "main.skiff",
            "main",
            false,
            r#"
                function readProdSecret() -> string {
                    return config.require<string>("prod.secret")
                }
            "#,
        );
        let test_source = package_source(
            "main.test.skiff",
            "main.__test",
            true,
            r#"
                const sharedConst: string = config.require<string>("test.sharedConst")
                const sharedEnabled: boolean = config.has("test.sharedEnabled")

                function sharedHelper() -> string {
                    return config.require<string>("test.sharedHelper")
                }

                test "case a" {
                    assert sharedHelper() == config.require<string>("test.caseA")
                }

                test "case b" {
                    assert sharedConst == config.require<string>("test.caseB")
                }
            "#,
        );
        let package_aliases = BTreeMap::new();
        let production_sources = vec![production_source];
        let dependency_artifacts = PackageDependencyArtifacts::default();
        let production_compiled = package_source_file_ir_artifacts_with_dependency_publications(
            &manifest,
            package_root,
            &production_sources,
            &package_aliases,
            &dependency_artifacts.dependency_publications,
        )
        .expect("production package should compile");
        let package_dependencies = production_compiled.package_unit.dependencies.clone();
        let production_package_unit = production_compiled.package_unit.clone();
        let production_config_and_effect_metadata =
            production_compiled.config_and_effect_metadata.clone();
        let production_artifacts = production_compiled.artifacts;
        let compile_context = PackageTestBatchCompileContext {
            current_manifest: &manifest,
            package_root,
            production_sources: &production_sources,
            test_package_aliases: &package_aliases,
            dependency_artifacts: &dependency_artifacts,
            package_dependencies: &package_dependencies,
            production_package_unit: &production_package_unit,
            production_config_and_effect_metadata: &production_config_and_effect_metadata,
            production_artifacts: &production_artifacts,
        };
        let test_cases = collect_package_test_cases(std::slice::from_ref(&test_source));
        let ready_tests = test_cases
            .into_iter()
            .enumerate()
            .map(|(result_index, test)| ReadyPackageTest {
                result_index,
                test,
                test_config: json!({}),
                service_db_mongo_url: None,
                doubles: None,
                request_payload: None,
                expected_error: None,
            })
            .collect::<Vec<_>>();

        let artifact_input =
            package_test_artifact_input_for_ready_tests(&compile_context, &ready_tests, None)
                .expect("package test artifact input should compile");
        let single_case_metadata = ready_tests
            .iter()
            .map(|ready| {
                let test_ast = package_test_ast(
                    &ready.test.source.ast,
                    ready.test.test_index,
                    &ready.test.function_name,
                );
                package_test_config_and_effect_metadata_for_test_function(
                    &manifest,
                    package_root,
                    &production_sources,
                    &ready.test.source,
                    test_ast,
                    &package_aliases,
                    &dependency_artifacts.dependency_publications,
                )
                .expect("single-case metadata should compile")
            })
            .collect::<Vec<_>>();

        assert_eq!(artifact_input.entrypoints.len(), 2);
        let first_metadata =
            serde_json::to_value(&artifact_input.entrypoints[0].config_and_effect_metadata)
                .expect("entrypoint metadata should serialize");
        let second_metadata =
            serde_json::to_value(&artifact_input.entrypoints[1].config_and_effect_metadata)
                .expect("entrypoint metadata should serialize");
        assert_config_shape_path(&first_metadata, "prod.secret");
        assert_config_shape_path(&first_metadata, "test.sharedConst");
        assert_config_shape_path(&first_metadata, "test.sharedHelper");
        assert_config_activation_path(&first_metadata, "test.sharedEnabled");
        assert_config_shape_path(&first_metadata, "test.caseA");
        assert!(!config_shape_has_path(&first_metadata, "test.caseB"));
        assert_config_shape_path(&second_metadata, "prod.secret");
        assert_config_shape_path(&second_metadata, "test.sharedConst");
        assert_config_shape_path(&second_metadata, "test.sharedHelper");
        assert_config_activation_path(&second_metadata, "test.sharedEnabled");
        assert_config_shape_path(&second_metadata, "test.caseB");
        assert!(!config_shape_has_path(&second_metadata, "test.caseA"));
        assert!(artifact_input.entrypoints[0]
            .config_and_effect_metadata
            .effects
            .is_empty());
        assert!(artifact_input.entrypoints[1]
            .config_and_effect_metadata
            .effects
            .is_empty());
        assert_eq!(
            first_metadata,
            serde_json::to_value(&single_case_metadata[0])
                .expect("single-case metadata should serialize")
        );
        assert_eq!(
            second_metadata,
            serde_json::to_value(&single_case_metadata[1])
                .expect("single-case metadata should serialize")
        );
    }

    #[test]
    fn package_test_metadata_entrypoints_reuse_dependency_publications() {
        let package_root = Path::new("/tmp/skiff-test-runner-package-test-metadata-reuse-current");
        let dependency_root = Path::new("/tmp/skiff-test-runner-package-test-metadata-reuse-dep");
        let _ = fs::remove_dir_all(dependency_root);
        fs::create_dir_all(dependency_root).expect("dependency package root");
        fs::write(
            dependency_root.join("package.yml"),
            "id: example.com/dep\nversion: 1.0.0\n",
        )
        .expect("dependency package manifest");
        fs::write(
            dependency_root.join("dep.skiff"),
            r#"
                function depSecret() -> string {
                    return config.require<string>("dep.secret")
                }
            "#,
        )
        .expect("dependency package source");

        let dependency = PackageManifest {
            id: "example.com/dep".to_string(),
            version: "1.0.0".to_string(),
            api: Vec::new(),
            dependencies: Vec::new(),
            path: dependency_root.join("package.yml"),
            synthetic: false,
        };
        let mut dependency_ref = PackageDependency::id("example.com/dep");
        dependency_ref.alias = Some("dep".to_string());
        let manifest = PackageManifest {
            id: "example.com/meta".to_string(),
            version: "1.0.0".to_string(),
            api: Vec::new(),
            dependencies: vec![dependency_ref],
            path: package_root.join("package.yml"),
            synthetic: false,
        };
        let dependency_package = TestResolvedPackage {
            manifest: dependency.clone(),
            config: JsonValue::Null,
        };
        let available_packages = BTreeMap::from([
            (
                (manifest.id.clone(), manifest.version.clone()),
                manifest.clone(),
            ),
            (
                (dependency.id.clone(), dependency.version.clone()),
                dependency.clone(),
            ),
        ]);
        let dependency_publications = compile_package_dependency_publications_for_test(
            &manifest,
            std::slice::from_ref(&dependency_package),
            &available_packages,
        )
        .expect("dependency publications should compile once");
        let production_sources = vec![package_source(
            "main.skiff",
            "main",
            false,
            r#"
                function readProdSecret() -> string {
                    return config.require<string>("prod.secret")
                }
            "#,
        )];
        let test_source = package_source(
            "main.test.skiff",
            "main.__test",
            true,
            r#"
                function sharedHelper() -> string {
                    return config.require<string>("test.sharedHelper")
                }

                test "case a" {
                    assert sharedHelper() == config.require<string>("test.caseA")
                }
            "#,
        );
        let package_aliases = BTreeMap::new();
        let dependency_artifacts = PackageDependencyArtifacts {
            dependency_publications,
            ..PackageDependencyArtifacts::default()
        };
        let production_compiled = package_source_file_ir_artifacts_with_dependency_publications(
            &manifest,
            package_root,
            &production_sources,
            &package_aliases,
            &dependency_artifacts.dependency_publications,
        )
        .expect("production package should compile");
        let package_dependencies = production_compiled.package_unit.dependencies.clone();
        let production_package_unit = production_compiled.package_unit.clone();
        let production_config_and_effect_metadata =
            production_compiled.config_and_effect_metadata.clone();
        let production_artifacts = production_compiled.artifacts;
        let compile_context = PackageTestBatchCompileContext {
            current_manifest: &manifest,
            package_root,
            production_sources: &production_sources,
            test_package_aliases: &package_aliases,
            dependency_artifacts: &dependency_artifacts,
            package_dependencies: &package_dependencies,
            production_package_unit: &production_package_unit,
            production_config_and_effect_metadata: &production_config_and_effect_metadata,
            production_artifacts: &production_artifacts,
        };
        let test_cases = collect_package_test_cases(std::slice::from_ref(&test_source));
        let ready_tests = test_cases
            .into_iter()
            .enumerate()
            .map(|(result_index, test)| ReadyPackageTest {
                result_index,
                test,
                test_config: json!({}),
                service_db_mongo_url: None,
                doubles: None,
                request_payload: None,
                expected_error: None,
            })
            .collect::<Vec<_>>();

        let artifact_input =
            package_test_artifact_input_for_ready_tests(&compile_context, &ready_tests, None)
                .expect("package test artifact input should compile");

        let entrypoint_metadata =
            serde_json::to_value(&artifact_input.entrypoints[0].config_and_effect_metadata)
                .expect("entrypoint metadata should serialize");
        assert_config_shape_path(&entrypoint_metadata, "prod.secret");
        assert_config_shape_path(&entrypoint_metadata, "test.sharedHelper");
        assert_config_shape_path(&entrypoint_metadata, "test.caseA");
        assert_dependency_requirement(&entrypoint_metadata, "dep.secret", "example.com/dep", "dep");

        let _ = fs::remove_dir_all(dependency_root);
    }

    #[test]
    fn package_test_artifact_input_includes_all_ready_owner_files() {
        let package_root = Path::new("/tmp/skiff-test-runner-package-test-multi-file-artifact");
        let manifest = PackageManifest {
            id: "example.com/multi".to_string(),
            version: "1.0.0".to_string(),
            api: Vec::new(),
            dependencies: Vec::new(),
            path: package_root.join("package.yml"),
            synthetic: false,
        };
        let production_sources = vec![package_source(
            "main.skiff",
            "main",
            false,
            r#"
                function answer() -> string {
                    return "ok"
                }
            "#,
        )];
        let first_test_source = package_source(
            "alpha.test.skiff",
            "alpha.__test",
            true,
            r#"
                test "alpha one" {
                    assert true
                }

                test "alpha two" {
                    assert true
                }
            "#,
        );
        let second_test_source = package_source(
            "beta.test.skiff",
            "beta.__test",
            true,
            r#"
                test "beta one" {
                    assert true
                }
            "#,
        );
        let package_aliases = BTreeMap::new();
        let dependency_artifacts = PackageDependencyArtifacts::default();
        let production_compiled = package_source_file_ir_artifacts_with_dependency_publications(
            &manifest,
            package_root,
            &production_sources,
            &package_aliases,
            &dependency_artifacts.dependency_publications,
        )
        .expect("production package should compile");
        let package_dependencies = production_compiled.package_unit.dependencies.clone();
        let production_package_unit = production_compiled.package_unit.clone();
        let production_config_and_effect_metadata =
            production_compiled.config_and_effect_metadata.clone();
        let production_artifacts = production_compiled.artifacts;
        let compile_context = PackageTestBatchCompileContext {
            current_manifest: &manifest,
            package_root,
            production_sources: &production_sources,
            test_package_aliases: &package_aliases,
            dependency_artifacts: &dependency_artifacts,
            package_dependencies: &package_dependencies,
            production_package_unit: &production_package_unit,
            production_config_and_effect_metadata: &production_config_and_effect_metadata,
            production_artifacts: &production_artifacts,
        };
        let test_cases = collect_package_test_cases(&[first_test_source, second_test_source]);
        let ready_tests = test_cases
            .into_iter()
            .enumerate()
            .map(|(result_index, test)| ReadyPackageTest {
                result_index,
                test,
                test_config: json!({}),
                service_db_mongo_url: None,
                doubles: None,
                request_payload: None,
                expected_error: None,
            })
            .collect::<Vec<_>>();

        let artifact_input =
            package_test_artifact_input_for_ready_tests(&compile_context, &ready_tests, None)
                .expect("package-level artifact input should compile all ready files");

        assert_eq!(
            artifact_input
                .test_files
                .iter()
                .map(|file| file.source_path.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha.test.skiff", "beta.test.skiff"]
        );
        assert_eq!(
            artifact_input
                .entrypoints
                .iter()
                .map(|entrypoint| (
                    entrypoint.display_name.as_str(),
                    entrypoint.source_path.as_str(),
                    entrypoint.test_ordinal
                ))
                .collect::<Vec<_>>(),
            vec![
                ("alpha one", "alpha.test.skiff", 0),
                ("alpha two", "alpha.test.skiff", 1),
                ("beta one", "beta.test.skiff", 0),
            ]
        );
    }

    #[test]
    fn package_test_dispatch_concurrency_defaults_and_overrides() {
        assert_eq!(package_test_dispatch_concurrency(false, 8, None, 8), 8);
        assert_eq!(package_test_dispatch_concurrency(false, 2, None, 8), 2);
        assert_eq!(package_test_dispatch_concurrency(false, 8, None, 1), 1);
        assert_eq!(package_test_dispatch_concurrency(true, 8, None, 8), 1);
        assert_eq!(package_test_dispatch_concurrency(true, 8, Some(3), 8), 3);
        assert_eq!(package_test_dispatch_concurrency(false, 8, Some(2), 8), 2);
        assert_eq!(package_test_dispatch_concurrency(false, 8, Some(99), 8), 8);
    }

    #[test]
    fn package_test_artifact_input_compile_concurrency_uses_available_parallelism() {
        assert_eq!(
            package_test_artifact_input_compile_concurrency(0, None, 8),
            0
        );
        assert_eq!(
            package_test_artifact_input_compile_concurrency(2, None, 8),
            2
        );
        assert_eq!(
            package_test_artifact_input_compile_concurrency(8, None, 2),
            2
        );
        assert_eq!(
            package_test_artifact_input_compile_concurrency(8, None, 0),
            1
        );
        assert_eq!(
            package_test_artifact_input_compile_concurrency(8, Some(1), 8),
            1
        );
        assert_eq!(
            package_test_artifact_input_compile_concurrency(8, Some(99), 8),
            8
        );
    }

    #[test]
    fn package_test_dispatch_concurrency_override_prefers_options_over_env() {
        assert_eq!(
            package_test_dispatch_concurrency_override_for_values(Some(2), Some("3")).unwrap(),
            Some(2)
        );
        assert_eq!(
            package_test_dispatch_concurrency_override_for_values(None, Some("3")).unwrap(),
            Some(3)
        );
        assert_eq!(
            package_test_dispatch_concurrency_override_for_values(None, None).unwrap(),
            None
        );
        assert!(package_test_dispatch_concurrency_override_for_values(Some(0), Some("3")).is_err());
        assert!(package_test_dispatch_concurrency_override_for_values(None, Some("0")).is_err());
    }

    #[test]
    fn package_test_dispatch_report_merge_orders_results_and_cleanup_ids() {
        let ready_tests = vec![
            ready_package_test(0, "case a", "__skiff_test_0", Some("mongo://test")),
            ready_package_test(1, "case b", "__skiff_test_1", None),
            ready_package_test(2, "case c", "__skiff_test_2", Some("mongo://test")),
        ];
        let reports = vec![
            PackageTestDispatchReport {
                ready_index: 2,
                result_index: 2,
                runtime_result: Ok(()),
                service_db_cleanup: Some(("mongo://test".to_string(), "svc-c".to_string())),
            },
            PackageTestDispatchReport {
                ready_index: 0,
                result_index: 0,
                runtime_result: Err("case a failed".to_string()),
                service_db_cleanup: Some(("mongo://test".to_string(), "svc-a".to_string())),
            },
            PackageTestDispatchReport {
                ready_index: 1,
                result_index: 1,
                runtime_result: Ok(()),
                service_db_cleanup: None,
            },
        ];
        let mut result_slots = vec![None, None, None];
        let mut databases_to_drop = BTreeMap::new();

        merge_package_test_dispatch_reports(
            &ready_tests,
            reports,
            &SkiffTestOptions::default(),
            &mut result_slots,
            &mut databases_to_drop,
        );

        let results = result_slots
            .into_iter()
            .map(|result| result.expect("dispatch report should populate result"))
            .collect::<Vec<_>>();
        assert_eq!(
            results
                .iter()
                .map(|result| result.name.as_str())
                .collect::<Vec<_>>(),
            vec!["case a", "case b", "case c"]
        );
        assert!(!results[0].passed);
        assert_eq!(results[0].message.as_deref(), Some("case a failed"));
        assert!(results[1].passed);
        assert!(results[2].passed);
        assert_eq!(
            databases_to_drop.get("mongo://test").cloned(),
            Some(vec!["svc-a".to_string(), "svc-c".to_string()])
        );
    }

    #[test]
    fn package_test_dispatch_validates_entrypoint_alignment() {
        let package_id = "example.com/pkg";
        let package_version = "1.0.0";
        let ready_tests = vec![ready_package_test(0, "case a", "__skiff_test_0", None)];
        let expected_local_id =
            expected_package_test_entrypoint_local_id(package_id, package_version, &ready_tests[0])
                .expect("expected local id should compute");

        assert!(validate_prepared_package_test_entrypoints(
            &ready_tests,
            &[TestPackageTestEntrypointSummary {
                display_name: "case a".to_string(),
                entrypoint_local_id: expected_local_id.clone(),
                entrypoint_id: "entrypoint-a".to_string(),
            }],
            package_id,
            package_version,
        )
        .is_ok());

        let message = validate_prepared_package_test_entrypoints(
            &ready_tests,
            &[TestPackageTestEntrypointSummary {
                display_name: "case b".to_string(),
                entrypoint_local_id: expected_local_id,
                entrypoint_id: "entrypoint-a".to_string(),
            }],
            package_id,
            package_version,
        )
        .expect_err("display name mismatch should fail validation");
        assert!(message.contains("display name mismatch"));

        let local_id_message = validate_prepared_package_test_entrypoints(
            &ready_tests,
            &[TestPackageTestEntrypointSummary {
                display_name: "case a".to_string(),
                entrypoint_local_id:
                    "skiff-package-test-entrypoint-local-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                entrypoint_id: "entrypoint-a".to_string(),
            }],
            package_id,
            package_version,
        )
        .expect_err("wrong local id should fail validation");
        assert!(local_id_message.contains("local id mismatch"));
    }

    fn package_source(
        path: &str,
        module_path: &str,
        is_test_file: bool,
        text: &str,
    ) -> PackageTestSource {
        PackageTestSource {
            relative_path: PathBuf::from(path),
            module_path: module_path.to_string(),
            is_test_file,
            text: text.to_string(),
            ast: parse_source(text).expect("test source should parse"),
            synthetic_imports: BTreeSet::new(),
            friend_module_path: None,
        }
    }

    fn ready_package_test(
        result_index: usize,
        name: &str,
        function_name: &str,
        service_db_mongo_url: Option<&str>,
    ) -> ReadyPackageTest {
        ReadyPackageTest {
            result_index,
            test: PackageTestCase {
                module_path: "cases.__test".to_string(),
                name: name.to_string(),
                test_index: result_index,
                source: package_source(
                    "cases.test.skiff",
                    "cases.__test",
                    true,
                    r#"test "case" { assert true }"#,
                ),
                function_name: function_name.to_string(),
            },
            test_config: json!({}),
            service_db_mongo_url: service_db_mongo_url.map(ToString::to_string),
            doubles: None,
            request_payload: None,
            expected_error: None,
        }
    }

    fn config_shape_has_path(metadata: &JsonValue, path: &str) -> bool {
        metadata["config"]["shape"]["entries"]
            .as_array()
            .expect("shape entries should be an array")
            .iter()
            .any(|entry| entry["path"] == path)
    }

    fn assert_config_shape_path(metadata: &JsonValue, path: &str) {
        assert!(
            config_shape_has_path(metadata, path),
            "expected config shape path {path} in {metadata}"
        );
    }

    fn assert_config_activation_path(metadata: &JsonValue, path: &str) {
        let has_paths = metadata["config"]["activation"]["hasPaths"]
            .as_array()
            .expect("activation hasPaths should be an array");
        assert!(
            has_paths.iter().any(|entry| entry == path),
            "expected activation path {path} in {metadata}"
        );
    }

    fn assert_dependency_requirement(
        metadata: &JsonValue,
        path: &str,
        package_id: &str,
        alias: &str,
    ) {
        let requirements = metadata["config"]["requirements"]["dependency"]
            .as_array()
            .expect("dependency requirements should be an array");
        let requirement = requirements
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("expected dependency requirement {path} in {metadata}"));
        assert_eq!(
            requirement["provenance"][0]["declaringPublication"]["id"],
            package_id
        );
        assert_eq!(
            requirement["provenance"][0]["dependencyPath"][0]["alias"],
            alias
        );
    }
}

struct TestExecutableRef {
    index: u32,
    symbol: String,
}

fn test_executable_ref(file_ir: &FileIrUnit, symbol: &str) -> Result<TestExecutableRef, String> {
    if let Some(declaration) = file_ir.declarations.executables.get(symbol) {
        let executable = file_ir
            .executables
            .get(declaration.executable_index as usize)
            .ok_or_else(|| {
                format!(
                    "package test entrypoint executable `{symbol}` index {} was not emitted",
                    declaration.executable_index
                )
            })?;
        return Ok(TestExecutableRef {
            index: declaration.executable_index,
            symbol: executable.symbol.clone(),
        });
    }
    file_ir
        .executables
        .iter()
        .enumerate()
        .find(|(_, executable)| executable.symbol == symbol)
        .map(|(index, executable)| TestExecutableRef {
            index: index as u32,
            symbol: executable.symbol.clone(),
        })
        .ok_or_else(|| format!("package test entrypoint executable `{symbol}` was not emitted"))
}

pub(super) fn current_package_manifest(
    package_root: &Path,
) -> Result<PackageManifest, SkiffTestError> {
    let manifest_path = package_root.join(PACKAGE_CONFIG_FILE);
    let manifests = discover_package_manifests(package_root)?;
    let canonical_manifest_path =
        fs::canonicalize(&manifest_path).map_err(|source| SkiffTestError::ReadSource {
            path: manifest_path.display().to_string(),
            source,
        })?;
    for manifest in manifests.values() {
        let Ok(candidate_path) = fs::canonicalize(&manifest.path) else {
            continue;
        };
        if candidate_path == canonical_manifest_path {
            return Ok(manifest.clone());
        }
    }
    read_user_package_manifest(&manifest_path).map_err(Into::into)
}
