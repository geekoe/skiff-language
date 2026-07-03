mod common;
use common::artifacts::{
    assert_publish_error_contains, assert_service_package_absent, assert_service_package_id,
    build_temp_service_publication, package_assembly, package_source_artifact,
    service_assembly_value, source_artifact,
};
use skiff_compiler::{
    test_support::{
        project_fixtures::{
            write_package_api_yml, write_package_manifest, write_package_manifest_in_dir,
            write_package_source, ServiceProjectBuilder,
        },
        read_user_package_manifest,
    },
    PublishedFileIrArtifact,
};
use skiff_compiler_core::artifact::{CallIr, CallTargetIr, ExprIr, TypeDescriptorIr, TypeRefIr};

#[test]
fn publishes_service_assembly_http_response_limit() {
    let temp = ServiceProjectBuilder::package_model("http-response-limit-config", "", "return {}");
    temp.add_root_file(
        "service.yml",
        &service_config_with_packages_and_extra(
            "",
            r#"
http:
  response:
    maxBytes: 134217728
"#,
        ),
    );

    let published = build_temp_service_publication(temp.root());
    let service = &published.artifacts.service_assembly.value["service"];

    assert_eq!(
        service["http"]["response"]["maxBytes"],
        serde_json::json!(134217728u64)
    );
}

#[test]
fn std_root_package_imports_and_user_package_ids_are_rejected() {
    let temp = ServiceProjectBuilder::package_model(
        "std-package-import",
        "import skiff.run/foo",
        "return {}",
    );
    assert_publish_error_contains(
        temp.root(),
        &["import name must be a single ASCII identifier"],
    );

    let temp = ServiceProjectBuilder::package_model(
        "std-mongo-package-import",
        "import std.mongo",
        "return {}",
    );
    assert_publish_error_contains(
        temp.root(),
        &["import name must be a single ASCII identifier"],
    );

    let temp = ServiceProjectBuilder::package_model(
        "std-http-nested-import",
        "import skiff.run/foo",
        "return {}",
    );
    assert_publish_error_contains(
        temp.root(),
        &["import name must be a single ASCII identifier"],
    );

    let temp = ServiceProjectBuilder::package_model(
        "std-anything-nested-import",
        "import std.anything",
        "return {}",
    );
    assert_publish_error_contains(
        temp.root(),
        &["import name must be a single ASCII identifier"],
    );

    let temp =
        ServiceProjectBuilder::package_model("user-simple-package-id", "import app", "return {}");
    write_package_manifest_in_dir(
        temp.root(),
        "single",
        r#"
id: example.com/app
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/app",
        r#"
run: main_impl.run
"#,
    );
    write_package_source(
        temp.root(),
        "single",
        "main_impl.skiff",
        r#"
          function run() -> string {
            return "ok"
          }
        "#,
    );
    set_service_package_dependencies(&temp, &[("example.com/app", "0.1.0", Some("app"))]);

    let published = build_temp_service_publication(temp.root());
    assert_service_package_id(&published, "example.com/app");

    let temp = ServiceProjectBuilder::package_model("config-user-package-id", "", "return {}");
    write_package_manifest_in_dir(
        temp.root(),
        "config",
        r#"
id: config
version: 0.1.0
"#,
    );

    assert_package_manifest_error_contains(
        temp.root(),
        "config",
        "0.1.0",
        &["id config", "must be a publication id"],
    );

    let temp = ServiceProjectBuilder::package_model("std-user-package-id", "", "return {}");
    write_package_manifest_in_dir(
        temp.root(),
        "std-foo",
        r#"
id: skiff.run/foo
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "skiff.run/foo",
        r#"
skiff.run/foo: foo.run
"#,
    );

    assert_package_manifest_error_contains(
        temp.root(),
        "skiff.run/foo",
        "0.1.0",
        &[
            "api.yml key skiff.run/foo",
            "dotted public keys are not supported; use nested mapping",
        ],
    );

    let temp =
        ServiceProjectBuilder::package_model("std-foo-dependency", "import app", "return {}");
    write_package_manifest(
        temp.root(),
        "example.com/bad",
        r#"
id: example.com/bad
version: 0.1.0
packages:
  - id: skiff.run/foo
    version: 0.1.0
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/bad",
        "bad.skiff",
        r#"
          function run() -> string {
            return "ok"
          }
        "#,
    );
    set_service_package_dependencies(&temp, &[("example.com/bad", "0.1.0", Some("app"))]);
    assert_publish_error_contains(
        temp.root(),
        &["packages entry skiff.run/foo", "requires alias"],
    );

    let temp = ServiceProjectBuilder::package_model("std-core-user-package-id", "", "return {}");
    write_package_manifest_in_dir(
        temp.root(),
        "std-core",
        r#"
id: skiff.run/core
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "skiff.run/core",
        r#"
skiff.run/core: core.run
"#,
    );

    assert_package_manifest_error_contains(
        temp.root(),
        "skiff.run/core",
        "0.1.0",
        &[
            "api.yml key skiff.run/core",
            "dotted public keys are not supported; use nested mapping",
        ],
    );

    let temp =
        ServiceProjectBuilder::package_model("image-user-package-id", "import image", "return {}");
    write_package_manifest_in_dir(
        temp.root(),
        "user-image",
        r#"
id: example.com/image
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/image",
        r#"
run: main_impl.run
"#,
    );
    write_package_source(
        temp.root(),
        "user-image",
        "main_impl.skiff",
        r#"
          function run() -> string {
            return "ok"
          }
        "#,
    );
    set_service_package_dependencies(&temp, &[("example.com/image", "0.1.0", Some("image"))]);

    let published = build_temp_service_publication(temp.root());
    assert_service_package_id(&published, "example.com/image");

    let temp = ServiceProjectBuilder::package_model(
        "platform-user-package-id",
        "import platform",
        "return {}",
    );
    write_package_manifest(
        temp.root(),
        "platform",
        r#"
id: example.com/platform
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "platform",
        r#"
main:
  run: main_impl.run
"#,
    );
    write_package_source(
        temp.root(),
        "platform",
        "main_impl.skiff",
        r#"
          function run() -> string {
            return "ok"
          }
        "#,
    );
    set_service_package_dependencies(
        &temp,
        &[("example.com/platform", "0.1.0", Some("platform"))],
    );
    let published = build_temp_service_publication(temp.root());
    assert_service_package_id(&published, "example.com/platform");
}

