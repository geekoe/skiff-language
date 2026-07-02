use std::fs;

use skiff_compiler::{
    test_support::project_fixtures::{
        write_package_api_yml, write_package_manifest, write_package_manifest_in_dir,
        write_package_source,
    },
    PublicationId,
};
use skiff_compiler_emission::identity::OPERATION_ABI_IDENTITY_PREFIX;

mod common;
use common::{
    artifacts::{assert_publish_error_contains, build_temp_service_publication, source_artifact},
    TestDir,
};

fn entry_target(service_id: &str, suffix: &str) -> String {
    let component = PublicationId::parse(service_id)
        .expect("test service id should parse")
        .runtime_target_component();
    format!("entry.{component}.{suffix}")
}

fn package_http_handler_target(package_id: &str, symbol_path: &str) -> String {
    format!(
        "package.{}.{}",
        encode_package_target_segment(package_id),
        encode_package_target_segment(symbol_path)
    )
}

fn encode_package_target_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn service_unit_operation<'a>(
    service_unit: &'a serde_json::Value,
    public_path: &str,
) -> &'a serde_json::Value {
    service_unit["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"]["publicPath"] == public_path)
        .unwrap_or_else(|| panic!("service unit operation {public_path} should exist"))
}

fn service_unit_operation_module_path(operation: &serde_json::Value) -> &str {
    operation["executable"]["fileRef"]["modulePath"]
        .as_str()
        .expect("local executable operation should carry fileRef.modulePath")
}

#[test]
fn http_route_to_root_handler_projects_route_metadata() {
    let temp = TestDir::new("skiff-http-routes", "root-handler");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  routes:
    - method: POST
      path: /track
      handler: root.internal.http.record
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("http.skiff"),
        r#"
            type RouteMarker {}

            function record(request: std.http.HttpRequest) -> std.http.HttpResponse {
              return std.http.noContent()
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let route = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes[0];

    assert_eq!(route.method, "POST");
    assert_eq!(route.path, "/track");
    assert_eq!(route.operation, "http.route.internal.http.record");
    assert_eq!(
        route.target,
        entry_target("example.com/example", "http.route.internal.http.record")
    );
    assert_eq!(
        published.artifacts.service_assembly.value["gateway"]["http"]["routes"][0]["operation"],
        "http.route.internal.http.record"
    );
    assert_eq!(
        published.artifacts.service_assembly.value["gateway"]["http"]["routes"][0]["handler"],
        serde_json::json!({
            "kind": "serviceFunction",
            "source": "root.internal.http.record",
            "modulePath": "internal.http",
            "symbol": "record"
        })
    );
    assert!(published.artifacts.service_assembly.value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|operation| operation["operation"] == "http.route.internal.http.record"));
    let assembly_operation = published.artifacts.service_assembly.value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"] == "http.route.internal.http.record")
        .unwrap();
    let service_unit_operation = service_unit_operation(
        &published.artifacts.service_unit.value,
        "http.route.internal.http.record",
    );
    let operation_abi_id = assembly_operation["operationAbiId"].as_str().unwrap();
    assert!(operation_abi_id.starts_with(&format!("{OPERATION_ABI_IDENTITY_PREFIX}:")));
    assert_eq!(
        service_unit_operation["operation"]["operationAbiId"],
        operation_abi_id
    );
    assert_eq!(
        published.artifacts.service_unit.value["gateway"]["routes"]["/track"]["operation"],
        "http.route.internal.http.record"
    );
    assert_eq!(
        published.artifacts.service_unit.value["gateway"]["routes"]["/track"]["operationAbiId"],
        operation_abi_id
    );
}

