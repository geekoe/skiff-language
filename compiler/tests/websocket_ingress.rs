mod common;

use common::{
    package_project::{compile_service_package_project, PublishedPackageProject},
    TestDir,
};
use serde_json::json;
use skiff_artifact_model::{
    GatewayAdapterKind, GatewayAdapterSource, GatewayEntryKey, GatewayProtocolSurface,
    GatewayWebSocketDownlinkFrame, IngressProtocol, PackageLocalAbiSymbol, PackageTypeRef,
    ServiceConfigProfileAuthoring, ServiceDeployment, ServiceManifestAuthoring, TypeRefIr,
    WEBSOCKET_GATEWAY_ENTRY_KEY,
};
use skiff_compiler::{
    generate_service_deployment, GeneratedServiceDeploymentError, GeneratedServiceDeploymentInput,
    ServiceApiProjection,
};
use skiff_compiler_input::read_service_package_root;
use skiff_deployment::assembly::resolve_runtime_assembly;

const PACKAGE_ID: &str = "example.com/websocket-provider";
const SERVICE_ID: &str = "example.com/websocket";

#[test]
fn connect_only_websocket_projects_exact_deployment_and_assembly_entry() {
    let fixture = compile_fixture(
        "connect-positive",
        "health: main.health\n",
        connect_source(),
        connect_authoring("main.onConnect", "request", "connectionId"),
    );
    let deployment = fixture.generate().expect("WebSocket connect projection");
    let key = websocket_key();
    let entry = &deployment.gateway_entries[&key];

    assert!(fixture.api.contract.operations.is_empty());
    assert!(deployment.operation_bindings.is_empty());
    assert_eq!(deployment.gateway_entries.len(), 1);
    assert_eq!(deployment.ingress.len(), 1);
    assert_eq!(deployment.ingress[0].gateway_entry_key, key);
    assert_eq!(
        deployment.ingress[0].selector.protocol,
        IngressProtocol::WebSocket
    );
    assert_eq!(deployment.ingress[0].selector.host, "*");
    assert_eq!(deployment.ingress[0].selector.path, "/chat");
    assert_eq!(deployment.ingress[0].selector.method, None);
    assert!(entry.pre.is_none());
    assert!(entry.guard.is_none());
    assert_eq!(
        entry.adapter_plan.kind,
        GatewayAdapterKind::WebSocketConnect
    );
    assert_eq!(
        entry
            .adapter_plan
            .args
            .iter()
            .map(|arg| (arg.param.as_str(), arg.source))
            .collect::<Vec<_>>(),
        vec![
            ("request", GatewayAdapterSource::WebSocketConnectRequest),
            ("connectionId", GatewayAdapterSource::WebSocketConnectionId)
        ]
    );
    let GatewayProtocolSurface::WebSocketConnect(surface) = &entry.protocol_surface.protocol else {
        panic!("WebSocket entry must have the websocketConnect surface");
    };
    assert_eq!(
        surface.external_sources,
        vec![
            GatewayAdapterSource::WebSocketConnectRequest,
            GatewayAdapterSource::WebSocketConnectionId
        ]
    );
    assert_eq!(
        surface.downlink_frames,
        vec![
            GatewayWebSocketDownlinkFrame::Binary,
            GatewayWebSocketDownlinkFrame::Text
        ]
    );
    assert_eq!(
        entry.gateway_entry_identity.as_str(),
        "skiff-gateway-entry-v1:sha256:d32884370c32e2a3923cbc7245d30c5a56c68b272825cde3645a1a48b49a5936"
    );

    let PackageLocalAbiSymbol::Callable {
        callable_id: implementation_callable,
        ..
    } = &fixture
        .project
        .package
        .artifact
        .package_local_abi
        .implementation_symbols["main.onConnect"]
    else {
        panic!("connect implementation symbol must be callable");
    };
    assert_eq!(entry.handler.as_ref(), Some(implementation_callable));

    let deployment_ref = skiff_artifact_identity::service_deployment_ref(&deployment);
    let mut packages = fixture.closure();
    packages.push(fixture.project.package.artifact.clone());
    let assembly = resolve_runtime_assembly(
        std::slice::from_ref(&deployment_ref),
        std::slice::from_ref(&deployment),
        std::slice::from_ref(&fixture.api.contract),
        &packages,
    )
    .expect("WebSocket deployment must resolve into RuntimeAssembly");
    assert_eq!(assembly.gateway_ingress.len(), 1);
    assert_eq!(
        assembly.gateway_ingress[0].selector,
        deployment.ingress[0].selector
    );
    assert_eq!(assembly.gateway_ingress[0].gateway_entry_key, key);
    assert_eq!(
        assembly.gateway_ingress[0].gateway_entry_identity,
        entry.gateway_entry_identity
    );
    assert_eq!(assembly.gateway_ingress[0].deployment, deployment_ref);
    skiff_artifact_identity::validate_runtime_assembly_identity(&assembly).unwrap();
}

