mod common;

use common::{
    package_project::{compile_service_package_project, PublishedPackageProject},
    TestDir,
};
use skiff_artifact_model::{
    GatewayAdapterKind, GatewayAdapterSource, GatewayDispatchMode, GatewayEntryKey,
    GatewayExternalSchema, GatewayProtocolSurface, GatewayWebSocketDownlinkFrame,
    GatewayWebSocketRpcProfile, HttpGatewayDocumentAuthoring, IngressProtocol,
    PackageLocalAbiSymbol, PackageTypeRef, ServiceDeployment, ServiceManifestAuthoring, TypeRefIr,
    WebSocketGatewayDocumentAuthoring, WEBSOCKET_GATEWAY_ENTRY_KEY,
};
use skiff_compiler::{
    generate_service_deployment, GeneratedServiceDeploymentError, GeneratedServiceDeploymentInput,
    ServiceApiProjection,
};
use skiff_compiler_input::read_service_package_root;
use skiff_deployment::assembly::resolve_runtime_assembly;

const PACKAGE_ID: &str = "example.com/websocket-provider";
const SERVICE_ID: &str = "example.com/websocket";

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

fn json_rpc_source() -> &'static str {
    r#"import std

type Params { value: string }
type Result { accepted: boolean }

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

function status(
  params: Params,
  connectionId: string,
  businessIdentity: string?
) -> Result {
  return { accepted: true }
}

function acknowledge(params: Params) -> void {}

function raw(request: std.http.HttpRequest) -> std.http.HttpResponse {
  return std.http.noContent()
}

function genericStatus<T>(params: T) -> Result {
  return { accepted: true }
}

function scalarParams(params: string) -> Result {
  return { accepted: true }
}

function streamStatus(params: Params) -> Stream<Result> {
  emit({ accepted: true })
  return null
}

function wrongConnectionIdRpc(params: Params, connectionId: integer) -> Result {
  return { accepted: true }
}

function wrongBusinessIdentityRpc(params: Params, businessIdentity: string) -> Result {
  return { accepted: true }
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
        r#"path: /chat
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
    websocket: WebSocketGatewayDocumentAuthoring,
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
        self.generate_with(Some(&self.websocket))
    }

    fn generate_with(
        &self,
        websocket: Option<&WebSocketGatewayDocumentAuthoring>,
    ) -> Result<ServiceDeployment, GeneratedServiceDeploymentError> {
        let closure = self.closure();
        generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &self.service,
            http: None,
            websocket,
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
    websocket_yml: impl AsRef<str>,
) -> Fixture {
    let root = TestDir::new("skiff-compiler-websocket", name);
    root.write("package.yml", format!("id: {PACKAGE_ID}\nversion: 1.0.0\n"));
    root.write("api.yml", api);
    root.write("service.yml", format!("id: {SERVICE_ID}\n"));
    root.write("websocket.yml", websocket_yml.as_ref());
    root.write("main.skiff", source.as_ref());
    let authoring = read_service_package_root(root.path()).expect("fixture service authoring");
    let service = authoring.service;
    let websocket = authoring.websocket.expect("fixture WebSocket authoring");
    let (project, api) =
        compile_service_package_project(root.path()).expect("fixture source compilation");
    Fixture {
        project,
        api,
        service,
        websocket,
    }
}

