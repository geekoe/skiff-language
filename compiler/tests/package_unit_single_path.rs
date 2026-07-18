use std::fs;

mod common;
use common::{artifacts::resource_blob, package_project::compile_package_project, TestDir};
use skiff_artifact_model::PackageArtifact;

#[test]
fn package_compile_has_one_terminal_package_artifact_path() {
    let temp = rich_package_project();
    let project = compile_package_project(temp.path()).expect("package graph should compile");
    let published = &project.package;

    let round_trip: PackageArtifact = serde_json::from_value(published.published.value.clone())
        .expect("published PackageArtifact should deserialize");
    assert_eq!(round_trip, published.artifact);
    assert_eq!(round_trip.package_id, "example.com/agent");
    assert_eq!(round_trip.package_version, "1.0.0");
    assert!(!round_trip.files.is_empty());
    assert!(!round_trip.package_local_abi.public_symbols.is_empty());
    assert!(!round_trip.callable_links.is_empty());
    assert_eq!(round_trip.package_requirements.len(), 1);
    assert_eq!(round_trip.package_requirements[0].alias, "base");
    assert_eq!(
        round_trip.package_requirements[0].package_id,
        "example.com/base"
    );
    assert_eq!(
        round_trip.runtime_requirements.config[0].path,
        "agent.token"
    );

    let resource = round_trip
        .static_resources
        .first()
        .expect("package resource ref");
    let resource_path = resource
        .artifact_path
        .as_deref()
        .expect("materialized package resource path");
    assert_eq!(
        resource_blob(published, resource_path).bytes,
        b"agent prompt\n"
    );
    assert!(project.dependency("example.com/base", "1.0.0").is_some());
}

fn rich_package_project() -> TestDir {
    let temp = TestDir::new("skiff-compiler", "package-artifact-single-path");
    write(
        &temp.path().join("package.yml"),
        r#"id: example.com/agent
version: 1.0.0
packages:
  - id: example.com/base
    version: 1.0.0
    alias: base
resources:
  - prompts/agent.md
"#,
    );
    write(
        &temp.path().join("api.yml"),
        "Agent: main.Agent\nrun: main.run\n",
    );
    write(
        &temp.path().join("main.skiff"),
        r#"import base

type Agent { label: string }

function run(input: base.Input) -> Agent {
  const token = config.require<string>("agent.token")
  return Agent { label: token }
}
"#,
    );
    write(&temp.path().join("prompts/agent.md"), "agent prompt\n");

    let dependency = temp.path().join(".skiff-packages/example~com~~base/1.0.0");
    write(
        &dependency.join("package.yml"),
        "id: example.com/base\nversion: 1.0.0\n",
    );
    write(&dependency.join("api.yml"), "Input: base.Input\n");
    write(
        &dependency.join("base.skiff"),
        "type Input { label: string }\n",
    );
    temp
}

fn write(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}
