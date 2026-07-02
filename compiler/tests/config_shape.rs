mod common;
use common::artifacts::{assert_publish_error_contains, build_temp_service_publication};
use skiff_compiler::read_service_config;
use skiff_compiler::test_support::project_fixtures::{
    write_package_api_yml, write_package_manifest, write_package_manifest_in_dir,
    write_package_source, ServiceProjectBuilder,
};

fn example_service_config(extra: &str) -> String {
    let extra = extra.trim_start_matches('\n');
    format!(
        r#"
id: example.com/example
version: 1.0.0
{extra}"#
    )
}

#[test]
fn std_module_native_calls_do_not_require_explicit_imports() {
    let temp = ServiceProjectBuilder::package_model(
        "std-module-native-no-import",
        "",
        r#"
          const decoded = std.json.decode<JsonObject>("{}")
          const response = std.http.request(std.http.HttpClientRequest {
            method: "GET",
            url: "https://example.com",
            headers: Array.empty<std.http.HttpHeader>(),
            body: null,
            timeoutMs: null,
          })
          return {}
        "#,
    );

    let published = build_temp_service_publication(temp.root());
    assert_eq!(published.manifest.service.id, "example.com/example");
}

#[test]
fn std_values_import_and_legacy_values_root_are_rejected() {
    let temp =
        ServiceProjectBuilder::package_model("std-values-import", "import std.values", "return {}");
    assert_publish_error_contains(
        temp.root(),
        &["import name must be a single ASCII identifier"],
    );

    let temp = ServiceProjectBuilder::package_model(
        "legacy-values-root",
        "",
        r#"
            const apiKey = values.string("dashscopeApiKey")
            return {}
        "#,
    );
    assert_publish_error_contains(
        temp.root(),
        &["values.* has been removed", "config.require<T>"],
    );
}

#[test]
fn config_is_reserved_for_declarations_and_local_bindings() {
    let temp = ServiceProjectBuilder::package_model("redeclare-config", "", "return {}");
    temp.add_source(
        "internal/example.skiff",
        r#"
            type config {}
            function run(input: root.api.example.Input) -> root.api.example.Output {
              return {}
            }
        "#,
    );
    assert_publish_error_contains(temp.root(), &["type config", "reserved prelude name"]);

    let temp = ServiceProjectBuilder::package_model(
        "local-config",
        "",
        r#"
            const config = 1
            return {}
        "#,
    );
    assert_publish_error_contains(
        temp.root(),
        &["local binding config", "reserved prelude name"],
    );
}

#[test]
fn env_root_is_not_supported() {
    let temp = ServiceProjectBuilder::package_model(
        "config-root-rejected",
        "",
        r#"
            const apiKey = env.require<string>("dashscopeApiKey")
            return {}
        "#,
    );
    assert_publish_error_contains(temp.root(), &["unresolved root env", "env.require"]);
}

#[test]
fn assembly_records_config_uses_and_shape_from_literal_reads() {
    let temp = ServiceProjectBuilder::package_model(
        "config-shape",
        "",
        r#"
            const apiKey = config.require<string>("dashscopeApiKey")
            const region = config.optional<string>("providerRegion")
            const provider = config.require<JsonObject>("providerConfig")
            const hasRegion = config.has("providerRegion")
            return {}
        "#,
    );

    let published = build_temp_service_publication(temp.root());
    let assembly = &published.artifacts.service_assembly.value;

    assert_eq!(
        assembly["configShape"],
        serde_json::json!({
            "schemaVersion": "skiff-config-shape-v1",
            "entries": [
                { "path": "dashscopeApiKey", "type": "string", "required": true },
                { "path": "providerConfig", "type": "JsonObject", "required": true },
                { "path": "providerRegion", "type": "string", "required": false }
            ]
        })
    );
    assert_eq!(
        assembly["configActivation"],
        serde_json::json!({
            "schemaVersion": "skiff-config-activation-v1",
            "hasPaths": ["providerRegion"]
        })
    );
    assert_eq!(
        assembly["configUses"],
        serde_json::json!([
            {
                "path": "dashscopeApiKey",
                "type": "string",
                "required": true,
                "sourcePath": "internal/example.skiff"
            },
            {
                "path": "providerConfig",
                "type": "JsonObject",
                "required": true,
                "sourcePath": "internal/example.skiff"
            },
            {
                "path": "providerRegion",
                "type": "string",
                "required": false,
                "sourcePath": "internal/example.skiff"
            }
        ])
    );
    assert!(assembly.get("valuesPolicy").is_none());
    assert!(assembly.get("valuesReads").is_none());
}

