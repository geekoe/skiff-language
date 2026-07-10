use super::*;

struct TestDir {
    path: std::path::PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "skiff-service-config-{name}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("temp dir should be created");
        Self { path }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn read_temp_service_config_with_api_yml(
    name: &str,
    service_yml: &str,
    api_yml: &str,
) -> Result<ServiceConfig, ServiceConfigError> {
    let temp = TestDir::new(name);
    std::fs::write(temp.path().join("service.yml"), service_yml).unwrap();
    std::fs::write(temp.path().join("api.yml"), api_yml).unwrap();
    read_service_config(temp.path())
}

#[test]
fn parses_service_config_v1_subset() {
    let config = read_temp_service_config_with_api_yml(
        "v1-subset",
        r#"
id: example.com/websocket_fixture
version: 26.05.11
timeout:
  default: 120000
  methods:
    WebSocketFixtureSocket.actionSync: 180000
websocket: internal.socket.SocketEntry
"#,
        r#"
websocket_fixture_socket: websocket_fixture_api.WebSocketFixtureSocket
admin_socket: admin_api.AdminSocket
"#,
    )
    .unwrap();

    assert_eq!(
        config.publication.id.as_str(),
        "example.com/websocket_fixture"
    );
    assert_eq!(config.publication.version, "26.05.11");
    assert_eq!(
        config.publication.provenance.owner,
        crate::ManifestOwner::ServicePublication
    );
    assert!(!config.publication.provenance.synthetic);
    assert_eq!(
        config.publication.api.module_map()["websocket_fixture_socket"],
        "websocket_fixture_api"
    );
    assert_eq!(
        config.publication.api.module_map()["admin_socket"],
        "admin_api"
    );
    assert!(config.runtime.components.is_empty());
    assert!(config.publication.dependencies.is_empty());
    assert!(config.runtime.services.is_empty());
    assert_eq!(config.runtime.timeout.default, Some(120000));
    assert_eq!(
        config.runtime.timeout.methods["WebSocketFixtureSocket.actionSync"],
        180000
    );
    assert_eq!(config.runtime.dependencies_timeout.default, None);
    assert!(config.runtime.dependencies_timeout.methods.is_empty());
    assert_eq!(
        config.runtime.websocket.unwrap().target.as_deref(),
        Some("internal.socket.SocketEntry")
    );
}

#[test]
fn accepts_hyphenated_service_id() {
    let config = parse_service_config(
        r#"
id: skiff.run/registry
version: 0.1.0
"#,
        Path::new("service.yml"),
    )
    .unwrap();

    assert_eq!(config.publication.id.as_str(), "skiff.run/registry");
}

#[test]
fn accepts_url_like_service_id() {
    let config = read_temp_service_config_with_api_yml(
        "url-like-service-id",
        r#"
id: skiff.run/account
version: 0.1.0
"#,
        r#"
public: api.Public
"#,
    )
    .unwrap();

    assert_eq!(config.publication.id.as_str(), "skiff.run/account");
}

#[test]
fn reads_service_api_yml_entries() {
    let temp = TestDir::new("api-yml");
    std::fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/api
version: 1.0.0
"#,
    )
    .unwrap();
    std::fs::write(
        temp.path().join("api.yml"),
        r#"
Socket: socket.Socket
events:
  send: internal.events.send
"#,
    )
    .unwrap();

    let config = read_service_config(temp.path()).unwrap();
    let entries = config.publication.api.entries().collect::<Vec<_>>();

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].public_path_string(), "Socket");
    assert_eq!(entries[0].source_module_hint(), "socket");
    assert_eq!(entries[0].source_symbol(), "Socket");
    assert_eq!(entries[1].public_path_string(), "events.send");
    assert!(config.publication.api.source.is_some());
}

