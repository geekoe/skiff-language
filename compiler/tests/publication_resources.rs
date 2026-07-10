use std::fs;

mod common;
use common::artifacts::{build_temp_service_publication, write_test_artifact_root};
use skiff_compiler::test_support::project_fixtures::{
    write_package_api_yml, write_package_manifest, write_package_source, ServiceProjectBuilder,
};
use skiff_compiler_core::json_utils::sha256_hex;

#[test]
fn publication_resources_emit_unit_refs_and_raw_blobs() {
    let project = resource_project("emit-unit-refs", "service prompt\n", "package prompt\n");
    let published = build_temp_service_publication(project.root());
    let artifact_root = project.temp_path().join("artifact-root");

    write_test_artifact_root(&artifact_root, &published);

    let service_sha = sha256_hex(b"service prompt\n");
    let package_sha = sha256_hex(b"package prompt\n");
    assert_eq!(
        published.artifacts.service_unit.value["resources"][0]["artifactPath"],
        format!("resources/sha256/{service_sha}")
    );
    assert!(
        published.artifacts.service_unit.value["resources"][0]
            .get("bytes")
            .is_none(),
        "resource bytes must not be embedded in service unit JSON"
    );
    let package_unit = published
        .artifacts
        .package_units
        .iter()
        .find(|unit| unit.value["packageId"] == "example.com/agent")
        .expect("dependency package unit should be emitted");
    assert_eq!(
        package_unit.value["resources"][0]["artifactPath"],
        format!("resources/sha256/{package_sha}")
    );
    assert_eq!(
        fs::read(artifact_root.join(format!("resources/sha256/{service_sha}"))).unwrap(),
        b"service prompt\n"
    );
    assert_eq!(
        fs::read(artifact_root.join(format!("resources/sha256/{package_sha}"))).unwrap(),
        b"package prompt\n"
    );
}

#[test]
fn resource_content_changes_build_identity_not_abi_or_protocol_identity() {
    let project = resource_project("identity", "service prompt\n", "package prompt\n");
    let first = build_temp_service_publication(project.root());
    let first_package = package_unit(&first, "example.com/agent");

    fs::write(
        project
            .root()
            .join(".skiff-packages/example~com~~agent/0.1.0/prompts/pkg.md"),
        "package prompt changed\n",
    )
    .unwrap();
    let package_changed = build_temp_service_publication(project.root());
    let changed_package = package_unit(&package_changed, "example.com/agent");
    assert_ne!(
        first_package["buildIdentity"],
        changed_package["buildIdentity"]
    );
    assert_eq!(first_package["abiIdentity"], changed_package["abiIdentity"]);
    assert_eq!(
        first.manifest.service.protocol_identity,
        package_changed.manifest.service.protocol_identity
    );

    fs::write(
        project.root().join("prompts/service.md"),
        "service prompt changed\n",
    )
    .unwrap();
    let service_changed = build_temp_service_publication(project.root());
    assert_ne!(
        first.artifacts.service_unit.identity,
        service_changed.artifacts.service_unit.identity
    );
    assert_eq!(
        first.manifest.service.protocol_identity,
        service_changed.manifest.service.protocol_identity
    );
}

fn resource_project(
    name: &str,
    service_resource: &str,
    package_resource: &str,
) -> ServiceProjectBuilder {
    let project = ServiceProjectBuilder::new(name)
        .write_root_file(
            "service.yml",
            r#"
id: example.com/example
version: 1.0.0
resources:
  - prompts/service.md
packages:
  - id: example.com/agent
    version: 0.1.0
    alias: agent
"#,
        )
        .write_root_file(
            "api.yml",
            r#"
ExampleService: internal.example.ExampleService
api:
  example:
    Input: api.example.Input
    Output: api.example.Output
    ExampleService: api.example.ExampleService
"#,
        )
        .write_source(
            "api/example.skiff",
            r#"
type Input {}
type Output {}
interface ExampleService {
  function run(input: Input) -> Output
}
"#,
        )
        .write_source(
            "internal/example.skiff",
            r#"
import agent

type ExampleService {}

impl ExampleService {
  function run(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    const value = agent.label()
    return root.api.example.Output {}
  }
}
"#,
        )
        .write_root_file("prompts/service.md", service_resource);

    write_package_manifest(
        project.root(),
        "example.com/agent",
        r#"
id: example.com/agent
version: 0.1.0
resources:
  - prompts/pkg.md
"#,
    );
    write_package_api_yml(project.root(), "example.com/agent", "label: agent.label\n");
    write_package_source(
        project.root(),
        "example.com/agent",
        "agent.skiff",
        r#"
function label() -> string {
  return "agent"
}
"#,
    );
    let package_resource_path = project
        .root()
        .join(".skiff-packages/example~com~~agent/0.1.0/prompts/pkg.md");
    fs::create_dir_all(package_resource_path.parent().unwrap()).unwrap();
    fs::write(package_resource_path, package_resource).unwrap();
    project
}

fn package_unit<'a>(
    published: &'a skiff_compiler::BuiltServicePublication,
    package_id: &str,
) -> &'a serde_json::Value {
    &published
        .artifacts
        .package_units
        .iter()
        .find(|unit| unit.value["packageId"] == package_id)
        .expect("package unit should be emitted")
        .value
}
