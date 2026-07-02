use std::fs;

use serde_json::{json, Value};
use skiff_compiler::PublicationId;

mod common;
use common::artifacts::{
    assert_publish_error_contains_without_package_dirs as assert_publish_error_contains,
    build_temp_service_publication_without_package_dirs as build_temp_service_publication,
};
use common::TestDir;

#[test]
fn raw_http_contract_uses_prelude_envelope_schemas_without_local_declarations() {
    let temp = write_service_project(
        "raw-http",
        r#"
            interface RawHttp {
              function handle(request: HttpRequest) -> HttpResponse
            }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.handle")
        .unwrap();
    let request_schema = serde_json::to_value(&operation.parameters[0].schema).unwrap();
    let response_schema = serde_json::to_value(&operation.response).unwrap();

    assert_eq!(
        published
            .manifest
            .gateway
            .as_ref()
            .unwrap()
            .http
            .as_ref()
            .unwrap()
            .raw
            .as_ref()
            .unwrap()
            .operation,
        "RawHttp.handle"
    );
    assert_eq!(
        published.artifacts.service_assembly.value["gateway"]["http"]["raw"]["target"],
        gateway_target("example.com/example", "http.raw")
    );

    assert_eq!(request_schema["type"], "object");
    assert_eq!(request_schema["xSkiffSymbol"], "std.http.HttpRequest");
    assert_eq!(request_schema["properties"]["method"]["type"], "string");
    assert_eq!(request_schema["properties"]["query"]["type"], "array");
    assert_eq!(
        request_schema["properties"]["query"]["items"]["xSkiffSymbol"],
        "std.http.HttpQueryParam"
    );
    assert_bytes_body_schema(&request_schema["properties"]["body"]);

    assert_eq!(response_schema["type"], "object");
    assert_eq!(response_schema["xSkiffSymbol"], "std.http.HttpResponse");
    assert_eq!(response_schema["properties"]["status"]["type"], "integer");
    assert_bytes_body_schema(&response_schema["properties"]["body"]);

    let assembly_operation = published.artifacts.service_assembly.value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"] == "RawHttp.handle")
        .unwrap();
    assert_eq!(
        assembly_operation["parameters"][0]["schema"]["xSkiffSymbol"],
        "std.http.HttpRequest"
    );
    assert_eq!(
        assembly_operation["response"]["xSkiffSymbol"],
        "std.http.HttpResponse"
    );

    let assembly = &published.artifacts.service_assembly.value;
    assert!(assembly.get("packages").is_none());
    assert!(assembly["preludeIdentity"]
        .as_str()
        .unwrap()
        .starts_with("skiff-prelude-v1:sha256:"));
    assert!(assembly["prelude"]["schemaIdentity"].is_string());
}

#[test]
fn explicit_std_imports_keep_prelude_metadata_separate_from_package_lock() {
    let temp = write_service_project_with_internal(
        "explicit-std-import-preinstalled-metadata",
        r#"
            interface RawHttp {
              function handle(request: HttpRequest) -> HttpResponse
            }
        "#,
        r#"
            import std
            import std
              function handle(request: HttpRequest) -> HttpResponse {
                return std.http.noContent()
              }
        "#,
    );
    add_raw_http_std_dependency(&temp);

    let published = build_temp_service_publication(temp.path());
    let assembly = &published.artifacts.service_assembly.value;

    assert!(assembly.get("packages").is_none());
    assert!(published
        .artifacts
        .package_assemblies
        .iter()
        .any(|assembly| assembly.value["package"]["id"] == "skiff.run/std"));
    assert!(
        published.artifacts.service_unit.value["packageDependencies"]
            .as_array()
            .is_some_and(|dependencies| dependencies
                .iter()
                .any(|dependency| dependency["id"] == "skiff.run/std"))
    );
    assert!(assembly["preludeIdentity"].is_string());
    assert!(!assembly["prelude"]["types"]
        .as_array()
        .unwrap()
        .iter()
        .any(|ty| ty == "HttpRequest"));
    assert!(!assembly["prelude"]["types"]
        .as_array()
        .unwrap()
        .iter()
        .any(|ty| ty == "SecretString"));
    assert!(assembly["prelude"]["roots"]
        .as_array()
        .unwrap()
        .iter()
        .any(|root| root == "std"));
    assert!(assembly["prelude"]["roots"]
        .as_array()
        .unwrap()
        .iter()
        .any(|root| root == "config"));
    assert!(published
        .artifacts
        .package_assemblies
        .iter()
        .all(|assembly| assembly.value["package"]["id"] != "skiff.run/core"));
}

#[test]
fn standard_package_schema_types_emit_qualified_boundary_schemas() {
    let temp = TestDir::new("skiff-prelude-schema-v1", "raw-http-client");
    fs::create_dir_all(temp.path().join("api")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api.yml"),
        r#"
RawHttpClient: internal.http_client.RawHttpClient
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("http_client.skiff"),
        r#"
            import std

            interface RawHttpClient {
              function fetch(request: std.http.HttpClientRequest) -> std.http.HttpClientResponse
            }
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("http_client.skiff"),
        source_with_api_implementation(
            "RawHttpClient",
            "internal.http_client",
            r#"
            import std

            function fetch(request: std.http.HttpClientRequest) -> std.http.HttpClientResponse {
              return {
                status: 200,
                headers: Array.empty<std.http.HttpHeader>(),
                body: bytes.fromUtf8(""),
              }
            }
        "#,
        ),
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttpClient.fetch")
        .unwrap();
    let request_schema = serde_json::to_value(&operation.parameters[0].schema).unwrap();
    let response_schema = serde_json::to_value(&operation.response).unwrap();

    assert_eq!(request_schema["xSkiffSymbol"], "std.http.HttpClientRequest");
    assert_eq!(
        request_schema["properties"]["headers"]["items"]["xSkiffSymbol"],
        "std.http.HttpHeader"
    );
    assert_bytes_body_schema(&request_schema["properties"]["body"]);
    assert_eq!(
        response_schema["xSkiffSymbol"],
        "std.http.HttpClientResponse"
    );
    assert_eq!(
        response_schema["properties"]["headers"]["items"]["xSkiffSymbol"],
        "std.http.HttpHeader"
    );
    assert_bytes_body_schema(&response_schema["properties"]["body"]);
}

#[test]
fn standard_package_discriminator_unions_emit_declared_discriminator_schema() {
    let temp = write_service_project_with_internal(
        "std-http-discriminator-union-schema",
        r#"
            import std

            interface RawHttp {
              function handle(event: std.http.HttpSseEvent) -> std.http.HttpSseEvent
            }
        "#,
        r#"
            import std
              function handle(event: std.http.HttpSseEvent) -> std.http.HttpSseEvent {
                return event
              }
        "#,
    );
    add_raw_http_std_dependency(&temp);

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.handle")
        .unwrap();
    let parameter_schema = serde_json::to_value(&operation.parameters[0].schema).unwrap();
    let response_schema = serde_json::to_value(&operation.response).unwrap();
    let branches = parameter_schema["oneOf"].as_array().unwrap();

    assert_eq!(parameter_schema["xSkiffUnionDiscriminator"], "tag");
    assert_eq!(response_schema["xSkiffUnionDiscriminator"], "tag");
    assert!(branches.iter().any(|branch| {
        branch["xSkiffUnionBranch"] == "event"
            && branch["properties"]["tag"]["enum"] == json!(["event"])
            && branch["properties"]["data"]["type"] == "string"
    }));
}

#[test]
fn service_assembly_operations_include_compiled_standard_package_schemas() {
    let temp = write_service_project_with_internal(
        "assembly-compiled-standard-package-schemas",
        r#"
            import std

            interface RawHttp {
              function handle(payload: Map<string, bytes>) -> JsonObject
            }
        "#,
        r#"
            import std
              function handle(payload: Map<string, bytes>) -> JsonObject {
                return {}
              }
        "#,
    );
    add_raw_http_std_dependency(&temp);

    let published = build_temp_service_publication(temp.path());
    let operation = published.artifacts.service_assembly.value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"] == "RawHttp.handle")
        .unwrap();

    assert_eq!(operation["parameters"][0]["schema"]["type"], "object");
    assert_eq!(
        operation["parameters"][0]["schema"]["additionalProperties"]["xSkiffSymbol"],
        "std.bytes.bytes"
    );
    assert_eq!(
        operation["parameters"][0]["schema"]["additionalProperties"]["contentEncoding"],
        "base64"
    );
    assert_eq!(operation["response"]["xSkiffSymbol"], "JsonObject");
}

#[test]
fn sample_like_bare_http_request_publish_succeeds() {
    let temp = TestDir::new("skiff-prelude-schema-v1", "sample-like-http");
    fs::create_dir_all(temp.path().join("api")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/sample
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api.yml"),
        r#"
SampleHttp: internal.http.SampleHttp
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("http.skiff"),
        r#"
            interface SampleHttp {
              function handle(request: HttpRequest) -> HttpResponse
            }
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("http.skiff"),
        source_with_api_implementation(
            "SampleHttp",
            "internal.http",
            r#"
            function handle(request: HttpRequest) -> HttpResponse {
              return std.http.noContent()
            }
        "#,
        ),
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());

    assert_eq!(published.manifest.service.id, "example.com/sample");
    assert!(published
        .manifest
        .operations
        .iter()
        .any(|operation| operation.operation == "SampleHttp.handle"));
    let assembly = &published.artifacts.service_assembly.value;
    assert!(assembly.get("packages").is_none());
    assert!(assembly["preludeIdentity"]
        .as_str()
        .unwrap()
        .starts_with("skiff-prelude-v1:sha256:"));
}

#[test]
fn redeclaring_reserved_standard_type_fails_publish() {
    let temp = write_service_project(
        "redeclare-http-request",
        r#"
            type HttpRequest {}
            interface RawHttp {
              function handle(request: HttpRequest) -> HttpResponse
            }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "type HttpRequest",
            "reserved prelude name",
        ],
    );
}

#[test]
fn redeclaring_reserved_connection_message_fails_publish() {
    let temp = write_service_project(
        "redeclare-connection-message",
        r#"
            type ConnectionMessage {}
            interface RawHttp {
              function receive(message: ConnectionMessage) -> ConnectionMessage?
            }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "type ConnectionMessage",
            "reserved prelude name",
        ],
    );
}

