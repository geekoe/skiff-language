use skiff_compiler::{
    test_support::compile_source_file_ir_artifact_for_test as compile_source_file_ir_artifact,
    test_support::read_user_package_manifest, PublishedJsonArtifact,
};
use skiff_syntax::parser::parse_source;

mod common;
use common::artifacts::{
    assert_file_ir_contains_package_symbol, assert_publish_error_contains,
    assert_service_package_id, build_temp_service_publication, package_assembly,
};
use skiff_compiler::test_support::project_fixtures::{
    write_complex_cloud_package, write_package_api_yml, write_package_manifest,
    write_package_manifest_in_dir, write_package_source, write_package_with_dependency_alias,
    ServiceProjectBuilder,
};

#[test]
fn source_import_alias_is_rejected() {
    let error = parse_source(
        r#"
            import std as foo
            function run() -> number { return 1 }
        "#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("import name must be a single ASCII identifier"),
        "unexpected error: {error}"
    );
}

#[test]
fn source_import_complex_package_is_rejected() {
    let error = parse_source(
        r#"
            import google.com/cloud
            function run() -> number { return 1 }
        "#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("import name must be a single ASCII identifier"),
        "unexpected error: {error}"
    );
}

#[test]
fn source_import_complex_package_with_alias_is_rejected() {
    let error = parse_source(
        r#"
            import google.com/cloud as gcloud
            function run() -> number { return 1 }
        "#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("import name must be a single ASCII identifier"),
        "unexpected error: {error}"
    );
}

#[test]
fn source_import_slash_package_is_rejected() {
    let error = parse_source(
        r#"
            import google/cloud
            function run() -> number { return 1 }
        "#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("import name must be a single ASCII identifier"),
        "unexpected error: {error}"
    );
}

#[test]
fn source_import_non_identifier_is_rejected_with_import_rule() {
    let error = parse_source(
        r#"
            import 123
            function run() -> number { return 1 }
        "#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("import name must be a single ASCII identifier"),
        "unexpected error: {error}"
    );
}

#[test]
fn import_simple_package_default_local_binding_is_name() {
    let ast = parse_source(
        r#"
            import billing
            function run() -> number { return 1 }
        "#,
    )
    .expect("simple package id should parse");
    let import = &ast.imports[0];
    assert_eq!(import.alias, None);
    assert_eq!(import.local_binding.as_deref(), Some("billing"));
}

#[test]
fn package_manifest_rejects_removed_metadata_fields() {
    for (field, yaml) in [
        ("transports", "transports: [legacy]"),
        ("providers", "providers: []"),
        (
            "effects",
            r#"effects:
  symbols: {}
"#,
        ),
        (
            "publicEffects",
            r#"publicEffects:
  example.com/removed.run:
    target: example.com/removed.run
"#,
        ),
    ] {
        let temp = ServiceProjectBuilder::package_model(
            &format!("removed-package-field-{field}"),
            "import app",
            "return {}",
        );
        temp.add_service_package_dependency("example.com/removed", Some("app"));
        write_package_manifest(
            temp.root(),
            "example.com/removed",
            &format!(
                r#"
id: example.com/removed
version: 0.1.0
{yaml}
"#
            ),
        );
        write_package_api_yml(
            temp.root(),
            "example.com/removed",
            r#"
run: removed.run
"#,
        );
        write_package_source(
            temp.root(),
            "example.com/removed",
            "removed.skiff",
            r#"
              function run() -> string { return "ok" }
            "#,
        );

        let expected = format!("unknown field `{field}`");
        assert_publish_error_contains(temp.root(), &[expected.as_str()]);
    }
}

