mod common;

use std::collections::BTreeMap;

use skiff_artifact_identity::{package_artifact_ref, service_contract_ref};
use skiff_artifact_model::{
    ActivationPolicy, BoundaryCallableProjection, BoundaryCallbackContract,
    BoundaryCancellationContract, BoundaryEffectGuarantee, BoundaryErrorContract,
    BoundaryOperationContract, BoundaryParameter, BoundaryReturn, BoundaryStreamContract,
    BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
    BoundaryValuePlan, ContractTypeRef, DeploymentDiagnosticText, DeploymentIngressBinding,
    DeploymentPolicy, DeploymentRevision, IngressProtocol, IngressSelector, PackageBinding,
    PackageLocalAbiSymbol, PackageRequirementKey, ResourcePolicy, ServiceDeploymentInput,
    ServiceDeploymentOperationInput, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_compiler::{
    definition_contract_operation_id, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};
use skiff_deployment::projection::project_service_deployment;

use common::{
    contracts::{compile_service_contract, package_contract_dependency},
    package_project::compile_package_project_with_contract_dependencies,
    TestDir,
};

const SERVICE_ID: &str = "example.websocket";
const CONTRACT_VERSION: &str = "1.0.0";
const PACKAGE_ID: &str = "example.com/websocket-provider";

#[test]
fn websocket_ingress_contract_first_source_projects_and_deploys_exactly() {
    let expected = websocket_operation(ContractTypeRef::builtin("null"));
    let contract = compile_service_contract(ServiceContractDefinition {
        service_id: SERVICE_ID.to_string(),
        contract_version: CONTRACT_VERSION.to_string(),
        operations: BTreeMap::from([("websocket".to_string(), expected.clone())]),
        boundary_schema: BTreeMap::new(),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "canonical websocket ingress probe".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    })
    .expect("code-free WebSocket ingress contract should compile");
    let operation_id =
        definition_contract_operation_id(SERVICE_ID, CONTRACT_VERSION, "websocket").unwrap();
    assert_eq!(contract.operations[&operation_id].contract, expected);

    let temp = TestDir::new("skiff-compiler", "websocket-ingress-contract-first-probe");
    temp.write("package.yml", format!("id: {PACKAGE_ID}\nversion: 1.0.0\n"));
    temp.write("api.yml", "websocket: main.websocket\n");
    temp.write(
        "main.skiff",
        r#"import std

function acceptConnection() -> std.websocket.WebSocketConnectResult<null> {
  return {
    tag: "accept",
    context: null,
    businessIdentity: null,
    connectionPolicy: null
  }
}

function websocket(event: std.websocket.WebSocketIngressEvent<null>) -> std.websocket.WebSocketConnectResult<null>? {
  if event.tag == "connect" {
    return acceptConnection()
  }
  if event.tag == "receive" {
    const receiveEvent = event.receiveEvent
    std.websocket.sendTextToConnection(receiveEvent.connection.id, "A")
  }
  return null
}
"#,
    );
    let dependencies = BTreeMap::from([(
        (PACKAGE_ID.to_string(), "1.0.0".to_string()),
        vec![package_contract_dependency("gateway", contract.clone())],
    )]);
    let project = compile_package_project_with_contract_dependencies(temp.path(), &dependencies)
        .expect("normal provider source should compile against the code-free contract");
    let PackageLocalAbiSymbol::Callable { callable_id, .. } =
        &project.package.artifact.package_local_abi.public_symbols["websocket"]
    else {
        panic!("websocket must project as a public callable")
    };
    let projection = &project.package.artifact.boundary_projections[callable_id];
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = projection
    else {
        panic!("canonical WebSocket ingress must have an available boundary projection: {projection:?}")
    };
    assert_eq!(operation_contract, &expected);

    let packages = project
        .artifacts()
        .map(|package| package.artifact.clone())
        .collect::<Vec<_>>();
    let implementation = package_artifact_ref(&project.package.artifact).unwrap();
    let deployment = project_service_deployment(
        ServiceDeploymentInput {
            schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
            contract: service_contract_ref(&contract).unwrap(),
            deployment_revision: DeploymentRevision::new("websocket-probe"),
            implementation,
            operation_bindings: vec![ServiceDeploymentOperationInput {
                contract_operation_id: operation_id.clone(),
                package_public_path: "websocket".to_string(),
            }],
            package_bindings: canonical_package_bindings(&packages),
            service_selectors: Vec::new(),
            ingress: vec![DeploymentIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::WebSocket,
                    host: "probe.skiff.localhost".to_string(),
                    method: None,
                    path: "/socket".to_string(),
                },
                contract_operation_id: operation_id,
            }],
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
                principal: "test:websocket-probe".to_string(),
            },
            diagnostic_text: DeploymentDiagnosticText {
                display_name: "canonical websocket ingress probe".to_string(),
                notes: BTreeMap::new(),
            },
        },
        &contract,
        &packages,
    )
    .expect("deployment projection must accept the exact compiler-produced ABI");
    assert_eq!(deployment.operation_bindings.len(), 1);
}

fn websocket_operation(context: ContractTypeRef) -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: vec![BoundaryParameter {
            name: "event".to_string(),
            ty: generic("std.websocket.WebSocketIngressEvent", context.clone()),
            value_plan: linkable(BoundaryValueOwner::Caller),
        }],
        return_value: BoundaryReturn {
            ty: ContractTypeRef::Nullable {
                inner: Box::new(generic("std.websocket.WebSocketConnectResult", context)),
            },
            value_plan: linkable(BoundaryValueOwner::Provider),
        },
        errors: BoundaryErrorContract::None,
        stream: BoundaryStreamContract::Unary,
        cancellation: BoundaryCancellationContract::NotCancellable,
        callbacks: BoundaryCallbackContract::None,
        may_suspend: false,
        effect_guarantee: BoundaryEffectGuarantee {
            detached_parameters: true,
            detached_return: true,
            detached_error: true,
            no_caller_reachable_mutation: true,
            no_caller_value_escape: true,
            no_same_heap_identity: true,
        },
    }
}

fn generic(name: &str, argument: ContractTypeRef) -> ContractTypeRef {
    ContractTypeRef::Builtin {
        name: name.to_string(),
        arguments: vec![argument],
    }
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn canonical_package_bindings(
    packages: &[skiff_artifact_model::PackageArtifact],
) -> Vec<PackageBinding> {
    let by_coordinate = packages
        .iter()
        .map(|package| {
            (
                (
                    package.package_id.as_str(),
                    package.package_version.as_str(),
                ),
                package,
            )
        })
        .collect::<BTreeMap<_, _>>();
    packages
        .iter()
        .flat_map(|caller| {
            caller
                .package_requirements
                .iter()
                .map(move |requirement| (caller, requirement))
        })
        .map(|(caller, requirement)| {
            let dependency = by_coordinate
                .get(&(
                    requirement.package_id.as_str(),
                    requirement.exact_version.as_str(),
                ))
                .expect("compiled dependency must exist in the exact closure");
            PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: caller.package_build_id.clone(),
                    package_requirement_alias: requirement.alias.clone(),
                },
                package: package_artifact_ref(dependency).unwrap(),
            }
        })
        .collect()
}