#[test]
fn service_and_user_packages_cannot_declare_native_functions() {
    let temp = ServiceProjectBuilder::package_model("service-native-function", "", "return {}");
    temp.add_source(
        "internal/native_host.skiff",
        r#"
          native function hostOnly() -> string
        "#,
    );
    assert_publish_error_contains(
        temp.root(),
        &["service source cannot declare native function hostOnly"],
    );

    let temp = ServiceProjectBuilder::package_model(
        "user-package-native-function",
        "import app",
        "return {}",
    );
    write_package_manifest(
        temp.root(),
        "app",
        r#"
id: example.com/app
version: 0.1.0
"#,
    );
    write_package_source(
        temp.root(),
        "app",
        "main.skiff",
        r#"
          native function hostOnly() -> string
        "#,
    );
    set_service_package_dependencies(&temp, &[("example.com/app", "0.1.0", Some("app"))]);
    assert_publish_error_contains(
        temp.root(),
        &["package example.com/app cannot declare native function hostOnly"],
    );
}

#[test]
fn service_and_user_packages_cannot_declare_native_types() {
    let temp = ServiceProjectBuilder::package_model("service-native-type", "", "return {}");
    temp.add_source(
        "internal/native_types.skiff",
        r#"
          native type HostOnly
        "#,
    );
    assert_publish_error_contains(
        temp.root(),
        &["service source cannot declare native type HostOnly"],
    );

    let temp =
        ServiceProjectBuilder::package_model("user-package-native-type", "import app", "return {}");
    write_package_manifest(
        temp.root(),
        "app",
        r#"
id: example.com/app
version: 0.1.0
"#,
    );
    write_package_source(
        temp.root(),
        "app",
        "main.skiff",
        r#"
          native type HostOnly
        "#,
    );
    set_service_package_dependencies(&temp, &[("example.com/app", "0.1.0", Some("app"))]);
    assert_publish_error_contains(
        temp.root(),
        &["package example.com/app cannot declare native type HostOnly"],
    );
}