#[test]
fn config_has_only_records_activation_without_shape_or_config_use() {
    let temp = ServiceProjectBuilder::package_model(
        "config-has-only",
        "",
        r#"
            const hasFlag = config.has("app.enabled")
            return {}
        "#,
    );

    let published = build_temp_service_publication(temp.root());
    let assembly = &published.artifacts.service_assembly.value;

    assert_eq!(assembly["configShape"]["entries"], serde_json::json!([]));
    assert_eq!(assembly["configUses"], serde_json::json!([]));
    assert_eq!(
        assembly["configActivation"],
        serde_json::json!({
            "schemaVersion": "skiff-config-activation-v1",
            "hasPaths": ["app.enabled"]
        })
    );
}

#[test]
fn config_uses_accept_const_foldable_paths() {
    let temp = ServiceProjectBuilder::package_model(
        "config-const-path",
        "",
        r#"
            const path = "dashscopeApiKey"
            const apiKey = config.require<string>(path)
            const base = "dashscope"
            const modelPath = base + "Model"
            const model = config.optional<string>(modelPath)
            return {}
        "#,
    );
    let published = build_temp_service_publication(temp.root());
    assert_eq!(
        published.artifacts.service_assembly.value["configShape"]["entries"],
        serde_json::json!([
            { "path": "dashscopeApiKey", "type": "string", "required": true },
            { "path": "dashscopeModel", "type": "string", "required": false }
        ])
    );
}

#[test]
fn config_intrinsics_reject_dynamic_empty_invalid_or_unsupported_paths_and_types() {
    let temp = ServiceProjectBuilder::package_model(
        "config-dynamic-path",
        "",
        r#"
            const prefix = request
            const apiKey = config.require<string>(prefix + ".apiKey")
            return {}
        "#,
    );
    assert_publish_error_contains(temp.root(), &["config.require", "const-foldable"]);

    let temp = ServiceProjectBuilder::package_model(
        "config-empty-path",
        "",
        r#"
            const apiKey = config.require<string>("")
            return {}
        "#,
    );
    assert_publish_error_contains(temp.root(), &["config.require path cannot be empty"]);

    let temp = ServiceProjectBuilder::package_model(
        "config-invalid-read-path",
        "",
        r#"
            const path = "9dashscopeApiKey"
            const apiKey = config.require<string>(path)
            return {}
        "#,
    );
    assert_publish_error_contains(
        temp.root(),
        &["config.require", "9dashscopeApiKey", "invalid segment"],
    );

    let temp = ServiceProjectBuilder::package_model(
        "config-unsupported-type",
        "",
        r#"
            const appConfig = config.require<Array<string>>("app.config")
            return {}
        "#,
    );
    assert_publish_error_contains(
        temp.root(),
        &[
            "config.require type Array<string> is unsupported",
            "JsonObject",
        ],
    );

    let temp = ServiceProjectBuilder::package_model(
        "config-get-removed",
        "",
        r#"
            const apiKey = config.get<string>("dashscopeApiKey")
            return {}
        "#,
    );
    assert_publish_error_contains(temp.root(), &["config.get<T>(path) has been removed"]);
}

#[test]
fn config_shape_rejects_type_conflicts_and_merges_requiredness() {
    let temp = ServiceProjectBuilder::package_model(
        "config-type-conflict",
        "",
        r#"
            const apiKey = config.require<string>("app.secret")
            const apiKeyNumber = config.require<number>("app.secret")
            return {}
        "#,
    );
    assert_publish_error_contains(temp.root(), &["app.secret", "conflicting type"]);

    let temp = ServiceProjectBuilder::package_model(
        "config-required-optional-merge",
        "",
        r#"
            const required = config.require<string>("app.secret")
            const optional = config.optional<string>("app.secret")
            return {}
        "#,
    );
    let published = build_temp_service_publication(temp.root());
    assert_eq!(
        published.artifacts.service_assembly.value["configShape"]["entries"],
        serde_json::json!([
            { "path": "app.secret", "type": "string", "required": true }
        ])
    );
    assert_eq!(
        published.artifacts.service_assembly.value["configUses"],
        serde_json::json!([
            {
                "path": "app.secret",
                "type": "string",
                "required": true,
                "sourcePath": "internal/example.skiff"
            }
        ])
    );
}