#[test]
fn connection_message_branch_types_emit_concrete_schemas() {
    let temp = write_service_project_with_internal(
        "connection-message-branches",
        r#"
            import std

            interface RawHttp {
              function handle(message: std.websocket.TextConnectionMessage) -> std.websocket.BinaryConnectionMessage
            }
        "#,
        r#"
            import std

              function handle(message: std.websocket.TextConnectionMessage) -> std.websocket.BinaryConnectionMessage {
                return { tag: "binary", base64: "" }
              }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.handle")
        .unwrap();
    let parameter_schema = serde_json::to_value(&operation.parameters[0].schema).unwrap();
    let response_schema = serde_json::to_value(&operation.response).unwrap();

    assert_eq!(
        parameter_schema["xSkiffSymbol"],
        "std.websocket.TextConnectionMessage"
    );
    assert_eq!(
        json_array_values(&parameter_schema["required"]),
        ["tag", "text"]
    );
    assert_eq!(
        response_schema["xSkiffSymbol"],
        "std.websocket.BinaryConnectionMessage"
    );
    assert_eq!(
        json_array_values(&response_schema["required"]),
        ["base64", "tag"]
    );
}

#[test]
fn standard_websocket_fixed_event_types_emit_schemas() {
    let temp = write_service_project_with_internal(
        "std-websocket-fixed-event-exports",
        r#"
            import std

            type ConnectionContext { userId: string }
            interface RawHttp {
              function receive(event: std.websocket.WebSocketReceiveEvent<ConnectionContext>) -> std.websocket.ConnectionMessage
              function connection(connection: std.websocket.WebSocketConnection<ConnectionContext>) -> std.websocket.WebSocketConnectResult<ConnectionContext>
            }
        "#,
        r#"
            import std
              function receive(event: std.websocket.WebSocketReceiveEvent<root.api.raw_http.ConnectionContext>) -> std.websocket.ConnectionMessage {
                return { tag: "text", text: "" }
              }

              function connection(connection: std.websocket.WebSocketConnection<root.api.raw_http.ConnectionContext>) -> std.websocket.WebSocketConnectResult<root.api.raw_http.ConnectionContext> {
                return {
                  tag: "accept",
                  context: root.api.raw_http.ConnectionContext { userId: "" },
                  businessIdentity: null,
                }
              }
        "#,
    );
    add_raw_http_std_dependency(&temp);

    let published = build_temp_service_publication(temp.path());

    let receive = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.receive")
        .unwrap();
    let receive_schema = serde_json::to_value(&receive.parameters[0].schema).unwrap();
    let receive_response_schema = serde_json::to_value(&receive.response).unwrap();
    let connection = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.connection")
        .unwrap();
    let connection_schema = serde_json::to_value(&connection.parameters[0].schema).unwrap();
    let connect_result_schema = serde_json::to_value(&connection.response).unwrap();

    assert_eq!(
        receive_schema["properties"]["connection"]["properties"]["businessIdentity"]["type"],
        "string"
    );
    assert!(receive_schema["properties"]["connection"]["properties"]
        .get("identity")
        .is_none());
    assert_eq!(
        receive_schema["properties"]["message"]["xSkiffSymbol"],
        "std.websocket.ConnectionMessage"
    );
    assert_eq!(
        receive_response_schema["xSkiffSymbol"],
        "std.websocket.ConnectionMessage"
    );
    assert_eq!(receive_response_schema["xSkiffUnionDiscriminator"], "tag");
    assert!(receive_response_schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .any(|branch| {
            branch["xSkiffUnionBranch"] == "text"
                && branch["properties"]["tag"]["enum"] == json!(["text"])
                && branch["properties"]["text"]["type"] == "string"
        }));
    assert_eq!(connection_schema["properties"]["id"]["type"], "string");
    assert_eq!(
        connection_schema["properties"]["businessIdentity"]["type"],
        "string"
    );
    assert!(connection_schema["properties"].get("identity").is_none());
    assert_eq!(connect_result_schema["oneOf"].as_array().unwrap().len(), 2);
}

#[test]
fn connection_message_union_with_extra_branch_does_not_collapse_to_transport_schema() {
    let temp = write_service_project_with_internal(
        "connection-message-extra-branch",
        r#"
            import std

            type OtherMessage {
              value: string
            }
            interface RawHttp {
              function handle(message: std.websocket.TextConnectionMessage | std.websocket.BinaryConnectionMessage | OtherMessage) -> std.websocket.TextConnectionMessage | std.websocket.BinaryConnectionMessage | OtherMessage
            }
        "#,
        r#"
            import std

              function handle(message: std.websocket.TextConnectionMessage | std.websocket.BinaryConnectionMessage | root.api.raw_http.OtherMessage) -> std.websocket.TextConnectionMessage | std.websocket.BinaryConnectionMessage | root.api.raw_http.OtherMessage {
                return message
              }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.handle")
        .unwrap();
    let parameter_schema = serde_json::to_value(&operation.parameters[0].schema).unwrap();
    let response_schema = serde_json::to_value(&operation.response).unwrap();
    let parameter_branches = parameter_schema["oneOf"].as_array().unwrap();
    let response_branches = response_schema["oneOf"].as_array().unwrap();

    assert_ne!(
        parameter_schema["xSkiffSymbol"],
        "std.websocket.ConnectionMessage"
    );
    assert_ne!(
        response_schema["xSkiffSymbol"],
        "std.websocket.ConnectionMessage"
    );
    assert_eq!(parameter_branches.len(), 3);
    assert_eq!(response_branches.len(), 3);
    assert!(parameter_branches
        .iter()
        .any(|branch| branch["properties"]["value"]["type"] == "string"));
    assert!(response_branches
        .iter()
        .any(|branch| branch["properties"]["value"]["type"] == "string"));
}

#[test]
fn explicit_discriminator_named_union_uses_declared_field() {
    let temp = write_service_project_with_internal(
        "explicit-discriminator-union",
        r#"
            type Event discriminator "kind" =
              { kind: "text", text: string }
              | { kind: "count", count: number }
            interface RawHttp {
              function handle(event: Event) -> Event
            }
        "#,
        r#"
              function handle(event: root.api.raw_http.Event) -> root.api.raw_http.Event {
                return event
              }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.handle")
        .unwrap();
    let parameter_schema = serde_json::to_value(&operation.parameters[0].schema).unwrap();
    let response_schema = serde_json::to_value(&operation.response).unwrap();
    let branches = parameter_schema["oneOf"].as_array().unwrap();

    assert_eq!(parameter_schema["xSkiffUnionDiscriminator"], "kind");
    assert_eq!(response_schema["xSkiffUnionDiscriminator"], "kind");
    assert!(branches.iter().any(|branch| {
        branch["type"] == "object"
            && branch["properties"]["kind"]["enum"] == json!(["text"])
            && branch["properties"]["text"]["type"] == "string"
            && branch["xSkiffUnionBranch"] == "text"
            && branch["required"] == json!(["kind", "text"])
    }));
    assert!(branches.iter().any(|branch| {
        branch["type"] == "object"
            && branch["properties"]["kind"]["enum"] == json!(["count"])
            && branch["properties"]["count"]["type"] == "number"
            && branch["xSkiffUnionBranch"] == "count"
            && branch["required"] == json!(["kind", "count"])
    }));

    let assembly_operation = published.artifacts.service_assembly.value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"] == "RawHttp.handle")
        .unwrap();
    assert_eq!(
        assembly_operation["parameters"][0]["type"]["discriminator"],
        "kind"
    );
    assert_eq!(assembly_operation["returnType"]["discriminator"], "kind");
}

#[test]
fn discriminator_field_changes_protocol_identity() {
    let tag_discriminator = write_service_project_with_internal(
        "tag-discriminator-identity",
        r#"
            type Event discriminator "tag" =
              { tag: "text", text: string }
              | { tag: "count", count: number }
            interface RawHttp {
              function handle(event: Event) -> Event
            }
        "#,
        r#"
              function handle(event: root.api.raw_http.Event) -> root.api.raw_http.Event {
                return event
              }
        "#,
    );
    let kind_discriminator = write_service_project_with_internal(
        "kind-discriminator-identity",
        r#"
            type Event discriminator "kind" =
              { kind: "text", text: string }
              | { kind: "count", count: number }
            interface RawHttp {
              function handle(event: Event) -> Event
            }
        "#,
        r#"
              function handle(event: root.api.raw_http.Event) -> root.api.raw_http.Event {
                return event
              }
        "#,
    );

    let tag_discriminator = build_temp_service_publication(tag_discriminator.path());
    let kind_discriminator = build_temp_service_publication(kind_discriminator.path());

    assert_ne!(
        tag_discriminator.manifest.service.protocol_identity,
        kind_discriminator.manifest.service.protocol_identity
    );
}

#[test]
fn anonymous_record_union_requires_explicit_discriminator() {
    let temp = write_service_project_with_internal(
        "missing-discriminator-union",
        r#"
            type Event =
              { tag: "text", text: string }
              | { tag: "count", count: number }
            interface RawHttp {
              function handle(event: Event) -> Event
            }
        "#,
        r#"
              function handle(event: Event) -> Event {
                return event
              }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "named union type Event uses anonymous record branches",
            "add discriminator \"tag\"",
        ],
    );
}

#[test]
fn explicit_discriminator_union_branch_field_is_required() {
    let temp = write_service_project_with_internal(
        "missing-explicit-discriminator-union-field",
        r#"
            type Event discriminator "kind" =
              { tag: "text", text: string }
              | { kind: "count", count: number }
            interface RawHttp {
              function handle(event: Event) -> Event
            }
        "#,
        r#"
              function handle(event: Event) -> Event {
                return event
              }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "anonymous record union branch in Event",
            "must declare kind as a string literal",
        ],
    );
}

#[test]
fn explicit_discriminator_union_branch_values_must_be_unique() {
    let temp = write_service_project_with_internal(
        "duplicate-explicit-discriminator-union-field",
        r#"
            type Event discriminator "kind" =
              { kind: "same", text: string }
              | { kind: "same", count: number }
            interface RawHttp {
              function handle(event: Event) -> Event
            }
        "#,
        r#"
              function handle(event: Event) -> Event {
                return event
              }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "anonymous record union branch kind \"same\" in Event",
            "must be unique",
        ],
    );
}

