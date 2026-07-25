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
    PackageLocalAbiSymbol, PackageRequirementKey, PackageSchemaTypeId, PackageSchemaTypeRecord,
    PackageTypeRequirement, ResourcePolicy, ServiceDeploymentInput,
    ServiceDeploymentOperationInput, TypeRefIr, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_compiler::{
    definition_contract_operation_id, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};
use skiff_deployment::projection::project_service_deployment;

use common::{
    artifacts::module_artifact,
    contracts::{compile_service_contract, package_contract_dependency},
    package_project::{
        compile_package_project, compile_package_project_with_contract_dependencies,
        compile_package_project_with_contract_dependencies_and_schemas,
    },
    package_schemas::{public_contract_type, resolved_package_schema},
    TestDir,
};

const SERVICE_ID: &str = "example.websocket";
const CONTRACT_VERSION: &str = "1.0.0";
const PACKAGE_ID: &str = "example.com/websocket-provider";

#[test]
fn websocket_ingress_contract_first_source_projects_and_deploys_exactly() {
    let mut expected = websocket_operation(ContractTypeRef::builtin("null"));
    expected.may_suspend = true;
    let contract = compile_service_contract(ServiceContractDefinition {
        service_id: SERVICE_ID.to_string(),
        contract_version: CONTRACT_VERSION.to_string(),
        operations: BTreeMap::from([("websocket".to_string(), expected.clone())]),
        package_type_requirements: Vec::new(),
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
  std.time.sleep(Duration.milliseconds(0))
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
    let deployment = project_websocket_fixture_deployment(
        &contract,
        &packages,
        &project.package.artifact,
        operation_id,
        "probe",
        &BTreeMap::new(),
    );
    assert_eq!(deployment.operation_bindings.len(), 1);
}

#[test]
fn websocket_nominal_context_normal_source_reaches_exact_deployment_and_erased_execution() {
    let seed = TestDir::new("skiff-compiler", "websocket-context-schema-seed");
    seed.write("package.yml", format!("id: {PACKAGE_ID}\nversion: 1.0.0\n"));
    seed.write("api.yml", "Context: main.Context\n");
    seed.write("main.skiff", "type Context {}\n");
    let seed = compile_package_project(seed.path()).expect("Context schema seed should compile");
    let (context, context_id) = public_contract_type(&seed.package, "Context");
    let schema_records = seed.package.package_schema_type_records.clone();
    let expected = websocket_operation(context);
    let contract = compile_service_contract(ServiceContractDefinition {
        service_id: SERVICE_ID.to_string(),
        contract_version: CONTRACT_VERSION.to_string(),
        operations: BTreeMap::from([("websocket".to_string(), expected.clone())]),
        package_type_requirements: vec![PackageTypeRequirement {
            package_id: PACKAGE_ID.to_string(),
            required_type_ids: vec![context_id.clone()],
        }],
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "canonical nominal websocket ingress probe".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::from([(context_id, "empty persisted Context".to_string())]),
        },
    })
    .expect("code-free nominal WebSocket contract should compile");
    let operation_id =
        definition_contract_operation_id(SERVICE_ID, CONTRACT_VERSION, "websocket").unwrap();

    let temp = TestDir::new("skiff-compiler", "websocket-nominal-context-normal-source");
    temp.write("package.yml", format!("id: {PACKAGE_ID}\nversion: 1.0.0\n"));
    temp.write(
        "api.yml",
        "Context: main.Context\nwebsocket: main.websocket\n",
    );
    temp.write(
        "main.skiff",
        r#"import std

type Context {}

function websocket(event: std.websocket.WebSocketIngressEvent<Context>) -> std.websocket.WebSocketConnectResult<Context>? {
  if event.tag == "connect" {
    const request = event.connectRequest
    return {
      tag: "accept",
      context: Context {},
      businessIdentity: request.connectionId,
      connectionPolicy: null
    }
  }
  if event.tag == "receive" {
    const receiveEvent = event.receiveEvent
    const context: Context = receiveEvent.connection.context
    std.websocket.sendTextToConnection(receiveEvent.connection.id, receiveEvent.message.tag)
  }
  return null
}
"#,
    );
    let dependencies = BTreeMap::from([(
        (PACKAGE_ID.to_string(), "1.0.0".to_string()),
        vec![package_contract_dependency("gateway", contract.clone())],
    )]);
    let schemas = BTreeMap::from([(
        (PACKAGE_ID.to_string(), "1.0.0".to_string()),
        vec![resolved_package_schema("contract-schema", &seed.package)
            .expect("Context schema should resolve")],
    )]);
    let project = compile_package_project_with_contract_dependencies_and_schemas(
        temp.path(),
        &dependencies,
        &schemas,
    )
    .expect("normal provider source should preserve contract-owned nominal Context");
    let PackageLocalAbiSymbol::Callable { callable_id, .. } =
        &project.package.artifact.package_local_abi.public_symbols["websocket"]
    else {
        panic!("websocket must project as a public callable")
    };
    let projection = &project.package.artifact.boundary_projections[callable_id];
    assert!(
        matches!(
            projection,
            BoundaryCallableProjection::Available {
                operation_contract,
                ..
            } if operation_contract == &expected
        ),
        "nominal websocket projection should remain available: {projection:?}"
    );

    let websocket = module_artifact(&project.package, "main")
        .unit
        .executables
        .iter()
        .find(|executable| executable.symbol.ends_with(".websocket"))
        .expect("normal source should emit the websocket executable");
    assert_eq!(websocket.params.len(), 1);
    assert_eq!(websocket.params[0].name, "event");
    assert_eq!(
        websocket.params[0].ty,
        generic_execution_type(
            "std.websocket.WebSocketIngressEvent",
            TypeRefIr::LocalType { type_index: 0 },
        )
    );
    assert_eq!(
        websocket.return_type,
        TypeRefIr::Nullable {
            inner: Box::new(generic_execution_type(
                "std.websocket.WebSocketConnectResult",
                TypeRefIr::LocalType { type_index: 0 },
            )),
        }
    );

    let packages = project
        .artifacts()
        .map(|package| package.artifact.clone())
        .collect::<Vec<_>>();
    let deployment = project_websocket_fixture_deployment(
        &contract,
        &packages,
        &project.package.artifact,
        operation_id.clone(),
        "nominal-probe",
        &schema_records,
    );
    assert_eq!(
        deployment.operation_bindings[0].contract_operation_id,
        operation_id
    );
}

#[test]
fn imported_generic_expands_builtin_and_nested_local_nominal_arguments() {
    for (fixture, context, assertion) in [
        ("builtin", "string", "const context: string"),
        (
            "nested-local",
            "Array<Context?>",
            "const context: Array<Context?>",
        ),
    ] {
        let temp = TestDir::new("skiff-compiler", &format!("websocket-{fixture}-context"));
        temp.write(
            "package.yml",
            "id: example.com/generic-probe\nversion: 1.0.0\n",
        );
        temp.write("api.yml", "probe: main.probe\n");
        temp.write(
            "main.skiff",
            format!(
                r#"import std

type Context {{
  value: string,
}}

function probe(event: std.websocket.WebSocketIngressEvent<{context}>) -> null {{
  if event.tag == "connect" {{
    const request = event.connectRequest
    const id: string = request.connectionId
  }}
  if event.tag == "receive" {{
    const receiveEvent = event.receiveEvent
    {assertion} = receiveEvent.connection.context
    const messageTag: string = receiveEvent.message.tag
  }}
  return null
}}
"#
            ),
        );
        compile_package_project(temp.path())
            .unwrap_or_else(|error| panic!("{fixture} generic context should expand: {error}"));
    }
}

#[test]
fn imported_generic_rejects_wrong_arity_unresolved_and_different_nominal_arguments() {
    for (fixture, source, expected) in [
        (
            "missing-argument",
            "function probe(event: std.websocket.WebSocketIngressEvent) -> null { return null }",
            "expects 1 type arguments, found 0",
        ),
        (
            "extra-argument",
            "function probe(event: std.websocket.WebSocketIngressEvent<string, bool>) -> null { return null }",
            "expects 1 type arguments, found 2",
        ),
        (
            "unresolved-argument",
            "function probe(event: std.websocket.WebSocketIngressEvent<Missing>) -> null { return null }",
            "unresolved type `Missing`",
        ),
        (
            "different-nominal",
            r#"
type Expected {}
type Actual {}
function take(value: Expected) -> null { return null }
function probe(event: std.websocket.WebSocketIngressEvent<Actual>) -> null {
  if event.tag == "receive" {
    take(event.receiveEvent.connection.context)
  }
  return null
}
"#,
            "argument",
        ),
    ] {
        let temp = TestDir::new("skiff-compiler", &format!("websocket-{fixture}"));
        temp.write("package.yml", "id: example.com/generic-probe\nversion: 1.0.0\n");
        temp.write("api.yml", "probe: main.probe\n");
        temp.write("main.skiff", format!("import std\n{source}\n"));
        let error = compile_package_project(temp.path())
            .expect_err("invalid imported generic instantiation must fail");
        assert!(
            error.to_string().contains(expected),
            "{fixture} should report {expected:?}, got: {error}"
        );
    }
}

fn project_websocket_fixture_deployment(
    contract: &skiff_artifact_model::ServiceContract,
    packages: &[skiff_artifact_model::PackageArtifact],
    implementation: &skiff_artifact_model::PackageArtifact,
    operation_id: skiff_artifact_model::ContractOperationId,
    fixture: &str,
    package_schema_records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> skiff_artifact_model::ServiceDeployment {
    project_service_deployment(
        ServiceDeploymentInput {
            schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
            contract: service_contract_ref(contract).unwrap(),
            deployment_revision: DeploymentRevision::new(format!("websocket-{fixture}")),
            implementation: package_artifact_ref(implementation).unwrap(),
            operation_bindings: vec![ServiceDeploymentOperationInput {
                contract_operation_id: operation_id.clone(),
                package_public_path: "websocket".to_string(),
            }],
            package_bindings: canonical_package_bindings(packages),
            service_selectors: Vec::new(),
            ingress: vec![DeploymentIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::WebSocket,
                    host: format!("{fixture}.skiff.localhost"),
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
                timeout_ms: Some(30_000),
                resources: ResourcePolicy {
                    cpu_millis: 100,
                    memory_bytes: 64 * 1024 * 1024,
                },
                activation: ActivationPolicy {
                    max_concurrency: 4,
                    idle_timeout_ms: None,
                },
                principal: format!("test:websocket-{fixture}"),
            },
            diagnostic_text: DeploymentDiagnosticText {
                display_name: format!("canonical WebSocket {fixture}"),
                notes: BTreeMap::new(),
            },
        },
        contract,
        packages,
        package_schema_records,
    )
    .expect("deployment must consume the exact normal-source WebSocket projection")
}

fn generic_execution_type(name: &str, argument: TypeRefIr) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: name.to_string(),
        args: vec![argument],
    }
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
