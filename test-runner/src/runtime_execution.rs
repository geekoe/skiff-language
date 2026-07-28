use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
    time::{Duration, Instant},
};

use skiff_artifact_identity::runtime_assembly_ref;
use skiff_artifact_model::{IngressProtocol, RuntimeAssemblyRef};

use crate::{
    canonical_fixture::CanonicalFixtureError,
    canonical_package::{read_root_package_manifest, CanonicalPackageProject},
    canonical_store::CanonicalBaseAssembly,
    inline_effects,
    package_test_assembly::{
        assemble_package_test_fixture_for_run, CanonicalPackageTestEntrypoint,
    },
    test_discovery::PackageTestCase,
    test_overlay::compile_package_test_overlay,
    SkiffTestOptions, SkiffTestResult, SkiffTestSummary,
};

mod http;
mod readiness;
mod wire;

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const READINESS_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;
const HEALTH_PATH: &str = "/__router/health";
const PACKAGE_TEST_REQUEST_AUTHORITY: &str = "localhost";
static PACKAGE_TEST_RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn run_package_cases(
    package_root: &Path,
    project: CanonicalPackageProject,
    cases: Vec<PackageTestCase>,
    source_artifact_root: &Path,
    runtime_artifact_root: &Path,
    activation_url: &str,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, CanonicalFixtureError> {
    inline_effects::reject_legacy_manifest(package_root)?;
    read_root_package_manifest(&options.platform_sources, package_root)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let candidate_generation = options.expected_generation.checked_add(1).ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "assembly activation expected generation cannot advance".to_string(),
        )
    })?;
    let base = CanonicalBaseAssembly::load(source_artifact_root, options.base_assembly.as_deref())?;
    let overlay = compile_package_test_overlay(
        &options.platform_sources,
        package_root,
        source_artifact_root,
        &project,
        &cases,
    )?;
    let run_scope = package_test_run_scope()?;
    let fixture = assemble_package_test_fixture_for_run(&project, overlay, base, &run_scope)?;
    fixture
        .records
        .publish(source_artifact_root, runtime_artifact_root)?;
    let assembly_ref = runtime_assembly_ref(&fixture.records.assembly)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let requested_assembly_identity = assembly_ref.assembly_identity.as_str().to_string();
    let activation_id = format!(
        "package-test-{}-{}",
        std::process::id(),
        requested_assembly_identity
            .rsplit(':')
            .next()
            .unwrap_or("assembly")
    );
    let activation_body = activation_request_body(
        &options.target_environment,
        &activation_id,
        options.expected_generation,
        &assembly_ref,
    )?;
    let activation = http::request_url(
        activation_url,
        "POST",
        None,
        &activation_body,
        deadline_after(HTTP_TIMEOUT)?,
        MAX_HTTP_RESPONSE_BYTES,
    )?;
    if !(200..300).contains(&activation.response.status) {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "assembly activation returned HTTP {}: {}",
            activation.response.status, activation.response.body
        )));
    }
    let receipt = wire::decode_activation_receipt(&activation.response.body)?;
    let active_assembly = receipt.assembly.clone();
    let active_generation = receipt.generation;
    let target = readiness::target_from_receipt(
        receipt,
        &options.target_environment,
        candidate_generation,
        &requested_assembly_identity,
    )?;
    let readiness_deadline = deadline_after(READINESS_TIMEOUT)?;
    readiness::poll(&target, readiness_deadline, |deadline| {
        http::request_peer(
            activation.peer_addr,
            &activation.authority,
            HEALTH_PATH,
            "GET",
            &[],
            deadline,
            MAX_HTTP_RESPONSE_BYTES,
        )
    })?;

    options.ingress_url.as_deref().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "canonical execution requires --ingress-url".to_string(),
        )
    })?;
    Ok(execute_entrypoints(
        fixture.entrypoints,
        activation_url,
        &active_assembly,
        active_generation,
    ))
}

