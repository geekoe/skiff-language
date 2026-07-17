use std::{collections::BTreeSet, fs};

mod common;
use common::artifacts::build_temp_service_publication;
use skiff_compiler::test_support::{
    compile_source_file_ir_artifact_for_test,
    project_fixtures::{
        write_package_api_yml, write_package_manifest, write_package_source, ServiceProjectBuilder,
    },
};
use skiff_compiler_emission::artifact::{PackageUnit, PublicationOperationKind};
use skiff_compiler_emission::package_test_artifacts::{
    build_package_test_artifacts, PackageTestArtifactBuildInput, PackageTestDependencyPackageInput,
    PackageTestEntrypointInput, PackageTestFileIrArtifact,
};

#[test]
fn package_test_reuses_the_rich_production_package_unit() {
    let project = rich_package_project();
    let published = build_temp_service_publication(project.root());

    let production_artifact = published
        .artifacts
        .package_units
        .iter()
        .find(|artifact| artifact.value["packageId"] == "example.com/agent")
        .expect("agent production PackageUnit");
    let production_unit: PackageUnit = serde_json::from_value(production_artifact.value.clone())
        .expect("production PackageUnit must deserialize");
    let production_files = package_files(&published, &production_unit);
    let production_resource_blobs = package_resources(&published, &production_unit);
    let projected_production_unit = without_emission_paths(production_unit.clone());

    assert_rich_package_facts(&production_unit);
    assert!(projected_production_unit
        .resources
        .iter()
        .all(|resource| resource.artifact_path.is_none()));
    assert_eq!(production_resource_blobs.len(), 1);
    assert_eq!(production_resource_blobs[0].bytes, b"agent prompt\n");

    let dependency_artifact = published
        .artifacts
        .package_units
        .iter()
        .find(|artifact| artifact.value["packageId"] == "example.com/base")
        .expect("base production PackageUnit");
    let dependency_unit: PackageUnit = serde_json::from_value(dependency_artifact.value.clone())
        .expect("dependency PackageUnit must deserialize");
    let dependency_files = package_files(&published, &dependency_unit);
    let dependency_resource_blobs = package_resources(&published, &dependency_unit);
    let projected_dependency_unit = without_emission_paths(dependency_unit.clone());

    let test_file = compile_source_file_ir_artifact_for_test(
        r#"
function packageTestEntry() -> string {
  return "ok"
}
"#,
        "agent.test.skiff",
        "agent.__test",
        "package-test",
    )
    .expect("package test File IR");
    let built = build_package_test_artifacts(PackageTestArtifactBuildInput {
        package_id: production_unit.package_id.clone(),
        package_version: production_unit.version.clone(),
        production_package_unit: projected_production_unit,
        package_test_config_and_effect_metadata: Default::default(),
        production_files,
        production_resource_blobs: production_resource_blobs.clone(),
        dependency_packages: vec![PackageTestDependencyPackageInput {
            package_id: dependency_unit.package_id.clone(),
            package_version: dependency_unit.version.clone(),
            production_files: dependency_files,
            production_resource_blobs: dependency_resource_blobs,
            package_unit: projected_dependency_unit,
        }],
        test_files: vec![PackageTestFileIrArtifact {
            source_path: test_file.source_path.clone(),
            module_path: test_file.module_path.clone(),
            file_ir: test_file.unit,
            explicit_const_type_annotations: BTreeSet::new(),
        }],
        entrypoints: vec![PackageTestEntrypointInput {
            display_name: "production parity".to_string(),
            source_path: "agent.test.skiff".to_string(),
            module_path: "agent.__test".to_string(),
            test_ordinal: 0,
            executable_index: 0,
            executable_local_id: "packageTestEntry".to_string(),
            symbol: Some("agent.__test.packageTestEntry".to_string()),
            default_run: true,
            config_and_effect_metadata: Default::default(),
        }],
    })
    .expect("package-test assembly must consume production projection");

    assert_eq!(built.production_package_unit.unit, production_unit);
    assert_eq!(
        built.production_package_unit.value,
        production_artifact.value
    );
    assert_eq!(
        built.production_package_unit.reference.build_identity,
        production_artifact.identity
    );
    assert_eq!(
        built.production_package_unit.unit_path,
        production_artifact.path
    );
    assert_eq!(
        built.production_package_unit.resource_blobs,
        production_resource_blobs
    );
    assert_eq!(built.dependency_package_units.len(), 1);
    assert_eq!(built.dependency_package_units[0].unit, dependency_unit);
    assert_eq!(built.assembly.assembly.dependency_package_units.len(), 1);
    assert_eq!(
        built.assembly.assembly.dependency_package_units[0].package_id,
        "example.com/base"
    );
}