#[test]
fn parses_service_config_resources() {
    let config = parse_service_config(
        r#"
id: example.com/api
version: 1.0.0
resources:
  - prompts/system.md
  - schemas/tool_input.schema.json
"#,
        Path::new("service.yml"),
    )
    .unwrap();

    assert_eq!(
        config
            .publication
            .resources
            .iter()
            .map(|resource| resource.path.as_str())
            .collect::<Vec<_>>(),
        vec!["prompts/system.md", "schemas/tool_input.schema.json"]
    );
}

#[test]
fn rejects_service_config_invalid_resources() {
    let error = parse_service_config(
        r#"
id: example.com/api
version: 1.0.0
resources:
  - main.skiff
"#,
        Path::new("service.yml"),
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("resources[0] main.skiff is invalid"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_service_profile_overlay_resources_before_merge() {
    let temp = TestDir::new("profile-resources");
    std::fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/api
version: 1.0.0
"#,
    )
    .unwrap();
    std::fs::write(
        temp.path().join("service.prod.yml"),
        r#"
resources:
  - prompts/system.md
"#,
    )
    .unwrap();

    let error = read_service_config_with_profile(temp.path(), Some("prod"))
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("service.prod.yml: field resources is invalid"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("resources must be declared in service.yml"),
        "unexpected error: {error}"
    );
}

#[test]
fn defaults_service_access_to_public() {
    let config = parse_service_config(
        r#"
id: skiff.run/public
version: 0.1.0
"#,
        Path::new("service.yml"),
    )
    .unwrap();

    assert_eq!(config.access.visibility, ServiceVisibility::Public);
    assert_eq!(config.access.organization_role, None);
}

#[test]
fn parses_internal_service_access_with_default_role() {
    let config = parse_service_config(
        r#"
id: skiff.run/account
version: 0.1.0
access:
  visibility: internal
"#,
        Path::new("service.yml"),
    )
    .unwrap();

    assert_eq!(config.access.visibility, ServiceVisibility::Internal);
    assert_eq!(
        config.access.organization_role,
        Some(ServiceOrganizationRole::Viewer)
    );
}

#[test]
fn parses_internal_service_access_role() {
    let config = parse_service_config(
        r#"
id: skiff.run/account
version: 0.1.0
access:
  visibility: internal
  organizationRole: maintainer
"#,
        Path::new("service.yml"),
    )
    .unwrap();

    assert_eq!(config.access.visibility, ServiceVisibility::Internal);
    assert_eq!(
        config.access.organization_role,
        Some(ServiceOrganizationRole::Maintainer)
    );
}

#[test]
fn rejects_invalid_service_access() {
    for (name, yaml, expected) in [
        (
            "invalid-visibility",
            r#"
id: skiff.run/account
version: 0.1.0
access:
  visibility: private
"#,
            "access.visibility",
        ),
        (
            "invalid-role",
            r#"
id: skiff.run/account
version: 0.1.0
access:
  visibility: internal
  organizationRole: admin
"#,
            "access.organizationRole",
        ),
        (
            "public-role",
            r#"
id: skiff.run/account
version: 0.1.0
access:
  visibility: public
  organizationRole: viewer
"#,
            "only allowed when access.visibility is internal",
        ),
    ] {
        let error = parse_service_config(yaml, Path::new("service.yml")).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn rejects_service_yml_top_level_api() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
api:
  WebSocketFixtureSocket:
    interface: api.example.WebSocketFixtureSocket
    handler: internal.websocket_fixture_service
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();
    let message = error.to_string();

    assert!(
        message.contains("field api is invalid")
            && message.contains("has been removed; declare public API in api.yml"),
        "unexpected error: {message}"
    );
}

#[test]
fn reads_service_api_yml_nested_entries() {
    let config = read_temp_service_config_with_api_yml(
        "nested-api-yml",
        r#"
id: example.com/exampleapp
version: 1.0.0
"#,
        r#"
App: api.App
storage:
  Bucket: storage_api.Bucket
"#,
    )
    .unwrap();

    assert_eq!(config.publication.api.module_map()["App"], "api");
    assert_eq!(
        config.publication.api.module_map()["storage.Bucket"],
        "storage_api"
    );
    assert!(config.publication.api.source.is_some());
}

#[test]
fn rejects_invalid_service_api_yml_shapes() {
    for (name, api_yml, expected) in [
        (
            "non-mapping-root",
            r#"
[]
"#,
            "api.yml root must be a mapping",
        ),
        (
            "dotted-public-key",
            r#"
skiff.run.account: api.Account
"#,
            "dotted public keys are not supported",
        ),
        (
            "invalid-key",
            r#"
1: api.Account
"#,
            "api.yml key under <root> must be an identifier segment",
        ),
        (
            "short-selector",
            r#"
storage: Storage
"#,
            "module.path.Symbol",
        ),
        (
            "non-string-leaf",
            r#"
chat:
  send: ["chat.send"]
"#,
            "api.yml public path chat.send must map to a string source selector or nested mapping",
        ),
        (
            "root-selector",
            r#"
User: root.types.User
"#,
            "root. prefix",
        ),
    ] {
        let error = read_temp_service_config_with_api_yml(
            name,
            r#"
id: example.com/exampleapp
version: 1.0.0
"#,
            api_yml,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn rejects_empty_service_version() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: ""
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "service.yml: field version cannot be empty"
    );
}

#[test]
fn rejects_legacy_service_exports() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
exports:
  - module: api.example
    path: ""
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("field exports is invalid")
            && error.to_string().contains("top-level api bindings"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_legacy_service_interfaces() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
interfaces:
  - module: api.example
    path: ""
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("field interfaces is invalid")
            && error.to_string().contains("top-level api bindings"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_legacy_gateway_websocket() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
gateway:
  websocket:
    id: client
    path: /ws
    receive:
      operation: WebSocketFixtureConnection.receive
      bind:
        message: message
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("gateway.websocket")
            && error.to_string().contains("use top-level websocket"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_top_level_http_and_websocket_entry_targets() {
    let config = parse_service_config(
        r#"
id: example.com/entry
version: 1.0.0
http: internal.http.HttpEntry
websocket: internal.socket.SocketEntry
"#,
        Path::new("service.yml"),
    )
    .unwrap();

    assert_eq!(
        config.runtime.http.unwrap().entry.unwrap().target,
        "internal.http.HttpEntry"
    );
    assert_eq!(
        config.runtime.websocket.unwrap().target.as_deref(),
        Some("internal.socket.SocketEntry")
    );
    assert!(config.publication.api.module_map().is_empty());
}

#[test]
fn rejects_websocket_route_object_config() {
    let error = parse_service_config(
        r#"
id: example.com/routes
version: 1.0.0
websocket:
  connect: internal.socket.connect
  routes:
    - path: /chat/send
      handler: internal.chat.send
    - path: /chat/cancel
      handler: internal.chat.cancel
  receive: internal.socket.receive
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("websocket routes are no longer supported"));
}

#[test]
fn rejects_top_level_websocket_bind_config() {
    let error = parse_service_config(
        r#"
id: example.com/routes
version: 1.0.0
websocket:
  connect: internal.socket.connect
  bind:
    request: request
  receive: internal.socket.receive
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("did not match any variant of untagged enum RawWebSocketConfig"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_invalid_websocket_route_object_config() {
    for (name, yaml, expected) in [
        (
            "empty-routes",
            r#"
id: example.com/routes
version: 1.0.0
websocket:
  routes: []
  receive: internal.socket.receive
"#,
            "websocket routes are no longer supported",
        ),
        (
            "relative-path",
            r#"
id: example.com/routes
version: 1.0.0
websocket:
  routes:
    - path: chat/send
      handler: internal.chat.send
  receive: internal.socket.receive
"#,
            "websocket routes are no longer supported",
        ),
        (
            "missing-receive",
            r#"
id: example.com/routes
version: 1.0.0
websocket:
  routes:
    - path: /chat/send
      handler: internal.chat.send
"#,
            "websocket routes are no longer supported",
        ),
        (
            "duplicate-route",
            r#"
id: example.com/routes
version: 1.0.0
websocket:
  routes:
    - path: /chat/send
      handler: internal.chat.send
    - path: /chat/send
      handler: internal.chat.sendAgain
  receive: internal.socket.receive
"#,
            "websocket routes are no longer supported",
        ),
    ] {
        let error = parse_service_config(yaml, Path::new("service.yml")).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn parses_http_routes_with_optional_and_explicit_method() {
    let config = parse_service_config(
        r#"
id: example.com/routes
version: 1.0.0
packages:
    - id: skiff.run/http-session
      version: 1.0.0
      alias: httpSession
http:
  guard: httpSession.guard
  routes:
    - path: /session
      handler: httpSession.issue
    - method: get
      path: /track
      handler: root.internal.track.record
  response:
    maxBytes: 1024
"#,
        Path::new("service.yml"),
    )
    .unwrap();

    let http = config.runtime.http.unwrap();
    assert!(http.entry.is_none());
    assert_eq!(http.guard.as_deref(), Some("httpSession.guard"));
    assert_eq!(http.routes[0].method, None);
    assert_eq!(http.routes[0].path, "/session");
    assert_eq!(http.routes[0].handler, "httpSession.issue");
    assert_eq!(http.routes[1].method.as_deref(), Some("GET"));
    assert_eq!(http.routes[1].path, "/track");
    assert_eq!(http.routes[1].handler, "root.internal.track.record");
    assert_eq!(http.response.unwrap().max_bytes, Some(1024));
    assert_eq!(
        config.publication.dependencies[0].id,
        "skiff.run/http-session"
    );
}

#[test]
fn rejects_invalid_http_routes() {
    for (name, yaml, expected) in [
        (
            "relative-path",
            r#"
id: example.com/routes
version: 1.0.0
http:
  routes:
    - path: session
      handler: root.internal.http.issue
"#,
            "path must start with /",
        ),
        (
            "empty-handler",
            r#"
id: example.com/routes
version: 1.0.0
http:
  routes:
    - path: /session
      handler: ""
"#,
            "handler cannot be empty",
        ),
        (
            "empty-guard",
            r#"
id: example.com/routes
version: 1.0.0
http:
  guard: ""
  routes:
    - path: /session
      handler: root.internal.http.issue
"#,
            "http.guard cannot be empty",
        ),
        (
            "duplicate-effective-method",
            r#"
id: example.com/routes
version: 1.0.0
http:
  routes:
    - path: /session
      handler: root.internal.http.issue
    - method: POST
      path: /session
      handler: root.internal.http.refresh
"#,
            "duplicate HTTP route key /session",
        ),
    ] {
        let error = parse_service_config(yaml, Path::new("service.yml")).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn rejects_legacy_gateway_websocket_even_with_top_level_websocket() {
    let error = parse_service_config(
        r#"
id: example.com/entry
version: 1.0.0
websocket: internal.socket.SocketEntry
gateway:
  websocket:
    id: client
    path: /ws
    receive:
      operation: Socket.receive
      bind:
        message: message
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("use top-level websocket"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_service_package_dependencies() {
    let config = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
packages:
    - id: example.com/billing
      version: 2.0.0
      alias: billing
    - id: google.com/cloud
      version: 3.0.0
      alias: cloud
      collection_name_mapping:
        Session: registry_session
"#,
        Path::new("service.yml"),
    )
    .unwrap();

    assert_eq!(config.publication.dependencies[0].id, "example.com/billing");
    assert_eq!(config.publication.dependencies[0].version, "2.0.0");
    assert_eq!(
        config.publication.dependencies[0].alias.as_deref(),
        Some("billing")
    );
    assert_eq!(
        config.publication.dependencies[0].effective_alias(),
        "billing"
    );
    assert_eq!(config.publication.dependencies[1].id, "google.com/cloud");
    assert_eq!(config.publication.dependencies[1].version, "3.0.0");
    assert_eq!(
        config.publication.dependencies[1].alias.as_deref(),
        Some("cloud")
    );
    assert_eq!(
        config.publication.dependencies[1].effective_alias(),
        "cloud"
    );
    assert_eq!(
        config.publication.dependencies[1]
            .collection_name_mapping
            .get("Session")
            .map(String::as_str),
        Some("registry_session")
    );
    assert!(config.publication.dependencies[0]
        .collection_name_mapping
        .is_empty());
}

#[test]
fn rejects_service_package_dependency_bindings() {
    let error = parse_service_config(
        r#"
id: example.com/agent
version: 1.0.0
packages:
    - id: skiff.run/server-side-agent
      version: 0.1.0
      alias: agent
      bindings:
        - alias: managedLlm
          instance: remoteLlm.managedLlmService
        - interface: llm.ManagedLlmService
          instance: root.internal.llm.managedLlmService
        - alias: streamingLlm
          interface: llm.StreamingLlmService
          instance: remoteLlm.streamingLlmService
"#,
        Path::new("service.yml"),
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains(
            "packages.bindings is invalid: has been removed; pass any interface values as package entry parameters"
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_service_binding_requirements() {
    let error = parse_service_config(
        r#"
id: example.com/agent
version: 1.0.0
requires:
  bindings:
    - alias: managedLlm
      interface: llm.ManagedLlmService
"#,
        Path::new("service.yml"),
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains(
            "requires.bindings has been removed; pass any interface values as package entry parameters"
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_legacy_service_requires_fields() {
    for (name, requires_yaml, expected) in [
        (
            "legacy-bindings",
            r#"
  bindings:
    - interface: llm.ManagedLlmService
"#,
            "requires.bindings has been removed",
        ),
        (
            "legacy-services",
            r#"
  services:
    - id: example.com/service
"#,
            "requires.services has been removed",
        ),
    ] {
        let error = parse_service_config(
            &format!(
                r#"
id: example.com/agent
version: 1.0.0
requires:{requires_yaml}
"#
            ),
            Path::new("service.yml"),
        )
        .expect_err(name)
        .to_string();

        assert!(error.contains(expected), "{name}: {error}");
    }
}

#[test]
fn rejects_invalid_service_package_dependency_bindings() {
    for (name, binding_yaml, expected) in [
        (
            "missing-alias-and-interface",
            r#"
        - instance: remoteLlm.managedLlmService
"#,
            "packages.bindings is invalid: has been removed",
        ),
        (
            "empty-alias",
            r#"
        - alias: ""
          instance: remoteLlm.managedLlmService
"#,
            "packages.bindings is invalid: has been removed",
        ),
        (
            "empty-interface",
            r#"
        - interface: " "
          instance: remoteLlm.managedLlmService
"#,
            "packages.bindings is invalid: has been removed",
        ),
        (
            "missing-instance",
            r#"
        - alias: managedLlm
"#,
            "packages.bindings is invalid: has been removed",
        ),
        (
            "empty-instance",
            r#"
        - alias: managedLlm
          instance: ""
"#,
            "packages.bindings is invalid: has been removed",
        ),
        (
            "unqualified-instance",
            r#"
        - alias: managedLlm
          instance: managedLlmService
"#,
            "packages.bindings is invalid: has been removed",
        ),
        (
            "remote-source-path",
            r#"
        - alias: managedLlm
          instance: remoteLlm.internal.llm.managedLlmService
"#,
            "packages.bindings is invalid: has been removed",
        ),
    ] {
        let yaml = format!(
            r#"
id: example.com/agent
version: 1.0.0
packages:
    - id: skiff.run/server-side-agent
      version: 0.1.0
      alias: agent
      bindings:{binding_yaml}
"#
        );
        let error = parse_service_config(&yaml, Path::new("service.yml")).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn parses_top_level_service_dependencies() {
    let config = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
services:
    - id: skiff.run/account
      version: 0.1.0
      alias: account
    - id: example.com/notifications
      version: 2.0.0
      alias: notifications
"#,
        Path::new("service.yml"),
    )
    .unwrap();

    assert_eq!(config.runtime.services[0].id, "skiff.run/account");
    assert_eq!(config.runtime.services[0].version, "0.1.0");
    assert_eq!(config.runtime.services[0].alias, "account");
    assert_eq!(config.runtime.services[1].id, "example.com/notifications");
    assert_eq!(config.runtime.services[1].version, "2.0.0");
    assert_eq!(config.runtime.services[1].alias, "notifications");
    assert_eq!(
        config.publication.service_dependencies,
        config.runtime.services
    );
}

#[test]
fn rejects_nested_service_dependencies() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
dependencies:
  services:
    - id: skiff.run/account
      version: 0.1.0
      alias: account
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("dependencies.services")
            && error.to_string().contains("top-level services"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_nested_package_dependencies() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
dependencies:
  packages:
    - id: skiff.run/example
      version: 1.0.0
      alias: example
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("dependencies.packages")
            && error.to_string().contains("top-level packages"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_invalid_service_dependency_aliases() {
    for (name, yaml, expected) in [
        (
            "missing-alias",
            r#"
id: example.com/websocket_fixture
version: 1.0.0
services:
    - id: skiff.run/account
      version: 0.1.0
"#,
            "missing required field services.alias",
        ),
        (
            "reserved",
            r#"
id: example.com/websocket_fixture
version: 1.0.0
services:
    - id: skiff.run/account
      version: 0.1.0
      alias: root
"#,
            "alias root uses a reserved service name",
        ),
        (
            "duplicate",
            r#"
id: example.com/websocket_fixture
version: 1.0.0
services:
    - id: skiff.run/account
      version: 0.1.0
      alias: account
    - id: skiff.run/account_v2
      version: 0.1.0
      alias: account
"#,
            "services alias account is assigned to more than one service",
        ),
    ] {
        let error = parse_service_config(yaml, Path::new("service.yml")).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn rejects_reserved_or_duplicate_service_package_aliases() {
    for (name, yaml) in [
        (
            "reserved",
            r#"
id: example.com/websocket_fixture
version: 1.0.0
packages:
    - id: google.com/cloud
      version: 1.0.0
      alias: std
"#,
        ),
        (
            "duplicate",
            r#"
id: example.com/websocket_fixture
version: 1.0.0
packages:
    - id: google.com/cloud
      version: 1.0.0
      alias: cloud
    - id: example.org/cloud
      version: 1.0.0
      alias: cloud
"#,
        ),
    ] {
        let error = parse_service_config(yaml, Path::new("service.yml")).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("packages"),
            "unexpected error for {name}: {message}"
        );
    }
}

#[test]
fn rejects_invalid_service_package_id() {
    for (package_id, expected) in [
        ("./cloud", "must be a publication id"),
        ("skiff.run/std", "platform std is built into the compiler"),
        ("std.foo", "official standard package is skiff.run/std"),
    ] {
        let yaml = format!(
            r#"
id: example.com/websocket_fixture
version: 1.0.0
packages:
    - id: {package_id}
      version: 1.0.0
"#
        );
        let error = parse_service_config(&yaml, Path::new("service.yml")).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains(expected),
            "unexpected error for {package_id}: {message}"
        );
    }
}

#[test]
fn rejects_service_package_dependency_missing_version() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
packages:
    - id: skiff.run/std
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("missing required field packages.version"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_std_and_parses_complex_service_package_dependencies() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
packages:
    - id: skiff.run/std
      version: 1.0.0
      alias: std
    - id: skiff.run/example
      version: 1.0.0
      alias: example
"#,
        Path::new("service.yml"),
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("platform std is built into the compiler"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_complex_service_package_dependency_without_alias() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
packages:
    - id: skiff.run/example
      version: 1.0.0
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("packages entry skiff.run/example requires alias"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_legacy_websocket_context_without_connect_before_legacy_validation() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
gateway:
  websocket:
    id: client
    path: /ws
    context:
      type: ConnectionContext
    receive:
      operation: WebSocketFixtureConnection.receive
      bind:
        message: message
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("use top-level websocket"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_non_empty_unimplemented_config_fields() {
    for (field, yaml) in [
        (
            "components",
            r#"
id: example.com/websocket_fixture
version: 1.0.0
components:
  UserRepo: internal.user_repo.MongoUserRepo
"#,
        ),
        (
            "dependencies.services",
            r#"
id: example.com/websocket_fixture
version: 1.0.0
dependencies:
  services:
    - account.UserQuery
"#,
        ),
        (
            "dependenciesTimeout",
            r#"
id: example.com/websocket_fixture
version: 1.0.0
dependenciesTimeout:
  default: 60000
"#,
        ),
    ] {
        let error = parse_service_config(yaml, Path::new("service.yml")).unwrap_err();
        let message = error.to_string();
        if field == "dependencies.services" {
            assert!(
                message.contains(field) && message.contains("top-level services"),
                "unexpected error for {field}: {message}"
            );
        } else {
            assert!(
                message.contains(field) && message.contains("not implemented yet"),
                "unexpected error for {field}: {message}"
            );
        }
    }
}

#[test]
fn rejects_legacy_http_gateway_routes() {
    let error = parse_service_config(
        r#"
id: example.com/sample
version: 1.0.0
gateway:
  http:
    routes:
      - id: legacy
        method: GET
        path: /api/legacy
        operation: LegacyApi.fetch
        bind: {}
        responseMode: httpEnvelope
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("gateway.http")
            || error.to_string().contains("unknown field `http`"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_unknown_root_field() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
unexpected: true
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("unexpected") || error.to_string().contains("unknown field"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_top_level_http_session_config_field() {
    let error = parse_service_config(
        r#"
id: example.com/example
version: 1.0.0
httpSession:
  cookieName: session
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("httpSession") || error.to_string().contains("unknown field"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_package_runtime_config_in_service_yml() {
    let error = parse_service_config(
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: skiff.run/llm
    version: 1.0.0
    alias: llm
    config:
      apiKey: sk-local
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    let message = error.to_string();
    assert!(
        message.contains("packages.config"),
        "unexpected error: {error}"
    );
    assert!(
        message.contains("config source packages.<alias>") && message.contains("not service.yml"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_http_response_limit() {
    let config = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
http:
  response:
    maxBytes: 134217728
"#,
        Path::new("service.yml"),
    )
    .unwrap();

    let max_bytes = config
        .runtime
        .http
        .and_then(|http| http.response)
        .and_then(|response| response.max_bytes)
        .unwrap();
    assert_eq!(max_bytes, 134217728);
}

#[test]
fn rejects_http_response_limit_zero() {
    let error = parse_service_config(
        r#"
id: example.com/websocket_fixture
version: 1.0.0
http:
  response:
    maxBytes: 0
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("http.response.maxBytes"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_invalid_service_id() {
    let error = parse_service_config(
        r#"
id: example.com/WebSocketFixture
version: 1.0.0
"#,
        Path::new("service.yml"),
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("publication id contains an invalid local segment"),
        "unexpected error: {error}"
    );
}

#[test]
fn reports_missing_core_field() {
    let error = parse_service_config("interfaces: []\n", Path::new("service.yml")).unwrap_err();
    assert_eq!(error.to_string(), "service.yml: missing required field id");
}

#[test]
fn reports_missing_service_version() {
    let error =
        parse_service_config("id: example.com/exampleapp\n", Path::new("service.yml")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "service.yml: missing required field version"
    );
}
