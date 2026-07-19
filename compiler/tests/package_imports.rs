mod common;

use std::{fs, path::Path};

use common::{artifacts::module_artifact, package_project::compile_package_project, TestDir};
use skiff_compiler_input::package_config::read_user_package_manifest;
use skiff_syntax::parser::parse_source;

#[test]
fn source_import_syntax_accepts_only_one_identifier() {
    for source in [
        "import std as foo\nfunction run() -> number { return 1 }",
        "import google.com/cloud\nfunction run() -> number { return 1 }",
        "import google.com/cloud as gcloud\nfunction run() -> number { return 1 }",
        "import google/cloud\nfunction run() -> number { return 1 }",
        "import 123\nfunction run() -> number { return 1 }",
    ] {
        let error = parse_source(source).unwrap_err().to_string();
        assert!(
            error.contains("import name must be a single ASCII identifier"),
            "unexpected import error: {error}"
        );
    }

    let ast = parse_source("import billing\nfunction run() -> number { return 1 }")
        .expect("simple import should parse");
    assert_eq!(ast.imports[0].alias, None);
    assert_eq!(ast.imports[0].local_binding.as_deref(), Some("billing"));
}

#[test]
fn package_manifest_rejects_removed_fields_and_unsafe_ids() {
    for (field, yaml) in [
        ("transports", "transports: [legacy]"),
        ("providers", "providers: []"),
        ("effects", "effects:\n  symbols: {}"),
        (
            "publicEffects",
            "publicEffects:\n  example.com/removed.run:\n    target: example.com/removed.run",
        ),
    ] {
        let temp = TestDir::new("skiff-compiler", field);
        fs::write(
            temp.path().join("package.yml"),
            format!("id: example.com/removed\nversion: 0.1.0\n{yaml}\n"),
        )
        .unwrap();
        let error = read_user_package_manifest(&temp.path().join("package.yml"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&format!("unknown field `{field}`")),
            "unexpected manifest error: {error}"
        );
    }

    let unsafe_id = TestDir::new("skiff-compiler", "unsafe-package-id");
    fs::write(
        unsafe_id.path().join("package.yml"),
        "id: app/escape/extra\nversion: 0.1.0\n",
    )
    .unwrap();
    let error = read_user_package_manifest(&unsafe_id.path().join("package.yml"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("id app/escape/extra"));
    assert!(error.contains("publication id"));
}

#[test]
fn dependency_alias_projects_each_public_operation_into_file_ir() {
    let temp = TestDir::new("skiff-compiler", "complex-package-alias");
    fs::write(
        temp.path().join("package.yml"),
        r#"id: example.com/import-app
version: 1.0.0
packages:
  - id: google.com/cloud
    version: 0.1.0
    alias: gcloud
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("main.skiff"),
        r#"
import gcloud
function run() -> string {
  const stored = gcloud/storage.upload()
  return gcloud/compute.start()
}
"#,
    )
    .unwrap();
    write_cloud_dependency(temp.path());

    let project = compile_package_project(temp.path()).expect("alias graph should compile");
    let cloud = project
        .dependency("google.com/cloud", "0.1.0")
        .expect("cloud artifact should be in the dependency closure");
    let requirement = project
        .package
        .artifact
        .package_requirements
        .iter()
        .find(|requirement| requirement.package_id == "google.com/cloud")
        .expect("root artifact should retain its canonical requirement");

    assert_eq!(requirement.alias, "gcloud");
    assert_eq!(
        requirement.expected_local_abi,
        cloud.artifact.package_local_abi.local_abi_identity
    );
    assert_file_ir_contains_package_callable(
        &project.package,
        "main",
        "gcloud",
        "google.com/cloud",
        "storage.upload",
    );
    assert_file_ir_contains_package_callable(
        &project.package,
        "main",
        "gcloud",
        "google.com/cloud",
        "compute.start",
    );
}

#[test]
fn public_path_shape_is_preserved_under_dependency_alias() {
    let nested = TestDir::new("skiff-compiler", "nested-public-path");
    fs::write(
        nested.path().join("package.yml"),
        r#"id: example.com/nested-consumer
version: 1.0.0
packages:
  - id: skiff.run/llm
    version: 0.1.0
    alias: llm
"#,
    )
    .unwrap();
    fs::write(
        nested.path().join("main.skiff"),
        "import llm\nfunction run() -> string { return llm/llm.chat() }\n",
    )
    .unwrap();
    write_llm_dependency(nested.path(), "llm:\n  chat: llm_impl.chat\n");

    let project = compile_package_project(nested.path()).expect("nested export should compile");
    assert_file_ir_contains_package_callable(
        &project.package,
        "main",
        "llm",
        "skiff.run/llm",
        "llm.chat",
    );

    let folded = TestDir::new("skiff-compiler", "folded-public-path");
    fs::write(
        folded.path().join("package.yml"),
        r#"id: example.com/folded-consumer
version: 1.0.0
packages:
  - id: skiff.run/llm
    version: 0.1.0
    alias: llm
"#,
    )
    .unwrap();
    fs::write(
        folded.path().join("main.skiff"),
        "import llm\nfunction run() -> string { return llm/chat() }\n",
    )
    .unwrap();
    write_llm_dependency(folded.path(), "llm:\n  chat: llm_impl.chat\n");
    let error = compile_package_project(folded.path())
        .expect_err("folded shorthand should stay invalid")
        .to_string();
    assert!(
        error.contains("package dependency `llm` has no callable public path `chat`"),
        "unexpected error: {error}"
    );

    let flat = TestDir::new("skiff-compiler", "flat-public-path");
    fs::write(
        flat.path().join("package.yml"),
        r#"id: example.com/flat-consumer
version: 1.0.0
packages:
  - id: skiff.run/llm
    version: 0.1.0
    alias: llm
"#,
    )
    .unwrap();
    fs::write(
        flat.path().join("main.skiff"),
        "import llm\nfunction run() -> string { return llm/chat() }\n",
    )
    .unwrap();
    write_llm_dependency(flat.path(), "chat: llm_impl.chat\n");
    let project = compile_package_project(flat.path()).expect("flat export should compile");
    assert_file_ir_contains_package_callable(
        &project.package,
        "main",
        "llm",
        "skiff.run/llm",
        "chat",
    );
}

#[test]
fn transitive_aliases_are_owned_by_each_package_artifact() {
    let temp = TestDir::new("skiff-compiler", "transitive-package-alias");
    fs::write(
        temp.path().join("package.yml"),
        r#"id: example.com/transitive-app
version: 1.0.0
packages:
  - id: example.com/facade
    version: 0.1.0
    alias: app
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("main.skiff"),
        "import app\nfunction run() -> string { return app/facade() }\n",
    )
    .unwrap();
    write_cloud_dependency(temp.path());

    let facade = temp
        .path()
        .join(".skiff-packages/example~com~~facade/0.1.0");
    fs::create_dir_all(&facade).unwrap();
    fs::write(
        facade.join("package.yml"),
        r#"id: example.com/facade
version: 0.1.0
packages:
  - id: google.com/cloud
    version: 0.1.0
    alias: platform
"#,
    )
    .unwrap();
    fs::write(facade.join("api.yml"), "facade: facade_impl.facade\n").unwrap();
    fs::write(
        facade.join("facade_impl.skiff"),
        "import platform\nfunction facade() -> string { return platform/storage.upload() }\n",
    )
    .unwrap();

    let project = compile_package_project(temp.path()).expect("transitive graph should compile");
    let facade = project
        .dependency("example.com/facade", "0.1.0")
        .expect("facade artifact should be present");
    assert!(project.dependency("google.com/cloud", "0.1.0").is_some());
    assert_eq!(
        project.package.artifact.package_requirements[0].alias,
        "app"
    );
    assert_eq!(facade.artifact.package_requirements[0].alias, "platform");
    assert_file_ir_contains_package_callable(
        facade,
        "facade_impl",
        "platform",
        "google.com/cloud",
        "storage.upload",
    );
    assert_file_ir_contains_package_callable(
        &project.package,
        "main",
        "app",
        "example.com/facade",
        "facade",
    );
}

#[test]
fn dependency_alias_participates_in_package_build_identity() {
    let mut identities = Vec::new();
    for alias in ["left", "right"] {
        let temp = TestDir::new("skiff-compiler", alias);
        fs::write(
            temp.path().join("package.yml"),
            format!(
                "id: example.com/identity-app\nversion: 1.0.0\npackages:\n  - id: google.com/cloud\n    version: 0.1.0\n    alias: {alias}\n"
            ),
        )
        .unwrap();
        fs::write(
            temp.path().join("main.skiff"),
            format!(
                "import {alias}\nfunction run() -> string {{ return {alias}/storage.upload() }}\n"
            ),
        )
        .unwrap();
        write_cloud_dependency(temp.path());
        identities.push(
            compile_package_project(temp.path())
                .expect("identity fixture should compile")
                .package
                .artifact
                .package_build_id,
        );
    }
    assert_ne!(identities[0], identities[1]);
}

#[test]
fn invalid_dependency_aliases_and_unknown_roots_fail_closed() {
    for (name, manifest, expected) in [
        (
            "complex-without-alias",
            "id: example.com/invalid\nversion: 1.0.0\npackages:\n  - id: google.com/cloud\n    version: 0.1.0\n",
            "google.com/cloud requires alias",
        ),
        (
            "duplicate-alias",
            "id: example.com/invalid\nversion: 1.0.0\npackages:\n  - id: google.com/cloud\n    version: 0.1.0\n    alias: cloud\n  - id: example.org/cloud\n    version: 0.1.0\n    alias: cloud\n",
            "packages alias cloud",
        ),
    ] {
        let temp = TestDir::new("skiff-compiler", name);
        fs::write(temp.path().join("package.yml"), manifest).unwrap();
        let error = read_user_package_manifest(&temp.path().join("package.yml"))
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "unexpected {name} error: {error}");
    }

    let unknown = TestDir::new("skiff-compiler", "unknown-root-call");
    fs::write(
        unknown.path().join("package.yml"),
        "id: example.com/unknown-root-call\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        unknown.path().join("main.skiff"),
        "function run() -> string { return unknown.root.call() }\n",
    )
    .unwrap();
    let error = compile_package_project(unknown.path())
        .expect_err("unknown root should fail")
        .to_string();
    assert!(error.contains("unresolved root unknown"));
    assert!(error.contains("unknown.root.call"));
}

