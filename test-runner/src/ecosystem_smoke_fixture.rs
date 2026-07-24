//! Canonical unary, WebSocket, and package-test fixtures used by the isolated ecosystem smoke.
//!
//! WebSocket is included only when the production package exports the canonical
//! `websocket` callable. The fixture consumes the compiler-produced boundary
//! projection directly and never edits or re-signs a package artifact.

use std::collections::BTreeMap;

use skiff_artifact_identity::{package_artifact_ref, service_contract_ref, service_deployment_ref};
use skiff_artifact_model::{
    ActivationPolicy, BoundaryCallableProjection, ContractOperationId, DeploymentDiagnosticText,
    DeploymentIngressBinding, DeploymentPolicy, DeploymentRevision, IngressProtocol,
    IngressSelector, PackageArtifactRef, PackageBinding, ResourcePolicy, ServiceContract,
    ServiceContractRef, ServiceDeploymentInput, ServiceDeploymentOperationInput,
    SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_compiler::{
    compile_contract, ServiceContractDefinition, ServiceContractDefinitionDiagnosticText,
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
    package_schema_contract::schema_closure,
    package_test_assembly::canonical_package_bindings,
    test_overlay::PublishedPackageTestOverlay,
};

const SMOKE_SERVICE_ID: &str = "test.skiff/ecosystem-smoke";
const SMOKE_CONTRACT_VERSION: &str = "1.0.0";
const SMOKE_HOST: &str = "ecosystem-smoke.skiff.localhost";
const SMOKE_WEBSOCKET_PATH: &str = "/socket";

#[derive(Debug, Clone)]
pub struct EcosystemSmokeEntrypoint {
    pub selector: IngressSelector,
    pub deployment: skiff_artifact_model::ServiceDeploymentRef,
    pub contract: ServiceContractRef,
    pub operation: ContractOperationId,
}

#[derive(Debug, Clone)]
pub struct CanonicalEcosystemSmokeFixture {
    pub production: PackageArtifactRef,
    pub overlay: PackageArtifactRef,
    pub records: CanonicalTestRecords,
    pub package_test: CanonicalPackageTestEntrypoint,
    pub unary: EcosystemSmokeEntrypoint,
    pub websocket: Option<EcosystemSmokeEntrypoint>,
}

pub fn assemble_ecosystem_smoke_fixture(
    project: &CanonicalPackageProject,
    overlay: PublishedPackageTestOverlay,
) -> Result<CanonicalEcosystemSmokeFixture, CanonicalFixtureError> {
    let test_fixture = assemble_package_test_fixture(project, overlay, Default::default())?;
    if test_fixture.entrypoints.len() != 1 {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "ecosystem smoke requires exactly one package-test entrypoint, found {}",
            test_fixture.entrypoints.len()
        )));
    }
    let smoke = compile_smoke_contract(project)?;
    let selector = IngressSelector {
        protocol: IngressProtocol::Http,
        host: SMOKE_HOST.to_string(),
        method: Some("POST".to_string()),
        path: "/probe".to_string(),
    };
    let websocket_selector = IngressSelector {
        protocol: IngressProtocol::WebSocket,
        host: SMOKE_HOST.to_string(),
        method: None,
        path: SMOKE_WEBSOCKET_PATH.to_string(),
    };
    let production = package_artifact_ref(&project.package.artifact)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let packages = project.artifacts().cloned().collect::<Vec<_>>();
    let package_bindings = canonical_package_bindings(&packages)?;
    let deployment = project_service_deployment(
        smoke_deployment_input(
            &smoke,
            production,
            selector.clone(),
            websocket_selector.clone(),
            package_bindings,
        ),
        &smoke.contract,
        &packages,
        &smoke.schema_records,
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let deployment_ref = service_deployment_ref(&deployment);

    let mut records = test_fixture.records;
    records.contracts.push(smoke.contract);
    records.deployments.push(deployment);
    let roots = vec![
        test_fixture.entrypoints[0].deployment.clone(),
        deployment_ref.clone(),
    ];
    let all_packages = records
        .packages
        .iter()
        .map(|package| package.artifact.clone())
        .chain(project.dependency_packages.iter().cloned())
        .collect::<Vec<_>>();
    records.assembly = resolve_runtime_assembly(
        &roots,
        &records.deployments,
        &records.contracts,
        &all_packages,
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;

    let websocket = smoke
        .websocket_operation
        .clone()
        .map(|operation| EcosystemSmokeEntrypoint {
            selector: websocket_selector,
            deployment: deployment_ref.clone(),
            contract: smoke.reference.clone(),
            operation,
        });
    Ok(CanonicalEcosystemSmokeFixture {
        production: test_fixture.production,
        overlay: test_fixture.overlay,
        package_test: test_fixture.entrypoints.into_iter().next().unwrap(),
        records,
        unary: EcosystemSmokeEntrypoint {
            selector,
            deployment: deployment_ref,
            contract: smoke.reference,
            operation: smoke.operation,
        },
        websocket,
    })
}

fn smoke_deployment_input(
    smoke: &SmokeContract,
    production: PackageArtifactRef,
    selector: IngressSelector,
    websocket_selector: IngressSelector,
    package_bindings: Vec<PackageBinding>,
) -> ServiceDeploymentInput {
    let revision = production
        .package_build_id
        .as_str()
        .rsplit(':')
        .next()
        .unwrap_or("package");
    let mut operation_bindings = vec![ServiceDeploymentOperationInput {
        contract_operation_id: smoke.operation.clone(),
        package_public_path: "marker".to_string(),
    }];
    let mut ingress = vec![DeploymentIngressBinding {
        selector,
        contract_operation_id: smoke.operation.clone(),
    }];
    if let Some(operation) = smoke.websocket_operation.as_ref() {
        operation_bindings.push(ServiceDeploymentOperationInput {
            contract_operation_id: operation.clone(),
            package_public_path: "websocket".to_string(),
        });
        ingress.push(DeploymentIngressBinding {
            selector: websocket_selector,
            contract_operation_id: operation.clone(),
        });
    }
    ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract: smoke.reference.clone(),
        deployment_revision: DeploymentRevision::new(format!("smoke-{revision}")),
        implementation: production,
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
                max_concurrency: 4,
                idle_timeout_ms: None,
            },
            principal: "test:ecosystem-smoke".to_string(),
        },
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "isolated ecosystem smoke".to_string(),
            notes: BTreeMap::from([
                ("configOwner".to_string(), "smoke deployment".to_string()),
                ("stateOwner".to_string(), "smoke deployment".to_string()),
                ("resourceOwner".to_string(), "smoke deployment".to_string()),
            ]),
        },
    }
}

