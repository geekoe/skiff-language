use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    time::Duration,
};

use skiff_artifact_identity::{
    package_artifact_ref, runtime_assembly_ref, service_contract_ref, service_deployment_ref,
};
use skiff_artifact_model::{
    ActivationPolicy, BoundaryCallableProjection, DeploymentDiagnosticText,
    DeploymentIngressBinding, DeploymentPolicy, DeploymentRevision, IngressProtocol,
    IngressSelector, PackageBinding, PackageOperationTarget, PackageRequirementKey, ResourcePolicy,
    RuntimeAssembly, ServiceContract, ServiceDeployment, ServiceDeploymentInput,
    ServiceDeploymentOperationInput, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_compiler::{
    compile_contract, PublishedPackageArtifact, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};
use skiff_deployment::{
    assembly::resolve_runtime_assembly,
    projection::project_service_deployment,
    storage::{CanonicalArtifactStore, EcosystemStorageError},
};
use skiff_syntax::{ast::SourceFile, error::CompileError, parser::parse_source};
use thiserror::Error;

use crate::{
    canonical_package::CanonicalPackageProject,
    test_overlay::{
        compile_package_test_overlay, PackageTestOverlayError, PublishedPackageTestOverlay,
    },
    SkiffTestOptions, SkiffTestResult, SkiffTestSummary,
};

#[derive(Debug, Clone)]
pub struct PackageTestCase {
    pub relative_path: PathBuf,
    pub module_path: String,
    pub name: String,
    pub function_name: String,
    pub test_index: usize,
    pub source_text: String,
    pub source_ast: SourceFile,
}

#[derive(Debug, Error)]
pub enum CanonicalFixtureError {
    #[error("failed to access {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: CompileError,
    },
    #[error(transparent)]
    Storage(#[from] EcosystemStorageError),
    #[error(transparent)]
    Overlay(#[from] PackageTestOverlayError),
    #[error("invalid canonical fixture: {0}")]
    InvalidInput(String),
}

/// Complete test-owned canonical record set.
///
/// Package code, protocol, deployment values and resolved assembly remain four
/// independent records; this struct is only a Rust fixture convenience and is
/// never serialized as another domain object.
#[derive(Debug, Clone)]
pub struct CanonicalTestRecords {
    pub packages: Vec<PublishedPackageArtifact>,
    pub contracts: Vec<ServiceContract>,
    pub deployments: Vec<ServiceDeployment>,
    pub assembly: RuntimeAssembly,
}

#[derive(Debug, Clone)]
pub struct CanonicalPackageTestEntrypoint {
    pub case: PackageTestCase,
    pub selector: IngressSelector,
    pub deployment: skiff_artifact_model::ServiceDeploymentRef,
    pub contract: skiff_artifact_model::ServiceContractRef,
    pub operation: skiff_artifact_model::ContractOperationId,
}

#[derive(Debug, Clone)]
pub struct CanonicalPackageTestFixture {
    pub production: skiff_artifact_model::PackageArtifactRef,
    pub overlay: skiff_artifact_model::PackageArtifactRef,
    pub records: CanonicalTestRecords,
    pub entrypoints: Vec<CanonicalPackageTestEntrypoint>,
}

impl CanonicalTestRecords {
    pub fn publish(&self, artifact_root: &Path) -> Result<Vec<PathBuf>, CanonicalFixtureError> {
        let store = CanonicalArtifactStore::create(artifact_root)?;
        let mut written = Vec::new();
        for package in &self.packages {
            let package = storage_canonical_package(package);
            let reference = package_artifact_ref(&package.artifact)
                .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
            for file in &package.file_ir_units {
                let file_ref = package
                    .artifact
                    .files
                    .iter()
                    .find(|candidate| candidate.file_ir_identity == file.unit.file_ir_identity)
                    .ok_or_else(|| {
                        CanonicalFixtureError::InvalidInput(format!(
                            "package {} emitted File IR {} outside its canonical refs",
                            package.artifact.package_build_id, file.unit.file_ir_identity
                        ))
                    })?;
                written.push(store.write_file_ir(&reference, file_ref, &file.unit)?);
            }
            for resource_ref in &package.artifact.static_resources {
                let blob = package
                    .resource_blobs
                    .iter()
                    .find(|candidate| {
                        candidate.logical_path == resource_ref.path
                            && candidate.sha256 == resource_ref.sha256
                            && candidate.byte_len == resource_ref.byte_len
                    })
                    .ok_or_else(|| {
                        CanonicalFixtureError::InvalidInput(format!(
                            "package {} resource {} has no exact emitted blob",
                            package.artifact.package_build_id, resource_ref.path
                        ))
                    })?;
                written.push(store.write_static_resource(&reference, resource_ref, &blob.bytes)?);
            }
            written.push(store.write_package_artifact(&package.artifact)?);
        }
        for contract in &self.contracts {
            written.push(store.write_service_contract(contract)?);
        }
        for deployment in &self.deployments {
            written.push(store.write_service_deployment(deployment)?);
        }
        written.push(store.write_runtime_assembly(&self.assembly)?);
        Ok(written)
    }

    pub fn assert_production_package_unchanged(
        before: &skiff_artifact_model::PackageArtifactRef,
        after: &PublishedPackageArtifact,
    ) -> Result<(), CanonicalFixtureError> {
        let after = package_artifact_ref(&after.artifact)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        if before != &after {
            return Err(CanonicalFixtureError::InvalidInput(
                "test overlay rewrote production PackageArtifact identity".to_string(),
            ));
        }
        Ok(())
    }
}

fn storage_canonical_package(package: &PublishedPackageArtifact) -> PublishedPackageArtifact {
    let mut package = package.clone();
    for file in &mut package.artifact.files {
        file.artifact_path = None;
    }
    for resource in &mut package.artifact.static_resources {
        resource.artifact_path = None;
    }
    for link in package.artifact.callable_links.values_mut() {
        link.target.file_ref.artifact_path = None;
    }
    for export in package.artifact.implementation_links.types.values_mut() {
        export.file.artifact_path = None;
    }
    for export in package.artifact.implementation_links.constants.values_mut() {
        export.file.artifact_path = None;
    }
    for export in package.artifact.implementation_links.functions.values_mut() {
        export.file.artifact_path = None;
    }
    for export in package
        .artifact
        .implementation_links
        .impl_methods
        .values_mut()
    {
        export.file.artifact_path = None;
    }
    for target in package
        .artifact
        .implementation_links
        .operation_targets
        .values_mut()
    {
        match target {
            PackageOperationTarget::LocalExecutable { target, .. } => {
                target.file_ref.artifact_path = None;
            }
            PackageOperationTarget::LocalConstReceiverExecutable { target, .. } => {
                target.receiver.file_ref.artifact_path = None;
                target.executable_target.file_ref.artifact_path = None;
            }
        }
    }
    package
}

pub fn discover_package_test_cases(
    input: &Path,
    package_root: &Path,
    input_is_file: bool,
) -> Result<Vec<PackageTestCase>, CanonicalFixtureError> {
    let mut files = Vec::new();
    if input_is_file {
        if is_test_file(input) {
            files.push(input.to_path_buf());
        }
    } else {
        collect_test_files(input, &mut files)?;
    }
    files.sort();
    let mut cases = Vec::new();
    for file in files {
        let source_text =
            fs::read_to_string(&file).map_err(|source| CanonicalFixtureError::Io {
                path: file.display().to_string(),
                source,
            })?;
        let source_ast =
            parse_source(&source_text).map_err(|source| CanonicalFixtureError::Parse {
                path: file.display().to_string(),
                source,
            })?;
        let relative_path = file
            .strip_prefix(package_root)
            .unwrap_or(&file)
            .to_path_buf();
        let module_path = test_module_path(&relative_path)?;
        let default_run = source_ast.test_default_run.unwrap_or(true);
        for (test_index, test) in source_ast.tests.iter().enumerate() {
            if input_is_file || default_run {
                cases.push(PackageTestCase {
                    relative_path: relative_path.clone(),
                    module_path: module_path.clone(),
                    name: test.name.clone(),
                    function_name: format!("skiffTestCase{}", cases.len()),
                    test_index,
                    source_text: source_text.clone(),
                    source_ast: source_ast.clone(),
                });
            }
        }
    }
    Ok(cases)
}

pub fn run_package_cases(
    package_root: &Path,
    project: CanonicalPackageProject,
    cases: Vec<PackageTestCase>,
    artifact_root: &Path,
    activation_url: &str,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, CanonicalFixtureError> {
    let overlay =
        compile_package_test_overlay(package_root, &project, &cases, &options.package_dirs)?;
    let fixture = assemble_package_test_fixture(&project, overlay)?;
    fixture.records.publish(artifact_root)?;
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

pub fn assemble_package_test_fixture(
    project: &CanonicalPackageProject,
    overlay: PublishedPackageTestOverlay,
) -> Result<CanonicalPackageTestFixture, CanonicalFixtureError> {
    if !overlay.overlay.artifact.service_requirements.is_empty() {
        return Err(CanonicalFixtureError::InvalidInput(
            "package test service requirements need explicit canonical provider deployments"
                .to_string(),
        ));
    }
    let contract = compile_package_test_contract(&overlay)?;
    let contract_ref = service_contract_ref(&contract)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let overlay_ref = package_artifact_ref(&overlay.overlay.artifact)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let (operation_bindings, ingress) = package_test_operation_inputs(&contract, &overlay)?;
    let package_bindings = package_test_package_bindings(project, &overlay, &overlay_ref)?;
    let deployment_packages = std::iter::once(overlay.overlay.artifact.clone())
        .chain(
            overlay
                .overlay
                .artifact
                .package_requirements
                .iter()
                .filter_map(|requirement| {
                    project
                        .artifact(&requirement.package_id, &requirement.exact_version)
                        .map(|package| package.artifact.clone())
                }),
        )
        .collect::<Vec<_>>();
    let deployment = project_service_deployment(
        package_test_deployment_input(
            &overlay,
            contract_ref.clone(),
            overlay_ref.clone(),
            operation_bindings,
            package_bindings,
            ingress.clone(),
        ),
        &contract,
        &deployment_packages,
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let deployment_ref = service_deployment_ref(&deployment);
    let mut package_records = project.packages().cloned().collect::<Vec<_>>();
    package_records.push(overlay.overlay.clone());
    let package_artifacts = package_records
        .iter()
        .map(|package| package.artifact.clone())
        .collect::<Vec<_>>();
    let assembly = resolve_runtime_assembly(
        std::slice::from_ref(&deployment_ref),
        std::slice::from_ref(&deployment),
        std::slice::from_ref(&contract),
        &package_artifacts,
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let entrypoints = overlay
        .bindings
        .into_iter()
        .zip(ingress)
        .map(|(binding, ingress)| CanonicalPackageTestEntrypoint {
            operation: ingress.contract_operation_id,
            selector: ingress.selector,
            case: binding.case,
            deployment: deployment_ref.clone(),
            contract: contract_ref.clone(),
        })
        .collect();
    Ok(CanonicalPackageTestFixture {
        production: overlay.production,
        overlay: overlay_ref,
        records: CanonicalTestRecords {
            packages: package_records,
            contracts: vec![contract],
            deployments: vec![deployment],
            assembly,
        },
        entrypoints,
    })
}

fn compile_package_test_contract(
    overlay: &PublishedPackageTestOverlay,
) -> Result<ServiceContract, CanonicalFixtureError> {
    let mut operations = BTreeMap::new();
    for (index, binding) in overlay.bindings.iter().enumerate() {
        let projection = overlay
            .overlay
            .artifact
            .boundary_projections
            .get(&binding.callable_id)
            .ok_or_else(|| {
                CanonicalFixtureError::InvalidInput(format!(
                    "test callable {} has no boundary projection",
                    binding.callable_id
                ))
            })?;
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = projection
        else {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "test callable {} cannot cross the canonical test boundary",
                binding.callable_id
            )));
        };
        operations.insert(format!("case{index}"), operation_contract.clone());
    }
    compile_contract(ServiceContractDefinition {
        service_id: format!(
            "test.skiff/package/{}",
            safe_coordinate(&overlay.production.package_id)
        ),
        contract_version: overlay.production.package_version.clone(),
        operations,
        boundary_schema: BTreeMap::new(),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: format!("package tests for {}", overlay.production.package_id),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    })
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))
}

fn package_test_operation_inputs(
    contract: &ServiceContract,
    overlay: &PublishedPackageTestOverlay,
) -> Result<
    (
        Vec<ServiceDeploymentOperationInput>,
        Vec<DeploymentIngressBinding>,
    ),
    CanonicalFixtureError,
> {
    let mut operations = contract
        .operations
        .values()
        .map(|descriptor| {
            (
                descriptor.stable_key.as_str(),
                descriptor.operation_id.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut operation_bindings = Vec::new();
    let mut ingress = Vec::new();
    for (index, binding) in overlay.bindings.iter().enumerate() {
        let stable_key = format!("case{index}");
        let operation = operations.remove(stable_key.as_str()).ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!(
                "compiled test contract omitted stable key {stable_key}"
            ))
        })?;
        operation_bindings.push(ServiceDeploymentOperationInput {
            contract_operation_id: operation.clone(),
            package_public_path: binding.public_path.clone(),
        });
        ingress.push(DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                host: format!("case-{index}.package-test.skiff.localhost"),
                method: Some("POST".to_string()),
                path: format!("/__skiff/package-test/{index}"),
            },
            contract_operation_id: operation,
        });
    }
    Ok((operation_bindings, ingress))
}