#[test]
fn service_dependency_package_alias_selects_complex_package_id() {
    let temp = ServiceProjectBuilder::package_model(
        "service-dependency-package-alias",
        "import gcloud",
        r#"
          const result = gcloud.storage.upload()
          return {}
        "#,
    );
    temp.add_root_file(
        "service.yml",
        &service_config_with_packages(
            r#"
  - id: google.com/cloud
    version: 0.1.0
    alias: gcloud
"#,
        ),
    );
    write_package_manifest_in_dir(
        temp.root(),
        "google.com/cloud",
        r#"
id: google.com/cloud
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "google.com/cloud",
        r#"
storage:
  upload: cloud.storage.upload
"#,
    );
    write_package_source(
        temp.root(),
        "google.com/cloud",
        "cloud/storage.skiff",
        r#"
          function upload() -> string { return "ok" }
        "#,
    );

    let published = build_temp_service_publication(temp.root());
    assert_service_package_id(&published, "google.com/cloud");
    assert_file_ir_contains_package_symbol(
        &published,
        "internal.example",
        "gcloud",
        "storage.upload",
    );
    assert!(package_assembly(&published, "google.com/cloud")
        .path
        .starts_with("assemblies/packages/google~com~~cloud/"));
}

#[test]
fn package_alias_resolves_each_exported_module_root() {
    let temp = ServiceProjectBuilder::package_model(
        "package-alias-multiple-export-roots",
        "import gcloud",
        r#"
          const storage = gcloud.storage.upload()
          const compute = gcloud.compute.start()
          return {}
        "#,
    );
    temp.add_root_file(
        "service.yml",
        &service_config_with_packages(
            r#"
  - id: google.com/cloud
    version: 0.1.0
    alias: gcloud
"#,
        ),
    );
    write_package_manifest_in_dir(
        temp.root(),
        "google.com/cloud",
        r#"
id: google.com/cloud
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "google.com/cloud",
        r#"
compute:
  start: cloud.compute.start
storage:
  upload: cloud.storage.upload
"#,
    );
    write_package_source(
        temp.root(),
        "google.com/cloud",
        "cloud/compute.skiff",
        r#"
          function start() -> string { return "ok" }
        "#,
    );
    write_package_source(
        temp.root(),
        "google.com/cloud",
        "cloud/storage.skiff",
        r#"
          function upload() -> string { return "ok" }
        "#,
    );
    let published = build_temp_service_publication(temp.root());
    assert_file_ir_contains_package_symbol(
        &published,
        "internal.example",
        "gcloud",
        "storage.upload",
    );
    assert_file_ir_contains_package_symbol(
        &published,
        "internal.example",
        "gcloud",
        "compute.start",
    );
}

#[test]
fn package_alias_canonicalizes_simple_export_path_to_package_export_key() {
    let temp = ServiceProjectBuilder::package_model(
        "package-alias-simple-export-key",
        "import app",
        r#"
          const secret = app.secrets.readProdSecret()
          return {}
        "#,
    );
    temp.add_service_package_dependency("example.com/pkg", Some("app"));
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
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/pkg",
        "secrets_impl.skiff",
        r#"
          function readProdSecret() -> string { return "ok" }
        "#,
    );
    let published = build_temp_service_publication(temp.root());
    assert_file_ir_contains_package_symbol(
        &published,
        "internal.example",
        "app",
        "secrets.readProdSecret",
    );
}

#[test]
fn package_alias_empty_public_path_exposes_module_at_alias_root() {
    let temp = ServiceProjectBuilder::package_model(
        "package-alias-empty-public-path",
        "import llm",
        r#"
          const result = llm.chat()
          return {}
        "#,
    );
    temp.add_service_package_dependency("skiff.run/llm", Some("llm"));
    write_package_manifest_in_dir(
        temp.root(),
        "skiff.run/llm",
        r#"
id: skiff.run/llm
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "skiff.run/llm",
        r#"
chat: llm_impl.chat
"#,
    );
    write_package_source(
        temp.root(),
        "skiff.run/llm",
        "llm_impl.skiff",
        r#"
          function chat() -> string { return "ok" }
        "#,
    );
    let published = build_temp_service_publication(temp.root());
    assert_file_ir_contains_package_symbol(&published, "internal.example", "llm", "chat");
}