#[test]
fn websocket_connect_result_emits_strict_discriminator_union_schema() {
    let temp = write_service_project_with_internal(
        "gateway-connect-result-schema",
        r#"
            type ConnectionContext { userId: string }
            interface RawHttp {
              function handle(input: string) -> std.websocket.WebSocketConnectResult<ConnectionContext>
            }
        "#,
        r#"
            import std

              function handle(input: string) -> std.websocket.WebSocketConnectResult<root.api.raw_http.ConnectionContext> {
                return {
                  tag: "accept",
                  context: root.api.raw_http.ConnectionContext { userId: "" },
                  businessIdentity: null,
                }
              }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.handle")
        .unwrap();
    let response_schema = serde_json::to_value(&operation.response).unwrap();
    let assembly_operation = published.artifacts.service_assembly.value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"] == "RawHttp.handle")
        .unwrap();
    let return_type = &assembly_operation["returnType"];
    let branches = response_schema["oneOf"].as_array().unwrap();
    let return_branches = return_type["representation"]["items"].as_array().unwrap();

    assert_eq!(branches.len(), 2);
    assert_eq!(return_type["kind"], "representation");
    assert_eq!(return_type["name"], "std.websocket.WebSocketConnectResult");
    assert_eq!(return_type["discriminator"], "tag");
    assert_eq!(return_type["representation"]["kind"], "union");
    assert_eq!(return_branches.len(), 2);
    assert!(return_branches.iter().any(|branch| {
        branch["kind"] == "record"
            && branch["fields"]["tag"]["value"]["value"] == "accept"
            && branch["fields"].get("context").is_some()
            && branch["fields"]["businessIdentity"]["kind"] == "nullable"
            && branch["fields"]["businessIdentity"]["inner"]["name"] == "string"
            && branch["fields"]["connectionPolicy"]["kind"] == "nullable"
    }));
    assert!(branches.iter().any(|branch| {
        branch["type"] == "object"
            && branch["additionalProperties"] == false
            && json_array_values(&branch["required"]) == ["context", "tag"]
            && branch["properties"]["tag"]["enum"] == json!(["accept"])
            && branch["properties"]["context"]["properties"]["userId"]["type"] == "string"
            && branch["properties"]["businessIdentity"]["type"] == "string"
            && branch["properties"]["businessIdentity"]["nullable"] == true
            && branch["properties"]["connectionPolicy"]["nullable"] == true
            && branch["properties"]["connectionPolicy"]["properties"]
                .get("scope")
                .is_none()
            && branch["properties"]["connectionPolicy"]["properties"]["overflow"]["enum"]
                == json!(["close-oldest", "reject-new"])
    }));
    assert!(branches.iter().any(|branch| {
        branch["type"] == "object"
            && branch["additionalProperties"] == false
            && json_array_values(&branch["required"]) == ["code", "reason", "tag"]
            && branch["properties"]["tag"]["enum"] == json!(["reject"])
            && branch["properties"]["code"]["type"] == "integer"
            && branch["properties"]["reason"]["type"] == "string"
    }));
}

#[test]
fn raw_http_detection_requires_exact_standard_envelope_types() {
    let temp = write_service_project_with_internal(
        "raw-http-nominal-envelope-alias",
        r#"
            type MyRequest = HttpRequest
            type MyResponse = HttpResponse
            interface RawHttp {
              function handle(request: MyRequest) -> MyResponse
            }
        "#,
        r#"
              function handle(request: root.api.raw_http.MyRequest) -> root.api.raw_http.MyResponse {
                return root.api.raw_http.MyResponse({
                  status: 200,
                  headers: Array.empty<std.http.HttpHeader>(),
                  body: bytes.fromUtf8(""),
                })
              }
        "#,
    );

    let published = build_temp_service_publication(temp.path());

    assert!(published
        .manifest
        .gateway
        .as_ref()
        .and_then(|gateway| gateway.http.as_ref())
        .is_none());
}

#[test]
fn map_key_representation_over_string_emits_map_schema() {
    let temp = write_service_project_with_internal(
        "map-key-representation",
        r#"
            type UserId = string
            type User { name: string }
            interface RawHttp {
              function handle(users: Map<UserId, User>) -> User
            }
        "#,
        r#"
              function handle(users: Map<root.api.raw_http.UserId, root.api.raw_http.User>) -> root.api.raw_http.User {
                return root.api.raw_http.User { name: "" }
              }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.handle")
        .unwrap();
    let parameter_schema = serde_json::to_value(&operation.parameters[0].schema).unwrap();

    assert_eq!(parameter_schema["type"], "object");
    assert_eq!(
        parameter_schema["xSkiffMapKeySymbol"],
        "api.raw_http.UserId"
    );
    assert_eq!(parameter_schema["xSkiffMapKeySchema"]["type"], "string");
    assert_eq!(
        parameter_schema["xSkiffMapKeySchema"]["xSkiffSymbol"],
        "api.raw_http.UserId"
    );
    assert_eq!(
        parameter_schema["additionalProperties"]["properties"]["name"]["type"],
        "string"
    );
    assert_ne!(parameter_schema["additionalProperties"]["type"], "any");
}

#[test]
fn map_key_must_not_be_nullable_or_identity_losing_union() {
    for (name, key_type) in [
        ("nullable-nominal-map-key", "UserId?"),
        ("null-union-nominal-map-key", "UserId | null"),
        ("direct-multi-nominal-map-key", "UserId | TenantId"),
        ("identity-losing-map-key-union", "UserId | string"),
    ] {
        let contract_source = format!(
            r#"
            type UserId = string
            type TenantId = string
            type User {{ name: string }}
            interface RawHttp {{
              function handle(users: Map<{key_type}, User>) -> User
            }}
        "#
        );
        let internal_key_type = key_type
            .replace("UserId", "root.api.raw_http.UserId")
            .replace("TenantId", "root.api.raw_http.TenantId");
        let internal_source = format!(
            r#"
              function handle(users: Map<{internal_key_type}, root.api.raw_http.User>) -> root.api.raw_http.User {{
                return root.api.raw_http.User {{ name: "" }}
              }}
        "#
        );
        let temp = write_service_project_with_internal(name, &contract_source, &internal_source);

        assert_publish_error_contains(
            temp.path(),
            &[
                "contract validation failed",
                "Map key type",
                "cannot be used in service boundary schema",
            ],
        );
    }
}

#[test]
fn named_map_key_must_not_hide_nullable_or_multi_nominal_union() {
    for (name, key_alias) in [
        ("alias-nullable-map-key", "UserId?"),
        ("alias-null-union-map-key", "UserId | null"),
        ("alias-multi-nominal-map-key", "UserId | TenantId"),
        ("alias-identity-losing-map-key", "UserId | string"),
    ] {
        let contract_source = format!(
            r#"
            type UserId = string
            type TenantId = string
            type Key = {key_alias}
            type User {{ name: string }}
            interface RawHttp {{
              function handle(users: Map<Key, User>) -> User
            }}
        "#
        );
        let temp = write_service_project_with_internal(
            name,
            &contract_source,
            r#"
              function handle(users: Map<root.api.raw_http.Key, root.api.raw_http.User>) -> root.api.raw_http.User {
                return root.api.raw_http.User { name: "" }
              }
        "#,
        );

        assert_publish_error_contains(
            temp.path(),
            &[
                "contract validation failed",
                "Map key type api.raw_http.Key cannot be used in service boundary schema",
            ],
        );
    }
}

#[test]
fn discriminator_union_branches_emit_object_schemas() {
    let temp = write_service_project_with_internal(
        "discriminator-union-branch-schema",
        r#"
            type Event discriminator "tag" =
              { tag: "text", text: string }
              | { tag: "count", count: number }
            interface RawHttp {
              function handle(event: Event) -> Event
            }
        "#,
        r#"
              function handle(event: root.api.raw_http.Event) -> root.api.raw_http.Event {
                return event
              }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.handle")
        .unwrap();
    let parameter_schema = serde_json::to_value(&operation.parameters[0].schema).unwrap();
    let branches = parameter_schema["oneOf"].as_array().unwrap();

    assert_eq!(branches.len(), 2);
    assert!(branches.iter().any(|branch| {
        branch["type"] == "object"
            && branch["properties"]["tag"]["enum"] == json!(["text"])
            && branch["properties"]["text"]["type"] == "string"
    }));
    assert!(branches.iter().any(|branch| {
        branch["type"] == "object"
            && branch["properties"]["tag"]["enum"] == json!(["count"])
            && branch["properties"]["count"]["type"] == "number"
    }));
}

#[test]
fn tag_discriminator_union_branch_values_must_be_unique() {
    let temp = write_service_project_with_internal(
        "duplicate-tag-discriminator-union-value",
        r#"
            type Event discriminator "tag" =
              { tag: "same", text: string }
              | { tag: "same", count: number }
            interface RawHttp {
              function handle(event: Event) -> Event
            }
        "#,
        r#"
              function handle(event: Event) -> Event {
                return event
              }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "anonymous record union branch tag \"same\" in Event",
            "must be unique",
        ],
    );
}

#[test]
fn recursive_alias_or_union_type_fails_boundary_schema() {
    let temp = write_service_project_with_internal(
        "recursive-alias",
        r#"
            type Alias = Array<Alias>
            interface RawHttp {
              function handle(value: Alias) -> string
            }
        "#,
        r#"
              function handle(value: root.api.raw_http.Alias) -> string {
                return ""
              }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "recursive representation or union type api.raw_http.Alias",
            "not supported",
        ],
    );
}

#[test]
fn guarded_recursive_record_fails_until_schema_definitions_are_published() {
    let temp = write_service_project_with_internal(
        "guarded-recursive-record",
        r#"
            type Node {
              value: string,
              next: Node?
            }
            interface RawHttp {
              function handle(node: Node) -> Node
            }
        "#,
        r#"
              function handle(node: root.api.raw_http.Node) -> root.api.raw_http.Node {
                return node
              }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "guarded recursive record type api.raw_http.Node",
            "not supported",
        ],
    );
}