fn assert_file_ir_contains_package_callable(
    package: &skiff_compiler::PublishedPackageArtifact,
    module_path: &str,
    dependency_ref: &str,
    package_id: &str,
    public_path: &str,
) {
    let file = module_artifact(package, module_path);
    let expected_id = format!("pkg-callable:{package_id}:{public_path}");
    assert!(
        file.unit
            .external_refs
            .package_callables
            .iter()
            .any(|callable| {
                matches!(
                    &callable.package_ref,
                    skiff_artifact_model::PackageRefIr::Dependency { dependency_ref: actual }
                        if actual == dependency_ref
                ) && callable.package_callable_id.as_str() == expected_id
            }),
        "File IR module {module_path} should reference {dependency_ref}:{expected_id}: {}",
        file.value()
    );
}

fn write_cloud_dependency(root: &Path) {
    let cloud = root.join(".skiff-packages/google~com~~cloud/0.1.0");
    fs::create_dir_all(cloud.join("cloud")).unwrap();
    fs::write(
        cloud.join("package.yml"),
        "id: google.com/cloud\nversion: 0.1.0\n",
    )
    .unwrap();
    fs::write(
        cloud.join("api.yml"),
        "compute:\n  start: cloud.compute.start\nstorage:\n  upload: cloud.storage.upload\n",
    )
    .unwrap();
    fs::write(
        cloud.join("cloud/compute.skiff"),
        "function start() -> string { return \"ok\" }\n",
    )
    .unwrap();
    fs::write(
        cloud.join("cloud/storage.skiff"),
        "function upload() -> string { return \"ok\" }\n",
    )
    .unwrap();
}

fn write_llm_dependency(root: &Path, api: &str) {
    let llm = root.join(".skiff-packages/skiff~run~~llm/0.1.0");
    fs::create_dir_all(&llm).unwrap();
    fs::write(
        llm.join("package.yml"),
        "id: skiff.run/llm\nversion: 0.1.0\n",
    )
    .unwrap();
    fs::write(llm.join("api.yml"), api).unwrap();
    fs::write(
        llm.join("llm_impl.skiff"),
        "function chat() -> string { return \"ok\" }\n",
    )
    .unwrap();
}
