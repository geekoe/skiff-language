use std::fs;

mod common;
use common::{package_project::compile_package_project, TestDir};
use skiff_artifact_identity::{
    validate_package_artifact_identities, PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
    PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
};

#[test]
fn package_compile_assigns_self_validating_canonical_identities() {
    let temp = package_project(
        "canonical-identities",
        "run: main.run\n",
        "function run() -> string { return \"ok\" }\n",
    );
    let project = compile_package_project(temp.path()).expect("package should compile");
    let artifact = &project.package.artifact;

    validate_package_artifact_identities(artifact).expect("artifact identities should validate");
    assert!(artifact
        .package_build_id
        .as_str()
        .starts_with(PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX));
    assert!(artifact
        .package_local_abi
        .local_abi_identity
        .as_str()
        .starts_with(PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX));
    assert_eq!(
        project.package.published.identity,
        artifact.package_build_id.to_string()
    );
}

#[test]
fn implementation_changes_build_identity_but_not_local_abi() {
    let left = package_project(
        "implementation-identity-left",
        "run: main.run\n",
        "function run() -> string { return \"left\" }\n",
    );
    let right = package_project(
        "implementation-identity-right",
        "run: main.run\n",
        "function run() -> string { return \"right\" }\n",
    );
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

#[test]
fn public_api_changes_local_abi_and_build_identity() {
    let left = package_project(
        "api-identity-left",
        "run: main.run\n",
        "function run() -> string { return \"ok\" }\n",
    );
    let right = package_project(
        "api-identity-right",
        "run: main.run\n",
        "function run(input: string) -> string { return input }\n",
    );
    let left = compile_package_project(left.path()).expect("left package should compile");
    let right = compile_package_project(right.path()).expect("right package should compile");

    assert_ne!(
        left.package.artifact.package_local_abi.local_abi_identity,
        right.package.artifact.package_local_abi.local_abi_identity
    );
    assert_ne!(
        left.package.artifact.package_build_id,
        right.package.artifact.package_build_id
    );
}

#[test]
fn package_test_sources_do_not_change_production_artifact_identity() {
    let left = package_project(
        "test-source-identity-left",
        "run: main.run\n",
        "function run() -> string { return \"ok\" }\n",
    );
    let right = package_project(
        "test-source-identity-right",
        "run: main.run\n",
        "function run() -> string { return \"ok\" }\n",
    );
    write(
        &left.path().join("main.test.skiff"),
        "test \"left\" { assert true, \"left\" }\n",
    );
    write(
        &right.path().join("main.test.skiff"),
        "test \"right\" { assert false == false, \"right\" }\n",
    );
    let left = compile_package_project(left.path()).expect("left package should compile");
    let right = compile_package_project(right.path()).expect("right package should compile");

    assert_eq!(
        left.package.artifact.package_build_id,
        right.package.artifact.package_build_id
    );
    assert!(left
        .package
        .file_ir_units
        .iter()
        .all(|file| !file.source_path.ends_with(".test.skiff")));
}

fn package_project(name: &str, api: &str, source: &str) -> TestDir {
    let temp = TestDir::new("skiff-compiler", name);
    write(
        &temp.path().join("package.yml"),
        "id: example.com/identity-fixture\nversion: 1.0.0\n",
    );
    write(&temp.path().join("api.yml"), api);
    write(&temp.path().join("main.skiff"), source);
    temp
}

fn write(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}