#[test]
fn unguarded_recursive_record_fails_boundary_schema() {
    let temp = write_service_project_with_internal(
        "unguarded-recursive-record",
        r#"
            type Node {
              next: Node
            }
            interface RawHttp {
              function handle(node: Node) -> Node
            }
        "#,
        r#"
              function handle(node: root.api.raw_http.Node) -> root.api.raw_http.Node {
                return node
              }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "recursive record type api.raw_http.Node",
            "not supported",
        ],
    );
}

#[test]
fn nullable_union_and_suffix_share_protocol_identity() {
    let suffix = write_service_project_with_internal(
        "nullable-suffix-identity",
        r#"
            interface RawHttp {
              function handle(value: string?) -> string?
            }
        "#,
        r#"
              function handle(value: string?) -> string? {
                return value
              }
        "#,
    );
    let union = write_service_project_with_internal(
        "nullable-union-identity",
        r#"
            interface RawHttp {
              function handle(value: string | null) -> string | null
            }
        "#,
        r#"
              function handle(value: string | null) -> string | null {
                return value
              }
        "#,
    );

    assert_eq!(
        build_temp_service_publication(suffix.path())
            .manifest
            .service
            .protocol_identity,
        build_temp_service_publication(union.path())
            .manifest
            .service
            .protocol_identity
    );
}

