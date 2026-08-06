use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::SystemTime,
    time::{Duration, Instant},
};

use skiff_artifact_identity::{runtime_assembly_ref, service_deployment_ref};
use skiff_artifact_model::{
    IngressProtocol, PackageArtifact, PackageArtifactRef, PackageBuildId, RuntimeAssemblyRef,
    RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef, ServiceDeployment,
};
use skiff_compiler::authoring::{
    package_actor_routing_input, project_assembly_actor_routing_from_inputs,
};
use skiff_deployment::storage::{CanonicalArtifactStore, ReleasePointer};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore;

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
    control_url: &str,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, CanonicalFixtureError> {
    inline_effects::reject_legacy_manifest(package_root)?;
    read_root_package_manifest(&options.platform_sources, package_root)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    // Non-live tests should run against whatever environment base they are
    // pointed at. The isolated release profile is derived from the base
    // config snapshot instead of a fixed "skiff-test", so the same test code
    // can run against a dev or skiff-test base without profile matching
    // errors. Live tests keep their explicit --profile contract.
    let mut effective_options = options.clone();
    if !effective_options.live {
        if let Some(snapshot_id) = effective_options.base_config_snapshot.as_deref() {
            effective_options.target_profile =
                base_snapshot_profile(source_artifact_root, snapshot_id)?;
        }
    }
    let options = &effective_options;
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
    let trusted_source =
        std::env::var("SKIFF_TEST_TRUSTED_SOURCE_ROOT").is_ok_and(|value| value == "1");
    let target_profile = options.target_profile.clone();
    let mut publish_session =
        CanonicalPublishSession::default().with_trusted_source(trusted_source);
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
            // The router resolves requests through the release pointer table;
            // publish every deployment record of this batch under
            // (profile, serviceId, version) so the batch is immediately
            // resolvable without any coordination round.
            write_batch_release_pointers(
                runtime_artifact_root,
                &batch.context.deployments,
                &target_profile,
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
        control_url,
        ingress_url,
        options,
    )
}

fn base_snapshot_profile(
    artifact_root: &Path,
    snapshot_id: &str,
) -> Result<String, CanonicalFixtureError> {
    let snapshot_ref = RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(snapshot_id).map_err(|error| {
            CanonicalFixtureError::InvalidInput(format!(
                "base config snapshot identity is invalid: {error}"
            ))
        })?,
    };
    RuntimeConfigSnapshotStore::open(artifact_root.join("runtime-config"))
        .and_then(|store| store.read(&snapshot_ref))
        .map(|snapshot| snapshot.profile().to_string())
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))
}

fn write_batch_release_pointers(
    runtime_artifact_root: &Path,
    deployments: &[ServiceDeployment],
    profile: &str,
) -> Result<(), CanonicalFixtureError> {
    let store = CanonicalArtifactStore::open(runtime_artifact_root).map_err(|error| {
        CanonicalFixtureError::InvalidInput(format!(
            "open runtime artifact store for release pointers: {error}"
        ))
    })?;
    for deployment in deployments {
        let deployment_ref = service_deployment_ref(deployment);
        let pointer = ReleasePointer::new(profile, deployment_ref.clone()).map_err(|error| {
            CanonicalFixtureError::InvalidInput(format!(
                "build release pointer for {}@{}: {error}",
                deployment_ref.service_id, deployment_ref.contract_version
            ))
        })?;
        store
            .write_release_pointer(&pointer)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    }
    Ok(())
}

