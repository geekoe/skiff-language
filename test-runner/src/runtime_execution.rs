use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::SystemTime,
    time::{Duration, Instant},
};

use skiff_artifact_identity::runtime_assembly_ref;
use skiff_artifact_model::{
    IngressProtocol, PackageArtifact, PackageArtifactRef, RuntimeAssemblyRef,
    RuntimeConfigSnapshotRef,
};
use skiff_compiler::authoring::project_assembly_actor_routing;
use skiff_deployment::storage::CanonicalArtifactStore;

use crate::{
    canonical_fixture::CanonicalFixtureError,
    canonical_package::{read_root_package_manifest, CanonicalPackageProject},
    canonical_store::{CanonicalBaseAssembly, CanonicalPublishSession, CanonicalTestRecords},
    inline_effects,
    test_discovery::TestServiceCase,
    test_service_fixture::{
        assemble_test_service_fixture_for_run_with_config, load_test_service_run_config,
        CanonicalTestServiceEntrypoint, PackageAdmissionCache,
    },
    SkiffTestOptions, SkiffTestResult, SkiffTestSummary,
};

mod batching;
mod http;
mod readiness;
mod wire;

const ACTIVATION_HTTP_TIMEOUT: Duration = Duration::from_secs(150);
const BUSINESS_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const READINESS_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;
const HEALTH_PATH: &str = "/__router/health";
static TEST_SERVICE_RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn run_package_cases(
    package_root: &Path,
    project: CanonicalPackageProject,
    cases: Vec<TestServiceCase>,
    source_artifact_root: &Path,
    runtime_artifact_root: &Path,
    activation_url: &str,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, CanonicalFixtureError> {
    inline_effects::reject_legacy_manifest(package_root)?;
    read_root_package_manifest(&options.platform_sources, package_root)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let base = CanonicalBaseAssembly::load(
        source_artifact_root,
        options.base_assembly.as_deref(),
        options.base_config_snapshot.as_deref(),
        &options.target_profile,
    )?;
    let run_scope = test_service_run_scope()?;
    let ingress_url = options.ingress_url.as_deref().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "canonical execution requires --ingress-url".to_string(),
        )
    })?;
    let ingress_url = ingress_url.strip_suffix('/').unwrap_or(ingress_url);
    let run_config = load_test_service_run_config(&project, Some(ingress_url))?;
    let case_batches = batching::partition_cases(cases, options.live);
    let mut publish_session = CanonicalPublishSession::default();
    let mut package_admissions = PackageAdmissionCache::default();
    let execution_batches = prepare_execution_batches_with(
        case_batches,
        |batch_index, cases| {
            let batch_scope = if options.live {
                run_scope.clone()
            } else {
                batching::batch_execution_scope(&run_scope, batch_index)
            };
            let fixture = assemble_test_service_fixture_for_run_with_config(
                &project,
                &cases,
                base.clone(),
                &batch_scope,
                &run_config,
                &options.target_profile,
                &mut package_admissions,
            )?;
            Ok(ExecutionBatch {
                context: fixture.records.clone(),
                entrypoints: fixture
                    .cases
                    .into_iter()
                    .map(|case| case.entrypoint)
                    .collect(),
            })
        },
        |batch| {
            batch.context.publish_with_session(
                source_artifact_root,
                runtime_artifact_root,
                &mut publish_session,
            )?;
            Ok(())
        },
    )?;
    if publish_session.owned_package_publication_count() != 1 {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "test-service batches must share one owned Package publication, observed {}",
            publish_session.owned_package_publication_count()
        )));
    }

    execute_assembly_batches(
        execution_batches,
        runtime_artifact_root,
        activation_url,
        ingress_url,
        options,
    )
}