#[test]
fn package_alias_matching_public_path_is_not_folded() {
    let temp = ServiceProjectBuilder::package_model(
        "package-alias-public-path-not-folded",
        "import llm",
        r#"
          const result = llm.llm.chat()
          return {}
        "#,
    );
    temp.add_service_package_dependency("skiff.run/llm", Some("llm"));
    write_package_manifest_in_dir(
        temp.root(),
        "skiff.run/llm",
        r#"
id: skiff.run/llm
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "skiff.run/llm",
        r#"
llm:
  chat: llm_impl.chat
"#,
    );
    write_package_source(
        temp.root(),
        "skiff.run/llm",
        "llm_impl.skiff",
        r#"
          function chat() -> string { return "ok" }
        "#,
    );
    let published = build_temp_service_publication(temp.root());
    assert_file_ir_contains_package_symbol(&published, "internal.example", "llm", "llm.chat");
}

#[test]
fn package_alias_matching_public_path_rejects_folded_shorthand() {
    let temp = ServiceProjectBuilder::package_model(
        "package-alias-public-path-shorthand-rejected",
        "import llm",
        r#"
          const result = llm.chat()
          return {}
        "#,
    );
    temp.add_service_package_dependency("skiff.run/llm", Some("llm"));
    write_package_manifest_in_dir(
        temp.root(),
        "skiff.run/llm",
        r#"
id: skiff.run/llm
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "skiff.run/llm",
        r#"
llm:
  chat: llm_impl.chat
"#,
    );
    write_package_source(
        temp.root(),
        "skiff.run/llm",
        "llm_impl.skiff",
        r#"
          function chat() -> string { return "ok" }
        "#,
    );
    assert_publish_error_contains(
        temp.root(),
        &["package dependency `llm` does not export public operation `chat` for source call `llm.chat`"],
    );
}

#[test]
fn complex_package_dependency_requires_alias_for_source_import() {
    let temp = ServiceProjectBuilder::package_model(
        "service-complex-package-without-alias",
        "import cloud",
        "return {}",
    );
    temp.add_root_file(
        "service.yml",
        &service_config_with_packages(
            r#"
  - id: google.com/cloud
    version: 0.1.0
"#,
        ),
    );
    write_package_manifest_in_dir(
        temp.root(),
        "google.com/cloud",
        r#"
id: google.com/cloud
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "google.com/cloud",
        r#"
storage:
  upload: cloud.storage.upload
"#,
    );
    write_package_source(
        temp.root(),
        "google.com/cloud",
        "cloud/storage.skiff",
        r#"
          function upload() -> string { return "ok" }
        "#,
    );
    let error = skiff_compiler::read_service_config(temp.root())
        .unwrap_err()
        .to_string();
    assert!(error.contains("google.com/cloud requires alias"));
}

