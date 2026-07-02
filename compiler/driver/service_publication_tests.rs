use std::{fs, path::Path};

use crate::{
    collect_source_tree,
    input::{PackageResolutionDirs, ServiceDependency},
    pipeline::{build_service_publication, ServicePublicationBuildInput},
    read_service_config,
    test_support::project_fixtures::TestDir,
};

use crate::input::service_dependencies::resolve_service_dependencies;

#[test]
fn service_dependency_resolution_requires_artifact_roots_for_declared_dependencies() {
    let dependencies = vec![ServiceDependency {
        id: "skiff.run/account".to_string(),
        version: "0.1.0".to_string(),
        alias: "account".to_string(),
    }];

    let error = resolve_service_dependencies(&dependencies, &[])
        .expect_err("declared service dependencies require external artifacts")
        .to_string();

    assert!(
        error.contains(
            "service dependencies require --service-artifact-root so callee artifacts can be resolved"
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn publishes_service_publication_rejects_legacy_mongo_provider_package() {
    let temp = TestDir::new("skiff-publish-service", "legacy-mongo-provider");
    write_legacy_mongo_dependent_service(temp.path());
    let root = temp.path();
    let config = read_service_config(&root).unwrap();
    let source_tree = collect_source_tree(&root).unwrap();
    let error = build_service_publication(ServicePublicationBuildInput {
        config: &config,
        source_tree: &source_tree,
        package_dirs: PackageResolutionDirs {
            package_dirs: vec![root.join(".skiff-packages")],
        },
        ..ServicePublicationBuildInput::new(&config, &source_tree)
    })
    .unwrap_err();
    let message = error.to_string();

    assert!(
        message.contains(".skiff-packages/skiff~run~~mongo/1.0.0/mongo.skiff"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("legacy provider syntax has been removed"),
        "unexpected error: {message}"
    );
}

fn write_legacy_mongo_dependent_service(root: &Path) {
    fs::create_dir_all(root.join("api")).unwrap();
    fs::create_dir_all(root.join("internal")).unwrap();
    fs::write(
        root.join("service.yml"),
        r#"
id: example.com/legacy-mongo-provider
version: 1.0.0
packages:
  - id: skiff.run/mongo
    version: 1.0.0
    alias: mongo
"#,
    )
    .unwrap();
    fs::write(
        root.join("api.yml"),
        r#"
ExampleService: internal.example.ExampleService
api:
  example:
    Output: api.example.Output
    ExampleService: api.example.ExampleService
"#,
    )
    .unwrap();
    fs::write(
        root.join("api").join("example.skiff"),
        r#"
type Output {}
interface ExampleService {
  function ping() -> Output
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("internal").join("example.skiff"),
        r#"
import mongo

type ExampleService {}

impl ExampleService {
  function ping(self: ExampleService) -> root.api.example.Output {
    const target = mongo.Target("test-cluster", "example")
    return {}
  }
}
"#,
    )
    .unwrap();
    write_legacy_mongo_package(root);
}

fn write_legacy_mongo_package(root: &Path) {
    let package_root = root
        .join(".skiff-packages")
        .join("skiff~run~~mongo")
        .join("1.0.0");
    fs::create_dir_all(&package_root).unwrap();
    fs::write(
        package_root.join("package.yml"),
        r#"
id: skiff.run/mongo
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_root.join("api.yml"),
        r#"
MongoTarget: mongo.MongoTarget
"#,
    )
    .unwrap();
    fs::write(
        package_root.join("mongo.skiff"),
        r#"
provider mongo

export type MongoTarget {}
"#,
    )
    .unwrap();
}