fn package_test_package_bindings(
    project: &CanonicalPackageProject,
    overlay: &PublishedPackageTestOverlay,
    overlay_ref: &skiff_artifact_model::PackageArtifactRef,
) -> Result<Vec<PackageBinding>, CanonicalFixtureError> {
    overlay
        .overlay
        .artifact
        .package_requirements
        .iter()
        .map(|requirement| {
            let package = project
                .artifact(&requirement.package_id, &requirement.exact_version)
                .ok_or_else(|| {
                    CanonicalFixtureError::InvalidInput(format!(
                        "test overlay dependency {}@{} is absent",
                        requirement.package_id, requirement.exact_version
                    ))
                })?;
            let package = package_artifact_ref(&package.artifact)
                .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
            if package.package_local_abi_identity != requirement.expected_local_abi {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "test overlay dependency {} ABI changed",
                    requirement.alias
                )));
            }
            Ok(PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: overlay_ref.package_build_id.clone(),
                    package_requirement_alias: requirement.alias.clone(),
                },
                package,
            })
        })
        .collect()
}

fn package_test_deployment_input(
    overlay: &PublishedPackageTestOverlay,
    contract: skiff_artifact_model::ServiceContractRef,
    implementation: skiff_artifact_model::PackageArtifactRef,
    operation_bindings: Vec<ServiceDeploymentOperationInput>,
    package_bindings: Vec<PackageBinding>,
    ingress: Vec<DeploymentIngressBinding>,
) -> ServiceDeploymentInput {
    let revision = implementation
        .package_build_id
        .as_str()
        .rsplit(':')
        .next()
        .unwrap_or("overlay");
    ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision: DeploymentRevision::new(format!("test-{revision}")),
        implementation,
        operation_bindings,
        package_bindings,
        service_selectors: Vec::new(),
        ingress,
        config_literals: Vec::new(),
        secret_refs: Vec::new(),
        state_bindings: Vec::new(),
        resource_bindings: Vec::new(),
        runtime_capability_bindings: Vec::new(),
        policy: DeploymentPolicy {
            timeout_ms: 30_000,
            resources: ResourcePolicy {
                cpu_millis: 100,
                memory_bytes: 64 * 1024 * 1024,
            },
            activation: ActivationPolicy {
                max_concurrency: 1,
                idle_timeout_ms: None,
            },
            principal: "test:package-runner".to_string(),
        },
        diagnostic_text: DeploymentDiagnosticText {
            display_name: format!("package tests for {}", overlay.production.package_id),
            notes: BTreeMap::from([
                ("configOwner".to_string(), "test deployment".to_string()),
                ("stateOwner".to_string(), "test deployment".to_string()),
                ("doubleOwner".to_string(), "test request".to_string()),
            ]),
        },
    }
}

fn safe_coordinate(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '/' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect()
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

fn collect_test_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), CanonicalFixtureError> {
    let entries = fs::read_dir(root).map_err(|source| CanonicalFixtureError::Io {
        path: root.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| CanonicalFixtureError::Io {
            path: root.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|source| CanonicalFixtureError::Io {
                path: path.display().to_string(),
                source,
            })?;
        if kind.is_dir() {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if name != "target" && name != "node_modules" && !name.starts_with('.') {
                collect_test_files(&path, output)?;
            }
        } else if kind.is_file() && is_test_file(&path) {
            output.push(path);
        }
    }
    Ok(())
}

fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".test.skiff"))
}

fn test_module_path(path: &Path) -> Result<String, CanonicalFixtureError> {
    let text = path.to_str().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(format!(
            "test source path {} is not valid UTF-8",
            path.display()
        ))
    })?;
    let stem = text.strip_suffix(".test.skiff").ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(format!(
            "test source {} must end with .test.skiff",
            path.display()
        ))
    })?;
    Ok(stem
        .split(std::path::MAIN_SEPARATOR)
        .filter(|part| !part.is_empty())
        .chain(std::iter::once("__test"))
        .collect::<Vec<_>>()
        .join("."))
}