#[test]
fn path_only_and_connect_variants_preserve_protocol_identity_boundaries() {
    let fixture = compile_fixture(
        "identity-boundaries",
        "health: main.health\npublicConnect: main.onConnect\n",
        connect_source(),
        connect_authoring("main.onConnect", "request", "connectionId"),
    );
    let connected = fixture.generate().unwrap();
    let connected_entry = &connected.gateway_entries[&websocket_key()];

    let mut path_only_service = fixture.service.clone();
    let websocket = path_only_service.websocket.as_mut().unwrap();
    websocket.path = "/other".to_string();
    websocket.connect = None;
    let path_only = fixture.generate_with(&path_only_service).unwrap();
    let path_only_entry = &path_only.gateway_entries[&websocket_key()];
    assert!(path_only_entry.handler.is_none());
    assert!(path_only_entry.adapter_plan.args.is_empty());
    assert_eq!(
        path_only_entry.gateway_entry_identity, connected_entry.gateway_entry_identity,
        "fixed external connect surface must not depend on handler, selected args, or path"
    );
    assert_eq!(
        path_only.contract.service_protocol_identity,
        connected.contract.service_protocol_identity
    );
    assert_ne!(
        path_only.deployment_revision, connected.deployment_revision,
        "authoring changes remain deployment-identifying"
    );
    assert_ne!(
        path_only.deployment_artifact_identity,
        connected.deployment_artifact_identity
    );

    let mut absent_service = fixture.service.clone();
    absent_service.websocket = None;
    let absent = fixture.generate_with(&absent_service).unwrap();
    assert!(absent.gateway_entries.is_empty());
    assert!(absent.ingress.is_empty());
    assert_eq!(
        absent.contract.service_protocol_identity, connected.contract.service_protocol_identity,
        "WebSocket authoring must not enter ServiceContract identity"
    );
    assert_eq!(fixture.api.contract.operations.len(), 0);

    let PackageLocalAbiSymbol::Callable {
        callable_id: public_callable,
        ..
    } = &fixture
        .project
        .package
        .artifact
        .package_local_abi
        .public_symbols["publicConnect"]
    else {
        panic!("publicConnect must be callable");
    };
    let implementation_callable = connected_entry.handler.as_ref().unwrap();
    assert_ne!(
        public_callable, implementation_callable,
        "gateway resolution must use the current-package implementation identity lane"
    );
    let public_target = &fixture.project.package.artifact.callable_links[public_callable].target;
    let implementation_target =
        &fixture.project.package.artifact.callable_links[implementation_callable].target;
    assert_eq!(public_target.file_ref, implementation_target.file_ref);
    assert_eq!(
        public_target.executable_index,
        implementation_target.executable_index
    );
}

#[test]
fn resolver_adapter_and_signature_mismatches_fail_closed() {
    let fixture = compile_fixture(
        "negative-signatures",
        "health: main.health\n",
        negative_source(),
        "websocket:\n  path: /chat\n",
    );

    let cases = [
        (
            "missing callable",
            connect_authoring("main.missing", "request", "connectionId"),
            "implementationSymbols",
        ),
        (
            "non-callable",
            connect_authoring("main.Context", "request", "connectionId"),
            "not a top-level function",
        ),
        (
            "generic callable",
            connect_authoring("main.generic", "request", "connectionId"),
            "generic parameters",
        ),
        (
            "wrong request",
            connect_authoring("main.wrongRequest", "request", "connectionId"),
            "WebSocketConnectRequest",
        ),
        (
            "wrong connection id",
            connect_authoring("main.wrongConnectionId", "request", "connectionId"),
            "builtin string",
        ),
        (
            "nullable result",
            connect_authoring("main.nullableResult", "request", "connectionId"),
            "WebSocketConnectResult",
        ),
        (
            "wrong result",
            connect_authoring("main.wrongResult", "request", "connectionId"),
            "WebSocketConnectResult",
        ),
        (
            "missing formal",
            r#"websocket:
  path: /chat
  connect:
    handler: main.onConnect
    adapterArgs:
      - param: request
        source: { kind: websocket.connectRequest }
"#
            .to_string(),
            "cover every handler formal exactly once",
        ),
        (
            "unknown formal",
            connect_authoring("main.onConnect", "unknown", "connectionId"),
            "cover every handler formal exactly once",
        ),
        (
            "duplicate formal",
            r#"websocket:
  path: /chat
  connect:
    handler: main.onConnect
    adapterArgs:
      - param: request
        source: { kind: websocket.connectRequest }
      - param: request
        source: { kind: websocket.connectionId }
"#
            .to_string(),
            "cover every handler formal exactly once",
        ),
        (
            "reordered formals",
            r#"websocket:
  path: /chat
  connect:
    handler: main.onConnect
    adapterArgs:
      - param: connectionId
        source: { kind: websocket.connectionId }
      - param: request
        source: { kind: websocket.connectRequest }
"#
            .to_string(),
            "signature order",
        ),
        (
            "HTTP source",
            r#"websocket:
  path: /chat
  connect:
    handler: main.onConnect
    adapterArgs:
      - param: request
        source: { kind: http.request }
      - param: connectionId
        source: { kind: websocket.connectionId }
"#
            .to_string(),
            "not allowed for websocketConnect",
        ),
    ];

    for (label, authoring, expected) in cases {
        let service = parse_service(&authoring);
        let error = fixture.generate_with(&service).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "{label} expected {expected:?}, got {error}"
        );
    }
}

