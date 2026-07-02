use skiff_compiler::test_support::project_fixtures::{
    write_package_api_yml, ServiceProjectBuilder,
};
use skiff_compiler_core::artifact::{PackageUnit, ServiceUnit};

mod common;
use common::artifacts::build_temp_service_publication;

#[test]
fn compiler_artifacts_deserialize_as_artifact_model_dtos() {
    let project = ServiceProjectBuilder::package_model(
        "runtime-artifact-conformance",
        "import app\nimport std",
        r#"
          const headers = Array.empty<std.http.HttpHeader>()
          return {}
        "#,
    )
    .with_service_package_dependency("example.com/pkg", Some("app"));
    project.add_local_package(
        "example.com/pkg",
        r#"
id: example.com/pkg
version: 0.1.0
"#,
    );
    write_package_api_yml(
        project.root(),
        "example.com/pkg",
        r#"
tools:
  StatusPayload: tools_impl.StatusPayload
"#,
    );
    project.add_package_source(
        "example.com/pkg",
        "tools_impl.skiff",
        r#"
          import std

          type StatusPayload {
            request: std.http.HttpClientRequest,
            event: std.http.HttpResponseStreamEvent,
            file: std.file.ImmutableFile,
            raw: Json,
            body: bytes,
          }
        "#,
    );
    let published = build_temp_service_publication(project.root());

    let service_unit: ServiceUnit =
        serde_json::from_value(published.artifacts.service_unit.value.clone()).unwrap();
    assert!(
        published
            .artifacts
            .package_units
            .iter()
            .any(|artifact| artifact.value["packageId"] == "skiff.run/std"),
        "fixture should publish the official std package unit"
    );
    assert_file_refs_are_lightweight_canonical(&published.artifacts.service_unit.value);

    assert!(
        !published.artifacts.package_units.is_empty(),
        "fixture should publish at least one package unit"
    );
    for artifact in &published.artifacts.package_units {
        let package_unit: PackageUnit = serde_json::from_value(artifact.value.clone()).unwrap();
        assert!(
            package_unit
                .files
                .iter()
                .all(|file_ref| !file_ref.module_path.is_empty()),
            "package unit file refs should include canonical modulePath"
        );
        assert_file_refs_are_lightweight_canonical(&artifact.value);
    }

    for artifact in published
        .artifacts
        .file_ir_units
        .iter()
        .chain(published.artifacts.package_file_ir_units.iter())
    {
        assert_eq!(artifact.unit.module_path, artifact.module_path);
    }

    assert!(
        service_unit
            .files
            .iter()
            .all(|file_ref| !file_ref.module_path.is_empty()),
        "service unit file refs should include canonical modulePath"
    );
}

fn assert_file_refs_are_lightweight_canonical(unit_value: &serde_json::Value) {
    let files = unit_value["files"]
        .as_array()
        .expect("unit files should be an array");
    assert!(!files.is_empty(), "unit should carry file refs");

    for file_ref in files {
        assert!(
            file_ref.get("fileIrIdentity").is_some(),
            "file ref should include fileIrIdentity: {file_ref}"
        );
        assert!(
            file_ref.get("modulePath").is_some(),
            "file ref should include modulePath: {file_ref}"
        );
        assert!(
            file_ref.get("artifactPath").is_some(),
            "file ref should include artifactPath for loader resolution: {file_ref}"
        );
        assert!(
            file_ref.get("typeTable").is_none()
                && file_ref.get("declarations").is_none()
                && file_ref.get("executables").is_none(),
            "file ref should be lightweight, not an inline File IR unit: {file_ref}"
        );
    }
}