#[test]
fn contract_source_cannot_import_internal_module() {
    let temp = write_service_project(
        "contract-import-internal",
        r#"
            import internal

            interface RawHttp {
              function handle(request: HttpRequest) -> HttpResponse
            }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    assert!(published
        .manifest
        .operations
        .iter()
        .any(|operation| operation.operation == "RawHttp.handle"));
}

#[test]
fn configured_api_yml_replaces_configured_api_source() {
    let temp = write_service_project(
        "configured-api-source-import-internal",
        r#"
            interface RawHttp {
              function handle(request: HttpRequest) -> HttpResponse
            }
        "#,
    );
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api.yml"),
        r#"
RawHttp: internal.raw_http.RawHttp
api:
  raw_http:
    RawHttp: api.raw_http.RawHttp
"#,
    )
    .unwrap();

    build_temp_service_publication(temp.path());
}

#[test]
fn source_import_alias_fails_publish() {
    let temp = write_service_project(
        "source-import-alias",
        r#"
            import other as Json
            interface RawHttp {
              function handle(request: HttpRequest) -> HttpResponse
            }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &["import name must be a single ASCII identifier"],
    );
}

#[test]
fn redeclaring_reserved_standard_type_in_internal_source_fails_publish() {
    let temp = write_service_project_with_internal(
        "internal-redeclare-http-request",
        r#"
            interface RawHttp {
              function handle(request: HttpRequest) -> HttpResponse
            }
        "#,
        r#"
            type HttpRequest {}
              function handle(request: HttpRequest) -> HttpResponse {
                return std.http.noContent()
              }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "type HttpRequest",
            "reserved prelude name",
        ],
    );
}