#[test]
fn typed_http_route_generates_json_wrapper_and_ingress_metadata() {
    let temp = TestDir::new("skiff-http-routes", "typed-json-route");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  routes:
    - path: /todos
      handler: root.internal.todos.create
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("todos.skiff"),
        r#"
            type CreateTodoRequest {
              title: string,
            }

            type CreateTodoResponse {
              id: string,
              title: string,
            }

            function create(input: CreateTodoRequest) -> CreateTodoResponse {
              return { id: "todo-1", title: input.title }
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let route = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes[0];
    let typed = route.typed.as_ref().expect("typed route metadata");

    assert_eq!(route.method, "POST");
    assert_eq!(
        route.target,
        entry_target("example.com/example", "http.route.internal.todos.create")
    );
    assert!(typed
        .ingress_identity
        .starts_with("skiff-http-ingress-v1:sha256:"));
    assert_eq!(
        serde_json::to_value(&typed.body.as_ref().unwrap().schema).unwrap()["xSkiffSymbol"],
        "internal.todos.CreateTodoRequest"
    );
    assert_eq!(
        serde_json::to_value(&typed.response.schema).unwrap()["xSkiffSymbol"],
        "internal.todos.CreateTodoResponse"
    );
    assert_eq!(
        published.artifacts.service_assembly.value["gateway"]["http"]["routes"][0]["typed"]
            ["ingressIdentity"],
        typed.ingress_identity
    );
    let route_json = http_route_manifest_value(&published, 0);
    assert_eq!(
        route_json["handler"],
        serde_json::json!({
            "kind": "serviceFunction",
            "source": "root.internal.todos.create",
            "modulePath": "internal.todos",
            "symbol": "create"
        })
    );
    assert_typed_json_adapter(route_json);
    assert_service_adapter_callable(
        &route_json["typed"]["adapter"]["handler"],
        "internal.todos",
        "create",
    );
    assert_eq!(
        route_json["typed"]["adapter"]["adapterArgs"],
        serde_json::json!([{ "param": "input", "source": { "kind": "http.body" } }])
    );
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

#[test]
fn typed_http_route_json_wrapper_preserves_body_and_response_type_args() {
    let temp = typed_todo_route_project("typed-json-route-type-args");

    let published = build_temp_service_publication(temp.path());
    let route_json = http_route_manifest_value(&published, 0);

    assert_eq!(
        route_json["typed"]["body"]["schema"]["xSkiffSymbol"],
        "internal.todos.CreateTodoRequest"
    );
    assert_eq!(
        route_json["typed"]["response"]["schema"]["xSkiffSymbol"],
        "internal.todos.CreateTodoResponse"
    );
    assert_eq!(
        route_json["typed"]["adapter"]["adapterArgs"],
        serde_json::json!([{ "param": "input", "source": { "kind": "http.body" } }])
    );
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

#[test]
fn typed_http_route_response_only_wrapper_omits_body_decode_and_encodes_response_type() {
    let temp = TestDir::new("skiff-http-routes", "typed-response-only-json-route");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  routes:
    - path: /todos
      handler: root.internal.todos.list
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("todos.skiff"),
        r#"
            type TodoListResponse {
              count: integer,
            }

            function list() -> TodoListResponse {
              return { count: 1 }
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let route_json = http_route_manifest_value(&published, 0);

    assert!(route_json["typed"].get("body").is_none());
    assert_eq!(
        route_json["typed"]["response"]["schema"]["xSkiffSymbol"],
        "internal.todos.TodoListResponse"
    );
    assert!(route_json["typed"]["adapter"].get("adapterArgs").is_none());
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

#[test]
fn raw_streaming_http_route_uses_raw_adapter_without_wrapper() {
    let temp = TestDir::new("skiff-http-routes", "raw-stream-wrapper");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  pre: root.internal.relay.pre
  routes:
    - method: POST
      path: /relay
      handler: root.internal.relay.stream
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("relay.skiff"),
        r#"
            import std

            type RelayContext {
              trace: string,
            }

            function pre(request: std.http.HttpRequest) -> RelayContext {
              return { trace: "trace-1" }
            }

            function stream(request: std.http.HttpRequest, context: RelayContext) -> Stream<std.http.HttpResponseStreamEvent> {
              emit std.http.streamStart(200, Array.empty<std.http.HttpHeader>())
              emit std.http.streamChunk(request.body)
              emit std.http.streamEnd()
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let route = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes[0];
    let route_json = http_route_manifest_value(&published, 0);
    assert_eq!(
        route.target,
        entry_target("example.com/example", "http.route.internal.relay.stream")
    );
    assert_raw_http_adapter(route_json);
    assert_service_adapter_callable(
        &route_json["adapter"]["handler"],
        "internal.relay",
        "stream",
    );
    assert_service_adapter_callable(&route_json["adapter"]["pre"], "internal.relay", "pre");
    assert_eq!(
        route_json["adapter"]["adapterArgs"],
        serde_json::json!([
            { "param": "request", "source": { "kind": "http.request" } },
            { "param": "context", "source": { "kind": "http.context" } }
        ])
    );
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

#[test]
fn typed_http_route_context_only_uses_http_pre_without_body_decode() {
    let temp = TestDir::new("skiff-http-routes", "typed-context-route");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  pre: root.internal.account.pre
  routes:
    - path: /me
      handler: root.internal.account.me
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("account.skiff"),
        r#"
            import std

            type RequestContext {
              userId: string,
            }

            type MeResponse {
              userId: string,
            }

            function pre(request: std.http.HttpRequest) -> RequestContext {
              return { userId: "user-1" }
            }

            function me(context: RequestContext) -> MeResponse {
              return { userId: context.userId }
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let route = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes[0];
    let typed = route.typed.as_ref().expect("typed route metadata");
    assert!(typed.body.is_none());
    let route_json = http_route_manifest_value(&published, 0);
    assert_typed_json_adapter(route_json);
    assert_service_adapter_callable(
        &route_json["typed"]["adapter"]["pre"],
        "internal.account",
        "pre",
    );
    assert_eq!(
        route_json["typed"]["adapter"]["adapterArgs"],
        serde_json::json!([{ "param": "context", "source": { "kind": "http.context" } }])
    );
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

fn typed_todo_route_project(name: &str) -> TestDir {
    let temp = TestDir::new("skiff-http-routes", name);
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  routes:
    - path: /todos
      handler: root.internal.todos.create
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("todos.skiff"),
        r#"
            type CreateTodoRequest {
              title: string,
            }

            type CreateTodoResponse {
              id: string,
              title: string,
            }

            function create(input: CreateTodoRequest) -> CreateTodoResponse {
              return { id: "todo-1", title: input.title }
            }
        "#,
    )
    .unwrap();
    temp
}

fn http_route_manifest_value(
    published: &skiff_compiler::BuiltServicePublication,
    index: usize,
) -> &serde_json::Value {
    &published.artifacts.service_assembly.value["gateway"]["http"]["routes"][index]
}

fn assert_source_artifact_absent(
    published: &skiff_compiler::BuiltServicePublication,
    source_path: &str,
) {
    assert!(
        !published
            .artifacts
            .file_ir_units
            .iter()
            .any(|artifact| artifact.source_path == source_path),
        "{source_path} should not be generated for HTTP adapter routes"
    );
}

fn assert_typed_json_adapter(route: &serde_json::Value) {
    assert_eq!(route["typed"]["adapter"]["kind"], "typedJson");
}

fn assert_raw_http_adapter(route: &serde_json::Value) {
    assert_eq!(route["adapter"]["kind"], "rawHttp");
}

fn assert_service_adapter_callable(callable: &serde_json::Value, module_path: &str, symbol: &str) {
    assert_eq!(
        callable,
        &serde_json::json!({
            "kind": "serviceFunction",
            "modulePath": module_path,
            "symbol": symbol
        })
    );
}

fn assert_package_adapter_callable(
    callable: &serde_json::Value,
    package_id: &str,
    symbol_path: &str,
) {
    assert_eq!(
        callable,
        &serde_json::json!({
            "kind": "packageFunction",
            "packageId": package_id,
            "symbolPath": symbol_path
        })
    );
}

#[test]
fn raw_http_route_accepts_root_level_std_http_json_helper() {
    let temp = TestDir::new("skiff-http-routes", "root-std-http-json");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  routes:
    - method: GET
      path: /health
      handler: root.internal.health.check
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("health.skiff"),
        r#"
	            function check(request: std.http.HttpRequest) -> std.http.HttpResponse {
	              const trace = std.http.header(request, "x-trace")
	              const traces = std.http.headers(request, "x-trace")
	              const query = std.http.query(request, "q")
	              const session = std.http.cookie(request, "sid")
	              const maybeBlocked = std.http.requireMethod(request, "GET")
	              const sseHeaders = std.http.sseHeaders()
	              const headers = std.http.forwardableHeaders(request.headers)
	              return std.http.jsonWithHeaders<JsonObject>(200, { ok: true }, headers)
	            }

	            function noContent(request: std.http.HttpRequest) -> std.http.HttpResponse {
	              return std.http.noContent()
	            }

	            function methodNotAllowed(request: std.http.HttpRequest) -> std.http.HttpResponse {
	              return std.http.methodNotAllowed("GET")
	            }

	            function errorResponse(request: std.http.HttpRequest) -> std.http.HttpResponse {
	              return std.http.errorResponse(400, "bad_request", "Bad request", null)
	            }
	        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let artifact_json =
        serde_json::to_string(&source_artifact(&published, "internal/health.skiff").value())
            .unwrap();

    for helper in [
        "header",
        "headers",
        "query",
        "cookie",
        "requireMethod",
        "sseHeaders",
        "forwardableHeaders",
        "jsonWithHeaders",
        "noContent",
        "methodNotAllowed",
        "errorResponse",
    ] {
        assert!(
            artifact_json.contains(helper),
            "{helper} missing from {artifact_json}"
        );
    }
}

#[test]
fn http_error_message_detail_payload_is_accepted() {
    let temp = TestDir::new("skiff-http-routes", "http-error-message-detail");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  routes:
    - path: /todos
      handler: root.internal.todos.create
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("todos.skiff"),
        r#"
            type CreateTodoResponse {
              ok: bool,
            }

            function create() -> CreateTodoResponse {
              throw std.http.HttpError {
                message: "not failure",
                detail: { title: "ship" },
              }
              return { ok: true }
            }
        "#,
    )
    .unwrap();

    build_temp_service_publication(temp.path());
}

#[test]
fn raw_http_route_rejects_missing_method() {
    let temp = TestDir::new("skiff-http-routes", "raw-route-missing-method");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  routes:
    - path: /track
      handler: root.internal.http.record
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("http.skiff"),
        r#"
            import std

            function record(request: std.http.HttpRequest) -> std.http.HttpResponse {
              return std.http.noContent()
            }
        "#,
    )
    .unwrap();

    assert_publish_error_contains(
        temp.path(),
        &[
            "raw HTTP route handler root.internal.http.record",
            "must configure method",
        ],
    );
}

#[test]
fn http_guard_wraps_service_route_without_manifest_protocol_changes() {
    let temp = TestDir::new("skiff-http-routes", "service-guard");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  guard: root.internal.http.guard
  routes:
    - method: POST
      path: /track
      handler: root.internal.http.record
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("http.skiff"),
        r#"
            import std

            function guard(request: std.http.HttpRequest) -> std.http.HttpResponse? {
              return null
            }

            function record(request: std.http.HttpRequest) -> std.http.HttpResponse {
              return std.http.noContent()
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let route = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes[0];
    let route_json = http_route_manifest_value(&published, 0);

    assert_eq!(route.method, "POST");
    assert_eq!(route.path, "/track");
    assert_eq!(route.operation, "http.route.internal.http.record");
    assert_eq!(
        route.target,
        entry_target("example.com/example", "http.route.internal.http.record")
    );
    assert_raw_http_adapter(route_json);
    assert_service_adapter_callable(&route_json["adapter"]["handler"], "internal.http", "record");
    assert_service_adapter_callable(&route_json["adapter"]["guard"], "internal.http", "guard");
    assert_eq!(
        route_json["adapter"]["adapterArgs"],
        serde_json::json!([{ "param": "request", "source": { "kind": "http.request" } }])
    );
    assert!(
        published.artifacts.service_assembly.value["gateway"]["http"]["routes"][0]
            .get("guard")
            .is_none()
    );
    let operation = service_unit_operation(
        &published.artifacts.service_unit.value,
        "http.route.internal.http.record",
    );
    assert_eq!(operation["kind"], "localExecutable");
    assert_eq!(
        service_unit_operation_module_path(operation),
        "internal.http"
    );
    assert_eq!(operation["executable"]["callableKind"], "publicFunction");
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

#[test]
fn http_guard_wrappers_do_not_collide_for_sanitized_handler_names() {
    let temp = TestDir::new("skiff-http-routes", "service-guard-name-collision");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  guard: root.internal.http.guard
  routes:
    - method: POST
      path: /nested
      handler: root.internal.a.b
    - method: POST
      path: /flat
      handler: root.internal_a.b
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("http.skiff"),
        r#"
            import std

            function guard(request: std.http.HttpRequest) -> std.http.HttpResponse? {
              return null
            }
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("a.skiff"),
        r#"
            import std

            function b(request: std.http.HttpRequest) -> std.http.HttpResponse {
              return std.http.noContent()
            }
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal_a.skiff"),
        r#"
            import std

            function b(request: std.http.HttpRequest) -> std.http.HttpResponse {
              return std.http.noContent()
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let routes = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes;

    assert_eq!(routes[0].operation, "http.route.internal.a.b");
    assert_eq!(
        routes[0].target,
        entry_target("example.com/example", "http.route.internal.a.b")
    );
    assert_eq!(routes[1].operation, "http.route.internal_a.b");
    assert_eq!(
        routes[1].target,
        entry_target("example.com/example", "http.route.internal_a.b")
    );
    let first_route_json = http_route_manifest_value(&published, 0);
    let second_route_json = http_route_manifest_value(&published, 1);
    assert_raw_http_adapter(first_route_json);
    assert_raw_http_adapter(second_route_json);
    assert_service_adapter_callable(
        &first_route_json["adapter"]["guard"],
        "internal.http",
        "guard",
    );
    assert_service_adapter_callable(
        &second_route_json["adapter"]["guard"],
        "internal.http",
        "guard",
    );

    let nested = service_unit_operation(
        &published.artifacts.service_unit.value,
        "http.route.internal.a.b",
    );
    let flat = service_unit_operation(
        &published.artifacts.service_unit.value,
        "http.route.internal_a.b",
    );
    assert_eq!(nested["kind"], "localExecutable");
    assert_eq!(flat["kind"], "localExecutable");
    assert_eq!(service_unit_operation_module_path(nested), "internal.a");
    assert_eq!(service_unit_operation_module_path(flat), "internal_a");
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

#[test]
fn http_route_to_project_handler_allows_rootless_module_path() {
    let temp = TestDir::new("skiff-http-routes", "rootless-project-handler");
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  routes:
    - method: POST
      path: /search
      handler: prompt_service.search
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("prompt_service.skiff"),
        r#"
            import std

            function search(request: std.http.HttpRequest) -> std.http.HttpResponse {
              return std.http.noContent()
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let route = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes[0];

    assert_eq!(route.method, "POST");
    assert_eq!(route.path, "/search");
    assert_eq!(route.operation, "http.route.prompt_service.search");
    assert_eq!(
        route.target,
        entry_target("example.com/example", "http.route.prompt_service.search")
    );
}

#[test]
fn http_route_to_package_handler_projects_route_and_package_db_metadata() {
    let temp = TestDir::new("skiff-http-routes", "package-handler");
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: example.com/http-session
    version: 1.0.0
    alias: httpSession
http:
  routes:
    - method: POST
      path: /session
      handler: httpSession.issue
"#,
    )
    .unwrap();
    write_package_manifest(
        temp.path(),
        "example.com/http-session",
        r#"
id: example.com/http-session
version: 1.0.0
"#,
    );
    write_package_api_yml(
        temp.path(),
        "example.com/http-session",
        r#"
issue: session_impl.issue
"#,
    );
    write_package_source(
        temp.path(),
        "example.com/http-session",
        "session_impl.skiff",
        r#"
          import std

          type BrowserSession {
            id: string
          }

          db object BrowserSession {
            name "session"

            primary key(id)
          }

          function issue(request: std.http.HttpRequest) -> std.http.HttpResponse {
            const cookieName = config.require<string>("cookieName")
            const cookieDomain = config.optional<string>("cookieDomain")
            const maxAgeSeconds = config.require<number>("maxAgeSeconds")
            return std.http.noContent()
          }
        "#,
    );
    let published = build_temp_service_publication(temp.path());
    let route = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes[0];
    let route_json = http_route_manifest_value(&published, 0);

    assert_eq!(route.method, "POST");
    assert_eq!(route.path, "/session");
    assert_eq!(route.operation, "http.route.httpSession.issue");
    assert_eq!(
        route.target,
        package_http_handler_target("example.com/http-session", "issue")
    );
    assert_raw_http_adapter(route_json);
    assert_package_adapter_callable(
        &route_json["adapter"]["handler"],
        "example.com/http-session",
        "issue",
    );
    assert_eq!(
        route_json["adapter"]["adapterArgs"],
        serde_json::json!([{ "param": "request", "source": { "kind": "http.request" } }])
    );
    assert_eq!(
        published.artifacts.service_assembly.value["gateway"]["http"]["routes"][0]["operation"],
        "http.route.httpSession.issue"
    );
    assert_eq!(
        published.artifacts.service_assembly.value["gateway"]["http"]["routes"][0]["handler"],
        serde_json::json!({
            "kind": "packageFunction",
            "source": "httpSession.issue",
            "packageId": "example.com/http-session",
            "alias": "httpSession",
            "symbolPath": "issue"
        })
    );
    let route_operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| {
            operation.operation == "http.route.httpSession.issue"
                && operation.target
                    == package_http_handler_target("example.com/http-session", "issue")
        })
        .expect("package HTTP route operation should be projected");
    assert_eq!(
        route_json["operationAbiId"],
        route_operation.operation_abi_id
    );
    assert_eq!(
        published.artifacts.service_unit.value["gateway"]["routes"]["/session"]["operationAbiId"],
        route_operation.operation_abi_id
    );
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
    assert!(published.artifacts.service_assembly.value["db"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["packageId"] == "example.com/http-session"
            && entry["typeName"] == "BrowserSession"
            && entry["collectionName"] == "session"));
    assert_eq!(
        published.artifacts.service_unit.value["db"],
        published.artifacts.service_assembly.value["db"]
    );
    assert_eq!(
        published.artifacts.service_assembly.value["configShape"]["entries"],
        serde_json::json!([])
    );
    assert!(
        published.artifacts.service_unit.value["config"]["packageConfigs"]
            ["example.com/http-session"]
            .get("config")
            .is_none()
    );
}

#[test]
fn typed_http_route_to_package_handler_projects_typed_adapter_without_wrapper() {
    let temp = TestDir::new("skiff-http-routes", "package-typed-handler");
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: example.com/http-session
    version: 1.0.0
    alias: httpSession
http:
  routes:
    - path: /session
      handler: httpSession.issue
"#,
    )
    .unwrap();
    write_package_manifest(
        temp.path(),
        "example.com/http-session",
        r#"
id: example.com/http-session
version: 1.0.0
"#,
    );
    write_package_api_yml(
        temp.path(),
        "example.com/http-session",
        r#"
IssueRequest: session_impl.IssueRequest
IssueResponse: session_impl.IssueResponse
issue: session_impl.issue
"#,
    );
    write_package_source(
        temp.path(),
        "example.com/http-session",
        "session_impl.skiff",
        r#"
          type IssueRequest {
            userId: string
          }

          type IssueResponse {
            sessionId: string
          }

          function issue(input: IssueRequest) -> IssueResponse {
            return { sessionId: input.userId }
          }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let route = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes[0];
    let route_json = http_route_manifest_value(&published, 0);

    assert_eq!(route.operation, "http.route.httpSession.issue");
    assert_eq!(
        route.target,
        package_http_handler_target("example.com/http-session", "issue")
    );
    assert_eq!(
        route_json["handler"],
        serde_json::json!({
            "kind": "packageFunction",
            "source": "httpSession.issue",
            "packageId": "example.com/http-session",
            "alias": "httpSession",
            "symbolPath": "issue"
        })
    );
    assert_eq!(route_json["typed"]["adapter"]["kind"], "typedJson");
    assert_eq!(
        route_json["typed"]["adapter"]["handler"],
        serde_json::json!({
            "kind": "packageFunction",
            "packageId": "example.com/http-session",
            "symbolPath": "issue"
        })
    );
    assert_eq!(
        route_json["typed"]["adapter"]["adapterArgs"],
        serde_json::json!([{ "param": "input", "source": { "kind": "http.body" } }])
    );
    assert!(!published.artifacts.service_assembly.value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|operation| operation["operation"] == "http.route.httpSession.issue"));
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

#[test]
fn http_route_package_handler_requires_public_path_segment() {
    let temp = TestDir::new("skiff-http-routes", "package-handler-export-path");
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: example.com/http-session-path
    version: 1.0.0
    alias: httpSession
http:
  routes:
    - method: POST
      path: /session
      handler: httpSession.session.issue
"#,
    )
    .unwrap();
    write_package_manifest(
        temp.path(),
        "example.com/http-session-path",
        r#"
id: example.com/http-session-path
version: 1.0.0
"#,
    );
    write_package_api_yml(
        temp.path(),
        "example.com/http-session-path",
        r#"
session:
  issue: session_impl.issue
"#,
    );
    write_package_source(
        temp.path(),
        "example.com/http-session-path",
        "session_impl.skiff",
        r#"
          import std

          function issue(request: std.http.HttpRequest) -> std.http.HttpResponse {
            return std.http.noContent()
          }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let route = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes[0];

    assert_eq!(route.operation, "http.route.httpSession.session.issue");
    assert_eq!(
        route.target,
        package_http_handler_target("example.com/http-session-path", "session.issue")
    );
    let route_json = http_route_manifest_value(&published, 0);
    assert_raw_http_adapter(route_json);
    assert_package_adapter_callable(
        &route_json["adapter"]["handler"],
        "example.com/http-session-path",
        "session.issue",
    );
    assert!(published.manifest.operations.iter().any(|operation| {
        operation.operation == "http.route.httpSession.session.issue"
            && operation.target
                == package_http_handler_target("example.com/http-session-path", "session.issue")
    }));
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

#[test]
fn http_route_package_handler_adapter_uses_visible_symbol_path_for_export_root() {
    let temp = TestDir::new("skiff-http-routes", "package-handler-export-root-call");
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: skiff.run/track
    version: 1.0.0
    alias: track
http:
  routes:
    - method: POST
      path: /track
      handler: track.track.record
"#,
    )
    .unwrap();
    write_package_manifest(
        temp.path(),
        "skiff.run/track",
        r#"
id: skiff.run/track
version: 1.0.0
"#,
    );
    write_package_api_yml(
        temp.path(),
        "skiff.run/track",
        r#"
track:
  record: track_impl.record
"#,
    );
    write_package_source(
        temp.path(),
        "skiff.run/track",
        "track_impl.skiff",
        r#"
          import std

          function record(request: std.http.HttpRequest) -> std.http.HttpResponse {
            return std.http.noContent()
          }
        "#,
    );
    let published = build_temp_service_publication(temp.path());
    let route = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes[0];
    let route_json = http_route_manifest_value(&published, 0);

    assert_eq!(
        route.target,
        package_http_handler_target("skiff.run/track", "track.record")
    );
    assert_raw_http_adapter(route_json);
    assert_package_adapter_callable(
        &route_json["adapter"]["handler"],
        "skiff.run/track",
        "track.record",
    );
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

#[test]
fn http_route_package_handler_can_target_callable_method() {
    let temp = TestDir::new("skiff-http-routes", "package-handler-method");
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: example.com/http-session
    version: 1.0.0
    alias: httpSession
http:
  routes:
    - method: POST
      path: /session
      handler: httpSession.Handler.issue
"#,
    )
    .unwrap();
    write_package_manifest(
        temp.path(),
        "example.com/http-session",
        r#"
id: example.com/http-session
version: 1.0.0
"#,
    );
    write_package_api_yml(
        temp.path(),
        "example.com/http-session",
        r#"
Request: session_impl.Request
Response: session_impl.Response
Handler: session_impl.Handler
"#,
    );
    write_package_source(
        temp.path(),
        "example.com/http-session",
        "session_impl.skiff",
        r#"
          import std

          alias Request = std.http.HttpRequest
          alias Response = std.http.HttpResponse

          type Handler {}
          impl Handler {
            static function issue(request: Request) -> Response {
              return std.http.noContent()
            }
          }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let route = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes[0];

    assert_eq!(route.operation, "http.route.httpSession.Handler.issue");
    assert_eq!(
        route.target,
        package_http_handler_target("example.com/http-session", "Handler.issue")
    );
    let route_json = http_route_manifest_value(&published, 0);
    assert_raw_http_adapter(route_json);
    assert_package_adapter_callable(
        &route_json["adapter"]["handler"],
        "example.com/http-session",
        "Handler.issue",
    );
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

#[test]
fn http_guard_wraps_package_routes() {
    let temp = TestDir::new("skiff-http-routes", "package-guard");
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: example.com/http-session
    version: 1.0.0
    alias: httpSession
  - id: skiff.run/track
    version: 1.0.0
    alias: track
http:
  guard: httpSession.session.guard
  routes:
    - method: POST
      path: /session
      handler: httpSession.session.issue
    - method: POST
      path: /track
      handler: track.track.record
"#,
    )
    .unwrap();
    write_package_manifest(
        temp.path(),
        "example.com/http-session",
        r#"
id: example.com/http-session
version: 1.0.0
"#,
    );
    write_package_api_yml(
        temp.path(),
        "example.com/http-session",
        r#"
session:
  issue: session_impl.issue
  guard: session_impl.guard
"#,
    );
    write_package_source(
        temp.path(),
        "example.com/http-session",
        "session_impl.skiff",
        r#"
          import std

          function issue(request: std.http.HttpRequest) -> std.http.HttpResponse {
            return std.http.noContent()
          }

          function guard(request: std.http.HttpRequest) -> std.http.HttpResponse? {
            return null
          }
        "#,
    );
    write_package_manifest(
        temp.path(),
        "skiff.run/track",
        r#"
id: skiff.run/track
version: 1.0.0
"#,
    );
    write_package_api_yml(
        temp.path(),
        "skiff.run/track",
        r#"
track:
  record: track_impl.record
"#,
    );
    write_package_source(
        temp.path(),
        "skiff.run/track",
        "track_impl.skiff",
        r#"
          import std

          function record(request: std.http.HttpRequest) -> std.http.HttpResponse {
            return std.http.noContent()
          }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let routes = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes;

    assert_eq!(routes[0].operation, "http.route.httpSession.session.issue");
    assert_eq!(
        routes[0].target,
        package_http_handler_target("example.com/http-session", "session.issue")
    );
    assert_eq!(routes[1].operation, "http.route.track.track.record");
    assert_eq!(
        routes[1].target,
        package_http_handler_target("skiff.run/track", "track.record")
    );
    let first_route_json = http_route_manifest_value(&published, 0);
    let second_route_json = http_route_manifest_value(&published, 1);
    assert_raw_http_adapter(first_route_json);
    assert_raw_http_adapter(second_route_json);
    assert_package_adapter_callable(
        &first_route_json["adapter"]["handler"],
        "example.com/http-session",
        "session.issue",
    );
    assert_package_adapter_callable(
        &first_route_json["adapter"]["guard"],
        "example.com/http-session",
        "session.guard",
    );
    assert_package_adapter_callable(
        &second_route_json["adapter"]["handler"],
        "skiff.run/track",
        "track.record",
    );
    assert_package_adapter_callable(
        &second_route_json["adapter"]["guard"],
        "example.com/http-session",
        "session.guard",
    );
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

#[test]
fn http_service_guard_wraps_package_route() {
    let temp = TestDir::new("skiff-http-routes", "service-guard-package-route");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: skiff.run/track
    version: 1.0.0
    alias: track
http:
  guard: root.internal.http.guard
  routes:
    - method: POST
      path: /track
      handler: track.track.record
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("http.skiff"),
        r#"
          import std

          function guard(request: std.http.HttpRequest) -> std.http.HttpResponse? {
            return null
          }
        "#,
    )
    .unwrap();
    write_package_manifest(
        temp.path(),
        "skiff.run/track",
        r#"
id: skiff.run/track
version: 1.0.0
"#,
    );
    write_package_api_yml(
        temp.path(),
        "skiff.run/track",
        r#"
track:
  record: track_impl.record
"#,
    );
    write_package_source(
        temp.path(),
        "skiff.run/track",
        "track_impl.skiff",
        r#"
          import std

          function record(request: std.http.HttpRequest) -> std.http.HttpResponse {
            return std.http.noContent()
          }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let route = &published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap()
        .routes[0];

    assert_eq!(route.operation, "http.route.track.track.record");
    assert_eq!(
        route.target,
        package_http_handler_target("skiff.run/track", "track.record")
    );
    let route_json = http_route_manifest_value(&published, 0);
    assert_raw_http_adapter(route_json);
    assert_service_adapter_callable(&route_json["adapter"]["guard"], "internal.http", "guard");
    assert_package_adapter_callable(
        &route_json["adapter"]["handler"],
        "skiff.run/track",
        "track.record",
    );
    assert_source_artifact_absent(&published, "__skiff/http_routes.skiff");
}

#[test]
fn http_guard_validates_signature() {
    let temp = TestDir::new("skiff-http-routes", "bad-guard");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  guard: root.internal.http.guard
  routes:
    - method: POST
      path: /track
      handler: root.internal.http.record
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("http.skiff"),
        r#"
            import std

            function guard(request: std.http.HttpRequest) -> std.http.HttpResponse {
              return std.http.noContent()
            }

            function record(request: std.http.HttpRequest) -> std.http.HttpResponse {
              return std.http.noContent()
            }
        "#,
    )
    .unwrap();

    assert_publish_error_contains(
        temp.path(),
        &[
            "http guard root.internal.http.guard",
            "std.http.HttpResponse?",
        ],
    );
}

#[test]
fn http_route_package_handler_validates_exported_function_signature() {
    let temp = TestDir::new("skiff-http-routes", "package-handler-signature");
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: example.com/http-session-bad
    version: 1.0.0
    alias: httpSession
http:
  routes:
    - method: POST
      path: /session
      handler: httpSession.issue
"#,
    )
    .unwrap();
    write_package_manifest(
        temp.path(),
        "example.com/http-session-bad",
        r#"
id: example.com/http-session-bad
version: 1.0.0
"#,
    );
    write_package_api_yml(
        temp.path(),
        "example.com/http-session-bad",
        r#"
issue: httpSession_impl.issue
"#,
    );
    write_package_source(
        temp.path(),
        "example.com/http-session-bad",
        "httpSession_impl.skiff",
        r#"
          type Marker {}

          function issue(request: string) -> std.http.HttpResponse {
            return std.http.noContent()
          }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "http route /session handler httpSession.issue",
            "std.http.HttpRequest",
            "std.http.HttpResponse",
        ],
    );
}

#[test]
fn http_route_package_handler_allows_package_to_package_dependency() {
    let temp = TestDir::new("skiff-http-routes", "package-to-package");
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: example.com/http-session-facade
    version: 1.0.0
    alias: httpSession
http:
  routes:
    - method: POST
      path: /session
      handler: httpSession.issue
"#,
    )
    .unwrap();
    write_package_manifest(
        temp.path(),
        "example.com/http-session-facade",
        r#"
id: example.com/http-session-facade
version: 1.0.0
packages:
  - id: google.com/cloud
    version: 0.1.0
    alias: gcloud
"#,
    );
    write_package_api_yml(
        temp.path(),
        "example.com/http-session-facade",
        r#"
issue: httpSession_impl.issue
"#,
    );
    write_package_source(
        temp.path(),
        "example.com/http-session-facade",
        "httpSession_impl.skiff",
        r#"
          import gcloud

          type Marker {}

          function issue(request: std.http.HttpRequest) -> std.http.HttpResponse {
            const uploaded = gcloud.storage.upload()
            return std.http.noContent()
          }
        "#,
    );
    write_package_manifest_in_dir(
        temp.path(),
        "google.com/cloud",
        r#"
id: google.com/cloud
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.path(),
        "google.com/cloud",
        r#"
storage:
  upload: cloud.storage.upload
"#,
    );
    write_package_source(
        temp.path(),
        "google.com/cloud",
        "cloud/storage.skiff",
        r#"
          function upload() -> string { return "ok" }
        "#,
    );

    let published = build_temp_service_publication(temp.path());

    assert!(published
        .artifacts
        .package_units
        .iter()
        .any(|unit| unit.value["packageId"] == "google.com/cloud"));
    assert!(published
        .artifacts
        .package_file_ir_units
        .iter()
        .any(|artifact| common::artifacts::json_contains_package_symbol(
            &artifact.value(),
            "gcloud",
            "storage.upload"
        )));
}
