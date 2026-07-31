use std::{
    path::PathBuf,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

#[test]
fn reads_split_external_manifests() {
    let root = fixture_root("positive");
    write(&root, "package.yml", "id: example.com/account-package\nversion: 2.1.0\nservices:\n  - id: example.com/payment\n    version: 3.0.0\n    alias: payment\n");
    write(&root, "api.yml", "{}\n");
    write(&root, "service.yml", "id: example.com/account\n");
    write(
        &root,
        "http.yml",
        r#"typed:
  method: post
  path: /users
  kind: typedJson
  handler: users.create
  adapterArgs:
    - param: body
      source: { kind: http.body }
raw:
  method: GET
  path: /raw
  kind: rawHttp
  handler: handlers.raw
  guard: handlers.guard
  pre: handlers.prepare
  adapterArgs:
    - param: request
      source: { kind: http.request }
    - param: context
      source: { kind: http.context }
"#,
    );
    write(
        &root,
        "websocket.yml",
        r#"path: /socket
jsonRpc:
  status:
    method: status.get
    handler: socket.status
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
"#,
    );
    write(
        &root,
        "config.dev.yml",
        "config: { feature: true }\nstate: { account: primary }\n",
    );

    let source = read_service_package_root(&root).unwrap();
    assert_eq!(source.package.id.as_str(), "example.com/account-package");
    assert_eq!(source.package.services[0].effective_alias(), "payment");
    assert_eq!(source.service.id, "example.com/account");
    assert_eq!(source.service.kind, ServiceAuthoringKind::Service);
    let http = source.http.as_ref().unwrap();
    let typed = &http.entries[&GatewayEntryKey::parse("typed").unwrap()];
    assert_eq!(typed.method, "POST");
    assert_eq!(
        typed.kind,
        skiff_artifact_model::GatewayAdapterKind::TypedJson
    );
    let raw = &http.entries[&GatewayEntryKey::parse("raw").unwrap()];
    assert_eq!(raw.pre.as_deref(), Some("handlers.prepare"));
    let websocket = source.websocket.as_ref().unwrap();
    assert_eq!(websocket.path, "/socket");
    assert_eq!(
        websocket.json_rpc[&GatewayEntryKey::parse("status").unwrap()].method,
        "status.get"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn top_level_alias_is_exclusive_to_test_services() {
    let package = "id: example.com/widget-tests\nversion: 1.0.0\npackages:\n  - id: example.com/widget\n    version: 1.0.0\n    alias: widget\n    topLevelAlias: widgetImpl\n";
    let test_root = fixture_root("test-top-level");
    write(&test_root, "package.yml", package);
    write(&test_root, "api.yml", "{}\n");
    write(
        &test_root,
        "service.yml",
        "id: example.com/widget-tests\nkind: test\n",
    );
    let source = read_service_package_root(&test_root).unwrap();
    assert_eq!(source.service.kind, ServiceAuthoringKind::Test);
    assert_eq!(
        source.package.dependencies[0].top_level_alias.as_deref(),
        Some("widgetImpl")
    );
    fs::remove_dir_all(test_root).unwrap();

    let production_root = fixture_root("production-top-level");
    write(&production_root, "package.yml", package);
    write(&production_root, "api.yml", "{}\n");
    write(
        &production_root,
        "service.yml",
        "id: example.com/widget-tests\n",
    );
    let error = read_service_package_root(&production_root).unwrap_err();
    assert!(error
        .to_string()
        .contains("allowed only when service.yml declares kind: test"));
    fs::remove_dir_all(production_root).unwrap();
}

#[test]
fn legacy_dependency_access_fails_during_manifest_parsing() {
    let root = fixture_root("legacy-top-level-access");
    write(
            &root,
            "package.yml",
            "id: example.com/widget-tests\nversion: 1.0.0\npackages:\n  - id: example.com/widget\n    version: 1.0.0\n    alias: widget\n    access: topLevel\n",
        );
    write(&root, "api.yml", "{}\n");
    write(
        &root,
        "service.yml",
        "id: example.com/widget-tests\nkind: test\n",
    );
    let error = read_service_package_root(&root).unwrap_err().to_string();
    assert!(error.contains("unknown field `access`"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn service_calls_accept_missing_empty_and_canonical_paths() {
    let missing = read_service_yml("service-calls-missing", "id: example.com/users\n").unwrap();
    let empty = read_service_yml(
        "service-calls-empty",
        "id: example.com/users\nserviceCalls: []\n",
    )
    .unwrap();
    assert!(missing.service.service_calls.is_empty());
    assert_eq!(missing.service.service_calls, empty.service.service_calls);

    let selected = read_service_yml(
        "service-calls-selected",
        "id: example.com/users\nserviceCalls: [worker.run, send]\n",
    )
    .unwrap();
    assert_eq!(
        selected.service.service_calls,
        vec!["send".to_string(), "worker.run".to_string()]
    );
    assert_eq!(selected.package.id, missing.package.id);
    assert_eq!(selected.package.version, missing.package.version);
    assert_eq!(selected.package.api, missing.package.api);
    assert_eq!(selected.package.dependencies, missing.package.dependencies);
    assert_eq!(selected.package.resources, missing.package.resources);
    assert_eq!(selected.package.services, missing.package.services);
}

#[test]
fn service_calls_reject_duplicates_before_canonical_sorting() {
    let error = read_service_yml(
        "service-calls-duplicate",
        "id: example.com/users\nserviceCalls: [worker.run, send, worker.run]\n",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("duplicate public path worker.run"),
        "unexpected error: {error}"
    );
}

#[test]
fn service_calls_reject_wrong_container_item_and_path_shapes() {
    for (name, source, expected) in [
        (
            "scalar",
            "id: example.com/users\nserviceCalls: worker.run\n",
            "must be a list of string public paths",
        ),
        (
            "non-string-item",
            "id: example.com/users\nserviceCalls: [worker.run, 7]\n",
            "serviceCalls[1] must be a string public path",
        ),
        (
            "boolean-item",
            "id: example.com/users\nserviceCalls: [true]\n",
            "serviceCalls[0] must be a string public path",
        ),
        (
            "empty-path",
            "id: example.com/users\nserviceCalls: [\"\"]\n",
            "canonical dotted public path",
        ),
        (
            "whitespace",
            "id: example.com/users\nserviceCalls: [\" worker.run\"]\n",
            "canonical dotted public path",
        ),
        (
            "empty-segment",
            "id: example.com/users\nserviceCalls: [worker..run]\n",
            "canonical dotted public path",
        ),
        (
            "non-identifier-segment",
            "id: example.com/users\nserviceCalls: [worker-run]\n",
            "canonical dotted public path",
        ),
    ] {
        let error = read_service_yml(name, source).unwrap_err().to_string();
        assert!(
            error.contains(expected),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn service_root_requires_package_api_and_service_manifests() {
    for (name, files) in [
        (
            "missing-package",
            vec![("api.yml", "{}\n"), ("service.yml", "id: example.com/a\n")],
        ),
        (
            "missing-api",
            vec![
                ("package.yml", "id: example.com/a\nversion: 1.0.0\n"),
                ("service.yml", "id: example.com/a\n"),
            ],
        ),
        (
            "missing-service",
            vec![
                ("package.yml", "id: example.com/a\nversion: 1.0.0\n"),
                ("api.yml", "{}\n"),
            ],
        ),
    ] {
        let root = fixture_root(name);
        for (file, text) in files {
            write(&root, file, text);
        }
        assert!(read_service_package_root(&root).is_err(), "{name}");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn service_manifest_rejects_package_owned_fields() {
    for field in [
        "version: 1.0.0",
        "packages: []",
        "services: []",
        "api: {}",
        "functions: {}",
    ] {
        let root = fixture_root("service-owned-fields");
        write(&root, "package.yml", "id: example.com/a\nversion: 1.0.0\n");
        write(&root, "api.yml", "{}\n");
        write(
            &root,
            "service.yml",
            &format!("id: example.com/a\n{field}\n"),
        );
        assert!(read_service_package_root(&root).is_err(), "{field}");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn service_manifest_rejects_inline_external_fields_and_removed_metadata() {
    for field in [
        "access: { visibility: internal, organizationRole: viewer }",
        "visibility: internal",
        "organizationRole: viewer",
        "http: {}",
        "websocket: { path: /chat }",
        "timeout: 1000",
        "http: { response: { maxBytes: 1024 }, routes: [] }",
    ] {
        let root = fixture_root("removed-service-metadata");
        write(&root, "package.yml", "id: example.com/a\nversion: 1.0.0\n");
        write(&root, "api.yml", "{}\n");
        write(
            &root,
            "service.yml",
            &format!("id: example.com/a\n{field}\n"),
        );
        assert!(read_service_package_root(&root).is_err(), "{field}");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn http_document_rejects_legacy_shapes_and_unknown_fields() {
    for (name, http) in [
            ("routes", "{ routes: [] }"),
            ("entries", "{ entries: {} }"),
            ("global-guard", "{ guard: users.guard }"),
            ("global-pre", "{ pre: users.prepare }"),
            (
                "operation",
                "{ create: { method: POST, path: /users, kind: typedJson, handler: users.create, operation: create } }",
            ),
            (
                "handler-args",
                "{ create: { method: POST, path: /users, kind: typedJson, handler: users.create, handlerArgs: [] } }",
            ),
            (
                "unknown-entry-field",
                "{ create: { method: POST, path: /users, kind: typedJson, handler: users.create, fallback: true } }",
            ),
            (
                "unknown-adapter-arg-field",
                "{ create: { method: POST, path: /users, kind: typedJson, handler: users.create, adapterArgs: [{ param: body, source: { kind: http.body }, field: value }] } }",
            ),
            (
                "missing-method",
                "{ create: { path: /users, kind: typedJson, handler: users.create } }",
            ),
            (
                "missing-path",
                "{ create: { method: POST, kind: typedJson, handler: users.create } }",
            ),
            (
                "missing-kind",
                "{ create: { method: POST, path: /users, handler: users.create } }",
            ),
            (
                "missing-handler",
                "{ create: { method: POST, path: /users, kind: typedJson } }",
            ),
        ] {
            assert!(
                read_http_yml(name, http).is_err(),
                "{name} must fail closed"
            );
        }
}

#[test]
fn http_document_validates_selectors_and_adapter_args() {
    for (name, entry) in [
            (
                "invalid-key",
                "\"bad key\": { method: GET, path: /users, kind: typedJson, handler: users.read }",
            ),
            (
                "invalid-handler-selector",
                "entry: { method: GET, path: /users, kind: typedJson, handler: read }",
            ),
            (
                "public-root-handler-fallback",
                "entry: { method: GET, path: /users, kind: typedJson, handler: root.users.read }",
            ),
            (
                "invalid-guard-selector",
                "entry: { method: GET, path: /users, kind: typedJson, handler: users.read, guard: guard }",
            ),
            (
                "invalid-pre-selector",
                "entry: { method: GET, path: /users, kind: typedJson, handler: users.read, pre: prepare }",
            ),
            (
                "legacy-host",
                "entry: { host: api.example.com, method: GET, path: /users, kind: typedJson, handler: users.read }",
            ),
            (
                "invalid-path",
                "entry: { method: GET, path: users, kind: typedJson, handler: users.read }",
            ),
            (
                "path-with-query",
                "entry: { method: GET, path: \"/users?admin=true\", kind: typedJson, handler: users.read }",
            ),
            (
                "invalid-method",
                "entry: { method: \"G ET\", path: /users, kind: typedJson, handler: users.read }",
            ),
            (
                "raw-body-source",
                "entry: { method: POST, path: /users, kind: rawHttp, handler: users.raw, adapterArgs: [{ param: body, source: { kind: http.body } }] }",
            ),
            (
                "typed-non-http-source",
                "entry: { method: POST, path: /users, kind: typedJson, handler: users.create, adapterArgs: [{ param: body, source: { kind: websocket.connectRequest } }] }",
            ),
            (
                "context-without-pre",
                "entry: { method: GET, path: /users, kind: typedJson, handler: users.read, adapterArgs: [{ param: context, source: { kind: http.context } }] }",
            ),
            (
                "duplicate-param",
                "entry: { method: POST, path: /users, kind: typedJson, handler: users.create, adapterArgs: [{ param: value, source: { kind: http.request } }, { param: value, source: { kind: http.body } }] }",
            ),
        ] {
            let source = format!("{entry}\n");
            assert!(
                read_http_yml(name, &source).is_err(),
                "{name} must fail closed"
            );
        }
}

#[test]
fn websocket_document_validates_strict_singleton() {
    let path_only = read_websocket_yml("websocket-path-only", "path: /chat\n").unwrap();
    let websocket = path_only.websocket.unwrap();
    assert_eq!(websocket.path, "/chat");
    assert!(websocket.connect.is_none());

    let connected = read_websocket_yml(
        "websocket-connect",
        r#"
path: /chat
connect:
  handler: handlers.connect
  adapterArgs:
    - param: request
      source: { kind: websocket.connectRequest }
    - param: connectionId
      source: { kind: websocket.connectionId }
"#,
    )
    .unwrap();
    assert_eq!(connected.websocket.unwrap().path, "/chat");

    for (name, websocket) in [
            ("websocket-invalid-path", "{ path: chat }"),
            (
                "websocket-invalid-handler",
                "{ path: /chat, connect: { handler: connect } }",
            ),
            (
                "websocket-http-source",
                "{ path: /chat, connect: { handler: handlers.connect, adapterArgs: [{ param: request, source: { kind: http.request } }] } }",
            ),
            (
                "websocket-duplicate-param",
                "{ path: /chat, connect: { handler: handlers.connect, adapterArgs: [{ param: request, source: { kind: websocket.connectRequest } }, { param: request, source: { kind: websocket.connectionId } }] } }",
            ),
            (
                "websocket-blank-param",
                "{ path: /chat, connect: { handler: handlers.connect, adapterArgs: [{ param: \"\", source: { kind: websocket.connectRequest } }] } }",
            ),
        ] {
            assert!(read_websocket_yml(name, websocket).is_err(), "{name}");
        }
}

#[test]
fn websocket_document_rejects_legacy_and_collection_shapes() {
    for (name, websocket) in [
        ("websocket-empty", ""),
        ("websocket-null", "null"),
        ("websocket-list", "[]"),
        ("websocket-scalar", "chat"),
        (
            "websocket-multi-map",
            "{ one: { path: /one }, two: { path: /two } }",
        ),
        ("websocket-missing-path", "{}"),
        ("websocket-null-connect", "{ path: /chat, connect: null }"),
        ("websocket-routes", "{ path: /chat, routes: [] }"),
        ("websocket-operation", "{ path: /chat, operation: receive }"),
        (
            "websocket-receive",
            "{ path: /chat, receive: handlers.receive }",
        ),
        (
            "websocket-message",
            "{ path: /chat, message: handlers.message }",
        ),
        ("websocket-context", "{ path: /chat, context: Context }"),
        ("websocket-author-id", "{ id: chat, path: /chat }"),
        ("websocket-host", "{ host: chat.example.com, path: /chat }"),
        ("websocket-wrapper", "{ websocket: { path: /chat } }"),
    ] {
        assert!(read_websocket_yml(name, websocket).is_err(), "{name}");
    }
}

#[test]
fn http_document_rejects_duplicate_keys_and_selectors() {
    let duplicate_key = r#"
entry:
  method: GET
  path: /users
  kind: typedJson
  handler: users.read
entry:
  method: POST
  path: /users
  kind: typedJson
  handler: users.create
"#;
    assert!(read_http_yml("duplicate-key", duplicate_key).is_err());

    let duplicate_selector = r#"
first:
  method: GET
  path: /users
  kind: typedJson
  handler: users.first
second:
  method: get
  path: /users
  kind: rawHttp
  handler: users.second
"#;
    let error = read_http_yml("duplicate-selector", duplicate_selector).unwrap_err();
    assert!(error.to_string().contains("duplicates the selector"));
}

#[test]
fn http_routes_are_validated_per_service_root() {
    let route = r#"
models:
  method: GET
  path: /v1/models
  kind: typedJson
  handler: models.list
"#;
    let first = read_http_yml("service-one-shared-route", route).unwrap();
    let second = read_http_yml("service-two-shared-route", route).unwrap();
    assert_eq!(first.http.unwrap(), second.http.unwrap());
}

#[test]
fn http_document_accepts_explicit_empty_and_rejects_non_mapping_documents() {
    let empty = read_http_yml("http-empty-map", "{}\n").unwrap();
    assert!(empty.http.unwrap().entries.is_empty());
    for (name, source) in [
        ("http-empty-file", ""),
        ("http-null", "null"),
        ("http-scalar", "route"),
        ("http-list", "[]"),
        ("http-wrapper", "http: {}"),
    ] {
        assert!(read_http_yml(name, source).is_err(), "{name}");
    }
}

#[test]
fn websocket_json_rpc_methods_have_strict_keys_methods_and_sources() {
    let valid = read_websocket_yml(
        "json-rpc-valid",
        r#"path: /chat
jsonRpc:
  status:
    method: status.get
    handler: handlers.status
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
      - param: connectionId
        source: { kind: websocket.connectionId }
      - param: businessIdentity
        source: { kind: websocket.businessIdentity }
"#,
    )
    .unwrap();
    assert_eq!(valid.websocket.unwrap().json_rpc.len(), 1);
    assert!(read_websocket_yml("json-rpc-empty-map", "path: /chat\njsonRpc: {}\n").is_ok());

    for (name, json_rpc, expected) in [
            (
                "json-rpc-null",
                "null",
                "mapping of unique WebSocket JSON-RPC",
            ),
            (
                "json-rpc-empty-method",
                "{ status: { method: \"\", handler: handlers.status, adapterArgs: [{ param: params, source: { kind: websocket.jsonRpcParams } }] } }",
                "must not be empty",
            ),
            (
                "json-rpc-reserved-method",
                "{ status: { method: $/cancelRequest, handler: handlers.status, adapterArgs: [{ param: params, source: { kind: websocket.jsonRpcParams } }] } }",
                "reserved $/",
            ),
            (
                "json-rpc-duplicate-method",
                "{ first: { method: status.get, handler: handlers.first, adapterArgs: [{ param: params, source: { kind: websocket.jsonRpcParams } }] }, second: { method: status.get, handler: handlers.second, adapterArgs: [{ param: params, source: { kind: websocket.jsonRpcParams } }] } }",
                "duplicates",
            ),
            (
                "json-rpc-missing-params",
                "{ status: { method: status.get, handler: handlers.status, adapterArgs: [{ param: connectionId, source: { kind: websocket.connectionId } }] } }",
                "exactly once",
            ),
            (
                "json-rpc-duplicate-params",
                "{ status: { method: status.get, handler: handlers.status, adapterArgs: [{ param: first, source: { kind: websocket.jsonRpcParams } }, { param: second, source: { kind: websocket.jsonRpcParams } }] } }",
                "exactly once",
            ),
            (
                "json-rpc-duplicate-connection",
                "{ status: { method: status.get, handler: handlers.status, adapterArgs: [{ param: params, source: { kind: websocket.jsonRpcParams } }, { param: first, source: { kind: websocket.connectionId } }, { param: second, source: { kind: websocket.connectionId } }] } }",
                "at most once",
            ),
            (
                "json-rpc-wrong-phase-source",
                "{ status: { method: status.get, handler: handlers.status, adapterArgs: [{ param: params, source: { kind: http.body } }] } }",
                "not allowed",
            ),
            (
                "json-rpc-transport-id",
                "{ status: { method: status.get, handler: handlers.status, adapterArgs: [{ param: params, source: { kind: websocket.jsonRpcParams } }, { param: id, source: { kind: websocket.requestId } }] } }",
                "unknown variant",
            ),
            (
                "json-rpc-guard",
                "{ status: { method: status.get, handler: handlers.status, guard: handlers.guard, adapterArgs: [{ param: params, source: { kind: websocket.jsonRpcParams } }] } }",
                "unknown field",
            ),
        ] {
            let source = format!("path: /chat\njsonRpc: {json_rpc}\n");
            let error = read_websocket_yml(name, &source).unwrap_err().to_string();
            assert!(
                error.contains(expected),
                "{name}: expected {expected:?}, got {error}"
            );
        }

    for (name, source) in [
            (
                "json-rpc-duplicate-key",
                "path: /chat\njsonRpc:\n  status: { method: status.get, handler: handlers.one, adapterArgs: [{ param: params, source: { kind: websocket.jsonRpcParams } }] }\n  status: { method: status.set, handler: handlers.two, adapterArgs: [{ param: params, source: { kind: websocket.jsonRpcParams } }] }\n",
            ),
            (
                "json-rpc-duplicate-handler-field",
                "path: /chat\njsonRpc:\n  status:\n    method: status.get\n    handler: handlers.one\n    handler: handlers.two\n    adapterArgs: [{ param: params, source: { kind: websocket.jsonRpcParams } }]\n",
            ),
            (
                "websocket-duplicate-top-level",
                "path: /chat\npath: /other\n",
            ),
        ] {
            assert!(read_websocket_yml(name, source).is_err(), "{name}");
        }
}

#[test]
fn external_documents_require_a_complete_service_root_and_regular_files() {
    for (name, files) in [
        ("external-only", vec![("http.yml", "{}\n")]),
        (
            "package-external-no-service",
            vec![
                ("package.yml", "id: example.com/a\nversion: 1.0.0\n"),
                ("api.yml", "{}\n"),
                ("websocket.yml", "path: /chat\n"),
            ],
        ),
        (
            "service-external-no-api",
            vec![
                ("package.yml", "id: example.com/a\nversion: 1.0.0\n"),
                ("service.yml", "id: example.com/a\n"),
                ("http.yml", "{}\n"),
            ],
        ),
    ] {
        let root = fixture_root(name);
        for (file, source) in files {
            write(&root, file, source);
        }
        assert!(read_service_package_root(&root).is_err(), "{name}");
        fs::remove_dir_all(root).unwrap();
    }

    let root = fixture_root("external-directory");
    write(&root, "package.yml", "id: example.com/a\nversion: 1.0.0\n");
    write(&root, "api.yml", "{}\n");
    write(&root, "service.yml", "id: example.com/a\n");
    fs::create_dir(root.join("http.yml")).unwrap();
    let error = read_service_package_root(&root).unwrap_err().to_string();
    assert!(error.contains("must be a regular file"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

fn read_service_yml(
    name: &str,
    service_yml: &str,
) -> Result<ServicePackageRoot, ServiceSourceConfigError> {
    let root = fixture_root(name);
    write(
        &root,
        "package.yml",
        "id: example.com/users\nversion: 1.0.0\n",
    );
    write(&root, "api.yml", "{}\n");
    write(&root, "service.yml", service_yml);
    let result = read_service_package_root(&root);
    fs::remove_dir_all(root).unwrap();
    result
}

fn read_http_yml(
    name: &str,
    http_yml: &str,
) -> Result<ServicePackageRoot, ServiceSourceConfigError> {
    read_split_yml(name, Some(http_yml), None)
}

fn read_websocket_yml(
    name: &str,
    websocket_yml: &str,
) -> Result<ServicePackageRoot, ServiceSourceConfigError> {
    read_split_yml(name, None, Some(websocket_yml))
}

fn read_split_yml(
    name: &str,
    http_yml: Option<&str>,
    websocket_yml: Option<&str>,
) -> Result<ServicePackageRoot, ServiceSourceConfigError> {
    let root = fixture_root(name);
    write(
        &root,
        "package.yml",
        "id: example.com/users\nversion: 1.0.0\n",
    );
    write(&root, "api.yml", "{}\n");
    write(&root, "service.yml", "id: example.com/users\n");
    if let Some(source) = http_yml {
        write(&root, HTTP_CONFIG_FILE, source);
    }
    if let Some(source) = websocket_yml {
        write(&root, WEBSOCKET_CONFIG_FILE, source);
    }
    let result = read_service_package_root(&root);
    fs::remove_dir_all(root).unwrap();
    result
}

fn fixture_root(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "skiff-service-source-{name}-{}-{unique}",
        process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write(root: &Path, file: &str, text: &str) {
    fs::write(root.join(file), text).unwrap();
}
