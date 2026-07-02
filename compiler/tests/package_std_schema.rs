use skiff_compiler::PublishedJsonArtifact;

mod common;
use common::artifacts::{
    assert_publish_error_contains, build_temp_service_publication, package_assembly,
    package_source_artifact,
};
use skiff_compiler::test_support::project_fixtures::{
    write_package_api_yml, write_package_manifest, write_package_source, ServiceProjectBuilder,
};

fn write_models_package(root: &std::path::Path) {
    write_package_manifest(
        root,
        "example.com/models",
        r#"
id: example.com/models
version: 0.1.0
"#,
    );
    write_package_api_yml(
        root,
        "example.com/models",
        r#"
ModelRequest: models_impl.ModelRequest
make: models_impl.make
"#,
    );
    write_package_source(
        root,
        "example.com/models",
        "models_impl.skiff",
        r#"
          type ModelRequest {}

          function make() -> ModelRequest {
            return {}
          }
        "#,
    );
}

#[test]
fn package_source_std_schema_types_require_declared_dependency() {
    let temp = ServiceProjectBuilder::package_model(
        "package-model-type-no-dependency",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/schema", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/schema",
        r#"
id: example.com/schema
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/schema",
        r#"
ChatEnvelope: schema.ChatEnvelope
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/schema",
        "schema.skiff",
        r#"
          import models

          type ChatEnvelope {
            request: models.ModelRequest,
          }
        "#,
    );
    assert_publish_error_contains(temp.root(), &["import models", "packages"]);

    let temp = ServiceProjectBuilder::package_model(
        "package-std-type-no-http-dependency",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/http-schema", Some("app"));
    write_package_manifest(
        temp.root(),
        "app.http_schema",
        r#"
id: example.com/http-schema
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "app.http_schema",
        r#"
RequestEnvelope: http_schema_impl.RequestEnvelope
"#,
    );
    write_package_source(
        temp.root(),
        "app.http_schema",
        "http_schema_impl.skiff",
        r#"
          type RequestEnvelope {
            request: std.http.HttpClientRequest,
          }
        "#,
    );
    build_temp_service_publication(temp.root());
}

#[test]
fn package_source_top_level_const_std_schema_type_requires_dependency() {
    let temp = ServiceProjectBuilder::package_model(
        "package-model-top-const-no-import",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/constschema", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/constschema",
        r#"
id: example.com/constschema
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/constschema",
        r#"
request: const_schema.request
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/constschema",
        "const_schema.skiff",
        r#"
          import models

          const request: models.ModelRequest = {}
        "#,
    );

    assert_publish_error_contains(temp.root(), &["import models", "packages"]);
}

#[test]
fn package_source_package_expression_requires_dependency() {
    let temp = ServiceProjectBuilder::package_model(
        "package-expression-no-dependency",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/plugin", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/plugin",
        r#"
id: example.com/plugin
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/plugin",
        r#"
make: plugin.make
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/plugin",
        "plugin.skiff",
        r#"
          import models

          const make = models.make
"#,
    );

    assert_publish_error_contains(temp.root(), &["import models", "packages"]);
}

#[test]
fn package_source_std_schema_type_rejects_explicit_std_dependency_alias() {
    let temp =
        ServiceProjectBuilder::package_model("package-std-schema-alias", "import app", "return {}");
    temp.add_service_package_dependency("example.com/schema", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/schema",
        r#"
id: example.com/schema
version: 0.1.0
packages:
  - id: skiff.run/std
    version: 1.0.0
    alias: corelib
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/schema",
        r#"
ChatEnvelope: schema.ChatEnvelope
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/schema",
        "schema.skiff",
        r#"
          import corelib

          type ChatEnvelope {
            request: corelib.http.HttpClientRequest,
          }
        "#,
    );

    assert_publish_error_contains(temp.root(), &["platform std is built into the compiler"]);
}

#[test]
fn package_source_top_level_const_value_std_schema_type_requires_dependency() {
    let temp = ServiceProjectBuilder::package_model(
        "package-std-top-const-value-no-model-import",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/constvalue", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/constvalue",
        r#"
id: example.com/constvalue
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/constvalue",
        r#"
decoded: const_value.decoded
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/constvalue",
        "const_value.skiff",
        r#"
          import models
          import std

          const decoded = std.json.decode<models.ModelRequest>("{}")
        "#,
    );

    assert_publish_error_contains(temp.root(), &["import models", "packages"]);
}

#[test]
fn package_source_top_level_const_function_type_std_schema_type_requires_dependency() {
    let temp = ServiceProjectBuilder::package_model(
        "package-std-function-const-no-model-import",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/callback-schema", Some("app"));
    write_package_manifest(
        temp.root(),
        "app.callback_schema",
        r#"
id: example.com/callback-schema
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "app.callback_schema",
        r#"
cb: callback_schema.cb
"#,
    );
    write_package_source(
        temp.root(),
        "app.callback_schema",
        "callback_schema.skiff",
        r#"
          import models

          const cb: fn(input: models.ModelRequest) -> void = handler
        "#,
    );

    assert_publish_error_contains(temp.root(), &["import models", "packages"]);
}

#[test]
fn package_source_generic_function_type_arg_std_schema_type_requires_dependency() {
    let temp = ServiceProjectBuilder::package_model(
        "package-std-generic-function-arg-no-model-import",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/callback-value", Some("app"));
    write_package_manifest(
        temp.root(),
        "app.callback_value",
        r#"
id: example.com/callback-value
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "app.callback_value",
        r#"
CallbackBag: callback_value.CallbackBag
"#,
    );
    write_package_source(
        temp.root(),
        "app.callback_value",
        "callback_value.skiff",
        r#"
          import models
          import std

          type CallbackBag {
            callbacks: Array<fn(input: models.ModelRequest) -> void>,
          }
        "#,
    );

    assert_publish_error_contains(temp.root(), &["import models", "packages"]);
}

#[test]
fn package_source_function_type_std_schema_type_passes_with_dependency_and_import() {
    let temp = ServiceProjectBuilder::package_model(
        "package-std-function-type-with-dependency",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/callback-ok", Some("app"));
    write_package_manifest(
        temp.root(),
        "app.callback_ok",
        r#"
id: example.com/callback-ok
version: 0.1.0
packages:
  - id: example.com/models
    version: 0.1.0
    alias: models
"#,
    );
    write_package_api_yml(
        temp.root(),
        "app.callback_ok",
        r#"
CallbackBag: callback_ok_impl.CallbackBag
"#,
    );
    write_models_package(temp.root());
    write_package_source(
        temp.root(),
        "app.callback_ok",
        "callback_ok_impl.skiff",
        r#"
          import models
          import std

          type CallbackBag {
            cb: fn(input: models.ModelRequest) -> void,
          }
        "#,
    );

    let published = build_temp_service_publication(temp.root());
    assert!(published
        .artifacts
        .service_assembly
        .value
        .get("packages")
        .is_none());
    assert_direct_service_package_only(
        &published.artifacts.service_unit.value,
        "example.com/callback-ok",
    );
    let callback_assembly = package_assembly(&published, "example.com/callback-ok");
    let models_assembly = package_assembly(&published, "example.com/models");
    assert_package_lock_entry(
        &callback_assembly.value["dependencies"][0],
        "example.com/models",
        "0.1.0",
        "models",
        models_assembly,
    );
    assert_eq!(
        callback_assembly.value["dependencies"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn package_source_std_schema_types_pass_with_dependency_and_import() {
    let temp = ServiceProjectBuilder::package_model(
        "package-std-type-with-dependency",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/schema", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/schema",
        r#"
id: example.com/schema
version: 0.1.0
packages:
  - id: example.com/models
    version: 0.1.0
    alias: models
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/schema",
        r#"
defaultModel: schema_impl.defaultModel
ChatEnvelope: schema_impl.ChatEnvelope
"#,
    );
    write_models_package(temp.root());
    write_package_source(
        temp.root(),
        "example.com/schema",
        "schema_impl.skiff",
        r#"
          import models
          import std

          const defaultModel: models.ModelRequest = {}

          type ChatEnvelope {
            model: models.ModelRequest,
            request: std.http.HttpClientRequest,
          }
        "#,
    );

    let published = build_temp_service_publication(temp.root());
    let schema_artifact = package_source_artifact(&published, "schema_impl.skiff");
    let schema_value = schema_artifact.value();
    assert_eq!(
        schema_value["linkTargets"]["constants"]["defaultModel"]["constIndex"],
        0
    );
    assert_eq!(schema_value["constants"][0]["name"], "defaultModel");
    let schema_package_unit = published
        .artifacts
        .package_units
        .iter()
        .find(|unit| unit.value["packageId"] == "example.com/schema")
        .expect("example.com/schema package unit should be published");
    assert_eq!(
        schema_package_unit.value["implementationLinks"]["constants"]["defaultModel"]["symbol"],
        "defaultModel"
    );
    assert_eq!(
        schema_package_unit.value["dependencies"],
        serde_json::json!([
            {
                "id": "example.com/models",
                "version": "0.1.0",
                "alias": "models",
                "config": {}
            },
            {
                "id": "skiff.run/std",
                "version": "1.0.0",
                "alias": "std",
                "config": {}
            }
        ])
    );
    assert!(published
        .artifacts
        .service_unit
        .value
        .get("packageAbiExpectations")
        .is_none());
    assert!(published
        .artifacts
        .service_assembly
        .value
        .get("packages")
        .is_none());
    assert_direct_service_package_only(
        &published.artifacts.service_unit.value,
        "example.com/schema",
    );
    let schema_assembly = package_assembly(&published, "example.com/schema");
    let models_assembly = package_assembly(&published, "example.com/models");
    assert_package_lock_entry(
        &schema_assembly.value["dependencies"][0],
        "example.com/models",
        "0.1.0",
        "models",
        models_assembly,
    );
    assert_eq!(
        schema_assembly.value["dependencies"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn package_publication_direct_refs_do_not_leak_into_publication_abi() {
    let temp = ServiceProjectBuilder::package_model(
        "package-publication-direct-ref-public-abi",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/direct-ref", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/direct-ref",
        r#"
id: example.com/direct-ref
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/direct-ref",
        r#"
echo: api.echo
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/direct-ref",
        "api.skiff",
        r#"
          function echo(input: root.models.Payload) -> root.models.Payload {
            return input
          }
        "#,
    );
    write_package_source(
        temp.root(),
        "example.com/direct-ref",
        "models.skiff",
        r#"
          type Payload {
            value: string,
          }
        "#,
    );

    let published = build_temp_service_publication(temp.root());
    let api_artifact = package_source_artifact(&published, "api.skiff");
    let models_artifact = package_source_artifact(&published, "models.skiff");
    let payload_type_index = declared_type_index(&models_artifact.value(), "Payload");
    assert!(
        json_contains_publication_type(&api_artifact.value(), "models", payload_type_index),
        "package file IR should exercise a sibling-module PublicationType: {}",
        api_artifact.value()
    );

    let package_unit = published
        .artifacts
        .package_units
        .iter()
        .find(|unit| unit.value["packageId"] == "example.com/direct-ref")
        .expect("example.com/direct-ref package unit should be published");
    let publication_abi = &package_unit.value["publicationAbi"];
    let publication_abi_text = publication_abi.to_string();
    assert!(
        !publication_abi_text.contains("publicationType"),
        "package publication ABI leaked publicationType: {publication_abi}"
    );
    assert!(
        !publication_abi_text.contains("$type"),
        "package publication ABI leaked raw type placeholder: {publication_abi}"
    );
    assert!(
        !publication_abi_text.contains("__unresolved_publication_type"),
        "package publication ABI leaked unresolved placeholder: {publication_abi}"
    );
    assert!(
        json_contains_service_symbol(publication_abi, "models", "Payload"),
        "package publication ABI should use stable source symbol identity: {publication_abi}"
    );
}

#[test]
fn package_source_std_discriminator_union_field_access_passes_with_import() {
    let temp = ServiceProjectBuilder::package_model(
        "package-std-discriminator-union-field-access",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/http-sse", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/http-sse",
        r#"
id: example.com/http-sse
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/http-sse",
        r#"
eventStatus: http_sse.eventStatus
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/http-sse",
        "http_sse.skiff",
        r#"
          import std

          function eventStatus(event: std.http.HttpSseEvent) -> integer? {
            if event.tag == "response" {
              return event.status
            }
            if event.tag == "event" {
              const data = event.data
              if data == "" {
                return null
              }
            }
            return null
          }
        "#,
    );

    build_temp_service_publication(temp.root());
}

fn assert_package_lock_entry(
    entry: &serde_json::Value,
    id: &str,
    version: &str,
    alias: &str,
    assembly: &PublishedJsonArtifact,
) {
    assert_eq!(entry["id"], id);
    assert_eq!(entry["version"], version);
    assert_eq!(entry["alias"], alias);
    assert_eq!(entry["assemblyIdentity"], assembly.identity);
    assert_eq!(entry["assemblyPath"], assembly.path);
}

fn assert_direct_service_package_only(service_unit: &serde_json::Value, id: &str) {
    let dependencies = service_unit["packageDependencies"]
        .as_array()
        .expect("Service Unit packageDependencies should be an array");
    assert!(
        dependencies.iter().any(|dependency| dependency["id"] == id),
        "Service Unit should include direct service package {id}"
    );
    assert!(
        dependencies
            .iter()
            .all(|dependency| dependency["id"] != "skiff.run/std"
                && dependency["id"] != "example.com/models"),
        "Service Unit should not flatten transitive package dependencies"
    );
}

fn declared_type_index(value: &serde_json::Value, symbol: &str) -> u64 {
    value["declarations"]["types"][symbol]["typeIndex"]
        .as_u64()
        .unwrap_or_else(|| panic!("missing type declaration index for {symbol}: {value}"))
}

fn json_contains_publication_type(
    value: &serde_json::Value,
    module_path: &str,
    type_index: u64,
) -> bool {
    if value.get("kind").and_then(serde_json::Value::as_str) == Some("publicationType")
        && value.get("modulePath").and_then(serde_json::Value::as_str) == Some(module_path)
        && value.get("typeIndex").and_then(serde_json::Value::as_u64) == Some(type_index)
    {
        return true;
    }

    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| json_contains_publication_type(item, module_path, type_index)),
        serde_json::Value::Object(object) => object
            .values()
            .any(|item| json_contains_publication_type(item, module_path, type_index)),
        _ => false,
    }
}

fn json_contains_service_symbol(
    value: &serde_json::Value,
    module_path: &str,
    symbol: &str,
) -> bool {
    if value.get("kind").and_then(serde_json::Value::as_str) == Some("serviceSymbol")
        && value
            .get("symbol")
            .and_then(|symbol| symbol.get("modulePath"))
            .and_then(serde_json::Value::as_str)
            == Some(module_path)
        && value
            .get("symbol")
            .and_then(|symbol| symbol.get("symbol"))
            .and_then(serde_json::Value::as_str)
            == Some(symbol)
    {
        return true;
    }

    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| json_contains_service_symbol(item, module_path, symbol)),
        serde_json::Value::Object(object) => object
            .values()
            .any(|item| json_contains_service_symbol(item, module_path, symbol)),
        _ => false,
    }
}

#[test]
fn package_source_raw_http_envelope_bare_types_do_not_require_std_dependency() {
    let temp =
        ServiceProjectBuilder::package_model("package-raw-envelope", "import app", "return {}");
    temp.add_service_package_dependency("example.com/raw", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/raw",
        r#"
id: example.com/raw
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/raw",
        r#"
rawRequest: raw_impl.rawRequest
RawEnvelope: raw_impl.RawEnvelope
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/raw",
        "raw_impl.skiff",
        r#"
          const rawRequest: HttpRequest = {
            method: "GET",
            url: "https://example.com",
            path: "/",
            query: Array.empty<std.http.HttpQueryParam>(),
            headers: Array.empty<std.http.HttpHeader>(),
            body: bytes.fromUtf8(""),
          }

          type RawEnvelope {
            request: HttpRequest,
            response: HttpResponse,
          }
        "#,
    );

    let published = build_temp_service_publication(temp.root());
    let raw_artifact = package_source_artifact(&published, "raw_impl.skiff");
    assert_eq!(
        raw_artifact.value()["linkTargets"]["constants"]["rawRequest"]["constIndex"],
        0
    );
    assert!(published
        .artifacts
        .service_assembly
        .value
        .get("packages")
        .is_none());
    assert_direct_service_package_only(&published.artifacts.service_unit.value, "example.com/raw");
}
