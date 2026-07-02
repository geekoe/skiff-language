use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use skiff_compiler::read_service_config_with_profile;

use super::{
    artifacts::service_dependency_artifacts,
    doubles::{
        config_for_runtime_test_modules, doubles_for_runtime_test_modules,
        live_missing_config_skip_message, read_runtime_test_inputs,
        service_db_mongo_url_for_runtime_test,
    },
    root_paths::{resolve_parsed_root_paths_with_exports, resolve_service_test_root_paths},
    runtime_process::{
        drop_test_service_databases, execute_dev_synced_service_test, synthetic_test_service_id,
        RuntimeExpectedError,
    },
    service_publish::{build_service_publication_runtime_test, ServiceRuntimePublicationInput},
    sources::{
        collect_test_cases, find_service_root, is_test_file_path, read_root_sources,
        service_test_runtime_module_path,
    },
    visibility::{
        merge_function_return_types, private_visibility_error_in_ast,
        private_visibility_error_in_test_body, service_function_return_types,
        service_production_exports, ALL_PRIVATE_MODULES,
    },
    PrivateVisibilityScope, ResolvedPublicationTestInputs, SkiffTestError, SkiffTestOptions,
    SkiffTestResult, SkiffTestSummary,
};

pub(super) fn run_service_tests(
    input: &Path,
    profile: Option<&str>,
    input_is_file: bool,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, SkiffTestError> {
    let service_root = if input_is_file {
        find_service_root(input, true)
    } else if input.join(skiff_compiler::SERVICE_CONFIG_FILE).is_file() {
        Some(input.to_path_buf())
    } else {
        None
    };
    let Some(service_root) = service_root else {
        return Err(missing_service_root_error(input, input_is_file));
    };
    let service_config = read_service_config_with_profile(&service_root, profile)?;
    let parsed_sources = read_root_sources(&service_root, profile)?;
    let test_doubles = read_runtime_test_inputs(&service_root, false, options)?;
    let production_sources = parsed_sources.clone();
    let service_dependencies =
        service_dependency_artifacts(&service_root, profile, &production_sources, options)?;
    let mut production_exports = service_production_exports(&production_sources);
    production_exports.extend(service_dependencies.production_exports.clone());
    let mut function_return_types = service_function_return_types(&production_sources);
    merge_function_return_types(
        &mut function_return_types,
        service_dependencies.function_return_types.clone(),
    );
    let parsed_sources =
        resolve_service_test_root_paths(parsed_sources, &production_sources, &production_exports)?;
    let test_sources = if input_is_file {
        explicit_service_root_test_sources(parsed_sources, &service_root, input)?
    } else {
        parsed_sources
            .into_iter()
            .filter(|source| source.ast.test_default_run.unwrap_or(true))
            .collect()
    };
    let production_sources =
        resolve_parsed_root_paths_with_exports(production_sources, &production_exports)?;
    let package_ids = service_dependencies.package_ids.clone();
    let package_aliases = service_dependencies.package_aliases.clone();
    let disallowed_import_modules = BTreeSet::new();

    run_resolved_publication_tests(
        ResolvedPublicationTestInputs {
            service_config,
            service_id_scope: "service-test".to_string(),
            production_sources,
            test_sources,
            test_doubles,
            production_exports,
            package_ids,
            package_aliases,
            disallowed_import_modules,
        },
        input,
        options,
    )
}

pub(super) fn run_resolved_publication_tests(
    inputs: ResolvedPublicationTestInputs,
    input: &Path,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, SkiffTestError> {
    let ResolvedPublicationTestInputs {
        service_config,
        service_id_scope,
        production_sources,
        test_sources,
        test_doubles,
        production_exports,
        package_ids,
        package_aliases,
        disallowed_import_modules,
    } = inputs;
    let tests = collect_test_cases(&test_sources)?;
    let mut results = Vec::with_capacity(tests.len());
    // Per-test service ids whose Mongo database must be dropped after the run,
    // grouped by mongo url. Each test uses a unique service id (=> unique
    // database), so dropping them prevents test databases from accumulating
    // run-over-run. The drop is batched into one connection per url at the end
    // of the run rather than one connection per test, because the dev service
    // db may live behind a slow remote tunnel where 32 extra connections would
    // worsen contention.
    let mut databases_to_drop: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for test in tests {
        // Mint a fresh service id per test so each test runs against its own
        // Mongo database namespace (the runtime derives the storage database
        // name from the service id). Reusing one id across the run let a global
        // `db find` in one test observe sibling tests' rows.
        let service_id = synthetic_test_service_id(&service_id_scope);
        let visibility_error = if test.source.source.is_test_file {
            private_visibility_error_in_ast(
                &test.source.ast,
                &test.source.synthetic_imports,
                test.test_index,
                &production_exports,
                &test.source.private_visibility_scope,
                &package_ids,
                &package_aliases,
                &disallowed_import_modules,
            )
        } else {
            private_visibility_error_in_test_body(
                &test.source.ast,
                &test.source.synthetic_imports,
                test.test_index,
                &production_exports,
                &PrivateVisibilityScope::Module(test.source.source.module_path.clone()),
                &package_ids,
                &package_aliases,
                &disallowed_import_modules,
            )
        };
        if let Some(message) = visibility_error {
            results.push(SkiffTestResult {
                module_path: test.module_path,
                name: test.name,
                passed: false,
                skipped: false,
                message: Some(message),
            });
            continue;
        }

        let operation_module = service_test_runtime_module_path(&test.source);
        let test_module_paths = [test.module_path.as_str(), operation_module.as_str()];
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
        let publication =
            match build_service_publication_runtime_test(ServiceRuntimePublicationInput {
                service_config: &service_config,
                service_id: &service_id,
                production_sources: &production_sources,
                test_source: &test.source,
                test_index: test.test_index,
                function_name: &test.function_name,
                operation_module: &operation_module,
                request_payload_param: request_payload.is_some(),
                options,
            }) {
                Ok(publication) => publication,
                Err(error) => {
                    results.push(SkiffTestResult {
                        module_path: test.module_path,
                        name: test.name,
                        passed: false,
                        skipped: false,
                        message: Some(error.to_string()),
                    });
                    continue;
                }
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
        let result = execute_dev_synced_service_test(
            &publication,
            &package_aliases,
            test_config,
            service_db_mongo_url.as_deref(),
            doubles,
            request_payload.as_deref(),
            expected_error.as_ref(),
            options,
        );
        // Record this test's per-test database for batched teardown after the
        // run. Skipped in `live` mode, which runs against the real, persistent
        // service id that must not be dropped.
        if !options.live {
            if let Some(mongo_url) = service_db_mongo_url.as_deref() {
                databases_to_drop
                    .entry(mongo_url.to_string())
                    .or_default()
                    .push(service_id.clone());
            }
        }
        match result {
            Ok(()) => results.push(SkiffTestResult {
                module_path: test.module_path,
                name: test.name,
                passed: true,
                skipped: false,
                message: None,
            }),
            Err(message) => {
                if options.live {
                    if let Some(message) = live_missing_config_skip_message(&message) {
                        results.push(SkiffTestResult {
                            module_path: test.module_path,
                            name: test.name,
                            passed: false,
                            skipped: true,
                            message: Some(message),
                        });
                        continue;
                    }
                }
                results.push(SkiffTestResult {
                    module_path: test.module_path,
                    name: test.name,
                    passed: false,
                    skipped: false,
                    message: Some(message),
                });
            }
        }
    }

    // Best-effort teardown of every per-test database created by this run.
    // Failures (e.g. mongosh unavailable) are intentionally ignored: leaving a
    // database behind is preferable to failing an otherwise-passing run.
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

pub(crate) fn runtime_live_request_payload(
    config: &serde_json::Value,
) -> Result<Option<String>, String> {
    match config.pointer("/runtimeLive/requestPayload") {
        Some(serde_json::Value::String(value)) => Ok(Some(value.clone())),
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(_) => Err("runtimeLive.requestPayload must be a string".to_string()),
    }
}

pub(crate) fn runtime_live_expected_error(
    config: &serde_json::Value,
) -> Result<Option<RuntimeExpectedError>, String> {
    let Some(value) = config.pointer("/runtimeLive/expectedError") else {
        return Ok(None);
    };
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(code) => RuntimeExpectedError::new(code.clone(), None).map(Some),
        serde_json::Value::Object(object) => {
            for key in object.keys() {
                if !matches!(key.as_str(), "code" | "messageContains") {
                    return Err(format!("runtimeLive.expectedError has unknown field {key}"));
                }
            }
            let code = match object.get("code") {
                Some(serde_json::Value::String(value)) => value.clone(),
                Some(_) => {
                    return Err(
                        "runtimeLive.expectedError.code must be a non-empty string".to_string()
                    )
                }
                None => return Err("runtimeLive.expectedError.code is required".to_string()),
            };
            let message_contains = match object.get("messageContains") {
                Some(serde_json::Value::String(value)) => Some(value.clone()),
                Some(serde_json::Value::Null) | None => None,
                Some(_) => {
                    return Err(
                        "runtimeLive.expectedError.messageContains must be a string".to_string()
                    )
                }
            };
            RuntimeExpectedError::new(code, message_contains).map(Some)
        }
        _ => Err("runtimeLive.expectedError must be a string or object".to_string()),
    }
}

fn missing_service_root_error(input: &Path, input_is_file: bool) -> SkiffTestError {
    let message = if input_is_file {
        format!(
            "service test file input {} is not inside a service root with {}",
            input.display(),
            skiff_compiler::SERVICE_CONFIG_FILE
        )
    } else {
        format!(
            "service test directory input {} must be a service root containing {}",
            input.display(),
            skiff_compiler::SERVICE_CONFIG_FILE
        )
    };
    SkiffTestError::RuntimeSetup { message }
}

fn explicit_service_root_test_sources(
    sources: Vec<super::ParsedSource>,
    service_root: &Path,
    input: &Path,
) -> Result<Vec<super::ParsedSource>, SkiffTestError> {
    if is_test_file_path(input) {
        return Ok(sources
            .into_iter()
            .filter_map(|mut source| {
                if source_path_matches(service_root, &source, input) {
                    source.friend_module_path = Some(ALL_PRIVATE_MODULES.to_string());
                    source.private_visibility_scope = PrivateVisibilityScope::AllModules;
                    Some(source)
                } else {
                    None
                }
            })
            .collect());
    }
    let production_module = sources
        .iter()
        .find(|source| source_path_matches(service_root, source, input))
        .map(|source| source.source.module_path.clone());
    Ok(sources
        .into_iter()
        .filter(|source| {
            source.source.is_test_file
                && source.ast.test_default_run.unwrap_or(true)
                && production_module
                    .as_deref()
                    .is_some_and(|module| source.friend_module_path.as_deref() == Some(module))
        })
        .collect())
}

fn source_path_matches(service_root: &Path, source: &super::ParsedSource, input: &Path) -> bool {
    let path = if source.source.file_path.is_absolute() {
        source.source.file_path.clone()
    } else {
        service_root.join(&source.source.file_path)
    };
    path == input || fs::canonicalize(path).ok() == fs::canonicalize(input).ok()
}