#[test]
fn std_root_import_allows_official_std_modules_and_records_std_package() {
    let temp = ServiceProjectBuilder::package_model(
        "std-root-import",
        "import std",
        r#"
            const decoded = std.json.decode<JsonObject>("{\"ok\":true}")
            return {}
        "#,
    );
    let published = build_temp_service_publication(temp.root());
    let std_assembly = package_assembly(&published, "skiff.run/std");
    assert_eq!(
        std_assembly.value["apiSource"]["relativePath"],
        serde_json::json!("api.yml")
    );
    assert!(std_assembly.value["apiSource"]["contentHash"]
        .as_str()
        .is_some_and(|hash| hash.len() == 64));
    let mut entries = std_assembly.value["exports"]["entries"]
        .as_array()
        .unwrap()
        .clone();
    entries.sort_by_key(|entry| entry["path"].as_str().unwrap().to_string());
    assert_eq!(entries.len(), 90);
    for (path, module) in [
        ("actor.Actor", "actor"),
        ("bytes.DecodeError", "bytes"),
        ("crypto.sha256", "crypto"),
        ("db.DecodeError", "db"),
        ("file.FileError", "file"),
        ("file.ImmutableFile", "file"),
        ("http.HttpRequest", "http"),
        ("http.json", "http"),
        ("json.DecodeError", "json"),
        ("json.decode", "json"),
        ("log.info", "log"),
        ("number.DecodeError", "number"),
        ("service.ProviderUnavailableError", "service"),
        ("service.ProtocolError", "service"),
        ("string.split", "string"),
        ("telemetry.emit", "telemetry"),
        ("time.DecodeError", "time"),
        ("time.sleep", "time"),
        ("websocket.ConnectionMessage", "websocket"),
        ("websocket.sendJsonToBusinessIdentity", "websocket"),
    ] {
        assert!(
            entries
                .iter()
                .any(|entry| entry == &serde_json::json!({ "module": module, "path": path })),
            "missing std api.yml export {path} -> {module}: {entries:?}",
        );
    }
    let removed_suffix = ["_", "api"].concat();
    assert!(
        entries.iter().all(|entry| {
            entry["path"].as_str().unwrap().contains('.')
                && !entry["module"].as_str().unwrap().ends_with(&removed_suffix)
        }),
        "std exports must be symbol-level api.yml entries: {entries:?}",
    );
    assert_service_package_absent(&published, "std.json");
    assert_service_package_absent(&published, "std.http");
    assert!(service_assembly_value(&published)
        .get("transportSelection")
        .is_none());
    let service_artifact = source_artifact(&published, "internal/example.skiff");
    let service_value = service_artifact.value();
    assert!(
        json_contains_native_symbol(&service_value, "std.json", "decode"),
        "std.json.decode should lower directly to a native target: {service_value}",
    );

    let json_artifact = package_source_artifact(&published, "json.skiff");
    assert_native_wrapper_type_args(json_artifact, "decode", &[("T0", "T")]);
    assert_native_wrapper_type_args(json_artifact, "encode", &[("T0", "T")]);

    let http_artifact = package_source_artifact(&published, "http.skiff");
    assert_native_wrapper_type_args(http_artifact, "decodeJson", &[("T0", "T")]);
    assert_native_wrapper_type_args(http_artifact, "json", &[("T0", "T")]);
    assert_native_wrapper_type_args(http_artifact, "jsonWithHeaders", &[("T0", "T")]);
    assert_native_wrapper_type_args(http_artifact, "noContent", &[]);
}

#[test]
fn module_decode_error_catch_types_are_public_source_types() {
    let temp = ServiceProjectBuilder::package_model(
        "module-decode-error-catch-types",
        "import std",
        r#"
            const jsonResult = catch<std.json.DecodeError>(std.json.decode<string>("{}"))
            const numberResult = catch<std.number.DecodeError>(number.assertSafeInteger(1.5))
            const timeResult = catch<std.time.DecodeError>(Date.requireParse("not-a-date"))
            const configResult = catch<config.DecodeError>(config.require<string>("app.secret"))
            return {}
        "#,
    );

    build_temp_service_publication(temp.root());
}