#[test]
fn config_intrinsics_must_not_be_aliased_or_called_indirectly() {
    let temp = ServiceProjectBuilder::package_model(
        "config-root-alias",
        "",
        r#"
            const accessor = config
            const apiKey = accessor.require<string>("dashscopeApiKey")
            return {}
        "#,
    );
    assert_publish_error_contains(
        temp.root(),
        &[
            "config require/optional/has cannot be aliased",
            "direct config.require<T>",
        ],
    );

    let temp = ServiceProjectBuilder::package_model(
        "config-accessor-alias",
        "",
        r#"
            const read = config.require
            const apiKey = read<string>("dashscopeApiKey")
            return {}
        "#,
    );
    assert_publish_error_contains(
        temp.root(),
        &[
            "config require/optional/has cannot be aliased",
            "direct config.require<T>",
        ],
    );
}

#[test]
fn values_requirements_are_rejected_in_service_and_package_config() {
    let temp = ServiceProjectBuilder::package_model("service-values-requirements", "", "return {}");
    temp.add_root_file(
        "service.yml",
        &example_service_config(
            r#"
valuesRequirements:
  - path: app.secret
    type: string
"#,
        ),
    );
    let error = skiff_compiler::read_service_config(temp.root())
        .unwrap_err()
        .to_string();
    assert!(error.contains("valuesRequirements"));
    assert!(error.contains("config.require<T>(path)"));

    let temp = ServiceProjectBuilder::package_model(
        "package-values-requirements",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/secrets", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/secrets",
        r#"
id: example.com/secrets
version: 0.1.0
valuesRequirements:
  - path: app.secret
    type: string
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/secrets",
        r#"
readSecret: secrets_impl.readSecret
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/secrets",
        "secrets_impl.skiff",
        r#"
          function readSecret() -> string {
            return config.require<string>("app.secret")
          }
        "#,
    );
    assert_publish_error_contains(
        temp.root(),
        &["valuesRequirements", "config.require<T>(path)"],
    );
}

#[test]
fn package_config_projection_stays_package_scoped() {
    let temp = ServiceProjectBuilder::package_model(
        "package-config-precision",
        "import app",
        r#"
            let _secret = app.secrets.readProdSecret()
            return {}
        "#,
    );
    temp.add_root_file(
        "service.yml",
        &example_service_config(
            r#"
packages:
  - id: example.com/pkg
    version: 0.1.0
    alias: app
"#,
        ),
    );
    write_package_manifest(
        temp.root(),
        "example.com/pkg",
        r#"
id: example.com/pkg
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/pkg",
        r#"
secrets:
  readProdSecret: secrets_impl.readProdSecret
  hasFeatureFlag: secrets_impl.hasFeatureFlag
unused:
  readUnusedSecret: unused_impl.readUnusedSecret
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/pkg",
        "secrets_impl.skiff",
        r#"
          function readProdSecret() -> string {
            return config.require<string>("prodKey")
          }

          function hasFeatureFlag() -> boolean {
            return config.has("featureFlag")
          }
        "#,
    );
    write_package_source(
        temp.root(),
        "example.com/pkg",
        "unused_impl.skiff",
        r#"
          function readUnusedSecret() -> string {
            return config.require<string>("unusedKey")
          }
        "#,
    );
    write_package_source(
        temp.root(),
        "example.com/pkg",
        "secrets.test.skiff",
        r#"
          const testOnly = config.require<string>("testOnly")

          test "package config helper" {
            assert testOnly == testOnly, "test-only config read stays out of production metadata"
          }
        "#,
    );

    let published = build_temp_service_publication(temp.root());
    let assembly_text = serde_json::to_string(&published.artifacts.service_assembly.value).unwrap();
    let package_unit = published
        .artifacts
        .package_units
        .iter()
        .find(|unit| unit.value["packageId"] == "example.com/pkg")
        .expect("app package unit");

    assert_eq!(
        published.artifacts.service_assembly.value["configShape"]["entries"],
        serde_json::json!([])
    );
    assert_eq!(
        published.artifacts.service_assembly.value["configUses"],
        serde_json::json!([])
    );
    assert_eq!(
        published.artifacts.service_assembly.value["configActivation"],
        serde_json::json!({
            "schemaVersion": "skiff-config-activation-v1",
            "hasPaths": []
        })
    );
    assert_eq!(
        package_unit.value["configAndEffectMetadata"]["config"]["shape"]["entries"],
        serde_json::json!([
            { "path": "prodKey", "type": "string", "required": true },
            { "path": "unusedKey", "type": "string", "required": true }
        ])
    );
    assert!(
        published.artifacts.service_unit.value["config"]["packageConfigs"]["example.com/pkg"]
            .get("config")
            .is_none()
    );
    assert!(!assembly_text.contains("testOnly"));
}

