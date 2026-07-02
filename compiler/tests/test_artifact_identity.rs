use std::collections::BTreeSet;

mod common;
use common::artifacts::{
    build_temp_service_publication, package_assembly, package_source_artifact, source_artifact,
};
use skiff_compiler::test_support::project_fixtures::{
    write_package_api_yml, write_package_manifest, write_package_source,
    write_package_source_with_friend_test, ServiceProjectBuilder,
};
use skiff_compiler::PublishedJsonArtifact;

#[test]
fn publish_omits_test_sources_from_file_ir_units() {
    let temp = ServiceProjectBuilder::package_model("omit-test-artifacts", "", "return {}");
    temp.add_source(
        "api/example.test.skiff",
        r#"
            test "helper" {
              assert true, "should not be in production artifacts"
            }
        "#,
    );

    let published = build_temp_service_publication(temp.root());

    assert!(!published
        .artifacts
        .file_ir_units
        .iter()
        .any(|artifact| artifact.source_path.ends_with("example.test.skiff")));
}

#[test]
fn package_publish_omits_test_sources_from_package_file_ir_units() {
    let temp = ServiceProjectBuilder::package_model(
        "omit-package-test-artifacts",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/util", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/util",
        r#"
id: example.com/util
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/util",
        r#"
ok: util_impl.ok
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/util",
        "util_impl.skiff",
        r#"
          function ok() -> string {
            return "ok"
          }
        "#,
    );
    write_package_source(
        temp.root(),
        "example.com/util",
        "util.test.skiff",
        r#"
          test "package helper" {
            assert true, "should not be in production artifacts"
          }
        "#,
    );

    let published = build_temp_service_publication(temp.root());
    let package_source_paths = published
        .artifacts
        .package_file_ir_units
        .iter()
        .map(|artifact| artifact.source_path.as_str())
        .collect::<BTreeSet<_>>();

    assert!(package_source_paths.contains("util_impl.skiff"));
    assert!(!package_source_paths.contains("util.test.skiff"));

    let package_assembly = published
        .artifacts
        .package_assemblies
        .iter()
        .find(|artifact| artifact.value["package"]["id"] == "example.com/util")
        .expect("example.com/util package assembly should be published");
    let assembly_source_paths = package_assembly.value["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["sourcePath"].as_str().unwrap())
        .collect::<BTreeSet<_>>();

    assert!(assembly_source_paths.contains("util_impl.skiff"));
    assert!(!assembly_source_paths.contains("util.test.skiff"));

    let util_artifact = package_source_artifact(&published, "util_impl.skiff");
    let source_map = &package_assembly.value["sourceMap"];
    assert_eq!(source_map["format"], "skiff-source-map-v1");
    let source_map_sources = source_map["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|source| source["path"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(source_map_sources, BTreeSet::from(["util_impl.skiff"]));
    assert_eq!(
        source_map["sources"][0]["fileIrIdentity"],
        util_artifact.identity
    );
}

#[test]
fn friend_test_files_do_not_change_service_artifact_identity() {
    let left = ServiceProjectBuilder::package_model("source-tests-left", "", "return {}");
    write_service_handler_with_friend_test(
        left.root(),
        "false",
        "left case",
        r#"assert helper() == 1, "left""#,
    );
    let right = ServiceProjectBuilder::package_model("source-tests-right", "", "return {}");
    write_service_handler_with_friend_test(
        right.root(),
        "true",
        "right case",
        r#"assert helper() != 2, "right""#,
    );

    let left_published = build_temp_service_publication(left.root());
    let right_published = build_temp_service_publication(right.root());
    let left_artifact = source_artifact(&left_published, "internal/example.skiff");
    let right_artifact = source_artifact(&right_published, "internal/example.skiff");

    assert_eq!(
        left_artifact.unit.source_ast_hash,
        right_artifact.unit.source_ast_hash
    );
    assert_eq!(left_artifact.identity, right_artifact.identity);
    assert_eq!(
        left_published.artifacts.service_assembly.identity,
        right_published.artifacts.service_assembly.identity
    );
}

#[test]
fn service_api_yml_content_hash_changes_service_artifact_identity() {
    let left = service_api_identity_project("service-api-identity-left", service_api_yml(""));
    let right = service_api_identity_project(
        "service-api-identity-right",
        service_api_yml("# same API graph, different source content\n"),
    );
    let repeat = service_api_identity_project("service-api-identity-repeat", service_api_yml(""));

    let left_published = build_temp_service_publication(left.root());
    let right_published = build_temp_service_publication(right.root());
    let repeat_published = build_temp_service_publication(repeat.root());

    assert_eq!(
        left_published.manifest.service.protocol_identity,
        right_published.manifest.service.protocol_identity,
        "api.yml source-only changes should not alter the API graph/protocol identity"
    );
    assert_ne!(
        left_published.artifacts.service_assembly.identity,
        right_published.artifacts.service_assembly.identity,
        "api.yml content hash must participate in service assembly identity"
    );
    assert_ne!(
        left_published.artifacts.bundle.identity, right_published.artifacts.bundle.identity,
        "service bundle identity must reflect the service assembly identity change"
    );
    assert_eq!(
        left_published.artifacts.service_assembly.identity,
        repeat_published.artifacts.service_assembly.identity,
        "identical api.yml content should keep service assembly identity stable"
    );
    assert_eq!(
        left_published.artifacts.bundle.identity, repeat_published.artifacts.bundle.identity,
        "identical api.yml content should keep service bundle identity stable"
    );
    assert_eq!(
        left_published.artifacts.service_assembly.value["service"]["api"]["apiSource"]
            ["relativePath"],
        "api.yml"
    );
    assert_artifact_identity_matches_content_hash(
        &left_published.artifacts.service_assembly,
        &["service", "assemblyIdentity"],
    );
    assert_artifact_identity_matches_content_hash(
        &right_published.artifacts.service_assembly,
        &["service", "assemblyIdentity"],
    );
}

#[test]
fn friend_test_files_do_not_change_package_artifact_identity() {
    let left = ServiceProjectBuilder::package_model(
        "package-source-tests-left",
        "import app",
        "return {}",
    );
    left.add_service_package_dependency("example.com/util", Some("app"));
    write_package_manifest(
        left.root(),
        "example.com/util",
        r#"
id: example.com/util
version: 0.1.0
"#,
    );
    write_package_source_with_friend_test(
        left.root(),
        "false",
        "left package case",
        r#"assert helper() == 1, "left""#,
    );

    let right = ServiceProjectBuilder::package_model(
        "package-source-tests-right",
        "import app",
        "return {}",
    );
    right.add_service_package_dependency("example.com/util", Some("app"));
    write_package_manifest(
        right.root(),
        "example.com/util",
        r#"
id: example.com/util
version: 0.1.0
"#,
    );
    write_package_source_with_friend_test(
        right.root(),
        "true",
        "right package case",
        r#"assert helper() != 2, "right""#,
    );

    let left_published = build_temp_service_publication(left.root());
    let right_published = build_temp_service_publication(right.root());
    let left_artifact = package_source_artifact(&left_published, "util_impl.skiff");
    let right_artifact = package_source_artifact(&right_published, "util_impl.skiff");
    let left_assembly = package_assembly(&left_published, "example.com/util");
    let right_assembly = package_assembly(&right_published, "example.com/util");

    assert_eq!(
        left_artifact.unit.source_ast_hash,
        right_artifact.unit.source_ast_hash
    );
    assert_eq!(left_artifact.identity, right_artifact.identity);
    assert_eq!(left_assembly.identity, right_assembly.identity);
    assert_eq!(
        left_published.artifacts.service_assembly.identity,
        right_published.artifacts.service_assembly.identity
    );
}

#[test]
fn package_api_yml_content_hash_changes_package_artifact_identity() {
    let left = package_api_identity_service("package-api-identity-left", "ok: util_impl.ok\n");
    let right = package_api_identity_service(
        "package-api-identity-right",
        "# same API graph, different source content\nok: util_impl.ok\n",
    );
    let repeat = package_api_identity_service("package-api-identity-repeat", "ok: util_impl.ok\n");

    let left_published = build_temp_service_publication(left.root());
    let right_published = build_temp_service_publication(right.root());
    let repeat_published = build_temp_service_publication(repeat.root());
    let left_assembly = package_assembly(&left_published, "example.com/util");
    let right_assembly = package_assembly(&right_published, "example.com/util");
    let repeat_assembly = package_assembly(&repeat_published, "example.com/util");

    assert_ne!(
        left_assembly.identity, right_assembly.identity,
        "api.yml content hash must participate in package assembly identity"
    );
    assert_eq!(
        left_assembly.identity, repeat_assembly.identity,
        "identical api.yml content should keep package assembly identity stable"
    );
    assert_eq!(left_assembly.value["apiSource"]["relativePath"], "api.yml");
    assert_artifact_identity_matches_content_hash(left_assembly, &["package", "assemblyIdentity"]);
    assert_artifact_identity_matches_content_hash(right_assembly, &["package", "assemblyIdentity"]);
}

#[test]
fn contract_schema_artifact_identity_is_protocol_scoped_not_service_scoped() {
    let left = protocol_scoped_service("protocol-artifact-left", "app.test/alpha");
    let right = protocol_scoped_service("protocol-artifact-right", "app.test/beta");

    let left_published = build_temp_service_publication(left.root());
    let right_published = build_temp_service_publication(right.root());

    assert_eq!(
        left_published.manifest.service.protocol_identity,
        right_published.manifest.service.protocol_identity
    );
    assert_eq!(
        left_published.artifacts.contract_schema.path,
        right_published.artifacts.contract_schema.path
    );
    assert_eq!(
        left_published.artifacts.contract_schema.hash,
        right_published.artifacts.contract_schema.hash
    );
    assert_eq!(
        left_published.artifacts.contract_schema.identity,
        right_published.artifacts.contract_schema.identity
    );
    assert_eq!(
        left_published.artifacts.contract_schema.value,
        right_published.artifacts.contract_schema.value
    );
    for field in ["serviceId", "displayName", "revisionId"] {
        assert!(
            left_published
                .artifacts
                .contract_schema
                .value
                .get(field)
                .is_none(),
            "contract schema artifact should not contain service-scoped field {field}"
        );
    }

    assert_ne!(
        left_published.artifacts.index.path,
        right_published.artifacts.index.path
    );
    assert_eq!(
        left_published.artifacts.index.value["serviceId"],
        "app.test/alpha"
    );
    assert_eq!(
        right_published.artifacts.index.value["serviceId"],
        "app.test/beta"
    );
    assert_eq!(
        left_published.artifacts.index.value["contract"]["schemaPath"],
        left_published.artifacts.contract_schema.path
    );
    assert_eq!(
        right_published.artifacts.index.value["contract"]["schemaPath"],
        right_published.artifacts.contract_schema.path
    );
}

fn service_api_identity_project(name: &str, api_yml: String) -> ServiceProjectBuilder {
    let temp = ServiceProjectBuilder::package_model(name, "", "return {}");
    temp.add_root_file("api.yml", &api_yml);
    temp
}

fn service_api_yml(prefix: &str) -> String {
    format!(
        r#"{prefix}ExampleService: internal.example.ExampleService
api:
  example:
    Input: api.example.Input
    Output: api.example.Output
    ExampleService: api.example.ExampleService
"#
    )
}

fn assert_artifact_identity_matches_content_hash(
    artifact: &PublishedJsonArtifact,
    identity_path: &[&str],
) {
    let mut hash_input = artifact.value.clone();
    remove_nested_field(&mut hash_input, identity_path);
    let content_hash = sha256_json(&hash_input);
    assert_eq!(
        identity_hash(&artifact.identity),
        content_hash,
        "artifact content hash must match identity for {}",
        artifact.path
    );
    assert_eq!(
        artifact.hash, content_hash,
        "artifact hash metadata must match content for {}",
        artifact.path
    );
}

fn remove_nested_field(value: &mut serde_json::Value, path: &[&str]) {
    let Some((field, parents)) = path.split_last() else {
        return;
    };
    let mut current = value;
    for parent in parents {
        let Some(next) = current.get_mut(*parent) else {
            return;
        };
        current = next;
    }
    if let Some(object) = current.as_object_mut() {
        object.remove(*field);
    }
}

fn identity_hash(identity: &str) -> &str {
    identity
        .rsplit_once(":sha256:")
        .map(|(_, hash)| hash)
        .expect("artifact identity must include sha256 hash")
}

fn sha256_json(value: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};

    hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical_json(value)).expect("canonical JSON should serialize"),
    ))
}

