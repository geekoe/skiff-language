use std::fs;

mod common;
use common::{package_project::compile_package_project, TestDir};

#[test]
fn package_dependency_compiles_without_provider_or_transport_metadata() {
    let temp = package_graph();
    let project = compile_package_project(temp.path()).expect("package graph should compile");
    let requirement = project
        .package
        .artifact
        .package_requirements
        .first()
        .expect("dependency should become a package requirement");

    assert_eq!(requirement.alias, "tools");
    assert_eq!(requirement.package_id, "example.com/tools");
    assert_eq!(requirement.exact_version, "1.0.0");
    assert!(project.dependency("example.com/tools", "1.0.0").is_some());

    for artifact in project.artifacts() {
        let value = &artifact.published.value;
        for forbidden in ["providerRequirements", "transports", "publicEffects"] {
            assert!(
                value.get(forbidden).is_none(),
                "PackageArtifact must not contain deployment/provider field {forbidden}"
            );
        }
    }
}

#[test]
fn deployment_provider_fields_are_not_package_manifest_surface() {
    for (name, field) in [
        (
            "package-providers-field",
            "providers:\n  - capability: example.com/queue/v1\n",
        ),
        ("package-transports-field", "transports: [local]\n"),
        (
            "package-public-effects-field",
            "publicEffects:\n  run:\n    effect: external.read\n",
        ),
    ] {
        let temp = package_with_source(name, "function run() -> string { return \"ok\" }\n");
        write(
            &temp.path().join("package.yml"),
            &format!("id: example.com/provider-fixture\nversion: 1.0.0\n{field}"),
        );
        let error = compile_package_project(temp.path())
            .expect_err("deployment/provider package field should fail")
            .to_string();
        assert!(
            error.contains("unknown field"),
            "unexpected manifest error for {name}: {error}"
        );
    }
}

#[test]
fn package_source_cannot_call_removed_provider_primitives() {
    for (name, source, expected) in [
        (
            "connect-provider-wrapper",
            r#"
import connect
function run() -> {} {
  const db = connect.mongo.Target("cluster-a", "app")
  return {}
}
"#,
            "connect.mongo provider wrapper has been removed",
        ),
        (
            "internal-provider-primitive",
            r#"
function run() -> {} {
  return __providerCallFindOne({}, {})
}
"#,
            "internal provider-call primitive",
        ),
    ] {
        let error = compile_package_project(package_with_source(name, source).path())
            .expect_err("removed provider source surface should fail")
            .to_string();
        assert!(
            error.contains(expected),
            "expected {expected:?} in compile error: {error}"
        );
    }
}

fn package_graph() -> TestDir {
    let temp = package_with_source(
        "canonical-package-dependency",
        r#"
import tools
function run() -> string { return tools.label() }
"#,
    );
    write(
        &temp.path().join("package.yml"),
        r#"id: example.com/provider-fixture
version: 1.0.0
packages:
  - id: example.com/tools
    version: 1.0.0
    alias: tools
"#,
    );
    let dependency = temp.path().join(".skiff-packages/example~com~~tools/1.0.0");
    write(
        &dependency.join("package.yml"),
        "id: example.com/tools\nversion: 1.0.0\n",
    );
    write(&dependency.join("api.yml"), "label: tools.label\n");
    write(
        &dependency.join("tools.skiff"),
        "function label() -> string { return \"tools\" }\n",
    );
    temp
}

fn package_with_source(name: &str, source: &str) -> TestDir {
    let temp = TestDir::new("skiff-compiler", name);
    write(
        &temp.path().join("package.yml"),
        "id: example.com/provider-fixture\nversion: 1.0.0\n",
    );
    write(&temp.path().join("api.yml"), "run: main.run\n");
    write(&temp.path().join("main.skiff"), source);
    temp
}

fn write(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}