#[test]
fn missing_required_dependency_config_stays_runtime_scoped() {
    let temp = ServiceProjectBuilder::package_model(
        "missing-required-package-config",
        "import session",
        r#"
            const issued = session.issue()
            return {}
        "#,
    );
    temp.add_root_file(
        "service.yml",
        &example_service_config(
            r#"
packages:
  - id: example.com/session
    version: 0.1.0
    alias: session
"#,
        ),
    );
    write_session_package(temp.root());

    let published = build_temp_service_publication(temp.root());
    let package_unit = published
        .artifacts
        .package_units
        .iter()
        .find(|unit| unit.value["packageId"] == "example.com/session")
        .expect("example.com/session package unit");

    assert_eq!(
        published.artifacts.service_assembly.value["packageConfigs"]["example.com/session"]["id"],
        "example.com/session"
    );
    assert_eq!(
        published.artifacts.service_assembly.value["packageConfigs"]["example.com/session"]
            ["version"],
        "0.1.0"
    );
    assert_eq!(
        published.artifacts.service_assembly.value["packageConfigs"]["example.com/session"]
            ["alias"],
        "session"
    );
    assert!(published
        .artifacts
        .service_assembly
        .value
        .get("packages")
        .is_none());
    for field in [
        "dependencyRef",
        "runtimeConfigNamespace",
        "assemblyIdentity",
        "assemblyPath",
        "configShape",
        "configActivation",
    ] {
        assert!(
            published.artifacts.service_assembly.value["packageConfigs"]["example.com/session"]
                .get(field)
                .is_none(),
            "service assembly packageConfigs must not include legacy package field {field}"
        );
    }
    assert!(
        published.artifacts.service_assembly.value["packageConfigs"]["example.com/session"]
            .get("config")
            .is_none()
    );
    assert!(
        published.artifacts.service_assembly.value["packageConfigs"]["example.com/session"]
            .get("defaultConfig")
            .is_none()
    );
    assert_eq!(
        published.artifacts.service_assembly.value["configShape"]["entries"],
        serde_json::json!([])
    );
    assert_eq!(
        published.artifacts.service_assembly.value["configUses"],
        serde_json::json!([])
    );
    assert_eq!(
        published.artifacts.service_assembly.value["configActivation"],
        serde_json::json!({
            "schemaVersion": "skiff-config-activation-v1",
            "hasPaths": []
        })
    );
    assert_eq!(
        package_unit.value["configAndEffectMetadata"]["config"]["shape"]["entries"],
        serde_json::json!([
            { "path": "cookieDomain", "type": "string", "required": false },
            { "path": "cookieName", "type": "string", "required": true },
            { "path": "maxAgeSeconds", "type": "number", "required": true }
        ])
    );
    let service_dependency_requirements = published.artifacts.service_assembly.value
        ["configRequirements"]["dependency"]
        .as_array()
        .expect("service dependency config requirements");
    let cookie_name_requirement =
        config_requirement_by_path(service_dependency_requirements, "cookieName");
    assert_eq!(
        cookie_name_requirement["scope"],
        serde_json::json!({
            "kind": "package",
            "packageId": "example.com/session"
        })
    );
    assert_config_source_span(&cookie_name_requirement["provenance"][0]["sourceSpan"]);
    assert_eq!(
        cookie_name_requirement["provenance"][0]["sourcePath"],
        "session_impl.skiff"
    );
    assert_eq!(
        cookie_name_requirement["provenance"][0]["declaringPublication"],
        serde_json::json!({
            "id": "example.com/session",
            "version": "0.1.0"
        })
    );
    assert_eq!(
        cookie_name_requirement["provenance"][0]["dependencyPath"],
        serde_json::json!([
            {
                "id": "example.com/session",
                "version": "0.1.0",
                "alias": "session"
            }
        ])
    );
    let package_own_requirements = package_unit.value["configAndEffectMetadata"]["config"]
        ["requirements"]["own"]
        .as_array()
        .expect("package own config requirements");
    assert_eq!(package_own_requirements.len(), 3);
    let package_cookie_name_requirement =
        config_requirement_by_path(package_own_requirements, "cookieName");
    assert_eq!(
        package_cookie_name_requirement["provenance"][0]["sourcePath"], "session_impl.skiff",
        "package unit should retain source-path provenance for own config requirements"
    );
    assert_eq!(
        package_cookie_name_requirement["provenance"][0]["sourceSpan"],
        cookie_name_requirement["provenance"][0]["sourceSpan"],
        "dependency projection should retain the package own config source span"
    );
    let service_effective_requirements = published.artifacts.service_assembly.value
        ["configRequirements"]["effective"]
        .as_array()
        .expect("service effective config requirements");
    let effective_cookie_name_requirement =
        config_requirement_by_path(service_effective_requirements, "cookieName");
    assert_eq!(
        effective_cookie_name_requirement["provenance"][0]["sourceSpan"],
        cookie_name_requirement["provenance"][0]["sourceSpan"],
        "effective projection should retain dependency config source span"
    );
    assert!(
        published.artifacts.service_unit.value["config"]["packageConfigs"]["example.com/session"]
            .get("config")
            .is_none()
    );
}

