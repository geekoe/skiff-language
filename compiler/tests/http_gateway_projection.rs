mod common;

use std::collections::BTreeMap;

use common::{
    package_project::{compile_service_package_project, PublishedPackageProject},
    TestDir,
};
use serde_json::json;
use skiff_artifact_model::{
    GatewayAdapterKind, GatewayDispatchMode, GatewayExternalSchema, GatewayProtocolSurface,
    PackageLocalAbiSymbol, PackageSchemaTypeId, PackageSchemaTypeRecord,
    ServiceConfigProfileAuthoring, ServiceDeployment, ServiceManifestAuthoring,
};
use skiff_compiler::{
    generate_service_deployment, GeneratedServiceDeploymentError, GeneratedServiceDeploymentInput,
    ServiceApiProjection,
};
use skiff_compiler_input::read_service_package_root;

const PACKAGE_ID: &str = "example.com/http-gateway-package";
const SERVICE_ID: &str = "example.com/http-gateway";

#[test]
fn private_http_entries_project_typed_and_raw_unary_plus_raw_stream_without_contract_operations() {
    let fixture = compile_fixture(
        "all-modes",
        "health: main.health\n",
        r#"import std

type Context {
  requestId: string,
}

type Status = "new" | "old"

alias Label = string
type UserId = string

type Input {
  id: UserId,
  name: Label,
  note: string?,
  status: Status,
  tags: Array<string>,
}

type Output {
  accepted: boolean,
  status: Status,
}

type Box<T> {
  value: T,
}

type Envelope<T> {
  item: Box<T>,
}

function health() -> string {
  return "ok"
}

function guard(request: std.http.HttpRequest) -> std.http.HttpResponse? {
  return null
}

function prepare(request: std.http.HttpRequest) -> Context {
  return Context { requestId: request.path }
}

function typed(
  request: std.http.HttpRequest,
  body: Input,
  bodyAgain: Input,
  context: Context
) -> Output {
  return Output { accepted: true, status: body.status }
}

function boxed(body: Envelope<string>) -> Envelope<string> {
  return body
}

function raw(request: std.http.HttpRequest) -> std.http.HttpResponse {
  return std.http.noContent()
}

function rawStream(
  request: std.http.HttpRequest
) -> Stream<std.http.HttpResponseStreamEvent> {
  emit(std.http.streamChunk(request.body))
  emit(std.http.streamEnd())
  return null
}
"#,
        r#"http:
  typed:
    method: POST
    path: /typed
    kind: typedJson
    handler: main.typed
    guard: main.guard
    pre: main.prepare
    adapterArgs:
      - param: request
        source: { kind: http.request }
      - param: body
        source: { kind: http.body }
      - param: bodyAgain
        source: { kind: http.body }
      - param: context
        source: { kind: http.context }
  boxed:
    method: POST
    path: /boxed
    kind: typedJson
    handler: main.boxed
    adapterArgs:
      - param: body
        source: { kind: http.body }
  raw:
    method: GET
    path: /raw
    kind: rawHttp
    handler: main.raw
    adapterArgs:
      - param: request
        source: { kind: http.request }
  rawStream:
    method: GET
    path: /raw-stream
    kind: rawHttp
    handler: main.rawStream
    adapterArgs:
      - param: request
        source: { kind: http.request }
"#,
    );
    let deployment = fixture.generate().expect("all HTTP modes must project");

    assert!(fixture.api.contract.operations.is_empty());
    assert!(deployment.operation_bindings.is_empty());
    assert_eq!(deployment.gateway_entries.len(), 4);
    assert_eq!(deployment.ingress.len(), 4);
    assert_eq!(
        fixture
            .project
            .package
            .artifact
            .package_local_abi
            .public_symbols
            .len(),
        1,
        "private ingress must not expand the Package public surface"
    );
    assert!(fixture
        .project
        .package
        .artifact
        .package_schema_type_records
        .is_empty());

    let typed = http_surface(&deployment, "typed");
    assert_eq!(typed.adapter_kind, GatewayAdapterKind::TypedJson);
    assert_eq!(typed.dispatch_mode, GatewayDispatchMode::Unary);
    let GatewayExternalSchema::Record { fields, required } = typed
        .request_body_schema
        .as_ref()
        .expect("typed body schema")
    else {
        panic!("typed body must project the private Input record")
    };
    assert_eq!(
        fields.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["id", "name", "note", "status", "tags"]
    );
    assert_eq!(
        required,
        &[
            "id".to_string(),
            "name".to_string(),
            "status".to_string(),
            "tags".to_string()
        ]
    );
    assert_eq!(fields["id"], GatewayExternalSchema::String);
    assert_eq!(fields["name"], GatewayExternalSchema::String);
    assert!(matches!(
        fields["note"],
        GatewayExternalSchema::Nullable { .. }
    ));
    assert!(matches!(
        fields["status"],
        GatewayExternalSchema::ClosedUnion { .. }
    ));
    assert!(matches!(
        fields["tags"],
        GatewayExternalSchema::Array { .. }
    ));

    let boxed = http_surface(&deployment, "boxed");
    assert!(matches!(
        boxed.request_body_schema,
        Some(GatewayExternalSchema::Record { ref fields, .. })
            if matches!(
                fields["item"],
                GatewayExternalSchema::Record { ref fields, .. }
                    if fields["value"] == GatewayExternalSchema::String
            )
    ));

    let typed_surfaces = deployment
        .gateway_entries
        .values()
        .filter_map(|entry| match &entry.protocol_surface.protocol {
            GatewayProtocolSurface::Http(surface)
                if surface.adapter_kind == GatewayAdapterKind::TypedJson =>
            {
                Some(surface)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(typed_surfaces.len(), 2);
    for surface in typed_surfaces {
        assert_eq!(surface.dispatch_mode, GatewayDispatchMode::Unary);
        assert!(surface.response_schema.is_some());
        assert!(surface.stream_item_schema.is_none());
    }

    let raw = http_surface(&deployment, "raw");
    assert_eq!(raw.adapter_kind, GatewayAdapterKind::RawHttp);
    assert_eq!(raw.dispatch_mode, GatewayDispatchMode::Unary);
    assert!(raw.response_schema.is_none());

    let raw_stream = http_surface(&deployment, "rawStream");
    assert_eq!(raw_stream.dispatch_mode, GatewayDispatchMode::ServerStream);
    assert!(matches!(
        raw_stream.stream_item_schema,
        Some(GatewayExternalSchema::ClosedUnion { .. })
    ));
}

#[test]
fn typed_json_streams_fail_before_item_schema_projection() {
    let fixture = compile_fixture(
        "typed-stream-negative",
        "health: main.health\n",
        r#"function health() -> string { return "ok" }

type Input { value: string }
type Output { accepted: boolean }

function eligible(body: Input) -> Stream<Output> {
  emit({ accepted: true })
  return null
}

function unprojectable(body: Input) -> Stream<Map<string, string>> {
  return null
}
"#,
        "http: {}\n",
    );
    let expected = "typedJson supports only unary handler returns; HTTP streaming requires rawHttp + Stream<std.http.HttpResponseStreamEvent>";
    let errors = ["main.eligible", "main.unprojectable"].map(|handler| {
        let service = parse_service(&typed_http(handler, "body"));
        fixture
            .generate_error(&service, &fixture.project.package.artifact)
            .to_string()
    });
    for error in &errors {
        assert!(error.contains(expected), "{error}");
    }
    assert_eq!(
        errors[0], errors[1],
        "typedJson stream rejection must depend on adapter kind and outer return, not item schema"
    );
}

#[test]
fn service_call_and_http_gateway_keep_distinct_identity_domains() {
    let fixture = compile_fixture(
        "dual-surface",
        "health: main.health\ndual:\n  source: main.dual\n  serviceCall: true\n",
        r#"function health() -> string { return "ok" }
function dual(body: string) -> string { return "ok" }
"#,
        typed_http("main.dual", "body"),
    );
    let deployment = fixture.generate().expect("dual surface must project");
    assert_eq!(fixture.api.contract.operations.len(), 1);
    assert_eq!(deployment.operation_bindings.len(), 1);
    assert_eq!(deployment.gateway_entries.len(), 1);
    let gateway = &deployment.gateway_entries[&gateway_key("typed")];
    let operation = fixture.api.contract.operations.keys().next().unwrap();
    assert_ne!(operation.as_str(), gateway.gateway_entry_identity.as_str());
    let service_call_id = &deployment.operation_bindings[0].package_callable_id;
    assert_ne!(
        service_call_id, &gateway.handler,
        "public service-call and private implementation callable IDs are distinct identity domains"
    );
    let service_call_target =
        &fixture.project.package.artifact.callable_links[service_call_id].target;
    let gateway_target = &fixture.project.package.artifact.callable_links[&gateway.handler].target;
    assert_eq!(service_call_target.file_ref, gateway_target.file_ref);
    assert_eq!(
        service_call_target.executable_index, gateway_target.executable_index,
        "both protocol surfaces must still resolve to the same source executable"
    );
}

#[test]
fn selector_body_shape_implementation_and_adapter_plan_obey_identity_boundaries() {
    let base = compile_fixture(
        "identity-base",
        "health: main.health\n",
        typed_identity_source("return Output { value: body.value }", "string"),
        typed_http("main.typed", "body"),
    );
    let base_deployment = base.generate().unwrap();
    let base_gateway = gateway_identity(&base_deployment, "typed").to_string();

    let mut selector_service = base.service.clone();
    let selector = selector_service
        .http
        .as_mut()
        .unwrap()
        .get_mut(&gateway_key("typed"))
        .unwrap();
    selector.host = "api.example.com".to_string();
    selector.method = "PUT".to_string();
    selector.path = "/moved".to_string();
    let selector_deployment = base.generate_with(&selector_service, &base.project.package.artifact);
    assert_eq!(
        gateway_identity(&selector_deployment, "typed"),
        base_gateway
    );
    assert_eq!(
        selector_deployment.contract.service_protocol_identity,
        base_deployment.contract.service_protocol_identity
    );
    assert_ne!(
        selector_deployment.deployment_artifact_identity,
        base_deployment.deployment_artifact_identity
    );

    let body_changed = compile_fixture(
        "identity-body-change",
        "health: main.health\n",
        typed_identity_source(
            "const ignored = body.value\nreturn Output { value: \"changed\" }",
            "string",
        ),
        typed_http("main.typed", "body"),
    );
    let body_deployment = body_changed.generate().unwrap();
    assert_ne!(
        body_changed.project.package.artifact.package_build_id,
        base.project.package.artifact.package_build_id
    );
    assert_eq!(
        body_changed
            .project
            .package
            .artifact
            .package_local_abi
            .local_abi_identity,
        base.project
            .package
            .artifact
            .package_local_abi
            .local_abi_identity
    );
    assert_eq!(
        body_deployment.contract.service_protocol_identity,
        base_deployment.contract.service_protocol_identity
    );
    assert_eq!(gateway_identity(&body_deployment, "typed"), base_gateway);
    assert_ne!(
        body_deployment.deployment_artifact_identity,
        base_deployment.deployment_artifact_identity
    );

    let shape_changed = compile_fixture(
        "identity-shape-change",
        "health: main.health\n",
        typed_identity_source("return Output { value: body.value }", "integer"),
        typed_http("main.typed", "body"),
    );
    let shape_deployment = shape_changed.generate().unwrap();
    assert_eq!(
        shape_deployment.contract.service_protocol_identity,
        base_deployment.contract.service_protocol_identity
    );
    assert_ne!(gateway_identity(&shape_deployment, "typed"), base_gateway);
    assert_ne!(
        shape_deployment.deployment_artifact_identity,
        base_deployment.deployment_artifact_identity
    );

    let names = compile_fixture(
        "identity-param-name",
        "health: main.health\n",
        r#"function health() -> string { return "ok" }
type Input { value: string }
type Output { value: string }
function first(body: Input) -> Output { return { value: body.value } }
function second(payload: Input) -> Output { return { value: payload.value } }
"#,
        typed_http("main.first", "body"),
    );
    let first = names.generate().unwrap();
    let mut second_service = names.service.clone();
    let entry = second_service
        .http
        .as_mut()
        .unwrap()
        .get_mut(&gateway_key("typed"))
        .unwrap();
    entry.handler = "main.second".to_string();
    entry.adapter_args[0].param = "payload".to_string();
    let second = names.generate_with(&second_service, &names.project.package.artifact);
    assert_eq!(
        gateway_identity(&first, "typed"),
        gateway_identity(&second, "typed")
    );
    assert_ne!(
        first.deployment_artifact_identity,
        second.deployment_artifact_identity
    );
}

#[test]
fn resolver_and_signature_mismatches_fail_closed_without_public_fallback() {
    let fixture = compile_fixture(
        "resolver-negative",
        "health: main.health\npublicRaw: main.raw\n",
        raw_source("\"ok\""),
        raw_http("main.raw", "request"),
    );

    for (label, selector, expected) in [
        ("missing", "main.missing", "implementationSymbols"),
        ("wrong-kind", "main.Context", "not a top-level function"),
        (
            "public-path",
            "root.publicRaw",
            "current-package source selector",
        ),
    ] {
        let mut service = fixture.service.clone();
        service
            .http
            .as_mut()
            .unwrap()
            .get_mut(&gateway_key("raw"))
            .unwrap()
            .handler = selector.to_string();
        let error = fixture.generate_error(&service, &fixture.project.package.artifact);
        assert!(error.to_string().contains(expected), "{label}: {error}");
    }

    let mut forged = fixture.project.package.artifact.clone();
    let callable = implementation_callable_id(&forged, "main.raw");
    forged
        .callable_links
        .get_mut(&callable)
        .unwrap()
        .target
        .callable_abi_id = "forged".to_string();
    let error = fixture.generate_error(&fixture.service, &forged);
    assert!(
        error.to_string().contains("identity") || error.to_string().contains("callable"),
        "{error}"
    );
}

#[test]
fn generic_callables_and_invalid_adapter_signatures_fail_closed() {
    let fixture = compile_fixture(
        "signature-negative",
        "health: main.health\n",
        r#"import std

type Context { id: string }
type Input { value: string }

function health() -> string { return "ok" }
function generic<T>(body: T) -> string { return "x" }
function genericPre<T>(request: std.http.HttpRequest) -> T {
  return std.json.decode<T>("null")
}
function genericGuard<T>(request: std.http.HttpRequest) -> std.http.HttpResponse? { return null }
function prepare(request: std.http.HttpRequest) -> Context { return { id: "x" } }
function wrongPrepare(value: string) -> Context { return { id: value } }
function wrongGuard(request: string) -> std.http.HttpResponse? { return null }
function two(a: string, b: integer) -> string { return a }
function request(value: string) -> string { return value }
function context(body: Input, value: string) -> string { return body.value + value }
function typed(body: Input) -> string { return body.value }
function raw(request: std.http.HttpRequest) -> string { return request.path }
function rawBody(
  request: std.http.HttpRequest,
  body: Input
) -> std.http.HttpResponse {
  return std.http.noContent()
}
function nullableRaw(request: std.http.HttpRequest) -> std.http.HttpResponse? { return null }
function rawStream(request: std.http.HttpRequest) -> Stream<string> {
  emit(request.path)
  return null
}
"#,
        typed_http("main.typed", "body"),
    );

    let cases = [
        (
            "generic-handler",
            typed_http("main.generic", "body"),
            "generic parameters",
        ),
        (
            "missing-formal",
            "http:\n  typed:\n    method: POST\n    path: /typed\n    kind: typedJson\n    handler: main.typed\n    adapterArgs: []\n".to_string(),
            "cover every handler formal",
        ),
        (
            "unknown-formal",
            typed_http("main.typed", "unknown"),
            "cover every handler formal",
        ),
        (
            "duplicate-formal",
            "http:\n  typed:\n    method: POST\n    path: /typed\n    kind: typedJson\n    handler: main.typed\n    adapterArgs:\n      - param: body\n        source: { kind: http.body }\n      - param: body\n        source: { kind: http.body }\n".to_string(),
            "cover every handler formal exactly once",
        ),
        (
            "same-source-different-type",
            "http:\n  typed:\n    method: POST\n    path: /typed\n    kind: typedJson\n    handler: main.two\n    adapterArgs:\n      - param: a\n        source: { kind: http.body }\n      - param: b\n        source: { kind: http.body }\n".to_string(),
            "incompatible exact formal types",
        ),
        (
            "request-mismatch",
            "http:\n  typed:\n    method: POST\n    path: /typed\n    kind: typedJson\n    handler: main.request\n    adapterArgs:\n      - param: value\n        source: { kind: http.request }\n".to_string(),
            "HttpRequest",
        ),
        (
            "context-without-pre",
            "http:\n  typed:\n    method: POST\n    path: /typed\n    kind: typedJson\n    handler: main.context\n    adapterArgs:\n      - param: body\n        source: { kind: http.body }\n      - param: value\n        source: { kind: http.context }\n".to_string(),
            "requires an entry-local pre",
        ),
        (
            "typed-missing-body",
            "http:\n  typed:\n    method: GET\n    path: /typed\n    kind: typedJson\n    handler: main.health\n    adapterArgs: []\n".to_string(),
            "requires at least one http.body",
        ),
        (
            "raw-return",
            "http:\n  raw:\n    method: GET\n    path: /raw\n    kind: rawHttp\n    handler: main.raw\n    adapterArgs:\n      - param: request\n        source: { kind: http.request }\n".to_string(),
            "HttpResponse",
        ),
        (
            "raw-body",
            "http:\n  raw:\n    method: POST\n    path: /raw\n    kind: rawHttp\n    handler: main.rawBody\n    adapterArgs:\n      - param: request\n        source: { kind: http.request }\n      - param: body\n        source: { kind: http.body }\n".to_string(),
            "rawHttp cannot consume http.body",
        ),
        (
            "nullable-raw-response",
            raw_http("main.nullableRaw", "request"),
            "HttpResponse",
        ),
        (
            "raw-stream-item",
            "http:\n  raw:\n    method: GET\n    path: /raw\n    kind: rawHttp\n    handler: main.rawStream\n    adapterArgs:\n      - param: request\n        source: { kind: http.request }\n".to_string(),
            "HttpResponseStreamEvent",
        ),
    ];
    for (label, http, expected) in cases {
        let service = parse_service(&http);
        let error = fixture.generate_error(&service, &fixture.project.package.artifact);
        assert!(error.to_string().contains(expected), "{label}: {error}");
    }

    for (field, selector) in [("pre", "main.genericPre"), ("guard", "main.genericGuard")] {
        let mut service = parse_service(&typed_http("main.typed", "body"));
        let entry = service
            .http
            .as_mut()
            .unwrap()
            .get_mut(&gateway_key("typed"))
            .unwrap();
        if field == "pre" {
            entry.pre = Some(selector.to_string());
        } else {
            entry.guard = Some(selector.to_string());
        }
        let error = fixture.generate_error(&service, &fixture.project.package.artifact);
        assert!(error.to_string().contains("generic parameters"), "{error}");
    }

    for (label, field, selector, expected) in [
        ("pre-parameter", "pre", "main.wrongPrepare", "HttpRequest"),
        ("guard-parameter", "guard", "main.wrongGuard", "HttpRequest"),
    ] {
        let mut service = parse_service(&typed_http("main.typed", "body"));
        let entry = service
            .http
            .as_mut()
            .unwrap()
            .get_mut(&gateway_key("typed"))
            .unwrap();
        if field == "pre" {
            entry.pre = Some(selector.to_string());
        } else {
            entry.guard = Some(selector.to_string());
        }
        let error = fixture.generate_error(&service, &fixture.project.package.artifact);
        assert!(error.to_string().contains(expected), "{label}: {error}");
    }

    let context_mismatch = parse_service(
        "http:\n  typed:\n    method: POST\n    path: /typed\n    kind: typedJson\n    handler: main.context\n    pre: main.prepare\n    adapterArgs:\n      - param: body\n        source: { kind: http.body }\n      - param: value\n        source: { kind: http.context }\n",
    );
    let error = fixture.generate_error(&context_mismatch, &fixture.project.package.artifact);
    assert!(
        error
            .to_string()
            .contains("does not exactly match pre return type"),
        "{error}"
    );
}

#[test]
fn unsupported_external_types_and_recursive_expansion_fail_closed() {
    let fixture = compile_fixture(
        "schema-negative",
        "health: main.health\n",
        r#"function health() -> string { return "ok" }

type Node { value: string, next: Node? }

interface Reader {
  function read(self: Self) -> string
}

function mapBody(body: Map<string, string>) -> string { return "x" }
function recursive(body: Node) -> string { return body.value }
function callback(body: fn(value: string) -> string) -> string { return "x" }
function interfaceBody(body: any Reader) -> string { return "x" }
"#,
        "http: {}\n",
    );
    for (label, handler, expected) in [
        ("map", "main.mapBody", "Map"),
        ("recursive", "main.recursive", "recursive"),
        ("callback", "main.callback", "function"),
        ("interface", "main.interfaceBody", "interface"),
    ] {
        let service = parse_service(&typed_http(handler, "body"));
        let error = fixture.generate_error(&service, &fixture.project.package.artifact);
        assert!(error.to_string().contains(expected), "{label}: {error}");
    }
}

#[test]
fn dependency_schema_projection_validates_exact_owner_key_and_type_identity() {
    let fixture = compile_dependency_fixture();
    let deployment = fixture
        .generate()
        .expect("an exact dependency PackageSchema record must project");
    assert!(fixture.api.contract.operations.is_empty());
    let schema = http_surface(&deployment, "typed")
        .request_body_schema
        .as_ref()
        .expect("dependency request schema");
    assert!(matches!(
        schema,
        GatewayExternalSchema::Record { fields, required }
            if fields["value"] == GatewayExternalSchema::String
                && matches!(fields["note"], GatewayExternalSchema::Nullable { .. })
                && matches!(fields["detail"], GatewayExternalSchema::Record { .. })
                && required == &["detail".to_string(), "value".to_string()]
    ));

    let dependency_id = fixture
        .project
        .package
        .resolved_package_schema_type_records
        .iter()
        .find_map(|(id, record)| {
            (record.package_id == "example.com/http-gateway-models").then(|| id.clone())
        })
        .expect("dependency schema record");

    let mut wrong_owner = fixture
        .project
        .package
        .resolved_package_schema_type_records
        .clone();
    wrong_owner.get_mut(&dependency_id).unwrap().package_id =
        "example.com/forged-owner".to_string();
    assert_schema_records_fail_closed(&fixture, &wrong_owner, "owner");

    let mut wrong_key = fixture
        .project
        .package
        .resolved_package_schema_type_records
        .clone();
    wrong_key.get_mut(&dependency_id).unwrap().stable_schema_key = "ForgedPayload".to_string();
    assert_schema_records_fail_closed(&fixture, &wrong_key, "stable key");

    let mut wrong_id = fixture
        .project
        .package
        .resolved_package_schema_type_records
        .clone();
    wrong_id
        .get_mut(&dependency_id)
        .unwrap()
        .package_schema_type_id = PackageSchemaTypeId::new("package-schema:forged");
    assert_schema_records_fail_closed(&fixture, &wrong_id, "type identity");
}

fn raw_source(health_value: &str) -> String {
    format!(
        r#"import std
type Context {{ id: string }}
function health() -> string {{ return {health_value} }}
function raw(request: std.http.HttpRequest) -> std.http.HttpResponse {{
  return std.http.noContent()
}}
"#
    )
}

fn typed_identity_source(body: &str, value_type: &str) -> String {
    format!(
        r#"function health() -> string {{ return "ok" }}
type Input {{ value: {value_type} }}
type Output {{ value: {value_type} }}
function typed(body: Input) -> Output {{
  {body}
}}
"#
    )
}

fn raw_http(handler: &str, param: &str) -> String {
    format!(
        "http:\n  raw:\n    method: GET\n    path: /raw\n    kind: rawHttp\n    handler: {handler}\n    adapterArgs:\n      - param: {param}\n        source: {{ kind: http.request }}\n"
    )
}

fn typed_http(handler: &str, param: &str) -> String {
    format!(
        "http:\n  typed:\n    method: POST\n    path: /typed\n    kind: typedJson\n    handler: {handler}\n    adapterArgs:\n      - param: {param}\n        source: {{ kind: http.body }}\n"
    )
}

struct Fixture {
    project: PublishedPackageProject,
    api: ServiceApiProjection,
    service: ServiceManifestAuthoring,
}

impl Fixture {
    fn closure(&self) -> Vec<skiff_artifact_model::PackageArtifact> {
        self.project
            .dependency_packages
            .iter()
            .map(|package| package.artifact.clone())
            .collect()
    }

    fn generate(&self) -> Result<ServiceDeployment, GeneratedServiceDeploymentError> {
        self.generate_result(&self.service, &self.project.package.artifact)
    }

    fn generate_with(
        &self,
        service: &ServiceManifestAuthoring,
        implementation: &skiff_artifact_model::PackageArtifact,
    ) -> ServiceDeployment {
        self.generate_result(service, implementation).unwrap()
    }

    fn generate_error(
        &self,
        service: &ServiceManifestAuthoring,
        implementation: &skiff_artifact_model::PackageArtifact,
    ) -> GeneratedServiceDeploymentError {
        self.generate_result(service, implementation).unwrap_err()
    }

    fn generate_result(
        &self,
        service: &ServiceManifestAuthoring,
        implementation: &skiff_artifact_model::PackageArtifact,
    ) -> Result<ServiceDeployment, GeneratedServiceDeploymentError> {
        self.generate_result_with_records(
            service,
            implementation,
            &self.project.package.resolved_package_schema_type_records,
        )
    }

    fn generate_result_with_records(
        &self,
        service: &ServiceManifestAuthoring,
        implementation: &skiff_artifact_model::PackageArtifact,
        package_schema_records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    ) -> Result<ServiceDeployment, GeneratedServiceDeploymentError> {
        let closure = self.closure();
        generate_service_deployment(GeneratedServiceDeploymentInput {
            service,
            profile_name: "dev",
            profile: &profile(),
            service_api: &self.api,
            implementation,
            package_closure: &closure,
            package_schema_records,
        })
    }
}

fn compile_fixture(
    name: &str,
    api: &str,
    source: impl AsRef<str>,
    service_fields: impl AsRef<str>,
) -> Fixture {
    let root = TestDir::new("skiff-compiler-http-gateway", name);
    root.write("package.yml", format!("id: {PACKAGE_ID}\nversion: 1.0.0\n"));
    root.write("api.yml", api);
    root.write(
        "service.yml",
        format!("id: {SERVICE_ID}\n{}", service_fields.as_ref()),
    );
    root.write("main.skiff", source.as_ref());
    let service = read_service_package_root(root.path())
        .expect("fixture service authoring")
        .service;
    let (project, api) =
        compile_service_package_project(root.path()).expect("fixture source compilation");
    Fixture {
        project,
        api,
        service,
    }
}

fn compile_dependency_fixture() -> Fixture {
    let root = TestDir::new("skiff-compiler-http-gateway", "dependency-package-schema");
    root.write(
        "package.yml",
        format!(
            "id: {PACKAGE_ID}\nversion: 1.0.0\npackages:\n  - id: example.com/http-gateway-models\n    version: 1.0.0\n    alias: models\n"
        ),
    );
    root.write("api.yml", "health: main.health\n");
    root.write(
        "service.yml",
        format!("id: {SERVICE_ID}\n{}", typed_http("main.typed", "body")),
    );
    root.write(
        "main.skiff",
        r#"import models

function health() -> string { return "ok" }
function typed(body: models.Payload) -> models.Payload { return body }
"#,
    );
    root.write(
        ".skiff-packages/example~com~~http-gateway-models/1.0.0/package.yml",
        "id: example.com/http-gateway-models\nversion: 1.0.0\npackages:\n  - id: example.com/http-gateway-primitives\n    version: 1.0.0\n    alias: primitives\n",
    );
    root.write(
        ".skiff-packages/example~com~~http-gateway-models/1.0.0/api.yml",
        "Payload: models.Payload\n",
    );
    root.write(
        ".skiff-packages/example~com~~http-gateway-models/1.0.0/models.skiff",
        "import primitives\n\ntype Payload { value: string, note: string?, detail: primitives.Detail }\n",
    );
    root.write(
        ".skiff-packages/example~com~~http-gateway-primitives/1.0.0/package.yml",
        "id: example.com/http-gateway-primitives\nversion: 1.0.0\n",
    );
    root.write(
        ".skiff-packages/example~com~~http-gateway-primitives/1.0.0/api.yml",
        "Detail: detail.Detail\n",
    );
    root.write(
        ".skiff-packages/example~com~~http-gateway-primitives/1.0.0/detail.skiff",
        "type Detail { code: integer }\n",
    );
    let service = read_service_package_root(root.path())
        .expect("dependency fixture service authoring")
        .service;
    let (project, api) = compile_service_package_project(root.path())
        .expect("dependency fixture source compilation");
    Fixture {
        project,
        api,
        service,
    }
}

fn assert_schema_records_fail_closed(
    fixture: &Fixture,
    records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    label: &str,
) {
    let error = fixture
        .generate_result_with_records(&fixture.service, &fixture.project.package.artifact, records)
        .expect_err("forged dependency schema facts must fail closed");
    assert!(
        error.to_string().contains("schema")
            || error.to_string().contains("Schema")
            || error.to_string().contains("identity"),
        "{label}: {error}"
    );
}

fn parse_service(http: &str) -> ServiceManifestAuthoring {
    serde_yaml::from_str(&format!("id: {SERVICE_ID}\n{http}")).unwrap()
}

fn profile() -> ServiceConfigProfileAuthoring {
    ServiceConfigProfileAuthoring {
        config: json!({}),
        secrets: json!({}),
        state: json!({}),
        resources: json!({}),
        timeout: json!(1000),
        quota: json!({"cpuMillis": 100, "memoryBytes": 1048576}),
        principal: json!("service:http-gateway"),
        lifecycle: json!({"maxConcurrency": 4}),
    }
}

fn gateway_key(value: &str) -> skiff_artifact_model::GatewayEntryKey {
    skiff_artifact_model::GatewayEntryKey::parse(value).unwrap()
}

fn http_surface<'a>(
    deployment: &'a ServiceDeployment,
    key: &str,
) -> &'a skiff_artifact_model::GatewayHttpProtocolSurface {
    match &deployment.gateway_entries[&gateway_key(key)]
        .protocol_surface
        .protocol
    {
        GatewayProtocolSurface::Http(surface) => surface,
    }
}

fn gateway_identity<'a>(deployment: &'a ServiceDeployment, key: &str) -> &'a str {
    deployment.gateway_entries[&gateway_key(key)]
        .gateway_entry_identity
        .as_str()
}

fn implementation_callable_id(
    artifact: &skiff_artifact_model::PackageArtifact,
    selector: &str,
) -> skiff_artifact_model::PackageCallableId {
    let PackageLocalAbiSymbol::Callable { callable_id, .. } =
        &artifact.package_local_abi.implementation_symbols[selector]
    else {
        panic!("{selector} must be a callable")
    };
    callable_id.clone()
}