#[test]
fn std_log_import_lowers_runtime_target_and_effect_summary() {
    let temp = ServiceProjectBuilder::package_model(
        "std-log-import",
        "import std",
        r#"
            const attrs = std.json.decode<JsonObject>("{\"event\":\"hello\"}")
            std.log.info("hello", attrs)
            return {}
        "#,
    );
    let published = build_temp_service_publication(temp.root());
    let std_assembly = package_assembly(&published, "skiff.run/std");
    assert!(std_assembly.value["exports"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry == &serde_json::json!({ "module": "log", "path": "log.info" })));

    let service_artifact = source_artifact(&published, "internal/example.skiff");
    let service_value = service_artifact.value();
    assert!(
        json_contains_package_symbol(&service_value, "std", "std.log.info"),
        "service call site should lower std.log.info as a typed package symbol call: {service_value}",
    );

    let log_artifact = package_source_artifact(&published, "log.skiff");
    let log_value = log_artifact.value();
    // The publication-local direct refs lowering pass rewrites the cross-module
    // `std.telemetry.emit` call target inside std.log's File IR body from an
    // `externalServiceSymbol` into a direct `publicationExecutable` address. The
    // publication ABI stays symbolic; the wrapper implementation is direct.
    let telemetry_artifact = package_source_artifact(&published, "telemetry.skiff");
    assert_eq!(telemetry_artifact.module_path, "std.telemetry");
    let emit_executable_index = declared_executable_index(&telemetry_artifact.value(), "emit");
    assert!(
        json_contains_publication_executable(&log_value, "std.telemetry", emit_executable_index),
        "std.log wrapper should emit through direct std.telemetry publicationExecutable: {log_value}",
    );
    assert!(std_assembly.value["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|file| file["modulePath"] == "std.telemetry"));
    assert!(std_assembly.value["exports"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry
            == &serde_json::json!({ "module": "telemetry", "path": "telemetry.emit" })));
    assert!(!std_assembly.value["exports"]["symbols"]["telemetry.emit"].is_null());

    let telemetry_artifact = package_source_artifact(&published, "telemetry.skiff");
    assert_native_wrapper_type_args(telemetry_artifact, "emit", &[]);
}

#[test]
fn std_normal_types_emit_package_symbols_not_native_refs() {
    let temp = ServiceProjectBuilder::package_model(
        "std-normal-types-not-native",
        "import std",
        r#"
            return {}
        "#,
    );
    temp.add_source(
        "api/std_types.skiff",
        r#"
            type Envelope {
              request: std.http.HttpRequest,
              event: std.http.HttpResponseStreamEvent,
              file: std.file.ImmutableFile,
              gateway: std.websocket.WebSocketConnectResult<string>,
              connect: std.websocket.ConnectionMessage,
              raw: Json,
              bytesValue: bytes,
            }
        "#,
    );

    let published = build_temp_service_publication(temp.root());
    let service_artifact = source_artifact(&published, "api/std_types.skiff");
    assert_std_normal_type_uses_package_symbol(service_artifact, "std.http.HttpRequest");
    assert_std_normal_type_uses_package_symbol(
        service_artifact,
        "std.http.HttpResponseStreamEvent",
    );
    assert_std_normal_type_uses_package_symbol(service_artifact, "std.file.ImmutableFile");
    assert_native_type_ref_present(service_artifact, "std.websocket.WebSocketConnectResult");
    assert_std_normal_type_uses_package_symbol(service_artifact, "std.websocket.ConnectionMessage");
    assert_no_native_std_normal_type_refs(service_artifact);

    for source_path in ["http.skiff", "file.skiff", "websocket.skiff"] {
        let artifact = package_source_artifact(&published, source_path);
        assert_no_native_std_normal_type_refs(artifact);
    }

    let http_artifact = package_source_artifact(&published, "http.skiff");
    assert_std_normal_type_uses_package_symbol(http_artifact, "std.http.HttpClientRequest");
    assert_std_normal_type_uses_package_symbol(http_artifact, "std.http.HttpClientResponse");
    assert_std_normal_type_uses_package_symbol(http_artifact, "std.http.HttpResponseStreamEvent");
    assert_std_normal_type_uses_package_symbol(http_artifact, "std.http.HttpSseEvent");
    assert_native_type_ref_present(http_artifact, "bytes");
    assert_native_type_ref_present(http_artifact, "Json");
}

#[test]
fn package_dependency_on_std_allows_std_root_import_and_records_std_package() {
    let temp = ServiceProjectBuilder::package_model("package-std-root", "import app", "return {}");
    temp.packages().add_package(
        "example.com/schema",
        r#"
id: example.com/schema
version: 0.1.0
"#,
        &[
            (
                "api.yml",
                r#"
Schema: schema_impl.Schema
"#,
            ),
            (
                "schema_impl.skiff",
                r#"
          type Schema {
            request: std.http.HttpRequest,
          }
        "#,
            ),
        ],
    );
    set_service_package_dependencies(&temp, &[("example.com/schema", "0.1.0", Some("app"))]);

    let published = build_temp_service_publication(temp.root());
    assert_service_package_id(&published, "example.com/schema");
    assert_service_package_absent(&published, "skiff.run/std");
    assert_service_package_absent(&published, "std.http");
}

fn json_contains_package_symbol(
    value: &serde_json::Value,
    dependency_ref: &str,
    symbol_path: &str,
) -> bool {
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| json_contains_package_symbol(value, dependency_ref, symbol_path)),
        serde_json::Value::Object(object) => {
            object.get("kind").and_then(|value| value.as_str()) == Some("packageSymbol")
                && object
                    .get("packageRef")
                    .and_then(|package| package.get("kind"))
                    .and_then(|value| value.as_str())
                    == Some("dependency")
                && object
                    .get("packageRef")
                    .and_then(|package| package.get("dependencyRef"))
                    .and_then(|value| value.as_str())
                    == Some(dependency_ref)
                && object
                    .get("operation")
                    .and_then(|operation| operation.get("publicPath"))
                    .and_then(|value| value.as_str())
                    == Some(symbol_path)
                || object
                    .values()
                    .any(|value| json_contains_package_symbol(value, dependency_ref, symbol_path))
        }
        _ => false,
    }
}

