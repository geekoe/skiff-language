use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryParameter, BoundaryReturn,
    BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractRequirement, ContractTypeRef,
};
use skiff_compiler::{
    ManifestOwner, ManifestProvenance, PackageContractCompileDependency, PackageSourceInput,
    PublicationManifest, PublicationSourceGraph, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText, SourceTree,
};

#[test]
fn package_source_uses_the_canonical_package_artifact_pipeline() {
    let manifest = PublicationManifest::new(
        "example.com/test-package"
            .parse()
            .expect("valid package id"),
        "1.0.0".to_string(),
        Default::default(),
        Vec::new(),
        ManifestProvenance::synthetic("package.yml", ManifestOwner::UserOrBuiltinPackage),
    );
    let package = PackageSourceInput::new(
        manifest,
        SourceTree {
            root: PathBuf::from("."),
            sources: Vec::new(),
        },
        PublicationSourceGraph::from_compiler_sources(Vec::new()),
        Vec::new(),
    );

    let published = skiff_test_runner::canonical_package::compile_package_artifact(
        &package,
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    )
    .expect("package source should compile directly to a canonical artifact");

    assert_eq!(published.artifact.package_id, "example.com/test-package");
    assert_eq!(published.artifact.package_version, "1.0.0");
}

#[test]
fn contract_dependent_package_compiles_from_source_and_a_code_free_contract() {
    let package_root = TestRoot::new("contract-dependent-package");
    fs::write(
        package_root.path().join("package.yml"),
        "id: example.com/contract-consumer\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(package_root.path().join("api.yml"), "run: main.run\n").unwrap();
    fs::write(
        package_root.path().join("main.skiff"),
        r#"function run(input: string) -> string {
  return payments/echo(input)
}
"#,
    )
    .unwrap();

    let operation_contract = BoundaryOperationContract {
        parameters: vec![BoundaryParameter {
            name: "input".to_string(),
            ty: ContractTypeRef::builtin("string"),
            value_plan: detached_plan(BoundaryValueOwner::Caller),
        }],
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("string"),
            value_plan: detached_plan(BoundaryValueOwner::Provider),
        },
        errors: BoundaryErrorContract::None,
        stream: BoundaryStreamContract::Unary,
        cancellation: BoundaryCancellationContract::NotCancellable,
        callbacks: BoundaryCallbackContract::None,
        may_suspend: false,
        effect_guarantee: BoundaryEffectGuarantee {
            detached_parameters: true,
            detached_return: true,
            detached_error: true,
            no_caller_reachable_mutation: true,
            no_caller_value_escape: true,
            no_same_heap_identity: true,
        },
    };
    let contract = skiff_compiler::compile_contract(ServiceContractDefinition {
        service_id: "example.com/payments".to_string(),
        contract_version: "1.0.0".to_string(),
        operations: BTreeMap::from([("echo".to_string(), operation_contract)]),
        boundary_schema: BTreeMap::new(),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "payments contract".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    })
    .expect("code-free contract");
    let operation = contract
        .operations
        .values()
        .next()
        .unwrap()
        .operation_id
        .clone();
    let requirement = ContractRequirement {
        alias: "payments".to_string(),
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    };
    let dependencies = BTreeMap::from([(
        (
            "example.com/contract-consumer".to_string(),
            "1.0.0".to_string(),
        ),
        vec![PackageContractCompileDependency {
            requirement: requirement.clone(),
            contract,
        }],
    )]);

    let project = skiff_test_runner::canonical_package::compile_package_project(
        package_root.path(),
        &[],
        &dependencies,
    )
    .expect("source package should compile from the canonical ServiceContract");

    assert!(project.dependency_packages.is_empty());
    assert_eq!(
        project.package.artifact.contract_requirements,
        vec![requirement]
    );
    assert_eq!(project.package.artifact.service_requirements.len(), 1);
    assert_eq!(project.package.artifact.service_call_refs.len(), 1);
    assert_eq!(
        project.package.artifact.service_call_refs[0].contract_operation_id,
        operation
    );
}