fn execute_assembly_batches(
    batches: Vec<ExecutionBatch<Arc<CanonicalTestRecords>>>,
    runtime_artifact_root: &Path,
    activation_url: &str,
    ingress_url: &str,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, CanonicalFixtureError> {
    // The runtime store is immutable for package records; reuse validated
    // reads across activation batches instead of re-running canonical JSON and
    // SHA-256 admission for the same package closure on every batch.
    let mut package_reads = BTreeMap::<PackageArtifactRef, Arc<PackageArtifact>>::new();
    execute_batches_with(
        batches,
        options.expected_generation,
        |records, expected_generation, candidate_generation| {
            // The router's A0/A3 actor method catalog is loaded from the
            // artifact root's actor routing projection record at activation
            // time. Publish the exact projection for this batch's deployments
            // before activating so cross-package actor method invocations are
            // admitted instead of silently dropped.
            let store = CanonicalArtifactStore::open(runtime_artifact_root).map_err(|error| {
                CanonicalFixtureError::InvalidInput(format!(
                    "open runtime artifact store for actor routing projection: {error}"
                ))
            })?;
            let mut packages = records
                .packages
                .iter()
                .map(|published| published.artifact.clone())
                .collect::<Vec<_>>();
            let mut loaded_builds = packages
                .iter()
                .map(|artifact| artifact.package_build_id.clone())
                .collect::<std::collections::BTreeSet<_>>();
            for deployment in &records.deployments {
                let refs = deployment
                    .package_bindings
                    .iter()
                    .map(|binding| &binding.package)
                    .chain(std::iter::once(&deployment.implementation));
                for package_ref in refs {
                    if loaded_builds.insert(package_ref.package_build_id.clone()) {
                        let artifact = match package_reads.get(package_ref) {
                            Some(artifact) => artifact.clone(),
                            None => {
                                let artifact = store
                                    .read_package_artifact(package_ref)
                                    .map_err(|error| {
                                        CanonicalFixtureError::InvalidInput(format!(
                                            "read package artifact {}@{} for actor routing projection: {error}",
                                            package_ref.package_id, package_ref.package_version
                                        ))
                                    })?;
                                package_reads.insert(package_ref.clone(), artifact.clone());
                                artifact
                            }
                        };
                        packages.push((*artifact).clone());
                    }
                }
            }
            let projection =
                project_assembly_actor_routing(&store, &records.deployments, &packages).map_err(
                    |error| {
                        CanonicalFixtureError::InvalidInput(format!(
                            "actor routing projection failed for the activation batch: {error}"
                        ))
                    },
                )?;
            store
                .write_actor_routing_projection(&projection)
                .map_err(|error| {
                    CanonicalFixtureError::InvalidInput(format!(
                        "write actor routing projection for the activation batch: {error}"
                    ))
                })?;
            let assembly_ref = runtime_assembly_ref(&records.assembly)
                .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
            let config_snapshot_ref = records.config_snapshot.snapshot_ref().clone();
            let requested_identity = assembly_ref.assembly_identity.as_str().to_string();
            let activation_id = format!(
                "test-service-{}-{candidate_generation}-{}",
                std::process::id(),
                requested_identity.rsplit(':').next().unwrap_or("assembly")
            );
            let body = activation_request_body(
                &options.target_profile,
                &activation_id,
                expected_generation,
                &assembly_ref,
                &config_snapshot_ref,
            )?;
            let activation = http::request_url(
                activation_url,
                "POST",
                None,
                &body,
                deadline_after(ACTIVATION_HTTP_TIMEOUT)?,
                MAX_HTTP_RESPONSE_BYTES,
            )?;
            if !(200..300).contains(&activation.response.status) {
                let error = wire::decode_control_error_response(&activation.response.body)?;
                return Err(CanonicalFixtureError::RemoteControl {
                    status: activation.response.status,
                    code: error.code,
                    message: error.message,
                });
            }
            let receipt = wire::decode_activation_receipt(&activation.response.body)?;
            let target = readiness::target_from_receipt(
                receipt.clone(),
                &options.target_profile,
                candidate_generation,
                &requested_identity,
                &config_snapshot_ref,
            )?;
            Ok(ActivatedAssembly {
                assembly: receipt.assembly,
                generation: receipt.generation,
                readiness: HttpReadiness {
                    target,
                    peer_addr: activation.peer_addr,
                    authority: activation.authority,
                },
            })
        },
        |active| {
            readiness::poll(
                &active.readiness.target,
                deadline_after(READINESS_TIMEOUT)?,
                |deadline| {
                    http::request_peer(
                        active.readiness.peer_addr,
                        &active.readiness.authority,
                        HEALTH_PATH,
                        "GET",
                        &[],
                        deadline,
                        MAX_HTTP_RESPONSE_BYTES,
                    )
                },
            )
        },
        |active, entrypoint| {
            execute_business_request_once(|| {
                execute_control_test_dispatch(
                    activation_url,
                    ingress_url,
                    &active.assembly,
                    active.generation,
                    entrypoint,
                )
            })
        },
    )
}

#[derive(Debug)]
struct ExecutionBatch<Context> {
    context: Context,
    entrypoints: Vec<CanonicalTestServiceEntrypoint>,
}

fn prepare_execution_batches_with<Input, Context>(
    inputs: Vec<Input>,
    mut assemble: impl FnMut(usize, Input) -> Result<ExecutionBatch<Context>, CanonicalFixtureError>,
    mut publish: impl FnMut(&ExecutionBatch<Context>) -> Result<(), CanonicalFixtureError>,
) -> Result<Vec<ExecutionBatch<Context>>, CanonicalFixtureError> {
    let batches = inputs
        .into_iter()
        .enumerate()
        .map(|(batch_index, input)| assemble(batch_index, input))
        .collect::<Result<Vec<_>, _>>()?;
    for batch in &batches {
        publish(batch)?;
    }
    Ok(batches)
}

#[derive(Debug)]
struct ActivatedAssembly<Readiness> {
    assembly: RuntimeAssemblyRef,
    generation: u64,
    readiness: Readiness,
}

#[derive(Debug)]
struct HttpReadiness {
    target: readiness::ReadinessTarget,
    peer_addr: SocketAddr,
    authority: String,
}

#[cfg(test)]
fn execute_shared_assembly_with<Readiness>(
    entrypoints: Vec<CanonicalTestServiceEntrypoint>,
    expected_generation: u64,
    activate: impl FnOnce(u64, u64) -> Result<ActivatedAssembly<Readiness>, CanonicalFixtureError>,
    await_readiness: impl FnOnce(&ActivatedAssembly<Readiness>) -> Result<(), CanonicalFixtureError>,
    mut dispatch: impl FnMut(
        &ActivatedAssembly<Readiness>,
        &CanonicalTestServiceEntrypoint,
    ) -> Result<DispatchOutcome, CanonicalFixtureError>,
) -> Result<SkiffTestSummary, CanonicalFixtureError> {
    let mut activate = Some(activate);
    let mut await_readiness = Some(await_readiness);
    execute_batches_with(
        vec![ExecutionBatch {
            context: (),
            entrypoints,
        }],
        expected_generation,
        |(), expected_generation, candidate_generation| {
            activate
                .take()
                .expect("one execution batch activates exactly once")(
                expected_generation,
                candidate_generation,
            )
        },
        |active| {
            await_readiness
                .take()
                .expect("one execution batch checks readiness exactly once")(active)
        },
        |active, entrypoint| dispatch(active, entrypoint),
    )
}

fn execute_batches_with<Context, Readiness>(
    batches: Vec<ExecutionBatch<Context>>,
    mut expected_generation: u64,
    mut activate: impl FnMut(
        &Context,
        u64,
        u64,
    ) -> Result<ActivatedAssembly<Readiness>, CanonicalFixtureError>,
    mut await_readiness: impl FnMut(&ActivatedAssembly<Readiness>) -> Result<(), CanonicalFixtureError>,
    mut dispatch: impl FnMut(
        &ActivatedAssembly<Readiness>,
        &CanonicalTestServiceEntrypoint,
    ) -> Result<DispatchOutcome, CanonicalFixtureError>,
) -> Result<SkiffTestSummary, CanonicalFixtureError> {
    let mut results = Vec::new();
    for batch in batches {
        let first = batch.entrypoints.first().ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(
                "test-service activation batch requires at least one entrypoint".to_string(),
            )
        })?;
        let first_case = (first.case.module_path.clone(), first.case.name.clone());
        let candidate_generation = expected_generation.checked_add(1).ok_or_else(|| {
            suite_execution_error(
                results.clone(),
                &first_case.0,
                &first_case.1,
                CanonicalFixtureError::InvalidInput(
                    "assembly activation expected generation cannot advance".to_string(),
                ),
            )
        })?;
        skiff_artifact_model::validate_transition_generations(
            expected_generation,
            candidate_generation,
        )
        .map_err(|message| {
            suite_execution_error(
                results.clone(),
                &first_case.0,
                &first_case.1,
                CanonicalFixtureError::InvalidInput(message),
            )
        })?;
        let active = activate(&batch.context, expected_generation, candidate_generation).map_err(
            |source| suite_execution_error(results.clone(), &first_case.0, &first_case.1, source),
        )?;
        if active.generation != candidate_generation {
            return Err(suite_execution_error(
                results,
                &first_case.0,
                &first_case.1,
                CanonicalFixtureError::InvalidInput(format!(
                    "assembly activation returned generation {}, expected {candidate_generation}",
                    active.generation
                )),
            ));
        }
        await_readiness(&active).map_err(|source| {
            suite_execution_error(results.clone(), &first_case.0, &first_case.1, source)
        })?;
        for entrypoint in batch.entrypoints {
            let outcome = dispatch(&active, &entrypoint).map_err(|source| {
                suite_execution_error(
                    results.clone(),
                    &entrypoint.case.module_path,
                    &entrypoint.case.name,
                    source,
                )
            })?;
            let (passed, message) = match outcome {
                DispatchOutcome::Passed => (true, None),
                DispatchOutcome::Failed(message) => (false, Some(message)),
            };
            results.push(SkiffTestResult {
                module_path: entrypoint.case.module_path,
                name: entrypoint.case.name,
                passed,
                skipped: false,
                message,
            });
        }
        expected_generation = active.generation;
    }
    Ok(summary_from_results(results))
}

