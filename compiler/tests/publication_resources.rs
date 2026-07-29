use std::fs;

mod common;
use common::{artifacts::write_resource_blobs, package_project::compile_package_project, TestDir};

#[test]
fn package_static_resources_emit_refs_and_raw_blobs() {
    let temp = package_resource_project("emit-package-resource", "package prompt\n");
    let project = compile_package_project(temp.path()).expect("package project should compile");
    let resource = project
        .package
        .artifact
        .static_resources
        .first()
        .expect("package artifact should reference its static resource");
    let artifact_path = resource
        .artifact_path
        .as_deref()
        .expect("materialized resource should carry an artifact path");
    assert!(artifact_path.starts_with("records/package-artifacts/"));
    let blob = project
        .package
        .resource_blobs
        .iter()
        .find(|blob| blob.sha256 == resource.sha256 && blob.byte_len == resource.byte_len)
        .expect("resource blob by canonical content identity");

    assert_eq!(resource.path, "prompts/pkg.md");
    assert_eq!(blob.bytes, b"package prompt\n");

    let artifact_root = temp.path().join("artifact-root");
    write_resource_blobs(&artifact_root, &project.package);
    assert_eq!(
        fs::read(artifact_root.join(&blob.artifact_path)).unwrap(),
        b"package prompt\n"
    );
}

#[test]
fn package_resource_content_changes_build_identity_not_local_abi() {
    let left = package_resource_project("resource-identity-left", "package prompt\n");
    let right = package_resource_project("resource-identity-right", "changed prompt\n");
    let left = compile_package_project(left.path()).expect("left package should compile");
    let right = compile_package_project(right.path()).expect("right package should compile");

    assert_ne!(
        left.package.artifact.package_build_id,
        right.package.artifact.package_build_id
    );
    assert_eq!(
        left.package.artifact.package_local_abi.local_abi_identity,
        right.package.artifact.package_local_abi.local_abi_identity
    );
}

fn package_resource_project(name: &str, resource: &str) -> TestDir {
    let temp = TestDir::new("skiff-compiler", name);
    temp.write(
        "package.yml",
        "id: example.com/resource-package\nversion: 1.0.0\nresources:\n  - prompts/pkg.md\n",
    );
    temp.write("api.yml", "label: main.label\n");
    temp.write(
        "main.skiff",
        "function label() -> string {\n  return \"resource\"\n}\n",
    );
    temp.write("prompts/pkg.md", resource);
    temp
}
