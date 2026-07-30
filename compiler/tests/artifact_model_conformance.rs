use std::collections::BTreeSet;

mod common;
use common::{package_project::compile_package_project, TestDir};
use skiff_artifact_model::PackageArtifact;

#[test]
fn compiler_output_deserializes_as_the_canonical_package_artifact() {
    let temp = package_project("artifact-model-conformance");
    let project = compile_package_project(temp.path()).expect("package project should compile");
    let value = serde_json::to_value(&project.package.artifact)
        .expect("canonical PackageArtifact should serialize");
    let artifact: PackageArtifact =
        serde_json::from_value(value.clone()).expect("canonical DTO should deserialize");

    assert_eq!(artifact, project.package.artifact);
    assert_eq!(artifact.package_id, "example.com/artifact-model");
    assert!(!artifact.files.is_empty());
    assert_file_refs_are_lightweight_canonical(&value);
    assert_eq!(
        value
            .as_object()
            .expect("PackageArtifact should serialize as an object")
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "boundaryProjections",
            "callableLinks",
            "callableSemanticFacts",
            "contractRequirements",
            "files",
            "implementationLinks",
            "packageBuildId",
            "packageId",
            "packageLocalAbi",
            "packageRequirements",
            "packageSchemaIndex",
            "packageSchemaTypeRecords",
            "packageVersion",
            "runtimeRequirements",
            "schemaVersion",
            "serviceCallRefs",
            "serviceRequirements",
            "staticResources",
        ])
    );
}

fn assert_file_refs_are_lightweight_canonical(artifact: &serde_json::Value) {
    let files = artifact["files"]
        .as_array()
        .expect("artifact files should be an array");
    assert!(!files.is_empty(), "artifact should carry file refs");
    for file_ref in files {
        assert!(file_ref.get("fileIrIdentity").is_some());
        assert!(file_ref.get("modulePath").is_some());
        assert!(file_ref.get("artifactPath").is_some());
        assert!(file_ref.get("typeTable").is_none());
        assert!(file_ref.get("declarations").is_none());
        assert!(file_ref.get("executables").is_none());
    }
}

fn package_project(name: &str) -> TestDir {
    let temp = TestDir::new("skiff-compiler", name);
    temp.write(
        "package.yml",
        "id: example.com/artifact-model\nversion: 1.0.0\n",
    );
    temp.write("api.yml", "Status: main.Status\nstatus: main.status\n");
    temp.write(
        "main.skiff",
        "type Status { ok: boolean }\nfunction status() -> Status {\n  return Status { ok: true }\n}\n",
    );
    temp
}
