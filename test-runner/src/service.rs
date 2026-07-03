use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::Path,
};

use sha2::{Digest, Sha256};
use skiff_compiler::{read_service_config_with_profile, ServiceConfig};

use super::types::TestEffectDouble;
use super::{
    artifacts::service_dependency_artifacts,
    doubles::{
        config_for_runtime_test_modules, doubles_for_runtime_test_modules,
        live_missing_config_skip_message, read_runtime_test_inputs,
        service_db_mongo_url_for_runtime_test,
    },
    root_paths::{resolve_parsed_root_paths_with_exports, resolve_service_test_root_paths},
    runtime_process::{
        drop_test_service_databases, execute_dev_synced_service_test_case_with_context,
        prepare_dev_synced_service_test_suite, synthetic_test_service_id, RuntimeExpectedError,
        ServiceTestSuiteActivationInput,
    },
    service_publish::{
        build_service_publication_runtime_test_suite, ServiceRuntimeSuiteCaseInput,
        ServiceRuntimeSuitePublicationInput,
    },
    sources::{
        collect_test_cases, find_service_root, is_test_file_path, read_root_sources,
        service_test_runtime_module_path,
    },
    visibility::{
        merge_function_return_types, private_visibility_error_in_ast,
        private_visibility_error_in_test_body, service_function_return_types,
        service_production_exports, ALL_PRIVATE_MODULES,
    },
    ParsedSource, PrivateVisibilityScope, ResolvedPublicationTestInputs, SkiffTestError,
    SkiffTestOptions, SkiffTestResult, SkiffTestSummary, TestCase,
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
    let test_count = tests.len();
    let mut results = (0..test_count).map(|_| None).collect::<Vec<_>>();
    let mut ready_tests = Vec::new();
    // Per-test service ids whose Mongo database must be dropped after the run,
    // grouped by mongo url. Each test uses a unique service id (=> unique
    // database), so dropping them prevents test databases from accumulating
    // run-over-run. The drop is batched into one connection per url at the end
    // of the run rather than one connection per test, because the dev service
    // db may live behind a slow remote tunnel where 32 extra connections would
    // worsen contention.
    let mut databases_to_drop: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (result_index, test) in tests.into_iter().enumerate() {
        // Mint a fresh service id per test so each test runs against its own
        // Mongo database namespace (the runtime derives the storage database
        // name from the service id). Reusing one id across the run let a global
        // `db find` in one test observe sibling tests' rows.
        let storage_service_id = synthetic_test_service_id(&service_id_scope);
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
            results[result_index] = Some(SkiffTestResult {
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
        let activation_identity =
            service_test_activation_identity(&storage_service_id, &test.function_name);
        ready_tests.push(ReadyServiceTest {
            result_index,
            test,
            operation_module,
            test_config,
            doubles,
            request_payload,
            expected_error,
            service_db_mongo_url,
            activation_identity,
            storage_service_id,
        });
    }

    run_ready_service_tests(
        &ready_tests,
        &service_config,
        &service_id_scope,
        &production_sources,
        &package_aliases,
        options,
        &mut results,
        &mut databases_to_drop,
    );
    let results = results
        .into_iter()
        .map(|result| result.expect("service test result should be populated before collection"))
        .collect::<Vec<_>>();

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

struct ReadyServiceTest {
    result_index: usize,
    test: TestCase,
    operation_module: String,
    test_config: serde_json::Value,
    doubles: Option<HashMap<String, Vec<TestEffectDouble>>>,
    request_payload: Option<String>,
    expected_error: Option<RuntimeExpectedError>,
    service_db_mongo_url: Option<String>,
    activation_identity: String,
    storage_service_id: String,
}

fn run_ready_service_tests(
    ready_tests: &[ReadyServiceTest],
    service_config: &ServiceConfig,
    service_id_scope: &str,
    production_sources: &[ParsedSource],
    package_aliases: &BTreeMap<String, Vec<String>>,
    options: &SkiffTestOptions,
    results: &mut [Option<SkiffTestResult>],
    databases_to_drop: &mut BTreeMap<String, Vec<String>>,
) {
    for batch in ready_service_test_owner_batches(ready_tests) {
        run_ready_service_test_batch(
            ready_tests,
            &batch,
            service_config,
            service_id_scope,
            production_sources,
            package_aliases,
            options,
            results,
            databases_to_drop,
        );
    }
}

fn run_ready_service_test_batch(
    ready_tests: &[ReadyServiceTest],
    batch: &[usize],
    service_config: &ServiceConfig,
    service_id_scope: &str,
    production_sources: &[ParsedSource],
    package_aliases: &BTreeMap<String, Vec<String>>,
    options: &SkiffTestOptions,
    results: &mut [Option<SkiffTestResult>],
    databases_to_drop: &mut BTreeMap<String, Vec<String>>,
) {
    if batch.is_empty() {
        return;
    }
    let suite_service_id = synthetic_test_service_id(service_id_scope);
    let case_inputs = batch
        .iter()
        .map(|ready_index| {
            let ready = &ready_tests[*ready_index];
            ServiceRuntimeSuiteCaseInput {
                test_source: &ready.test.source,
                test_index: ready.test.test_index,
                function_name: &ready.test.function_name,
                operation_module: &ready.operation_module,
                request_payload_param: ready.request_payload.is_some(),
                activation_identity: &ready.activation_identity,
                storage_service_id: &ready.storage_service_id,
            }
        })
        .collect::<Vec<_>>();
    let publication =
        match build_service_publication_runtime_test_suite(ServiceRuntimeSuitePublicationInput {
            service_config,
            service_id: &suite_service_id,
            production_sources,
            cases: &case_inputs,
            options,
        }) {
            Ok(publication) => publication,
            Err(error) => {
                for ready_index in batch {
                    record_service_test_failure(
                        &ready_tests[*ready_index],
                        error.to_string(),
                        results,
                    );
                }
                return;
            }
        };
    if publication.cases.len() != batch.len() {
        let message = format!(
            "service test suite returned {} case(s) for {} selected test(s)",
            publication.cases.len(),
            batch.len()
        );
        for ready_index in batch {
            record_service_test_failure(&ready_tests[*ready_index], message.clone(), results);
        }
        return;
    }
    let activation_inputs = batch
        .iter()
        .zip(publication.cases.iter())
        .map(|(ready_index, case)| {
            let ready = &ready_tests[*ready_index];
            ServiceTestSuiteActivationInput {
                case: case.clone(),
                values: ready.test_config.clone(),
                service_db_mongo_url: ready.service_db_mongo_url.clone(),
            }
        })
        .collect::<Vec<_>>();
    let prepared = match prepare_dev_synced_service_test_suite(
        &publication,
        package_aliases,
        &activation_inputs,
        options,
    ) {
        Ok(prepared) => prepared,
        Err(message) => {
            for ready_index in batch {
                record_service_test_error(
                    &ready_tests[*ready_index],
                    message.clone(),
                    options,
                    results,
                );
            }
            return;
        }
    };
    let dispatch_context = prepared.dispatch_context();
    for (ready_index, case) in batch.iter().zip(publication.cases.iter()) {
        let ready = &ready_tests[*ready_index];
        let result = execute_dev_synced_service_test_case_with_context(
            &dispatch_context,
            case,
            ready.doubles.clone(),
            ready.request_payload.as_deref(),
            ready.expected_error.as_ref(),
            options,
        );
        if !options.live {
            if let Some(mongo_url) = ready.service_db_mongo_url.as_deref() {
                databases_to_drop
                    .entry(mongo_url.to_string())
                    .or_default()
                    .push(ready.storage_service_id.clone());
            }
        }
        record_service_test_runtime_result(ready, result, options, results);
    }
}

fn ready_service_test_owner_batches(ready_tests: &[ReadyServiceTest]) -> Vec<Vec<usize>> {
    let mut batches: Vec<Vec<usize>> = Vec::new();
    for (ready_index, ready) in ready_tests.iter().enumerate() {
        if let Some(batch) = batches
            .iter_mut()
            .find(|batch| same_service_test_owner(&ready_tests[batch[0]].test, &ready.test))
        {
            batch.push(ready_index);
        } else {
            batches.push(vec![ready_index]);
        }
    }
    batches
}

fn same_service_test_owner(left: &TestCase, right: &TestCase) -> bool {
    left.source.source.file_path == right.source.source.file_path
        && left.source.source.module_path == right.source.source.module_path
}

fn record_service_test_runtime_result(
    ready: &ReadyServiceTest,
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
            record_service_test_error(ready, message, options, results);
        }
    }
}

/// Record a test error, converting live-mode missing-config failures into
/// skips. Suite-level failures (publication/reload) must go through here too:
/// a live artifact reload rejected for missing required config (e.g. a live
/// API key) means "environment cannot run this live test", not a test failure.
fn record_service_test_error(
    ready: &ReadyServiceTest,
    message: String,
    options: &SkiffTestOptions,
    results: &mut [Option<SkiffTestResult>],
) {
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
    record_service_test_failure(ready, message, results);
}

fn record_service_test_failure(
    ready: &ReadyServiceTest,
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

fn service_test_activation_identity(storage_service_id: &str, function_name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(storage_service_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(function_name.as_bytes());
    let digest = hasher.finalize();
    let hash = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("skiff-runtime-activation-v1:opaque:service-test-{hash}")
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