#[test]
fn service_yml_package_config_is_rejected_before_shape_validation() {
    let temp = ServiceProjectBuilder::package_model(
        "wrong-type-package-config",
        "import session",
        r#"
            const issued = session.issue()
            return {}
        "#,
    );
    temp.add_root_file(
        "service.yml",
        &example_service_config(
            r#"
packages:
  - id: example.com/session
    version: 0.1.0
    alias: session
    config:
      cookieName: session
      maxAgeSeconds: long
"#,
        ),
    );
    write_session_package(temp.root());

    assert_service_config_error_contains(
        temp.root(),
        &[
            "packages.config",
            "package runtime config belongs in config source packages.<alias>",
            "not service.yml",
        ],
    );
}

#[test]
fn package_yml_package_dependency_config_is_rejected() {
    let temp = ServiceProjectBuilder::package_model(
        "package-dependency-config",
        "import tracka",
        r#"
            const recorded = tracka.record()
            return {}
        "#,
    );
    temp.add_root_file(
        "service.yml",
        &example_service_config(
            r#"
packages:
  - id: example.com/tracka
    version: 0.1.0
    alias: tracka
"#,
        ),
    );
    write_session_package(temp.root());
    write_track_package(
        temp.root(),
        "example.com/tracka",
        "tracka",
        "track_session",
        3600,
    );

    assert_publish_error_contains(
        temp.root(),
        &["unknown field `config`", "id", "version", "alias"],
    );
}

#[test]
fn transitive_package_dependency_config_is_rejected() {
    let temp = ServiceProjectBuilder::package_model(
        "transitive-config-rejected",
        "import tracka\nimport trackb",
        r#"
            const a = tracka.record()
            const b = trackb.record()
            return {}
        "#,
    );
    temp.add_root_file(
        "service.yml",
        &example_service_config(
            r#"
packages:
  - id: example.com/tracka
    version: 0.1.0
    alias: tracka
  - id: example.com/trackb
    version: 0.1.0
    alias: trackb
"#,
        ),
    );
    write_session_package(temp.root());
    write_track_package(temp.root(), "example.com/tracka", "tracka", "track_a", 3600);
    write_track_package(temp.root(), "example.com/trackb", "trackb", "track_b", 3600);

    assert_publish_error_contains(
        temp.root(),
        &["unknown field `config`", "id", "version", "alias"],
    );
}