fn activation_request_body(
    target_profile: &str,
    activation_id: &str,
    expected_generation: u64,
    assembly: &RuntimeAssemblyRef,
    config_snapshot: &RuntimeConfigSnapshotRef,
) -> Result<Vec<u8>, CanonicalFixtureError> {
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": "skiff-assembly-activation-request-v3",
        "profile": target_profile,
        "activationId": activation_id,
        "expectedGeneration": expected_generation,
        "assembly": assembly,
        "configSnapshot": config_snapshot,
    }))
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))
}

fn test_service_run_scope() -> Result<String, CanonicalFixtureError> {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| {
            CanonicalFixtureError::InvalidInput(format!(
                "test-service clock is before the Unix epoch: {error}"
            ))
        })?
        .as_nanos();
    let sequence = TEST_SERVICE_RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!("{}-{timestamp}-{sequence}", std::process::id()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DispatchOutcome {
    Passed,
    Failed(String),
}

#[cfg(test)]
fn execute_entrypoints_with(
    entrypoints: Vec<CanonicalTestServiceEntrypoint>,
    mut dispatch: impl FnMut(
        &CanonicalTestServiceEntrypoint,
    ) -> Result<DispatchOutcome, CanonicalFixtureError>,
) -> Result<SkiffTestSummary, CanonicalFixtureError> {
    let mut results = Vec::with_capacity(entrypoints.len());
    for entrypoint in entrypoints {
        let outcome = dispatch(&entrypoint).map_err(|source| {
            suite_execution_error(
                results.clone(),
                &entrypoint.case.module_path,
                &entrypoint.case.name,
                source,
            )
        })?;
        let (passed, message) = match outcome {
            DispatchOutcome::Passed => (true, None),
            DispatchOutcome::Failed(message) => (false, Some(message)),
        };
        results.push(SkiffTestResult {
            module_path: entrypoint.case.module_path,
            name: entrypoint.case.name,
            passed,
            skipped: false,
            message,
        });
    }
    Ok(summary_from_results(results))
}

fn summary_from_results(results: Vec<SkiffTestResult>) -> SkiffTestSummary {
    let passed = results.iter().filter(|result| result.passed).count();
    let failed = results.len() - passed;
    SkiffTestSummary {
        passed,
        skipped: 0,
        failed,
        results,
    }
}

fn suite_execution_error(
    completed: Vec<SkiffTestResult>,
    module_path: &str,
    name: &str,
    source: CanonicalFixtureError,
) -> CanonicalFixtureError {
    CanonicalFixtureError::SuiteExecution {
        completed,
        module_path: module_path.to_string(),
        name: name.to_string(),
        source: Box::new(source),
    }
}

fn execute_control_test_dispatch(
    activation_url: &str,
    ingress_url: &str,
    assembly: &RuntimeAssemblyRef,
    generation: u64,
    entrypoint: &CanonicalTestServiceEntrypoint,
) -> Result<http::HttpResponse, CanonicalFixtureError> {
    let control_url = activation_url
        .strip_suffix("/__skiff/activate-assembly")
        .ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(
                "activation URL is not canonical for test dispatch".to_string(),
            )
        })?;
    let body = test_dispatch_body(ingress_url, assembly, generation, entrypoint)?;
    let connected = http::request_url(
        &format!("{control_url}/__skiff/test-dispatch"),
        "POST",
        None,
        &body,
        deadline_after(BUSINESS_HTTP_TIMEOUT)?,
        MAX_HTTP_RESPONSE_BYTES,
    )?;
    Ok(connected.response)
}