struct SmokeContract {
    contract: ServiceContract,
    schema_records: BTreeMap<
        skiff_artifact_model::PackageSchemaTypeId,
        skiff_artifact_model::PackageSchemaTypeRecord,
    >,
    reference: ServiceContractRef,
    operation: ContractOperationId,
    websocket_operation: Option<ContractOperationId>,
}

fn compile_smoke_contract(
    project: &CanonicalPackageProject,
) -> Result<SmokeContract, CanonicalFixtureError> {
    let marker_contract = public_operation_contract(project, "marker", true)?
        .expect("required marker projection checked");
    let websocket_contract = public_operation_contract(project, "websocket", false)?;
    let mut operations = BTreeMap::from([("marker".to_string(), marker_contract)]);
    if let Some(websocket) = websocket_contract {
        operations.insert("websocket".to_string(), websocket);
    }
    let (package_type_requirements, schema_records) = schema_closure(
        &operations,
        &project.package.resolved_package_schema_type_records,
    )
    .map_err(CanonicalFixtureError::InvalidInput)?;
    let contract = compile_contract(ServiceContractDefinition {
        service_id: SMOKE_SERVICE_ID.to_string(),
        contract_version: SMOKE_CONTRACT_VERSION.to_string(),
        operations,
        package_type_requirements,
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "ecosystem smoke".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    })
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let operation = contract
        .operations
        .values()
        .find(|descriptor| descriptor.stable_key == "marker")
        .ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(
                "smoke contract omitted marker operation".to_string(),
            )
        })?
        .operation_id
        .clone();
    let websocket_operation = contract
        .operations
        .values()
        .find(|descriptor| descriptor.stable_key == "websocket")
        .map(|descriptor| descriptor.operation_id.clone());
    Ok(SmokeContract {
        reference: service_contract_ref(&contract)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?,
        operation,
        websocket_operation,
        schema_records,
        contract,
    })
}

fn public_operation_contract(
    project: &CanonicalPackageProject,
    public_path: &str,
    required: bool,
) -> Result<Option<skiff_artifact_model::BoundaryOperationContract>, CanonicalFixtureError> {
    let Some(symbol) = project
        .package
        .artifact
        .package_local_abi
        .public_symbols
        .get(public_path)
    else {
        if required {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "smoke package omitted public callable {public_path}"
            )));
        }
        return Ok(None);
    };
    let skiff_artifact_model::PackageLocalAbiSymbol::Callable { callable_id, .. } = symbol else {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "smoke public path {public_path} is not callable"
        )));
    };
    let projection = project
        .package
        .artifact
        .boundary_projections
        .get(callable_id)
        .ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!(
                "smoke {public_path} has no boundary projection"
            ))
        })?;
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = projection
    else {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "smoke {public_path} cannot cross the canonical boundary"
        )));
    };
    Ok(Some(operation_contract.clone()))
}