#[test]
fn service_dependency_config_overrides_conflicting_transitive_defaults() {
    let temp = ServiceProjectBuilder::package_model(
        "transitive-config-service-override",
        "import tracka\nimport trackb",
        r#"
            const a = tracka.record()
            const b = trackb.record()
            return {}
        "#,
    );
    temp.add_root_file(
        "service.yml",
        &example_service_config(
            r#"
packages:
  - id: example.com/tracka
    version: 0.1.0
    alias: tracka
  - id: example.com/trackb
    version: 0.1.0
    alias: trackb
  - id: example.com/session
    version: 0.1.0
    alias: session
    config:
      cookieName: service_session
      maxAgeSeconds: 7200
"#,
        ),
    );
    write_session_package(temp.root());
    write_track_package(temp.root(), "example.com/tracka", "tracka", "track_a", 3600);
    write_track_package(temp.root(), "example.com/trackb", "trackb", "track_b", 3600);

    assert_service_config_error_contains(
        temp.root(),
        &[
            "packages.config",
            "package runtime config belongs in config source packages.<alias>",
            "not service.yml",
        ],
    );
}

fn assert_service_config_error_contains(root: &std::path::Path, expected: &[&str]) {
    let error = read_service_config(root).expect_err("service config should fail");
    let message = error.to_string();
    for part in expected {
        assert!(
            message.contains(part),
            "expected error to contain {part:?}; actual: {message}"
        );
    }
}

fn write_session_package(root: &std::path::Path) {
    write_package_manifest(
        root,
        "example.com/session",
        r#"
id: example.com/session
version: 0.1.0
"#,
    );
    write_package_api_yml(
        root,
        "example.com/session",
        r#"
issue: session_impl.issue
"#,
    );
    write_package_source(
        root,
        "example.com/session",
        "session_impl.skiff",
        r#"
          function issue() -> string {
            const cookieName = config.require<string>("cookieName")
            const cookieDomain = config.optional<string>("cookieDomain")
            const maxAgeSeconds = config.require<number>("maxAgeSeconds")
            return cookieName
          }
        "#,
    );
}

fn write_track_package(
    root: &std::path::Path,
    package_id: &str,
    module: &str,
    cookie_name: &str,
    max_age_seconds: u64,
) {
    write_package_manifest_in_dir(
        root,
        package_id,
        &format!(
            r#"
id: {package_id}
version: 0.1.0
packages:
  - id: example.com/session
    version: 0.1.0
    alias: session
    config:
      cookieName: {cookie_name}
      maxAgeSeconds: {max_age_seconds}
"#
        ),
    );
    write_package_api_yml(root, package_id, &format!("record: {module}.record\n"));
    write_package_source(
        root,
        package_id,
        &format!("{module}.skiff"),
        &format!(
            r#"
          import session

          function record() -> string {{
            return session.issue()
          }}
        "#
        ),
    );
}

fn config_requirement_by_path<'a>(
    requirements: &'a [serde_json::Value],
    path: &str,
) -> &'a serde_json::Value {
    requirements
        .iter()
        .find(|entry| entry["path"] == path)
        .unwrap_or_else(|| panic!("config requirement {path}"))
}

fn assert_config_source_span(span: &serde_json::Value) {
    let start = &span["start"];
    let end = &span["end"];
    assert!(
        start["line"].as_u64().is_some_and(|line| line > 0),
        "config source span should have a real start line: {span}"
    );
    assert!(
        start["column"].as_u64().is_some_and(|column| column > 0),
        "config source span should have a real start column: {span}"
    );
    let start_offset = start["offset"]
        .as_u64()
        .unwrap_or_else(|| panic!("config source span start offset: {span}"));
    let end_offset = end["offset"]
        .as_u64()
        .unwrap_or_else(|| panic!("config source span end offset: {span}"));
    assert!(
        end_offset > start_offset,
        "config source span should cover source text: {span}"
    );
}