fn without_emission_paths(mut unit: PackageUnit) -> PackageUnit {
    for file in &mut unit.files {
        file.artifact_path = None;
    }
    for resource in &mut unit.resources {
        resource.artifact_path = None;
    }
    unit
}

fn assert_rich_package_facts(unit: &PackageUnit) {
    assert!(unit
        .publication_abi
        .operation_exports
        .iter()
        .any(|operation| operation.kind == PublicationOperationKind::PublicFunction));
    assert!(unit
        .publication_abi
        .operation_exports
        .iter()
        .any(|operation| operation.kind == PublicationOperationKind::PublicInstanceMethod));
    assert!(!unit.publication_abi.public_instances.is_empty());
    assert!(!unit.publication_abi.schema_closure.is_empty());
    assert!(!unit.abi_identity_projection.public_symbols.is_empty());
    assert!(!unit.abi_identity_projection.type_nameability.is_empty());
    assert_eq!(unit.dependencies.len(), 1);
    assert_eq!(unit.dependencies[0].id, "example.com/base");
    assert!(!unit.config_and_effect_metadata.config.is_empty());
    assert_eq!(
        unit.config_and_effect_metadata.effects.operations.len(),
        unit.publication_abi.operation_exports.len()
    );
    assert!(!unit.resources.is_empty());
    assert!(unit
        .resources
        .iter()
        .all(|resource| resource.artifact_path.is_some()));
}

fn package_files(
    published: &skiff_compiler::BuiltServicePublication,
    unit: &PackageUnit,
) -> Vec<skiff_compiler::PublishedFileIrArtifact> {
    let identities = unit
        .files
        .iter()
        .map(|file| file.file_ir_identity.as_str())
        .collect::<BTreeSet<_>>();
    published
        .artifacts
        .package_file_ir_units
        .iter()
        .filter(|file| identities.contains(file.identity.as_str()))
        .cloned()
        .collect()
}

fn package_resources(
    published: &skiff_compiler::BuiltServicePublication,
    unit: &PackageUnit,
) -> Vec<skiff_compiler::PublishedResourceArtifact> {
    let paths = unit
        .resources
        .iter()
        .filter_map(|resource| resource.artifact_path.as_deref())
        .collect::<BTreeSet<_>>();
    published
        .artifacts
        .resource_blobs
        .iter()
        .filter(|resource| paths.contains(resource.artifact_path.as_str()))
        .cloned()
        .collect()
}

fn rich_package_project() -> ServiceProjectBuilder {
    let project = ServiceProjectBuilder::new("package-unit-single-path")
        .write_root_file(
            "service.yml",
            r#"
id: example.com/example
version: 1.0.0
packages:
  - id: example.com/agent
    version: 0.1.0
    alias: agent
"#,
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
import agent

type ExampleService {}

impl ExampleService {
  function run(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    const label = agent.label()
    return root.api.example.Output {}
  }
}
"#,
        );

    write_package_manifest(
        project.root(),
        "example.com/base",
        r#"
id: example.com/base
version: 0.1.0
"#,
    );
    write_package_api_yml(project.root(), "example.com/base", "suffix: base.suffix\n");
    write_package_source(
        project.root(),
        "example.com/base",
        "base.skiff",
        r#"
function suffix() -> string {
  return "base"
}
"#,
    );

    write_package_manifest(
        project.root(),
        "example.com/agent",
        r#"
id: example.com/agent
version: 0.1.0
packages:
  - id: example.com/base
    version: 0.1.0
    alias: base
resources:
  - prompts/agent.md
"#,
    );
    write_package_api_yml(
        project.root(),
        "example.com/agent",
        r#"
label: agent.label
User: agent.User
managed:
  const: root.agent.managed
  interfaces:
    - root.agent.Managed
"#,
    );
    write_package_source(
        project.root(),
        "example.com/agent",
        "agent.skiff",
        r#"
import base

type User {
  name: string
}

interface Managed {
  function send(input: string) -> string
}

type ManagedImpl implements Managed {}
const managed: ManagedImpl = ManagedImpl {}

impl ManagedImpl {
  function send(self: ManagedImpl, input: string) -> string {
    return input
  }
}

function label() -> string {
  const prefix = config.require<string>("agent.prefix")
  return prefix + base.suffix()
}
"#,
    );
    let resource_path = project
        .root()
        .join(".skiff-packages/example~com~~agent/0.1.0/prompts/agent.md");
    fs::create_dir_all(resource_path.parent().expect("resource parent")).unwrap();
    fs::write(resource_path, "agent prompt\n").unwrap();
    project
}