#[test]
fn outer_stream_return_publishes_as_server_stream() {
    let temp = write_service_project_with_internal(
        "stream-return",
        r#"
            interface RawHttp {
              function handle(request: string) -> Stream<string>
            }
        "#,
        r#"
              function handle(request: string) -> Stream<string> {
                return
              }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.handle")
        .unwrap();
    let response_schema = serde_json::to_value(&operation.response).unwrap();
    let assembly_operation = published.artifacts.service_assembly.value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"] == "RawHttp.handle")
        .unwrap();
    let service_unit_operation_abi = published.artifacts.service_unit.value["publicationAbi"]
        ["operationAbi"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"]["publicPath"] == "RawHttp.handle")
        .unwrap();

    assert_eq!(operation.mode, "serverStream");
    assert_eq!(response_schema["type"], "string");
    assert_eq!(assembly_operation["mode"], "serverStream");
    assert_eq!(assembly_operation["response"]["type"], "string");
    assert_eq!(assembly_operation["returnType"]["name"], "string");
    assert_eq!(
        service_unit_operation_abi["publicSignature"]["maySuspend"],
        true
    );
    assert_eq!(
        service_unit_operation_abi["publicSignature"]["returnType"]["name"],
        "Stream"
    );
    assert_eq!(
        service_unit_operation_abi["publicSignature"]["returnType"]["args"][0]["name"],
        "string"
    );
}

#[test]
fn secret_string_cannot_enter_service_boundary_schema() {
    let temp = write_service_project_with_internal(
        "secret-string-boundary",
        r#"
            interface RawHttp {
              function handle(request: SecretString) -> string
            }
        "#,
        r#"
              function handle(request: SecretString) -> string {
                return ""
              }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "unresolved type `SecretString`",
        ],
    );
}

#[test]
fn top_level_config_usage_does_not_create_std_values_schema_owner() {
    let temp = write_service_project_with_internal(
        "config-root-local",
        r#"
            interface RawHttp {
              function handle(request: string) -> string
            }
        "#,
        r#"
              function handle(request: string) -> string {
                const secret = config.require<string>("dashscope.apiKey")
                return ""
              }
        "#,
    );
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let assembly_text = serde_json::to_string(&published.artifacts.service_assembly.value).unwrap();

    assert!(published
        .artifacts
        .service_assembly
        .value
        .get("packages")
        .is_none());
    assert!(
        !assembly_text.contains("std.values.SecretString"),
        "SecretString must not be emitted as a std.values schema owner:\n{assembly_text}"
    );
}

#[test]
fn user_source_function_type_fails_publish() {
    let temp = write_service_project(
        "user-function-type",
        r#"
            type CallbackBox {
              callback: fn(item: string) -> string,
            }

            interface RawHttp {
              function handle(request: HttpRequest) -> HttpResponse
            }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "callback function type",
            "only allowed in standard_library/platform native API metadata",
        ],
    );
}

