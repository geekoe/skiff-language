use std::{
    io::{Read, Write},
    net::TcpStream,
    path::Path,
    time::Duration,
};

use skiff_artifact_identity::runtime_assembly_ref;

use crate::{
    canonical_fixture::CanonicalFixtureError, canonical_package::CanonicalPackageProject,
    canonical_store::CanonicalBaseAssembly, package_test_assembly::assemble_package_test_fixture,
    test_discovery::PackageTestCase, test_overlay::compile_package_test_overlay, SkiffTestOptions,
    SkiffTestResult, SkiffTestSummary,
};

pub fn run_package_cases(
    package_root: &Path,
    project: CanonicalPackageProject,
    cases: Vec<PackageTestCase>,
    source_artifact_root: &Path,
    runtime_artifact_root: &Path,
    activation_url: &str,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, CanonicalFixtureError> {
    let base = CanonicalBaseAssembly::load(source_artifact_root, options.base_assembly.as_deref())?;
    let overlay = compile_package_test_overlay(package_root, &project, &cases)?;
    let fixture = assemble_package_test_fixture(&project, overlay, base)?;
    fixture
        .records
        .publish(source_artifact_root, runtime_artifact_root)?;
    let assembly_ref = runtime_assembly_ref(&fixture.records.assembly)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let activation_id = format!(
        "package-test-{}-{}",
        std::process::id(),
        assembly_ref
            .assembly_identity
            .as_str()
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
    let activation = send_http_request(activation_url, "POST", None, &activation_body)?;
    if !(200..300).contains(&activation.status) {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "assembly activation returned HTTP {}: {}",
            activation.status, activation.body
        )));
    }
    let ingress_url = options.ingress_url.as_deref().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "canonical execution requires --ingress-url".to_string(),
        )
    })?;
    let mut results = Vec::with_capacity(fixture.entrypoints.len());
    for entrypoint in fixture.entrypoints {
        let url = format!(
            "{}{}",
            ingress_url.trim_end_matches('/'),
            entrypoint.selector.path
        );
        let response = send_http_request(
            &url,
            entrypoint.selector.method.as_deref().unwrap_or("POST"),
            Some(&entrypoint.selector.host),
            &[],
        );
        let (passed, message) = match response {
            Ok(response) if (200..300).contains(&response.status) => (true, None),
            Ok(response) => (
                false,
                Some(format!("HTTP {}: {}", response.status, response.body)),
            ),
            Err(error) => (false, Some(error.to_string())),
        };
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
    Ok(SkiffTestSummary {
        passed,
        skipped: 0,
        failed,
        results,
    })
}

struct HttpResponse {
    status: u16,
    body: String,
}

fn send_http_request(
    url: &str,
    method: &str,
    host_override: Option<&str>,
    body: &[u8],
) -> Result<HttpResponse, CanonicalFixtureError> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(format!("HTTP fixture URL must use http://: {url}"))
    })?;
    let (authority, path) = rest
        .split_once('/')
        .map(|(authority, path)| (authority, format!("/{path}")))
        .unwrap_or((rest, "/".to_string()));
    let mut stream = TcpStream::connect(authority).map_err(|source| CanonicalFixtureError::Io {
        path: url.to_string(),
        source,
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|source| CanonicalFixtureError::Io {
            path: url.to_string(),
            source,
        })?;
    let host = host_override.unwrap_or(authority);
    let header = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|()| stream.write_all(body))
        .map_err(|source| CanonicalFixtureError::Io {
            path: url.to_string(),
            source,
        })?;
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .map_err(|source| CanonicalFixtureError::Io {
            path: url.to_string(),
            source,
        })?;
    let response = String::from_utf8_lossy(&bytes);
    let (head, body) = response.split_once("\r\n\r\n").ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(format!("invalid HTTP response from {url}"))
    })?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!("invalid HTTP status from {url}"))
        })?;
    Ok(HttpResponse {
        status,
        body: body.to_string(),
    })
}
