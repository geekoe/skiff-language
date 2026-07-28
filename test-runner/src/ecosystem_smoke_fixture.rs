//! Canonical package-test and unary HTTP fixtures used by isolated ecosystem checks.
//!
//! This projection deliberately excludes WebSocket ingress. The production
//! package must compile the private HTTP wrapper as ordinary source; the
//! fixture consumes that immutable artifact without editing or re-signing it.

use std::collections::BTreeMap;

use skiff_artifact_identity::{package_artifact_ref, service_contract_ref, service_deployment_ref};
use skiff_artifact_model::{
    ActivationPolicy, DeploymentDiagnosticText, DeploymentIngressBinding, DeploymentPolicy,
    DeploymentRevision, GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey,
    GatewayExternalSchema, IngressProtocol, IngressSelector, PackageArtifactRef, PackageBinding,
    ResourcePolicy, ServiceContract, ServiceDeploymentInput,
    SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_deployment::{
    assembly::resolve_runtime_assembly, projection::project_service_deployment,
};

use crate::{
    canonical_fixture::{
        assemble_package_test_fixture, CanonicalFixtureError, CanonicalPackageTestEntrypoint,
        CanonicalTestRecords,
    },
    canonical_package::CanonicalPackageProject,
    canonical_test_gateway::canonical_typed_null_gateway,
    package_test_assembly::{canonical_package_bindings, canonical_zero_operation_contract},
    test_overlay::PublishedPackageTestOverlay,
};

const SMOKE_SERVICE_ID: &str = "test.skiff/ecosystem-smoke";
const SMOKE_CONTRACT_VERSION: &str = "1.0.0";
const SMOKE_PROBE_PATH: &str = "/probe";
const SMOKE_PROBE_KEY: &str = "probe";
const SMOKE_PROBE_HANDLER: &str = "main.__skiffHttpProbe";

#[derive(Debug, Clone)]
pub struct EcosystemSmokeEntrypoint {
    pub selector: IngressSelector,
    pub deployment: skiff_artifact_model::ServiceDeploymentRef,
    pub gateway_entry_key: GatewayEntryKey,
    pub gateway_entry_identity: GatewayEntryIdentity,
    pub mode: GatewayDispatchMode,
}

#[derive(Debug, Clone)]
pub struct CanonicalEcosystemSmokeFixture {
    pub production: PackageArtifactRef,
    pub overlay: PackageArtifactRef,
    pub records: CanonicalTestRecords,
    pub package_test: CanonicalPackageTestEntrypoint,
    pub unary: EcosystemSmokeEntrypoint,
}

pub fn assemble_ecosystem_smoke_fixture(
    project: &CanonicalPackageProject,
    overlay: PublishedPackageTestOverlay,
) -> Result<CanonicalEcosystemSmokeFixture, CanonicalFixtureError> {
    let overlay_dependencies = overlay.dependency_packages.clone();
    let test_fixture = assemble_package_test_fixture(project, overlay, Default::default())?;
    let [package_test] = test_fixture.entrypoints.as_slice() else {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "ecosystem HTTP fixture requires exactly one package-test entrypoint, found {}",
            test_fixture.entrypoints.len()
        )));
    };
    let package_test = package_test.clone();

    let smoke_contract = compile_smoke_contract()?;
    let smoke_contract_ref = service_contract_ref(&smoke_contract)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let production = package_artifact_ref(&project.package.artifact)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    if production != test_fixture.production {
        return Err(CanonicalFixtureError::InvalidInput(
            "ecosystem HTTP fixture production package changed during overlay compilation"
                .to_string(),
        ));
    }
    let selector = IngressSelector {
        protocol: IngressProtocol::Http,
        method: Some("POST".to_string()),
        path: SMOKE_PROBE_PATH.to_string(),
    };
    let gateway_entry_key = GatewayEntryKey::parse(SMOKE_PROBE_KEY)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let gateway_entry = canonical_typed_null_gateway(
        &project.package.artifact,
        SMOKE_PROBE_HANDLER,
        GatewayExternalSchema::String,
    )
    .map_err(CanonicalFixtureError::InvalidInput)?;
    let packages = project.artifacts().cloned().collect::<Vec<_>>();
    let package_bindings = canonical_package_bindings(&packages)?;
    let deployment = project_service_deployment(
        smoke_deployment_input(
            smoke_contract_ref,
            production,
            gateway_entry_key.clone(),
            gateway_entry.clone(),
            selector.clone(),
            package_bindings,
        ),
        &smoke_contract,
        &packages,
        &BTreeMap::new(),
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let deployment_ref = service_deployment_ref(&deployment);

    let mut records = test_fixture.records;
    records.contracts.push(smoke_contract);
    records.deployments.push(deployment);
    let roots = vec![package_test.deployment.clone(), deployment_ref.clone()];
    let all_packages = records
        .packages
        .iter()
        .map(|package| package.artifact.clone())
        .chain(project.dependency_packages.iter().cloned())
        .chain(overlay_dependencies)
        .collect::<Vec<_>>();
    records.assembly = resolve_runtime_assembly(
        &roots,
        &records.deployments,
        &records.contracts,
        &all_packages,
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    if records.assembly.gateway_ingress.len() != 2 {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "ecosystem HTTP fixture must project exactly two gateway ingress entries, found {}",
            records.assembly.gateway_ingress.len()
        )));
    }

    Ok(CanonicalEcosystemSmokeFixture {
        production: test_fixture.production,
        overlay: test_fixture.overlay,
        package_test,
        records,
        unary: EcosystemSmokeEntrypoint {
            selector,
            deployment: deployment_ref,
            gateway_entry_key,
            gateway_entry_identity: gateway_entry.gateway_entry_identity,
            mode: GatewayDispatchMode::Unary,
        },
    })
}