fn canonical_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let mut sorted = serde_json::Map::new();
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                sorted.insert(key.clone(), canonical_json(&object[key]));
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonical_json).collect())
        }
        _ => value.clone(),
    }
}

fn package_api_identity_service(name: &str, api_yml: &str) -> ServiceProjectBuilder {
    let temp = ServiceProjectBuilder::package_model(name, "import app", "return {}");
    temp.add_service_package_dependency("example.com/util", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/util",
        r#"
id: example.com/util
version: 0.1.0
"#,
    );
    write_package_api_yml(temp.root(), "example.com/util", api_yml);
    write_package_source(
        temp.root(),
        "example.com/util",
        "util_impl.skiff",
        r#"
          function ok() -> string {
            return "ok"
          }
        "#,
    );
    temp
}

fn protocol_scoped_service(name: &str, service_id: &str) -> ServiceProjectBuilder {
    ServiceProjectBuilder::new(name)
        .write_root_file(
            "service.yml",
            &format!(
                r#"
id: {service_id}
version: 1.0.0
"#
            ),
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
            function run(input: root.api.example.Input) -> root.api.example.Output {
              return {}
            }

            type ExampleService {}

            impl ExampleService {
              function run(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
                return root.internal.example.run(input)
              }
            }
        "#,
        )
}

fn write_service_handler_with_friend_test(
    root: &std::path::Path,
    default_run: &str,
    test_name: &str,
    test_assertion: &str,
) {
    std::fs::write(
        root.join("internal").join("example.skiff"),
        r#"
            function helper() -> number { return 1 }

            function run(input: root.api.example.Input) -> root.api.example.Output {
              return {}
            }

            type ExampleService {}

            impl ExampleService {
              function run(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
                return root.internal.example.run(input)
              }
            }
        "#,
    )
    .unwrap();
    std::fs::write(
        root.join("internal").join("example.test.skiff"),
        &format!(
            r#"
            test defaultRun {default_run}

            function helper() -> number {{ return 1 }}

            test "{test_name}" {{
              {test_assertion}
            }}
        "#
        ),
    )
    .unwrap();
}