#[test]
fn fixed_key_collision_and_legacy_generic_std_types_fail_closed() {
    let collision = compile_fixture(
        "fixed-key-collision",
        "health: main.health\n",
        connect_source(),
        format!(
            r#"http:
  websocket:
    method: GET
    path: /http
    kind: rawHttp
    handler: main.raw
    adapterArgs:
      - param: request
        source: {{ kind: http.request }}
{}"#,
            connect_authoring("main.onConnect", "request", "connectionId")
        ),
    );
    let error = collision.generate().unwrap_err();
    assert!(
        error.to_string().contains("collid") || error.to_string().contains("already exists"),
        "{error}"
    );

    let root = TestDir::new("skiff-compiler-websocket", "legacy-generic-result");
    root.write("package.yml", format!("id: {PACKAGE_ID}\nversion: 1.0.0\n"));
    root.write("api.yml", "{}\n");
    root.write("service.yml", format!("id: {SERVICE_ID}\n"));
    root.write(
        "main.skiff",
        r#"import std
function legacy() -> std.websocket.WebSocketConnectResult<string> {
  return std.json.decode<std.websocket.WebSocketConnectResult<string>>("{}")
}
"#,
    );
    let error = compile_service_package_project(root.path())
        .expect_err("WebSocketConnectResult must be non-generic");
    assert!(
        error.to_string().contains("expects 0 type arguments")
            || error.to_string().contains("type arguments"),
        "{error}"
    );
}

#[test]
fn compiler_published_std_keeps_only_connect_shapes_and_exact_send_signatures() {
    let fixture = compile_fixture(
        "std-surface",
        "health: main.health\n",
        connect_source(),
        "websocket:\n  path: /chat\n",
    );
    let std = fixture
        .project
        .dependency_packages
        .iter()
        .find(|package| package.artifact.package_id == "skiff.run/std")
        .expect("compiler-owned std artifact");

    for retained in [
        "std.websocket.WebSocketConnectRequest",
        "std.websocket.WebSocketConnectionPolicy",
        "std.websocket.WebSocketConnectResult",
    ] {
        let Some(PackageLocalAbiSymbol::Type { type_params, .. }) =
            std.artifact.package_local_abi.public_symbols.get(retained)
        else {
            panic!("missing retained std type {retained}");
        };
        assert!(type_params.is_empty(), "{retained} must be non-generic");
    }
    for removed in [
        "std.websocket.TextConnectionMessage",
        "std.websocket.BinaryConnectionMessage",
        "std.websocket.ConnectionMessage",
        "std.websocket.WebSocketConnection",
        "std.websocket.WebSocketReceiveEvent",
        "std.websocket.WebSocketIngressEvent",
        "std.websocket.WebSocketCloseEvent",
    ] {
        assert!(
            !std.artifact
                .package_local_abi
                .public_symbols
                .contains_key(removed),
            "{removed} must not remain public"
        );
    }

    for (symbol, parameters, types) in [
        (
            "std.websocket.sendTextToConnection",
            ["connectionId", "text"],
            ["string", "string"],
        ),
        (
            "std.websocket.sendBinaryToConnection",
            ["connectionId", "value"],
            ["string", "bytes"],
        ),
        (
            "std.websocket.sendTextToBusinessIdentity",
            ["businessIdentity", "text"],
            ["string", "string"],
        ),
        (
            "std.websocket.sendBinaryToBusinessIdentity",
            ["businessIdentity", "value"],
            ["string", "bytes"],
        ),
    ] {
        let Some(PackageLocalAbiSymbol::Callable { signature, .. }) =
            std.artifact.package_local_abi.public_symbols.get(symbol)
        else {
            panic!("missing send callable {symbol}");
        };
        assert!(signature.type_params.is_empty(), "{symbol}");
        assert!(!signature.may_suspend, "{symbol}");
        assert_eq!(
            signature
                .parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>(),
            parameters
        );
        for (parameter, expected) in signature.parameters.iter().zip(types) {
            assert_builtin(&parameter.ty, expected, symbol);
        }
        assert_builtin(&signature.return_type, "void", symbol);
    }
}