fn smoke_deployment_input(
    contract: skiff_artifact_model::ServiceContractRef,
    production: PackageArtifactRef,
    gateway_entry_key: GatewayEntryKey,
    gateway_entry: skiff_artifact_model::DeploymentGatewayEntry,
    selector: IngressSelector,
    package_bindings: Vec<PackageBinding>,
) -> ServiceDeploymentInput {
    let revision = production
        .package_build_id
        .as_str()
        .rsplit(':')
        .next()
        .unwrap_or("package");
    ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision: DeploymentRevision::new(format!("smoke-{revision}")),
        implementation: production,
        operation_bindings: Vec::new(),
        package_bindings,
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::from([(gateway_entry_key.clone(), gateway_entry)]),
        ingress: vec![DeploymentIngressBinding {
            selector,
            gateway_entry_key,
        }],
        config_literals: Vec::new(),
        secret_refs: Vec::new(),
        state_bindings: Vec::new(),
        resource_bindings: Vec::new(),
        runtime_capability_bindings: Vec::new(),
        policy: DeploymentPolicy {
            timeout_ms: Some(30_000),
            resources: ResourcePolicy {
                cpu_millis: 100,
                memory_bytes: 64 * 1024 * 1024,
            },
            activation: ActivationPolicy {
                max_concurrency: 4,
                idle_timeout_ms: None,
            },
            principal: "test:ecosystem-smoke".to_string(),
        },
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "isolated ecosystem HTTP smoke".to_string(),
            notes: BTreeMap::from([
                ("configOwner".to_string(), "smoke deployment".to_string()),
                ("stateOwner".to_string(), "smoke deployment".to_string()),
                ("resourceOwner".to_string(), "smoke deployment".to_string()),
            ]),
        },
    }
}

fn compile_smoke_contract() -> Result<ServiceContract, CanonicalFixtureError> {
    canonical_zero_operation_contract(
        SMOKE_SERVICE_ID.to_string(),
        SMOKE_CONTRACT_VERSION.to_string(),
        "ecosystem HTTP smoke".to_string(),
    )
}