fn parse_service(fields: &str) -> WebSocketGatewayDocumentAuthoring {
    serde_yaml::from_str(fields).unwrap()
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let GatewayProtocolSurface::WebSocketConnect(surface) = &entry.protocol_surface.protocol
        else {
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
            surface.rpc_profiles,
            vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text]
        );
        assert!(entry
            .gateway_entry_identity
            .as_str()
            .starts_with("skiff-gateway-entry-v2:sha256:"));

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
    fn declared_json_rpc_methods_project_independent_typed_unary_entries() {
        let fixture = compile_fixture(
            "json-rpc-positive",
            "health: main.health\n",
            json_rpc_source(),
            r#"path: /chat
connect:
  handler: main.onConnect
  adapterArgs:
    - param: request
      source: { kind: websocket.connectRequest }
    - param: connectionId
      source: { kind: websocket.connectionId }
jsonRpc:
  status:
    method: status.get
    handler: main.status
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
      - param: connectionId
        source: { kind: websocket.connectionId }
      - param: businessIdentity
        source: { kind: websocket.businessIdentity }
  acknowledge:
    method: status.acknowledge
    handler: main.acknowledge
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
"#,
        );
        let deployment = fixture.generate().expect("JSON-RPC method projection");
        assert!(fixture.api.contract.operations.is_empty());
        assert_eq!(deployment.gateway_entries.len(), 3);
        assert_eq!(deployment.ingress.len(), 3);

        let status_key = GatewayEntryKey::parse("status").unwrap();
        let status = &deployment.gateway_entries[&status_key];
        assert_eq!(
            status.adapter_plan.kind,
            GatewayAdapterKind::WebSocketJsonRpc
        );
        assert_eq!(
            status
                .adapter_plan
                .args
                .iter()
                .map(|argument| (argument.param.as_str(), argument.source))
                .collect::<Vec<_>>(),
            vec![
                ("params", GatewayAdapterSource::WebSocketJsonRpcParams),
                ("connectionId", GatewayAdapterSource::WebSocketConnectionId),
                (
                    "businessIdentity",
                    GatewayAdapterSource::WebSocketBusinessIdentity
                )
            ]
        );
        let GatewayProtocolSurface::WebSocketJsonRpc(surface) = &status.protocol_surface.protocol
        else {
            panic!("declared method must have the websocketJsonRpc surface")
        };
        assert_eq!(surface.profile, GatewayWebSocketRpcProfile::JsonRpc2_0Text);
        assert_eq!(surface.dispatch_mode, GatewayDispatchMode::Unary);
        assert_eq!(
            surface.external_sources,
            vec![
                GatewayAdapterSource::WebSocketBusinessIdentity,
                GatewayAdapterSource::WebSocketConnectionId,
                GatewayAdapterSource::WebSocketJsonRpcParams
            ]
        );
        assert!(matches!(
            surface.params_schema,
            GatewayExternalSchema::Record { .. }
        ));
        assert!(matches!(
            surface.result_schema,
            GatewayExternalSchema::Record { .. }
        ));
        let binding = deployment
            .ingress
            .iter()
            .find(|binding| binding.gateway_entry_key == status_key)
            .unwrap();
        assert_eq!(binding.selector.protocol, IngressProtocol::WebSocket);
        assert_eq!(binding.selector.path, "/chat");
        assert_eq!(binding.selector.method.as_deref(), Some("status.get"));

        let acknowledge =
            &deployment.gateway_entries[&GatewayEntryKey::parse("acknowledge").unwrap()];
        let GatewayProtocolSurface::WebSocketJsonRpc(surface) =
            &acknowledge.protocol_surface.protocol
        else {
            panic!("void method must have the websocketJsonRpc surface")
        };
        assert_eq!(surface.result_schema, GatewayExternalSchema::Null);
    }

    #[test]
    fn json_rpc_signature_source_and_return_mismatches_fail_closed() {
        let fixture = compile_fixture(
            "json-rpc-negative",
            "health: main.health\n",
            json_rpc_source(),
            "path: /chat\n",
        );
        for (label, handler, args, expected) in [
        (
            "generic",
            "main.genericStatus",
            "      - param: params\n        source: { kind: websocket.jsonRpcParams }\n",
            "generic parameters",
        ),
        (
            "scalar params",
            "main.scalarParams",
            "      - param: params\n        source: { kind: websocket.jsonRpcParams }\n",
            "top-level object or array",
        ),
        (
            "stream return",
            "main.streamStatus",
            "      - param: params\n        source: { kind: websocket.jsonRpcParams }\n",
            "only unary",
        ),
        (
            "wrong connection id",
            "main.wrongConnectionIdRpc",
            "      - param: params\n        source: { kind: websocket.jsonRpcParams }\n      - param: connectionId\n        source: { kind: websocket.connectionId }\n",
            "builtin string",
        ),
        (
            "non-null business identity",
            "main.wrongBusinessIdentityRpc",
            "      - param: params\n        source: { kind: websocket.jsonRpcParams }\n      - param: businessIdentity\n        source: { kind: websocket.businessIdentity }\n",
            "nullable",
        ),
        (
            "missing params",
            "main.status",
            "      - param: connectionId\n        source: { kind: websocket.connectionId }\n      - param: businessIdentity\n        source: { kind: websocket.businessIdentity }\n",
            "signature order",
        ),
    ] {
        let websocket = parse_service(&format!(
            "path: /chat\njsonRpc:\n  status:\n    method: status.get\n    handler: {handler}\n    adapterArgs:\n{args}"
        ));
        let error = fixture.generate_with(Some(&websocket)).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "{label}: expected {expected:?}, got {error}"
        );
    }
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

        let mut path_only_service = fixture.websocket.clone();
        path_only_service.path = "/other".to_string();
        path_only_service.connect = None;
        let path_only = fixture.generate_with(Some(&path_only_service)).unwrap();
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

        let absent = fixture.generate_with(None).unwrap();
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
        let public_target =
            &fixture.project.package.artifact.callable_links[public_callable].target;
        let implementation_target =
            &fixture.project.package.artifact.callable_links[implementation_callable].target;
        assert_eq!(public_target.file_ref, implementation_target.file_ref);
        assert_eq!(
            public_target.executable_index,
            implementation_target.executable_index
        );
    }

    #[test]
    fn json_rpc_method_rename_changes_selector_and_revision_but_not_gateway_identity() {
        let fixture = compile_fixture(
            "json-rpc-method-identity",
            "health: main.health\n",
            json_rpc_source(),
            r#"path: /chat
jsonRpc:
  status:
    method: status.get
    handler: main.status
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
      - param: connectionId
        source: { kind: websocket.connectionId }
      - param: businessIdentity
        source: { kind: websocket.businessIdentity }
"#,
        );
        let first = fixture.generate().unwrap();
        let status_key = GatewayEntryKey::parse("status").unwrap();
        let first_binding = first
            .ingress
            .iter()
            .find(|binding| binding.gateway_entry_key == status_key)
            .unwrap();
        assert_eq!(first_binding.selector.method.as_deref(), Some("status.get"));

        let mut renamed_authoring = fixture.websocket.clone();
        renamed_authoring
            .json_rpc
            .get_mut(&status_key)
            .unwrap()
            .method = "status.read".to_string();
        let renamed = fixture.generate_with(Some(&renamed_authoring)).unwrap();
        let renamed_binding = renamed
            .ingress
            .iter()
            .find(|binding| binding.gateway_entry_key == status_key)
            .unwrap();
        assert_eq!(
            renamed_binding.selector.method.as_deref(),
            Some("status.read")
        );
        assert_eq!(
            first.gateway_entries[&status_key].gateway_entry_identity,
            renamed.gateway_entries[&status_key].gateway_entry_identity,
            "external method is a deployment selector, not a gateway protocol identity input"
        );
        assert_ne!(first.deployment_revision, renamed.deployment_revision);
    }

    #[test]
    fn real_split_websocket_path_mutation_preserves_package_and_contract_bytes() {
        let root = TestDir::new("skiff-compiler-websocket", "real-file-mutation");
        root.write("package.yml", format!("id: {PACKAGE_ID}\nversion: 1.0.0\n"));
        root.write("api.yml", "health: main.health\n");
        root.write("service.yml", format!("id: {SERVICE_ID}\n"));
        root.write("main.skiff", connect_source());
        root.write(
            "websocket.yml",
            connect_authoring("main.onConnect", "request", "connectionId"),
        );

        let first_root = read_service_package_root(root.path()).unwrap();
        let (first_project, first_api) = compile_service_package_project(root.path()).unwrap();
        let first_closure = first_project
            .dependency_packages
            .iter()
            .map(|package| package.artifact.clone())
            .collect::<Vec<_>>();
        let first = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &first_root.service,
            http: None,
            websocket: first_root.websocket.as_ref(),
            service_api: &first_api,
            implementation: &first_project.package.artifact,
            package_closure: &first_closure,
            package_schema_records: &first_project.package.resolved_package_schema_type_records,
        })
        .unwrap();

        root.write(
            "websocket.yml",
            connect_authoring("main.onConnect", "request", "connectionId")
                .replace("path: /chat", "path: /moved"),
        );
        let second_root = read_service_package_root(root.path()).unwrap();
        let (second_project, second_api) = compile_service_package_project(root.path()).unwrap();
        let second_closure = second_project
            .dependency_packages
            .iter()
            .map(|package| package.artifact.clone())
            .collect::<Vec<_>>();
        let second = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &second_root.service,
            http: None,
            websocket: second_root.websocket.as_ref(),
            service_api: &second_api,
            implementation: &second_project.package.artifact,
            package_closure: &second_closure,
            package_schema_records: &second_project.package.resolved_package_schema_type_records,
        })
        .unwrap();

        assert_eq!(
            serde_json::to_vec(&first_project.package.artifact).unwrap(),
            serde_json::to_vec(&second_project.package.artifact).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&first_api.contract).unwrap(),
            serde_json::to_vec(&second_api.contract).unwrap()
        );
        assert_eq!(
            first.gateway_entries[&websocket_key()].gateway_entry_identity,
            second.gateway_entries[&websocket_key()].gateway_entry_identity
        );
        assert_ne!(first.deployment_revision, second.deployment_revision);
    }

    #[test]
    fn resolver_adapter_and_signature_mismatches_fail_closed() {
        let fixture = compile_fixture(
            "negative-signatures",
            "health: main.health\n",
            negative_source(),
            "path: /chat\n",
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
                r#"path: /chat
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
                r#"path: /chat
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
                r#"path: /chat
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
                r#"path: /chat
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
            let websocket = parse_service(&authoring);
            let error = fixture.generate_with(Some(&websocket)).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "{label} expected {expected:?}, got {error}"
            );
        }
    }

    #[test]
    fn fixed_json_rpc_key_collision_and_legacy_generic_std_types_fail_closed() {
        let collision = compile_fixture(
            "fixed-key-collision",
            "health: main.health\n",
            connect_source(),
            r#"path: /chat
jsonRpc:
  websocket:
    method: status.get
    handler: main.health
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
"#,
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
    fn http_and_json_rpc_entry_keys_share_one_collision_domain() {
        let fixture = compile_fixture(
            "cross-document-key-collision",
            "health: main.health\n",
            json_rpc_source(),
            r#"path: /chat
jsonRpc:
  status:
    method: status.get
    handler: main.status
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
      - param: connectionId
        source: { kind: websocket.connectionId }
      - param: businessIdentity
        source: { kind: websocket.businessIdentity }
"#,
        );
        let http: HttpGatewayDocumentAuthoring = serde_yaml::from_str(
            r#"status:
  method: GET
  path: /status
  kind: rawHttp
  handler: main.raw
  adapterArgs:
    - param: request
      source: { kind: http.request }
"#,
        )
        .unwrap();
        let closure = fixture.closure();
        let error = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &fixture.service,
            http: Some(&http),
            websocket: Some(&fixture.websocket),
            service_api: &fixture.api,
            implementation: &fixture.project.package.artifact,
            package_closure: &closure,
            package_schema_records: &fixture.project.package.resolved_package_schema_type_records,
        })
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("both http.yml and websocket.yml"));
    }

    #[test]
    fn compiler_published_std_keeps_only_connect_shapes_and_exact_send_signatures() {
        let fixture = compile_fixture(
            "std-surface",
            "health: main.health\n",
            connect_source(),
            "path: /chat\n",
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
}
