use std::{
    path::Path,
    time::{Duration, Instant},
};

use skiff_artifact_identity::runtime_assembly_ref;

use crate::{
    canonical_fixture::CanonicalFixtureError,
    canonical_package::CanonicalPackageProject,
    canonical_store::CanonicalBaseAssembly,
    package_test_assembly::{assemble_package_test_fixture, CanonicalPackageTestEntrypoint},
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

pub fn run_package_cases(
    package_root: &Path,
    project: CanonicalPackageProject,
    cases: Vec<PackageTestCase>,
    source_artifact_root: &Path,
    runtime_artifact_root: &Path,
    activation_url: &str,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, CanonicalFixtureError> {
    let candidate_generation = options.expected_generation.checked_add(1).ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "assembly activation expected generation cannot advance".to_string(),
        )
    })?;
    let base = CanonicalBaseAssembly::load(source_artifact_root, options.base_assembly.as_deref())?;
    let overlay =
        compile_package_test_overlay(&options.platform_sources, package_root, &project, &cases)?;
    let fixture = assemble_package_test_fixture(&project, overlay, base)?;
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
    let activation_body = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": "skiff-assembly-activation-request-v1",
        "environment": options.environment.as_str(),
        "activationId": activation_id,
        "expectedGeneration": options.expected_generation,
        "assembly": assembly_ref,
    }))
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
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
    let target = readiness::target_from_receipt(
        receipt,
        &options.environment,
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

    let ingress_url = options.ingress_url.as_deref().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "canonical execution requires --ingress-url".to_string(),
        )
    })?;
    Ok(execute_entrypoints(fixture.entrypoints, ingress_url))
}

fn execute_entrypoints(
    entrypoints: Vec<CanonicalPackageTestEntrypoint>,
    ingress_url: &str,
) -> SkiffTestSummary {
    let mut results = Vec::with_capacity(entrypoints.len());
    for entrypoint in entrypoints {
        let url = format!(
            "{}{}",
            ingress_url.trim_end_matches('/'),
            entrypoint.selector.path
        );
        let (passed, message) = execute_business_request_once(|| {
            http::request_url(
                &url,
                entrypoint.selector.method.as_deref().unwrap_or("POST"),
                Some(&entrypoint.selector.host),
                &[],
                deadline_after(HTTP_TIMEOUT)?,
                MAX_HTTP_RESPONSE_BYTES,
            )
            .map(|connected| connected.response)
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