#[test]
fn package_test_overlay_is_a_separate_build_and_publishes_only_four_canonical_records() {
    let package_root = TestRoot::new("overlay-package");
    fs::write(
        package_root.path().join("package.yml"),
        "id: example.com/overlay-package\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        package_root.path().join("main.skiff"),
        "function helper() -> bool { return true }\n",
    )
    .unwrap();
    fs::write(
        package_root.path().join("main.test.skiff"),
        "test \"overlay executes\" { assert true }\n",
    )
    .unwrap();

    let project = skiff_test_runner::canonical_package::compile_package_project(
        package_root.path(),
        &[],
        &BTreeMap::new(),
    )
    .expect("production PackageArtifact");
    let production = skiff_artifact_identity::package_artifact_ref(&project.package.artifact)
        .expect("production ref");
    let cases = skiff_test_runner::canonical_fixture::discover_package_test_cases(
        package_root.path(),
        package_root.path(),
        false,
    )
    .expect("test cases");
    let overlay = skiff_test_runner::test_overlay::compile_package_test_overlay(
        package_root.path(),
        &project,
        &cases,
        &[],
    )
    .expect("test-only overlay PackageArtifact");

    assert_eq!(overlay.production, production);
    assert_eq!(overlay.overlay.artifact.package_id, production.package_id);
    assert_eq!(
        overlay.overlay.artifact.package_version,
        production.package_version
    );
    assert_ne!(
        overlay.overlay.artifact.package_build_id, production.package_build_id,
        "overlay must be an independent immutable build"
    );
    assert!(skiff_test_runner::canonical_fixture::CanonicalTestRecords::assert_production_package_unchanged(
        &production,
        &project.package,
    )
    .is_ok());
    assert!(skiff_test_runner::canonical_fixture::CanonicalTestRecords::assert_production_package_unchanged(
        &production,
        &overlay.overlay,
    )
    .is_err());

    let fixture =
        skiff_test_runner::canonical_fixture::assemble_package_test_fixture(&project, overlay)
            .expect("contract + deployment + RuntimeAssembly fixture");
    assert_eq!(fixture.production, production);
    assert_ne!(fixture.overlay, fixture.production);
    assert_eq!(fixture.records.contracts.len(), 1);
    assert_eq!(fixture.records.deployments.len(), 1);
    assert_eq!(fixture.entrypoints.len(), 1);
    assert_eq!(
        fixture.entrypoints[0].selector.host,
        "case-0.package-test.skiff.localhost"
    );

    let artifact_root = TestRoot::new("overlay-artifacts");
    let written = fixture
        .records
        .publish(artifact_root.path())
        .expect("immutable canonical fixture store");
    assert!(!written.is_empty());
    let output = read_tree(artifact_root.path());
    let retired_names = [
        ["package", "Unit"].concat(),
        ["service", "Unit"].concat(),
        ["service", "Assembly"].concat(),
        ["point", "er"].concat(),
        ["index", ".json"].concat(),
    ];
    for retired in retired_names {
        assert!(
            !output.contains(&retired),
            "canonical output tree retained {retired}"
        );
    }
}

#[test]
fn ecosystem_smoke_fixture_exposes_unary_stream_and_spawn_overlay_in_one_assembly() {
    let package_root = TestRoot::new("ecosystem-smoke-package");
    fs::write(
        package_root.path().join("package.yml"),
        "id: example.com/ecosystem-smoke\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        package_root.path().join("api.yml"),
        "marker: main.marker\nevents: main.events\n",
    )
    .unwrap();
    let pressure_chunk = "A".repeat(64 * 1024);
    let pressure_emits = std::iter::repeat_n("  emit(chunk)", 256)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        package_root.path().join("main.skiff"),
        format!(
            r#"import std

function marker() -> string {{ return "A" }}

function events() -> Stream<string> {{
  emit("A-start")
  const chunk = {pressure_chunk:?}
{pressure_emits}
  emit("A-end")
  return
}}
"#
        ),
    )
    .unwrap();
    fs::write(
        package_root.path().join("main.test.skiff"),
        r#"function typedSpawn(value: string) -> void { return }

test "spawn response" {
  spawn typedSpawn("A")
  assert true
}
"#,
    )
    .unwrap();

    let mut project = skiff_test_runner::canonical_package::compile_package_project(
        package_root.path(),
        &[],
        &BTreeMap::new(),
    )
    .expect("production PackageArtifact with unary and stream projections");
    skiff_test_runner::ecosystem_smoke_fixture::enable_ecosystem_smoke_server_stream(&mut project)
        .expect("narrow test-only stream projection bridge");
    let cases = skiff_test_runner::canonical_fixture::discover_package_test_cases(
        package_root.path(),
        package_root.path(),
        false,
    )
    .expect("spawn test case");
    let overlay = skiff_test_runner::test_overlay::compile_package_test_overlay(
        package_root.path(),
        &project,
        &cases,
        &[],
    )
    .expect("spawn test overlay");
    let fixture = skiff_test_runner::ecosystem_smoke_fixture::assemble_ecosystem_smoke_fixture(
        &project, overlay,
    )
    .expect("unary + stream + package test canonical assembly");

    assert_eq!(fixture.records.contracts.len(), 2);
    assert_eq!(fixture.records.deployments.len(), 2);
    assert_eq!(fixture.records.assembly.roots.len(), 2);
    assert_eq!(fixture.unary.selector.path, "/probe");
    assert_eq!(fixture.stream.selector.path, "/stream");
    assert_eq!(
        fixture.package_test.selector.path,
        "/__skiff/package-test/0"
    );
    assert_ne!(fixture.production, fixture.overlay);
}

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-test-runner-{label}-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read_tree(root: &Path) -> String {
    fn visit(root: &Path, path: &Path, output: &mut String) {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for entry in entries {
            if entry.is_dir() {
                visit(root, &entry, output);
            } else {
                output.push_str(entry.strip_prefix(root).unwrap().to_str().unwrap());
                output.push('\n');
                if entry
                    .extension()
                    .is_some_and(|extension| extension == "json")
                {
                    output.push_str(&fs::read_to_string(&entry).unwrap());
                }
            }
        }
    }
    let mut output = String::new();
    visit(root, root, &mut output);
    output
}

fn detached_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}