fn activation_request_body(
    target_environment: &str,
    activation_id: &str,
    expected_generation: u64,
    assembly: &RuntimeAssemblyRef,
) -> Result<Vec<u8>, CanonicalFixtureError> {
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": "skiff-assembly-activation-request-v1",
        "environment": target_environment,
        "activationId": activation_id,
        "expectedGeneration": expected_generation,
        "assembly": assembly,
    }))
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))
}

fn package_test_run_scope() -> Result<String, CanonicalFixtureError> {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| {
            CanonicalFixtureError::InvalidInput(format!(
                "package-test clock is before the Unix epoch: {error}"
            ))
        })?
        .as_nanos();
    let sequence = PACKAGE_TEST_RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!("{}-{timestamp}-{sequence}", std::process::id()))
}

fn execute_entrypoints(
    entrypoints: Vec<CanonicalPackageTestEntrypoint>,
    activation_url: &str,
    assembly: &RuntimeAssemblyRef,
    generation: u64,
) -> SkiffTestSummary {
    let mut results = Vec::with_capacity(entrypoints.len());
    for entrypoint in entrypoints {
        let (passed, message) = execute_business_request_once(|| {
            execute_control_test_dispatch(activation_url, assembly, generation, &entrypoint)
        });
        results.push(SkiffTestResult {
            module_path: entrypoint.case.module_path,
            name: entrypoint.case.name,
            passed,
            skipped: false,
            message,
        });
    }
    let passed = results.iter().filter(|result| result.passed).count();
    let failed = results.len() - passed;
    SkiffTestSummary {
        passed,
        skipped: 0,
        failed,
        results,
    }
}

fn execute_control_test_dispatch(
    activation_url: &str,
    assembly: &RuntimeAssemblyRef,
    generation: u64,
    entrypoint: &CanonicalPackageTestEntrypoint,
) -> Result<http::HttpResponse, CanonicalFixtureError> {
    let control_url = activation_url
        .strip_suffix("/__skiff/activate-assembly")
        .ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(
                "activation URL is not canonical for test dispatch".to_string(),
            )
        })?;
    let body = package_test_dispatch_body(assembly, generation, entrypoint)?;
    let connected = http::request_url(
        &format!("{control_url}/__skiff/test-dispatch"),
        "POST",
        None,
        &body,
        deadline_after(HTTP_TIMEOUT)?,
        MAX_HTTP_RESPONSE_BYTES,
    )?;
    if (200..300).contains(&connected.response.status) {
        wire::decode_package_test_dispatch_response(&connected.response.body)?;
    }
    Ok(connected.response)
}

fn package_test_dispatch_body(
    assembly: &RuntimeAssemblyRef,
    generation: u64,
    entrypoint: &CanonicalPackageTestEntrypoint,
) -> Result<Vec<u8>, CanonicalFixtureError> {
    if entrypoint.selector.protocol != IngressProtocol::Http {
        return Err(CanonicalFixtureError::InvalidInput(
            "package-test gateway selector must use HTTP".to_string(),
        ));
    }
    let method = entrypoint.selector.method.as_deref().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "package-test gateway selector must have an exact HTTP method".to_string(),
        )
    })?;
    let url = format!(
        "http://{PACKAGE_TEST_REQUEST_AUTHORITY}{}",
        entrypoint.selector.path
    );
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
) -> (bool, Option<String>) {
    match send() {
        Ok(response) if (200..300).contains(&response.status) => (true, None),
        Ok(response) => (
            false,
            Some(format!("HTTP {}: {}", response.status, response.body)),
        ),
        Err(error) => (false, Some(error.to_string())),
    }
}

fn deadline_after(duration: Duration) -> Result<Instant, CanonicalFixtureError> {
    Instant::now()
        .checked_add(duration)
        .ok_or_else(|| CanonicalFixtureError::InvalidInput("HTTP deadline overflow".to_string()))
}

#[cfg(test)]
#[path = "runtime_execution/tests/support.rs"]
mod test_support;

#[cfg(test)]
#[path = "runtime_execution/tests/orchestration.rs"]
mod tests;