fn test_dispatch_body(
    ingress_url: &str,
    assembly: &RuntimeAssemblyRef,
    generation: u64,
    entrypoint: &CanonicalTestServiceEntrypoint,
) -> Result<Vec<u8>, CanonicalFixtureError> {
    if entrypoint.selector.protocol != IngressProtocol::Http {
        return Err(CanonicalFixtureError::InvalidInput(
            "test-service case selector must use HTTP".to_string(),
        ));
    }
    let method = entrypoint.selector.method.as_deref().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "test-service case selector must have an exact HTTP method".to_string(),
        )
    })?;
    let url = format!("{ingress_url}{}", entrypoint.selector.path);
    serde_json::to_vec(&serde_json::json!({
        "kind": "test",
        "routing": {
            "kind": "runtimeAssembly",
            "assemblyIdentity": assembly.assembly_identity,
            "assemblyGeneration": generation,
            "deployment": entrypoint.deployment,
            "gatewayEntryIdentity": entrypoint.gateway_entry_identity,
            "ingress": entrypoint.selector,
        },
        "mode": entrypoint.mode,
        "httpRequest": {
            "method": method,
            "url": url,
            "path": entrypoint.selector.path,
            "query": [],
            "headers": [{
                "name": "content-type",
                "value": "application/json",
            }],
        },
        "payloadBase64": "bnVsbA==",
        "timeoutMs": 30_000,
    }))
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))
}