fn execute_assembly_batches(
    batches: Vec<ExecutionBatch<Arc<CanonicalTestRecords>>>,
    runtime_artifact_root: &Path,
    control_url: &str,
    ingress_url: &str,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, CanonicalFixtureError> {
    // The runtime store is immutable for package records; reuse validated
    // reads across batches instead of re-running canonical JSON and SHA-256
    // admission for the same package closure on every batch.
    let mut package_reads = BTreeMap::<PackageArtifactRef, Arc<PackageArtifact>>::new();
    let mut actor_routing_inputs = BTreeMap::<PackageBuildId, _>::new();
    let trusted_source =
        std::env::var("SKIFF_TEST_TRUSTED_SOURCE_ROOT").is_ok_and(|value| value == "1");
    execute_batches_with(
        batches,
        |records| {
            // The router's A0/A3 actor method catalog is loaded from the
            // artifact root's actor routing projection record when a release
            // is resolved. Publish the exact projection for this batch's
            // deployments before readiness so cross-package actor method
            // invocations are admitted instead of silently dropped.
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
                                let artifact = if trusted_source {
                                    store
                                        .read_package_artifact_unchecked(package_ref)
                                        .map_err(|error| {
                                            CanonicalFixtureError::InvalidInput(format!(
                                                "read package artifact {}@{} for actor routing projection: {error}",
                                                package_ref.package_id, package_ref.package_version
                                            ))
                                        })?
                                } else {
                                    store
                                        .read_package_artifact(package_ref)
                                        .map_err(|error| {
                                        CanonicalFixtureError::InvalidInput(format!(
                                            "read package artifact {}@{} for actor routing projection: {error}",
                                            package_ref.package_id, package_ref.package_version
                                        ))
                                    })?
                                };
                                package_reads.insert(package_ref.clone(), artifact.clone());
                                artifact
                            }
                        };
                        packages.push((*artifact).clone());
                    }
                }
            }
            for artifact in &packages {
                if !actor_routing_inputs.contains_key(&artifact.package_build_id) {
                    let input = package_actor_routing_input(&store, artifact).map_err(|error| {
                        CanonicalFixtureError::InvalidInput(format!(
                            "actor routing package input failed for the batch: {error}"
                        ))
                    })?;
                    actor_routing_inputs.insert(artifact.package_build_id.clone(), input);
                }
            }
            let projection = project_assembly_actor_routing_from_inputs(
                &records.deployments,
                &actor_routing_inputs,
            )
            .map_err(|error| {
                CanonicalFixtureError::InvalidInput(format!(
                    "actor routing projection failed for the batch: {error}"
                ))
            })?;
            store
                .write_actor_routing_projection(&projection)
                .map_err(|error| {
                    CanonicalFixtureError::InvalidInput(format!(
                        "write actor routing projection for the batch: {error}"
                    ))
                })?;
            let assembly_ref = runtime_assembly_ref(&records.assembly)
                .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
            let build_ids = records
                .deployments
                .iter()
                .map(|deployment| deployment.deployment_artifact_identity.to_string())
                .collect();
            let target = readiness::target_for_builds(&options.target_profile, build_ids)?;
            Ok(ActivatedAssembly {
                assembly: assembly_ref,
                readiness: HttpReadiness {
                    target,
                    control_url: control_url.to_string(),
                },
            })
        },
        |active| {
            // The first health fetch resolves the control origin once and
            // pins the peer for the remaining backoff poll attempts.
            let peer: std::cell::RefCell<Option<(std::net::SocketAddr, String)>> =
                std::cell::RefCell::new(None);
            readiness::poll(
                &active.readiness.target,
                deadline_after(READINESS_TIMEOUT)?,
                |deadline| {
                    if let Some((peer_addr, authority)) = peer.borrow().as_ref() {
                        return http::request_peer(
                            *peer_addr,
                            authority.as_str(),
                            HEALTH_PATH,
                            "GET",
                            &[],
                            deadline,
                            MAX_HTTP_RESPONSE_BYTES,
                        );
                    }
                    let connected = http::request_url(
                        &format!("{}{}", active.readiness.control_url, HEALTH_PATH),
                        "GET",
                        None,
                        &[],
                        deadline,
                        MAX_HTTP_RESPONSE_BYTES,
                    )?;
                    peer.replace(Some((connected.peer_addr, connected.authority)));
                    Ok(connected.response)
                },
            )
        },
        |active, entrypoint| {
            execute_business_request_once(|| {
                execute_control_test_dispatch(
                    control_url,
                    ingress_url,
                    &active.assembly,
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
    readiness: Readiness,
}

#[derive(Debug)]
struct HttpReadiness {
    target: readiness::ReadinessTarget,
    control_url: String,
}

#[cfg(test)]
fn execute_shared_assembly_with<Readiness>(
    entrypoints: Vec<CanonicalTestServiceEntrypoint>,
    activate: impl FnOnce() -> Result<ActivatedAssembly<Readiness>, CanonicalFixtureError>,
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
        |()| {
            activate
                .take()
                .expect("one execution batch prepares exactly once")()
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
    mut activate: impl FnMut(&Context) -> Result<ActivatedAssembly<Readiness>, CanonicalFixtureError>,
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
                "test-service batch requires at least one entrypoint".to_string(),
            )
        })?;
        let active = activate(&batch.context).map_err(|source| {
            suite_execution_error(
                results.clone(),
                &first.case.module_path,
                &first.case.name,
                source,
            )
        })?;
        await_readiness(&active).map_err(|source| {
            suite_execution_error(
                results.clone(),
                &first.case.module_path,
                &first.case.name,
                source,
            )
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
    }
    Ok(summary_from_results(results))
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
    control_url: &str,
    ingress_url: &str,
    assembly: &RuntimeAssemblyRef,
    entrypoint: &CanonicalTestServiceEntrypoint,
) -> Result<http::HttpResponse, CanonicalFixtureError> {
    let body = test_dispatch_body(ingress_url, assembly, entrypoint)?;
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