fn json_contains_native_symbol(
    value: &serde_json::Value,
    namespace: &str,
    symbol_name: &str,
) -> bool {
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| json_contains_native_symbol(value, namespace, symbol_name)),
        serde_json::Value::Object(object) => {
            object.get("kind").and_then(|value| value.as_str()) == Some("native")
                && object
                    .get("target")
                    .and_then(|target| target.get("namespace"))
                    .and_then(|value| value.as_str())
                    == Some(namespace)
                && object
                    .get("target")
                    .and_then(|target| target.get("symbol"))
                    .and_then(|value| value.as_str())
                    == Some(symbol_name)
                || object
                    .values()
                    .any(|value| json_contains_native_symbol(value, namespace, symbol_name))
        }
        _ => false,
    }
}

fn json_contains_publication_executable(
    value: &serde_json::Value,
    module_path: &str,
    executable_index: u64,
) -> bool {
    match value {
        serde_json::Value::Array(values) => values.iter().any(|value| {
            json_contains_publication_executable(value, module_path, executable_index)
        }),
        serde_json::Value::Object(object) => {
            object.get("kind").and_then(|value| value.as_str()) == Some("publicationExecutable")
                && object.get("modulePath").and_then(|value| value.as_str()) == Some(module_path)
                && object
                    .get("executableIndex")
                    .and_then(|value| value.as_u64())
                    == Some(executable_index)
                || object.values().any(|value| {
                    json_contains_publication_executable(value, module_path, executable_index)
                })
        }
        _ => false,
    }
}

fn declared_executable_index(file_ir: &serde_json::Value, symbol: &str) -> u64 {
    file_ir["declarations"]["executables"][symbol]["executableIndex"]
        .as_u64()
        .unwrap_or_else(|| panic!("File IR is missing declared executable {symbol}"))
}

