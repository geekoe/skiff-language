use std::fs;

mod common;
use common::{
    artifacts::{resource_blob, write_resource_blobs},
    package_project::compile_package_project,
    TestDir,
};

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

    assert_eq!(resource.path, "prompts/pkg.md");
    assert_eq!(
        resource_blob(&project.package, artifact_path).bytes,
        b"package prompt\n"
    );

    let artifact_root = temp.path().join("artifact-root");
    write_resource_blobs(&artifact_root, &project.package);
    assert_eq!(
        fs::read(artifact_root.join(artifact_path)).unwrap(),
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
    write(
        &temp.path().join("package.yml"),
        "id: example.com/resource-package\nversion: 1.0.0\nresources:\n  - prompts/pkg.md\n",
    );
    write(&temp.path().join("api.yml"), "label: main.label\n");
    write(
        &temp.path().join("main.skiff"),
        "function label() -> string {\n  return \"resource\"\n}\n",
    );
    write(&temp.path().join("prompts/pkg.md"), resource);
    temp
}

fn write(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}