fn execute_business_request_once(
    send: impl FnOnce() -> Result<http::HttpResponse, CanonicalFixtureError>,
) -> Result<DispatchOutcome, CanonicalFixtureError> {
    let response = send()?;
    if (200..300).contains(&response.status) {
        return wire::decode_test_dispatch_response(&response.body).map(|outcome| match outcome {
            wire::TestDispatchOutcome::Passed => DispatchOutcome::Passed,
            wire::TestDispatchOutcome::Failed(message) => DispatchOutcome::Failed(message),
        });
    }
    let error = wire::decode_control_error_response(&response.body)?;
    Err(CanonicalFixtureError::RemoteControl {
        status: response.status,
        code: error.code,
        message: error.message,
    })
}

fn deadline_after(duration: Duration) -> Result<Instant, CanonicalFixtureError> {
    deadline_after_from(Instant::now(), duration)
}

fn deadline_after_from(
    start: Instant,
    duration: Duration,
) -> Result<Instant, CanonicalFixtureError> {
    start
        .checked_add(duration)
        .ok_or_else(|| CanonicalFixtureError::InvalidInput("HTTP deadline overflow".to_string()))
}

#[cfg(test)]
#[path = "runtime_execution/tests/support.rs"]
mod test_support;

#[cfg(test)]
#[path = "runtime_execution/tests/orchestration.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_execution/tests/batching.rs"]
mod batching_tests;