#[test]
fn local_function_type_annotation_fails_publish() {
    let temp = write_service_project_with_internal(
        "local-function-type",
        r#"
            interface RawHttp {
              function handle(request: HttpRequest) -> HttpResponse
            }
        "#,
        r#"
              function handle(request: HttpRequest) -> HttpResponse {
                const cb: fn(item: string) -> string = request
                return std.http.noContent()
              }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "callback function type",
            "only allowed in standard_library/platform native API metadata",
        ],
    );
}

#[test]
fn bool_is_a_prelude_boolean_type() {
    let temp = TestDir::new("skiff-prelude-schema-v1", "bool-prelude");
    fs::create_dir_all(temp.path().join("api")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api.yml"),
        r#"
BoolApi: internal.bool_api.BoolApi
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("bool_api.skiff"),
        r#"
            interface BoolApi {
              function check(flag: bool) -> bool
            }
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("bool_api.skiff"),
        source_with_api_implementation(
            "BoolApi",
            "internal.bool_api",
            r#"
            function check(flag: bool) -> bool {
              return flag
            }
        "#,
        ),
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "BoolApi.check")
        .unwrap();

    assert_eq!(
        operation.parameters[0].schema.schema_type(),
        Some("boolean")
    );
    assert_eq!(operation.response.schema_type(), Some("boolean"));
}

#[test]
fn integer_is_a_prelude_numeric_type_with_integer_schema() {
    let temp = TestDir::new("skiff-prelude-schema-v1", "integer-prelude");
    fs::create_dir_all(temp.path().join("api")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api.yml"),
        r#"
IntegerApi: internal.integer_api.IntegerApi
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("integer_api.skiff"),
        r#"
            interface IntegerApi {
              function check(value: integer) -> integer
            }
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("integer_api.skiff"),
        source_with_api_implementation(
            "IntegerApi",
            "internal.integer_api",
            r#"
            function check(value: integer) -> integer {
              return value
            }
        "#,
        ),
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "IntegerApi.check")
        .unwrap();

    assert_eq!(
        operation.parameters[0].schema.schema_type(),
        Some("integer")
    );
    assert_eq!(operation.response.schema_type(), Some("integer"));
}

fn assert_bytes_body_schema(schema: &Value) {
    assert_eq!(schema["type"], "string");
    assert_eq!(schema["contentEncoding"], "base64");
    assert_eq!(schema["xSkiffSymbol"], "std.bytes.bytes");
}

fn json_array_values(value: &Value) -> Vec<String> {
    let mut values = value
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    values.sort();
    values
}