fn connect_source() -> &'static str {
    r#"import std

function health() -> string { return "ok" }

function onConnect(
  request: std.websocket.WebSocketConnectRequest,
  connectionId: string
) -> std.websocket.WebSocketConnectResult {
  return {
    tag: "accept",
    businessIdentity: connectionId,
    connectionPolicy: null
  }
}

function raw(request: std.http.HttpRequest) -> std.http.HttpResponse {
  return std.http.noContent()
}
"#
}

fn negative_source() -> &'static str {
    r#"import std

type Context {}

function health() -> string { return "ok" }

function reject() -> std.websocket.WebSocketConnectResult {
  return { tag: "reject", code: 4000, reason: "no" }
}

function onConnect(
  request: std.websocket.WebSocketConnectRequest,
  connectionId: string
) -> std.websocket.WebSocketConnectResult {
  return reject()
}

function generic<T>(
  request: T,
  connectionId: string
) -> std.websocket.WebSocketConnectResult {
  return reject()
}

function wrongRequest(
  request: string,
  connectionId: string
) -> std.websocket.WebSocketConnectResult {
  return reject()
}

function wrongConnectionId(
  request: std.websocket.WebSocketConnectRequest,
  connectionId: integer
) -> std.websocket.WebSocketConnectResult {
  return reject()
}

function nullableResult(
  request: std.websocket.WebSocketConnectRequest,
  connectionId: string
) -> std.websocket.WebSocketConnectResult? {
  return null
}

function wrongResult(
  request: std.websocket.WebSocketConnectRequest,
  connectionId: string
) -> string {
  return connectionId
}
"#
}

fn connect_authoring(handler: &str, request: &str, connection_id: &str) -> String {
    format!(
        r#"websocket:
  path: /chat
  connect:
    handler: {handler}
    adapterArgs:
      - param: {request}
        source: {{ kind: websocket.connectRequest }}
      - param: {connection_id}
        source: {{ kind: websocket.connectionId }}
"#
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
        self.generate_with(&self.service)
    }

    fn generate_with(
        &self,
        service: &ServiceManifestAuthoring,
    ) -> Result<ServiceDeployment, GeneratedServiceDeploymentError> {
        let closure = self.closure();
        generate_service_deployment(GeneratedServiceDeploymentInput {
            service,
            profile_name: "dev",
            profile: &profile(),
            service_api: &self.api,
            implementation: &self.project.package.artifact,
            package_closure: &closure,
            package_schema_records: &self.project.package.resolved_package_schema_type_records,
        })
    }
}

fn compile_fixture(
    name: &str,
    api: &str,
    source: impl AsRef<str>,
    service_fields: impl AsRef<str>,
) -> Fixture {
    let root = TestDir::new("skiff-compiler-websocket", name);
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

fn parse_service(fields: &str) -> ServiceManifestAuthoring {
    serde_yaml::from_str(&format!("id: {SERVICE_ID}\n{fields}")).unwrap()
}

fn profile() -> ServiceConfigProfileAuthoring {
    ServiceConfigProfileAuthoring {
        config: json!({}),
        secrets: json!({}),
        state: json!({}),
        resources: json!({}),
        timeout: json!(1000),
        quota: json!({"cpuMillis": 100, "memoryBytes": 1048576}),
        principal: json!("service:websocket"),
        lifecycle: json!({"maxConcurrency": 4}),
    }
}

fn websocket_key() -> GatewayEntryKey {
    GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap()
}

fn assert_builtin(ty: &PackageTypeRef, expected: &str, symbol: &str) {
    assert!(
        matches!(
            ty,
            PackageTypeRef::Local {
                local_type: TypeRefIr::Builtin { name, args }
            } if name == expected && args.is_empty()
        ) || matches!(
            ty,
            PackageTypeRef::Container { name, arguments }
                if name == expected && arguments.is_empty()
        ),
        "{symbol} expected builtin {expected}, got {ty:?}"
    );
}