fn assert_native_wrapper_type_args(
    artifact: &PublishedFileIrArtifact,
    symbol_name: &str,
    expected: &[(&str, &str)],
) {
    let call = native_wrapper_call(artifact, symbol_name);
    let actual = call
        .type_args
        .iter()
        .map(|(key, ty)| {
            let TypeRefIr::TypeParam { name } = ty else {
                panic!(
                    "expected native wrapper type arg {key} for {symbol_name} to be a type param, got {ty:?}"
                );
            };
            (key.as_str(), name.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

fn assert_no_native_std_normal_type_refs(artifact: &PublishedFileIrArtifact) {
    assert_json_has_no_native_std_normal_type_refs(&artifact.value());

    for ty in &artifact.unit.type_table {
        assert_descriptor_has_no_native_std_normal_refs(&ty.descriptor);
        for implemented in &ty.implements {
            assert_type_ref_has_no_native_std_normal_refs(implemented);
        }
    }
    for constant in &artifact.unit.constants {
        assert_type_ref_has_no_native_std_normal_refs(&constant.ty);
    }
    for executable in &artifact.unit.executables {
        for param in &executable.params {
            assert_type_ref_has_no_native_std_normal_refs(&param.ty);
        }
        assert_type_ref_has_no_native_std_normal_refs(&executable.return_type);
    }
}

fn assert_json_has_no_native_std_normal_type_refs(value: &serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                assert_json_has_no_native_std_normal_type_refs(value);
            }
        }
        serde_json::Value::Object(object) => {
            if object.get("kind").and_then(|value| value.as_str()) == Some("builtin") {
                if let Some(name) = object.get("name").and_then(|value| value.as_str()) {
                    assert!(
                        !std_normal_type_symbol(name),
                        "std normal type {name} must be emitted as packageSymbol, not builtin JSON: {value}"
                    );
                }
            }
            for value in object.values() {
                assert_json_has_no_native_std_normal_type_refs(value);
            }
        }
        _ => {}
    }
}

fn assert_descriptor_has_no_native_std_normal_refs(descriptor: &TypeDescriptorIr) {
    match descriptor {
        TypeDescriptorIr::Record { fields } => {
            for field in fields.values() {
                assert_type_ref_has_no_native_std_normal_refs(field);
            }
        }
        TypeDescriptorIr::Alias { target } => assert_type_ref_has_no_native_std_normal_refs(target),
        TypeDescriptorIr::Union { variants } => {
            for variant in variants {
                assert_type_ref_has_no_native_std_normal_refs(variant);
            }
        }
        TypeDescriptorIr::Native { .. } => {}
    }
}

fn assert_type_ref_has_no_native_std_normal_refs(ty: &TypeRefIr) {
    match ty {
        TypeRefIr::Native { name, args } => {
            assert!(
                !std_normal_type_symbol(name),
                "std normal type {name} must be emitted as packageSymbol, not Native"
            );
            for arg in args {
                assert_type_ref_has_no_native_std_normal_refs(arg);
            }
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                assert_type_ref_has_no_native_std_normal_refs(field);
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                assert_type_ref_has_no_native_std_normal_refs(item);
            }
        }
        TypeRefIr::Nullable { inner } => assert_type_ref_has_no_native_std_normal_refs(inner),
        TypeRefIr::AnyInterface { interface } => {
            for arg in &interface.canonical_type_args {
                assert_type_ref_has_no_native_std_normal_refs(arg);
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for param in params {
                assert_type_ref_has_no_native_std_normal_refs(&param.ty);
            }
            assert_type_ref_has_no_native_std_normal_refs(return_type);
        }
        TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
}

fn assert_std_normal_type_uses_package_symbol(
    artifact: &PublishedFileIrArtifact,
    expected_symbol_path: &str,
) {
    assert!(
        file_ir_contains_package_type_symbol(artifact, expected_symbol_path),
        "{} should refer to std normal type {expected_symbol_path} through packageSymbol",
        artifact.source_path
    );
}

fn file_ir_contains_package_type_symbol(
    artifact: &PublishedFileIrArtifact,
    expected_symbol_path: &str,
) -> bool {
    artifact.unit.type_table.iter().any(|ty| {
        descriptor_contains_package_type_symbol(&ty.descriptor, expected_symbol_path)
            || ty.implements.iter().any(|implemented| {
                type_ref_contains_package_type_symbol(implemented, expected_symbol_path)
            })
    }) || artifact
        .unit
        .constants
        .iter()
        .any(|constant| type_ref_contains_package_type_symbol(&constant.ty, expected_symbol_path))
        || artifact.unit.executables.iter().any(|executable| {
            executable
                .params
                .iter()
                .any(|param| type_ref_contains_package_type_symbol(&param.ty, expected_symbol_path))
                || type_ref_contains_package_type_symbol(
                    &executable.return_type,
                    expected_symbol_path,
                )
        })
}

fn descriptor_contains_package_type_symbol(
    descriptor: &TypeDescriptorIr,
    expected_symbol_path: &str,
) -> bool {
    match descriptor {
        TypeDescriptorIr::Record { fields } => fields
            .values()
            .any(|field| type_ref_contains_package_type_symbol(field, expected_symbol_path)),
        TypeDescriptorIr::Alias { target } => {
            type_ref_contains_package_type_symbol(target, expected_symbol_path)
        }
        TypeDescriptorIr::Union { variants } => variants
            .iter()
            .any(|variant| type_ref_contains_package_type_symbol(variant, expected_symbol_path)),
        TypeDescriptorIr::Native { .. } => false,
    }
}

fn type_ref_contains_package_type_symbol(ty: &TypeRefIr, expected_symbol_path: &str) -> bool {
    match ty {
        TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path == expected_symbol_path,
        TypeRefIr::Native { args, .. } => args
            .iter()
            .any(|arg| type_ref_contains_package_type_symbol(arg, expected_symbol_path)),
        TypeRefIr::Record { fields } => fields
            .values()
            .any(|field| type_ref_contains_package_type_symbol(field, expected_symbol_path)),
        TypeRefIr::Union { items } => items
            .iter()
            .any(|item| type_ref_contains_package_type_symbol(item, expected_symbol_path)),
        TypeRefIr::Nullable { inner } => {
            type_ref_contains_package_type_symbol(inner, expected_symbol_path)
        }
        TypeRefIr::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .any(|arg| type_ref_contains_package_type_symbol(arg, expected_symbol_path)),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params
                .iter()
                .any(|param| type_ref_contains_package_type_symbol(&param.ty, expected_symbol_path))
                || type_ref_contains_package_type_symbol(return_type, expected_symbol_path)
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}

fn assert_native_type_ref_present(artifact: &PublishedFileIrArtifact, expected_name: &str) {
    assert!(
        artifact.unit.type_table.iter().any(|ty| {
            descriptor_contains_native_type_ref(&ty.descriptor, expected_name)
                || ty.implements.iter().any(|implemented| {
                    type_ref_contains_native_type_ref(implemented, expected_name)
                })
        }) || artifact.unit.executables.iter().any(|executable| {
            executable
                .params
                .iter()
                .any(|param| type_ref_contains_native_type_ref(&param.ty, expected_name))
                || type_ref_contains_native_type_ref(&executable.return_type, expected_name)
        }),
        "{} should still use native type {expected_name}",
        artifact.source_path
    );
}

fn descriptor_contains_native_type_ref(descriptor: &TypeDescriptorIr, expected_name: &str) -> bool {
    match descriptor {
        TypeDescriptorIr::Record { fields } => fields
            .values()
            .any(|field| type_ref_contains_native_type_ref(field, expected_name)),
        TypeDescriptorIr::Alias { target } => {
            type_ref_contains_native_type_ref(target, expected_name)
        }
        TypeDescriptorIr::Union { variants } => variants
            .iter()
            .any(|variant| type_ref_contains_native_type_ref(variant, expected_name)),
        TypeDescriptorIr::Native { symbol } => symbol == expected_name,
    }
}

fn type_ref_contains_native_type_ref(ty: &TypeRefIr, expected_name: &str) -> bool {
    match ty {
        TypeRefIr::Native { name, args } => {
            name == expected_name
                || args
                    .iter()
                    .any(|arg| type_ref_contains_native_type_ref(arg, expected_name))
        }
        TypeRefIr::Record { fields } => fields
            .values()
            .any(|field| type_ref_contains_native_type_ref(field, expected_name)),
        TypeRefIr::Union { items } => items
            .iter()
            .any(|item| type_ref_contains_native_type_ref(item, expected_name)),
        TypeRefIr::Nullable { inner } => type_ref_contains_native_type_ref(inner, expected_name),
        TypeRefIr::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .any(|arg| type_ref_contains_native_type_ref(arg, expected_name)),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params
                .iter()
                .any(|param| type_ref_contains_native_type_ref(&param.ty, expected_name))
                || type_ref_contains_native_type_ref(return_type, expected_name)
        }
        TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}

fn std_normal_type_symbol(name: &str) -> bool {
    matches!(
        name,
        "std.bytes.DecodeError"
            | "std.db.DecodeError"
            | "std.file.FileError"
            | "std.json.DecodeError"
            | "std.number.DecodeError"
            | "std.time.DecodeError"
            | "std.service.ProviderUnavailableError"
            | "std.service.ProtocolError"
            | "std.http.HttpHeader"
            | "std.http.HttpQueryParam"
            | "std.http.HttpRequest"
            | "std.http.HttpResponse"
            | "std.http.HttpResponseStreamEvent"
            | "std.http.HttpClientRequest"
            | "std.http.HttpClientResponse"
            | "std.http.HttpClientStreamHandle"
            | "std.http.HttpSseEvent"
            | "std.http.HttpError"
            | "std.file.ImmutableFile"
            | "std.file.CreateOptions"
            | "std.file.FileInfo"
            | "std.websocket.TextConnectionMessage"
            | "std.websocket.BinaryConnectionMessage"
            | "std.websocket.ConnectionMessage"
            | "std.websocket.WebSocketConnectRequest"
            | "std.websocket.WebSocketCloseEvent"
            | "Duration"
            | "std.time.Duration"
            | "ImmutableFile"
            | "CreateOptions"
            | "FileInfo"
            | "ConnectionMessage"
            | "TextConnectionMessage"
            | "BinaryConnectionMessage"
    )
}

fn native_wrapper_call<'a>(artifact: &'a PublishedFileIrArtifact, symbol_name: &str) -> &'a CallIr {
    let expected_symbol = format!("{}.{}", artifact.module_path, symbol_name);
    let executable = artifact
        .unit
        .executables
        .iter()
        .find(|executable| executable.symbol == expected_symbol)
        .unwrap_or_else(|| {
            panic!(
                "package artifact {} should contain executable {expected_symbol}",
                artifact.source_path
            )
        });
    executable
        .body
        .expressions
        .iter()
        .find_map(|expr| {
            let ExprIr::Call { call } = expr else {
                return None;
            };
            let CallTargetIr::Native { target } = &call.target else {
                return None;
            };
            (target.namespace == artifact.module_path && target.symbol == symbol_name)
                .then_some(call)
        })
        .unwrap_or_else(|| {
            panic!(
                "executable {expected_symbol} should contain a direct native call to {}.{symbol_name}",
                artifact.module_path
            )
        })
}

#[test]
fn json_object_is_prelude_type_without_std_json_import() {
    let temp = ServiceProjectBuilder::package_model("bare-json-object", "", "return { raw: {} }");
    temp.add_source(
        "api/example.skiff",
        r#"
            type Input {}
            type Output {
              raw: JsonObject,
            }
            interface ExampleService {
              function run(input: Input) -> Output
            }
        "#,
    );

    let published = build_temp_service_publication(temp.root());
    let assembly_text = serde_json::to_string(&published.artifacts.service_assembly.value).unwrap();

    assert!(!assembly_text.contains("std.json.JsonObject"));
    assert!(assembly_text.contains("\"preludeIdentity\""));
}

fn set_service_package_dependencies(
    temp: &ServiceProjectBuilder,
    entries: &[(&str, &str, Option<&str>)],
) {
    let packages = entries
        .iter()
        .map(|(id, version, alias)| match alias {
            Some(alias) => {
                format!("  - id: {id}\n    version: {version}\n    alias: {alias}")
            }
            None => format!("  - id: {id}\n    version: {version}"),
        })
        .collect::<Vec<_>>()
        .join("\n");

    temp.add_root_file(
        "service.yml",
        &service_config_with_packages_and_extra(&packages, ""),
    );
}

fn service_config_with_packages_and_extra(packages: &str, extra: &str) -> String {
    let package_block = if packages.trim().is_empty() {
        String::new()
    } else {
        format!("packages:\n{packages}\n")
    };
    format!(
        r#"
id: example.com/example
version: 1.0.0
{package_block}
{extra}"#
    )
}

fn assert_package_manifest_error_contains(
    root: &std::path::Path,
    package_id: &str,
    version: &str,
    fragments: &[&str],
) {
    let manifest_path = root
        .join(".skiff-packages")
        .join(publication_storage_segment(package_id))
        .join(version)
        .join("package.yml");
    let message = read_user_package_manifest(&manifest_path)
        .unwrap_err()
        .to_string();
    for fragment in fragments {
        assert!(
            message.contains(fragment),
            "expected error to contain {fragment:?}, got:\n{message}"
        );
    }
}

fn publication_storage_segment(package_id: &str) -> String {
    package_id.replace('.', "~").replace('/', "~~")
}
