mod common;
use common::{package_project::compile_package_project, TestDir};
use skiff_artifact_model::PackageArtifact;

#[test]
fn package_compile_has_one_terminal_package_artifact_path() {
    let temp = rich_package_project();
    let project = compile_package_project(temp.path()).expect("package graph should compile");
    let published = &project.package;

    let round_trip: PackageArtifact = serde_json::from_value(
        serde_json::to_value(&published.artifact)
            .expect("canonical PackageArtifact should serialize"),
    )
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
    assert!(resource_path.starts_with("records/package-artifacts/"));
    let resource_blob = published
        .resource_blobs
        .iter()
        .find(|blob| blob.sha256 == resource.sha256 && blob.byte_len == resource.byte_len)
        .expect("package resource blob by canonical content identity");
    assert_eq!(resource_blob.bytes, b"agent prompt\n");
    assert!(project.dependency("example.com/base", "1.0.0").is_some());
}

fn rich_package_project() -> TestDir {
    let temp = TestDir::new("skiff-compiler", "package-artifact-single-path");
    temp.write(
        "package.yml",
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
    temp.write("api.yml", "Agent: main.Agent\nrun: main.run\n");
    temp.write(
        "main.skiff",
        r#"import base

type Agent { label: string }

function run(input: base.Input) -> Agent {
  const token = config.require<string>("agent.token")
  return Agent { label: token }
}
"#,
    );
    temp.write("prompts/agent.md", "agent prompt\n");

    temp.write(
        ".skiff-packages/example~com~~base/1.0.0/package.yml",
        "id: example.com/base\nversion: 1.0.0\n",
    );
    temp.write(
        ".skiff-packages/example~com~~base/1.0.0/api.yml",
        "Input: base.Input\n",
    );
    temp.write(
        ".skiff-packages/example~com~~base/1.0.0/base.skiff",
        "type Input { label: string }\n",
    );
    temp
}