#[test]
fn bare_package_schema_types_emit_concrete_boundary_schemas() {
    let http_client = write_service_project_with_internal(
        "bare-http-client-schema-type",
        r#"
            import std

            interface RawHttp {
              function handle(request: std.http.HttpClientRequest) -> HttpResponse
            }
        "#,
        r#"
            import std

              function handle(request: std.http.HttpClientRequest) -> HttpResponse {
                return std.http.noContent()
              }
        "#,
    );
    let published = build_temp_service_publication(http_client.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.handle")
        .unwrap();
    let request_schema = serde_json::to_value(&operation.parameters[0].schema).unwrap();

    assert_eq!(request_schema["xSkiffSymbol"], "std.http.HttpClientRequest");
    assert_eq!(
        request_schema["properties"]["headers"]["items"]["xSkiffSymbol"],
        "std.http.HttpHeader"
    );
    assert_bytes_body_schema(&request_schema["properties"]["body"]);
}

#[test]
fn raw_http_envelope_types_remain_bare_boundary_exceptions() {
    let temp = write_service_project(
        "raw-http-envelope-bare-exception",
        r#"
            interface RawHttp {
              function handle(request: HttpRequest) -> HttpResponse
            }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "RawHttp.handle")
        .unwrap();
    let request_schema = serde_json::to_value(&operation.parameters[0].schema).unwrap();
    let response_schema = serde_json::to_value(&operation.response).unwrap();
    assert_eq!(request_schema["xSkiffSymbol"], "std.http.HttpRequest");
    assert_eq!(response_schema["xSkiffSymbol"], "std.http.HttpResponse");
}

fn write_service_project(name: &str, contract_source: &str) -> TestDir {
    write_service_project_with_internal(
        name,
        contract_source,
        r#"
              function handle(request: HttpRequest) -> HttpResponse {
                return std.http.noContent()
              }
        "#,
    )
}

fn write_service_project_with_internal(
    name: &str,
    contract_source: &str,
    internal_source: &str,
) -> TestDir {
    let temp = TestDir::new("skiff-prelude-schema-v1", name);
    fs::create_dir_all(temp.path().join("api")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api.yml"),
        r#"
RawHttp: internal.raw_http.RawHttp
api:
  raw_http:
    RawHttp: api.raw_http.RawHttp
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("raw_http.skiff"),
        contract_source,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("raw_http.skiff"),
        source_with_api_implementation("RawHttp", "internal.raw_http", internal_source),
    )
    .unwrap();
    temp
}

fn add_raw_http_std_dependency(temp: &TestDir) {
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
}

fn source_with_api_implementation(api_type: &str, module_path: &str, source: &str) -> String {
    let methods = function_signatures(source)
        .into_iter()
        .map(|signature| api_implementation_method(api_type, module_path, &signature))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"
{source}

type {api_type} {{}}

impl {api_type} {{
{methods}
}}
"#,
    )
}

fn api_implementation_method(
    api_type: &str,
    module_path: &str,
    signature: &FunctionSignature,
) -> String {
    let params = if signature.params.is_empty() {
        format!("self: {api_type}")
    } else {
        format!("self: {api_type}, {}", signature.params)
    };
    let call_args = function_call_arguments(&signature.params);
    let call = if call_args.is_empty() {
        format!("root.{module_path}.{}()", signature.name)
    } else {
        format!("root.{module_path}.{}({call_args})", signature.name)
    };
    let body = if signature.return_type == "void" {
        format!("    {call}\n")
    } else {
        format!("    return {call}\n")
    };

    format!(
        r#"  function {name}({params}) -> {return_type} {{
{body}  }}"#,
        name = signature.name,
        return_type = signature.return_type,
    )
}

struct FunctionSignature {
    name: String,
    params: String,
    return_type: String,
}

fn function_signatures(source: &str) -> Vec<FunctionSignature> {
    let function_keyword = "function ";
    let mut signatures = Vec::new();
    let mut search_start = 0usize;
    while let Some(keyword_offset) = source[search_start..].find(function_keyword) {
        let function_start = search_start + keyword_offset + function_keyword.len();
        let name_end = source[function_start..]
            .find('(')
            .map(|offset| function_start + offset)
            .expect("handler function should have parameter list");
        let params_start = name_end + 1;
        let params_end = matching_paren(source, name_end);
        let after_params = &source[params_end + 1..];
        let return_start = after_params
            .find("->")
            .map(|offset| params_end + 1 + offset + 2)
            .expect("handler function should declare return type");
        let body_start = source[return_start..]
            .find('{')
            .map(|offset| return_start + offset)
            .expect("handler function should have a body");
        signatures.push(FunctionSignature {
            name: source[function_start..name_end].trim().to_string(),
            params: source[params_start..params_end].trim().to_string(),
            return_type: source[return_start..body_start].trim().to_string(),
        });
        search_start = body_start + 1;
    }
    assert!(
        !signatures.is_empty(),
        "test internal source should declare a handler function"
    );
    signatures
}

fn matching_paren(source: &str, open_index: usize) -> usize {
    let mut depth = 0usize;
    for (offset, ch) in source[open_index..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return open_index + offset;
                }
            }
            _ => {}
        }
    }
    panic!("handler function parameter list should close");
}

fn function_call_arguments(params: &str) -> String {
    split_top_level_params(params)
        .into_iter()
        .filter_map(|param| {
            param
                .split_once(':')
                .map(|(name, _)| name.trim().to_string())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn split_top_level_params(params: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut angle_depth = 0isize;
    let mut paren_depth = 0isize;
    let mut brace_depth = 0isize;
    for (index, ch) in params.char_indices() {
        match ch {
            '<' => angle_depth += 1,
            '>' => angle_depth -= 1,
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            '{' => brace_depth += 1,
            '}' => brace_depth -= 1,
            ',' if angle_depth == 0 && paren_depth == 0 && brace_depth == 0 => {
                let part = params[start..index].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = params[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

fn gateway_target(service_id: &str, suffix: &str) -> String {
    let component = PublicationId::parse(service_id)
        .expect("test service id should parse")
        .runtime_target_component();
    format!("gateway.{component}.{suffix}")
}
