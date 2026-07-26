mod common;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryCallbackContract, BoundaryEffectGuarantee,
    BoundaryOperationContract, BoundaryParameter, BoundaryReturn, BoundaryStreamContract,
    BoundaryUnavailableReason, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef, PackageLocalAbiSymbol,
    PackageTypeRequirement, TypeRefIr,
};
use skiff_compiler::{
    definition_contract_operation_id, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};

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
fn websocket_ingress_contract_first_source_is_structured_service_call_unavailable() {
    let expected = websocket_operation(ContractTypeRef::builtin("null"));
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
  if event.tag == "connect" {
    return acceptConnection()
  }
  if event.tag == "receive" {
    const receiveEvent = event.receiveEvent
    const connectionId: string = receiveEvent.connection.id
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
    assert!(
        matches!(
            projection,
            BoundaryCallableProjection::Unavailable { reasons }
                if reasons == &[BoundaryUnavailableReason::UnsupportedBoundaryType]
        ),
        "generic WebSocket platform types must not be service-call boundary types: {projection:?}"
    );
}

#[test]
fn websocket_nominal_context_source_preserves_execution_but_is_service_call_unavailable() {
    let seed = TestDir::new("skiff-compiler", "websocket-context-schema-seed");
    seed.write("package.yml", format!("id: {PACKAGE_ID}\nversion: 1.0.0\n"));
    seed.write("api.yml", "Context: main.Context\n");
    seed.write("main.skiff", "type Context {}\n");
    let seed = compile_package_project(seed.path()).expect("Context schema seed should compile");
    let (context, context_id) = public_contract_type(&seed.package, "Context");
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
    const connectionId: string = receiveEvent.connection.id
    const messageTag: string = receiveEvent.message.tag
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
            BoundaryCallableProjection::Unavailable { reasons }
                if reasons == &[BoundaryUnavailableReason::UnsupportedBoundaryType]
        ),
        "nominal WebSocket platform types must not be service-call boundary types: {projection:?}"
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
        stream: BoundaryStreamContract::Unary,
        callbacks: BoundaryCallbackContract::None,
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