#[test]
fn package_dependency_alias_is_published_and_resolves_transitively() {
    let temp =
        ServiceProjectBuilder::package_model("package-dependency-alias", "import app", "return {}");
    temp.add_service_package_dependency("example.com/facade", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/facade",
        r#"
id: example.com/facade
version: 0.1.0
packages:
  - id: google.com/cloud
    version: 0.1.0
    alias: gcloud
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/facade",
        r#"
facade: facade_impl.facade
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/facade",
        "facade_impl.skiff",
        r#"
          import gcloud

          function facade() -> string { return gcloud.storage.upload() }
        "#,
    );
    write_package_manifest_in_dir(
        temp.root(),
        "google.com/cloud",
        r#"
id: google.com/cloud
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "google.com/cloud",
        r#"
storage:
  upload: cloud.storage.upload
"#,
    );
    write_package_source(
        temp.root(),
        "google.com/cloud",
        "cloud/storage.skiff",
        r#"
          function upload() -> string { return "ok" }
        "#,
    );
    let published = build_temp_service_publication(temp.root());
    assert_service_package_id(&published, "example.com/facade");
    let cloud_assembly = package_assembly(&published, "google.com/cloud");
    let facade_assembly = package_assembly(&published, "example.com/facade");
    assert_package_lock_entry(
        &facade_assembly.value["dependencies"][0],
        "google.com/cloud",
        "0.1.0",
        "gcloud",
        cloud_assembly,
    );
    let facade_artifact = published
        .artifacts
        .package_file_ir_units
        .iter()
        .find(|artifact| artifact.module_path == "facade_impl")
        .expect("facade_impl package artifact");
    let facade_value = facade_artifact.value();
    assert!(
        common::artifacts::json_contains_package_symbol(&facade_value, "gcloud", "storage.upload"),
        "package alias should compile to exported dependency module path: {facade_value}",
    );
}
#[test]
fn package_dependency_alias_platform_is_allowed() {
    let temp =
        ServiceProjectBuilder::package_model("package-platform-alias", "import app", "return {}");
    temp.add_service_package_dependency("example.com/facade", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/facade",
        r#"
id: example.com/facade
version: 0.1.0
packages:
  - id: google.com/cloud
    version: 0.1.0
    alias: platform
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/facade",
        r#"
facade: facade_impl.facade
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/facade",
        "facade_impl.skiff",
        r#"
          import platform

          function facade() -> string { return platform.storage.upload() }
        "#,
    );
    write_package_manifest_in_dir(
        temp.root(),
        "google.com/cloud",
        r#"
id: google.com/cloud
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "google.com/cloud",
        r#"
storage:
  upload: cloud.storage.upload
"#,
    );
    write_package_source(
        temp.root(),
        "google.com/cloud",
        "cloud/storage.skiff",
        r#"
          function upload() -> string { return "ok" }
        "#,
    );
    let published = build_temp_service_publication(temp.root());
    assert_service_package_id(&published, "example.com/facade");
    let cloud_assembly = package_assembly(&published, "google.com/cloud");
    let facade_assembly = package_assembly(&published, "example.com/facade");
    assert_package_lock_entry(
        &facade_assembly.value["dependencies"][0],
        "google.com/cloud",
        "0.1.0",
        "platform",
        cloud_assembly,
    );
    let facade_artifact = published
        .artifacts
        .package_file_ir_units
        .iter()
        .find(|artifact| artifact.module_path == "facade_impl")
        .expect("facade_impl package artifact");
    let facade_value = facade_artifact.value();
    assert!(
        common::artifacts::json_contains_package_symbol(
            &facade_value,
            "platform",
            "storage.upload"
        ),
        "package alias platform should compile to exported dependency module path: {facade_value}",
    );
}

#[test]
fn unknown_dotted_root_call_is_rejected_without_special_root_rules() {
    let source = r#"
        function run() -> string {
          return unknown.root.call()
        }
    "#;

    let error =
        compile_source_file_ir_artifact(source, "internal/run.skiff", "internal.run", "service")
            .unwrap_err()
            .to_string();

    assert!(
        error.contains("unresolved root unknown")
            && error.contains("unknown.root.call")
            && !error.contains("platform"),
        "expected unresolved root error, got:\n{error}"
    );
}

#[test]
fn package_dependency_alias_changes_assembly_identity() {
    let left = ServiceProjectBuilder::package_model(
        "package-alias-identity-left",
        "import app",
        "return {}",
    );
    left.add_service_package_dependency("example.com/facade", Some("app"));
    write_package_with_dependency_alias(left.root(), "left");
    write_complex_cloud_package(left.root());
    let left_published = build_temp_service_publication(left.root());
    let left_assembly = package_assembly(&left_published, "example.com/facade");

    let right = ServiceProjectBuilder::package_model(
        "package-alias-identity-right",
        "import app",
        "return {}",
    );
    right.add_service_package_dependency("example.com/facade", Some("app"));
    write_package_with_dependency_alias(right.root(), "right");
    write_complex_cloud_package(right.root());
    let right_published = build_temp_service_publication(right.root());
    let right_assembly = package_assembly(&right_published, "example.com/facade");

    assert_ne!(left_assembly.identity, right_assembly.identity);
    assert_ne!(left_assembly.path, right_assembly.path);
}

#[test]
fn transitive_package_version_conflicts_are_rejected_after_selection() {
    let temp = ServiceProjectBuilder::package_model(
        "transitive-version-conflict",
        r#"
          import left
          import right
        "#,
        "return {}",
    );
    temp.add_root_file(
        "service.yml",
        &service_config_with_packages(
            r#"
  - id: example.com/left
    version: 0.1.0
    alias: left
  - id: example.com/right
    version: 0.1.0
    alias: right
"#,
        ),
    );
    write_package_manifest(
        temp.root(),
        "example.com/left",
        r#"
id: example.com/left
version: 0.1.0
packages:
  - id: example.com/shared
    version: 1.0.0
    alias: shared
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/left",
        r#"
left:
  run: left.run
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/left",
        "left.skiff",
        r#"
          function run() -> string { return "left" }
        "#,
    );
    write_package_manifest(
        temp.root(),
        "example.com/right",
        r#"
id: example.com/right
version: 0.1.0
packages:
  - id: example.com/shared
    version: 2.0.0
    alias: shared
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/right",
        r#"
right:
  run: right.run
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/right",
        "right.skiff",
        r#"
          function run() -> string { return "right" }
        "#,
    );
    write_package_manifest(
        temp.root(),
        "example.com/shared",
        r#"
id: example.com/shared
version: 1.0.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/shared",
        r#"
run: main.run
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/shared",
        "main.skiff",
        r#"
          function run() -> string { return "example.com/shared" }
        "#,
    );
    write_package_manifest_in_dir(
        temp.root(),
        "example.com/example.com/shared-2",
        r#"
id: example.com/shared
version: 2.0.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/example.com/shared-2",
        r#"
run: main.run
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/example.com/shared-2",
        "main.skiff",
        r#"
          function run() -> string { return "example.com/shared" }
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &[
            "package dependency example.com/shared version 1.0.0",
            "selected package.yml version 2.0.0",
        ],
    );
}

#[test]
fn package_complex_dependency_requires_alias_for_source_import() {
    let temp = ServiceProjectBuilder::package_model(
        "package-complex-dependency-without-alias",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/facade", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/facade",
        r#"
id: example.com/facade
version: 0.1.0
packages:
  - id: google.com/cloud
    version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/facade",
        r#"
facade: facade.facade
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/facade",
        "facade.skiff",
        r#"
          import cloud

          function facade() -> string { return "ok" }
        "#,
    );
    write_package_manifest_in_dir(
        temp.root(),
        "google.com/cloud",
        r#"
id: google.com/cloud
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "google.com/cloud",
        r#"
storage:
  upload: cloud.storage.upload
"#,
    );
    write_package_source(
        temp.root(),
        "google.com/cloud",
        "cloud/storage.skiff",
        r#"
          function upload() -> string { return "ok" }
        "#,
    );
    assert_publish_error_contains(temp.root(), &["google.com/cloud requires alias"]);
}

#[test]
fn package_dependency_alias_must_be_unique() {
    let temp = ServiceProjectBuilder::package_model(
        "package-duplicate-dependency-alias",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/facade", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/facade",
        r#"
id: example.com/facade
version: 0.1.0
packages:
  - id: google.com/cloud
    version: 0.1.0
    alias: cloud
  - id: example.org/cloud
    version: 0.1.0
    alias: cloud
"#,
    );

    assert_publish_error_contains(
        temp.root(),
        &["packages alias cloud", "more than one package"],
    );
}

#[test]
fn unsafe_package_ids_are_rejected_before_artifact_paths_are_built() {
    let temp = ServiceProjectBuilder::package_model("unsafe-package-id", "", "return {}");
    write_package_manifest_in_dir(
        temp.root(),
        "bad-package",
        r#"
id: app/escape/extra
version: 0.1.0
"#,
    );

    let error = read_user_package_manifest(
        &temp
            .root()
            .join(".skiff-packages")
            .join("app~~escape~~extra")
            .join("0.1.0")
            .join("package.yml"),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("id app/escape/extra"));
    assert!(error.contains("publication id"));
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

fn service_config_with_packages(packages: &str) -> String {
    format!(
        r#"
id: example.com/example
version: 1.0.0
packages:
{packages}"#
    )
}
