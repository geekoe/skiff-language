use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::json;
use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryCallbackContract, BoundaryCancellationContract,
    BoundaryEffectGuarantee, BoundaryErrorContract, BoundaryOperationContract, BoundaryParameter,
    BoundaryReturn, BoundaryStreamContract, BoundaryUnavailableReason, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, ContractTypeRef,
    PackageArtifactRef, PackageLocalAbiSymbol, RuntimeAssemblyRef, ServiceContractRef,
    ServiceDeploymentRef,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    ManifestOwner, ManifestProvenance, PackageSourceInput, PublicationManifest,
    PublicationSourceGraph, ServiceContractDefinition, ServiceContractDefinitionDiagnosticText,
    SourceTree,
};
use skiff_deployment::storage::{CanonicalArtifactStore, ServiceContractPointer};
use skiff_test_runner::{
    canonical_fixture::{
        assemble_package_test_fixture, discover_package_test_cases, CanonicalBaseAssembly,
        CanonicalTestRecords,
    },
    canonical_package::compile_package_project,
    ecosystem_smoke_fixture::assemble_ecosystem_smoke_fixture,
    run_skiff_tests_with_options,
    test_overlay::compile_package_test_overlay,
    SkiffTestError, SkiffTestOptions,
};

#[test]
fn runner_cli_exposes_only_the_canonical_test_target() {
    let runner = env!("CARGO_BIN_EXE_skiff-test-runner");
    let help = Command::new(runner).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    for option in [
        "--artifact-root",
        "--base-assembly",
        "--live",
        "--activation-url",
        "--ingress-url",
        "--environment",
        "--expected-generation",
        "--deny-skips",
        "--require-tests",
    ] {
        assert!(help.contains(option), "help omitted {option}");
    }
    for retired in [
        "--profile",
        "--service-artifact-root",
        "--config",
        "--package-test-concurrency",
        "--router-reload-url",
        "--packages-dir",
        "--allow-network",
    ] {
        assert!(!help.contains(retired), "help retained {retired}");
        let output = Command::new(runner)
            .args(["input", retired])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr)
            .unwrap()
            .contains(&format!("unknown option {retired}")));
    }

    let output = Command::new(runner)
        .arg("input")
        .env("SKIFF_TEST_ARTIFACT_ROOT", "/retired/env-fallback")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("missing --artifact-root"));

    let sentinel = "direct-runner-url-secret";
    for (option, value, expected) in [
        (
            "--activation-url",
            format!("http://user:{sentinel}@127.0.0.1:4001/__skiff/activate-assembly"),
            "activation URL must point exactly",
        ),
        (
            "--ingress-url",
            format!("http://127.0.0.1:4000/nested?token={sentinel}"),
            "ingress URL must be an http:// origin",
        ),
    ] {
        let output = Command::new(runner)
            .args(["input", "--artifact-root", "/missing", option, &value])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(expected));
        assert!(!stderr.contains(sentinel));
    }
}

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
}

#[test]
fn contract_dependency_is_loaded_from_a_typed_pointer_and_record() {
    let root = TestRoot::new("contract-store");
    let artifacts = root.child("artifacts");
    let package = root.child("consumer");
    create_store(&artifacts);
    publish_contract(&artifacts);
    write_package(
        &package,
        r#"id: example.com/contract-consumer
version: 1.0.0
contracts:
  - alias: payments
    serviceId: example.com/payments
    contractVersion: 1.0.0
"#,
        Some("run: main.run\n"),
        Some(
            r#"function run(input: string) -> string {
  return payments/echo(input)
}
"#,
        ),
    );

    let project = compile_package_project(&package, &artifacts)
        .expect("contract must resolve from the canonical store");
    assert_eq!(project.contract_dependencies.len(), 1);
    assert_eq!(project.package.artifact.service_requirements.len(), 1);
    assert_eq!(project.package.artifact.service_call_refs.len(), 1);
}

#[test]
fn package_dependencies_use_exact_transitive_store_closure_and_ignore_dependency_source() {
    let root = TestRoot::new("transitive-store");
    let artifacts = root.child("artifacts");
    let runtime = root.child("runtime-artifacts");
    create_store(&artifacts);
    let leaf = root.child("leaf");
    write_package(&leaf, "id: example.com/leaf\nversion: 1.0.0\n", None, None);
    publish_package(&leaf, &artifacts);
    let helper = root.child("helper");
    write_package(
        &helper,
        r#"id: example.com/helper
version: 1.0.0
packages:
  - id: example.com/leaf
    version: 1.0.0
    alias: leaf
"#,
        None,
        None,
    );
    publish_package(&helper, &artifacts);

    let consumer = root.child("consumer");
    write_package(
        &consumer,
        r#"id: example.com/consumer
version: 1.0.0
packages:
  - id: example.com/helper
    version: 1.0.0
    alias: helper
"#,
        None,
        None,
    );
    let decoy = consumer.join(".skiff-packages/example.com/helper/1.0.0");
    fs::create_dir_all(&decoy).unwrap();
    fs::write(
        decoy.join("package.yml"),
        "this is not valid package source",
    )
    .unwrap();

    let project = compile_package_project(&consumer, &artifacts)
        .expect("only canonical dependency records should be consulted");
    assert_eq!(project.dependency_packages.len(), 2);
    assert!(project
        .dependency_packages
        .iter()
        .any(|package| package.package_id == "example.com/leaf"));

    fs::write(
        consumer.join("main.test.skiff"),
        "test \"transitive closure\" { assert true }\n",
    )
    .unwrap();
    let source_before_publish = read_tree(&artifacts);
    let cases = discover_package_test_cases(&consumer, &consumer, false).unwrap();
    let overlay = compile_package_test_overlay(&consumer, &project, &cases).unwrap();
    let fixture =
        assemble_package_test_fixture(&project, overlay, CanonicalBaseAssembly::default()).unwrap();
    fixture.records.publish(&artifacts, &runtime).unwrap();
    assert_eq!(read_tree(&artifacts), source_before_publish);
    let runtime_store = CanonicalArtifactStore::open(&runtime).unwrap();
    for dependency in &project.dependency_packages {
        let reference = skiff_artifact_identity::package_artifact_ref(dependency).unwrap();
        runtime_store
            .read_package_artifact(&reference)
            .expect("the exact transitive package closure must be copied into the runtime root");
    }

    let missing_store = root.child("missing-store");
    create_store(&missing_store);
    let error = compile_package_project(&consumer, &missing_store).unwrap_err();
    assert!(error.to_string().contains("no published canonical pointer"));
}

#[test]
fn official_platform_package_is_compiled_as_the_selected_source_root() {
    let root = TestRoot::new("platform-source");
    let artifacts = root.child("artifacts");
    let runtime = root.child("runtime-artifacts");
    create_store(&artifacts);

    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("std");
    let project = compile_package_project(&platform_root, &artifacts).unwrap();
    assert_eq!(project.package.artifact.package_id, "skiff.run/std");
    assert!(project.dependency_packages.is_empty());

    let test_file = platform_root.join("string.test.skiff");
    let cases = discover_package_test_cases(&test_file, &platform_root, true).unwrap();
    let overlay = compile_package_test_overlay(&platform_root, &project, &cases).unwrap();
    assert!(overlay.bindings.iter().all(|binding| matches!(
        overlay
            .overlay
            .artifact
            .boundary_projections
            .get(&binding.callable_id),
        Some(BoundaryCallableProjection::Available { .. })
    )));
    assert_eq!(overlay.production.package_id, "skiff.run/std");
    assert_eq!(overlay.bindings.len(), cases.len());
    let fixture =
        assemble_package_test_fixture(&project, overlay, CanonicalBaseAssembly::default()).unwrap();
    assert_eq!(fixture.entrypoints.len(), cases.len());
    fixture.records.publish(&artifacts, &runtime).unwrap();
}

#[test]
fn base_assembly_supplies_provider_selectors_and_real_owner_bindings() {
    let BaseAssemblyScenario {
        _root,
        artifacts,
        runtime,
        consumer,
        helper_package,
        payments_contract,
        provider_deployment,
        consumer_deployment,
        base_assembly_ref,
        base,
    } = create_base_assembly_scenario();

    let source_before_publish = read_tree(&artifacts);
    let project = compile_package_project(&consumer, &artifacts).unwrap();
    assert_eq!(project.package.artifact.package_requirements.len(), 1);
    assert_eq!(project.package.artifact.service_call_refs.len(), 1);
    let package_requirement = project
        .package
        .artifact
        .package_requirements
        .first()
        .expect("consumer helper requirement");
    let service_requirement = project
        .package
        .artifact
        .service_requirements
        .first()
        .expect("consumer service requirement");
    let cases = discover_package_test_cases(&consumer, &consumer, false).unwrap();
    let overlay = compile_package_test_overlay(&consumer, &project, &cases).unwrap();
    let fixture = assemble_package_test_fixture(&project, overlay, base).unwrap();
    let test_deployment = fixture
        .records
        .deployments
        .first()
        .expect("test-owned deployment");
    let production_deployment = CanonicalArtifactStore::open(&artifacts)
        .unwrap()
        .read_service_deployment(&consumer_deployment)
        .unwrap();
    let [base_config] = production_deployment.config_literals.as_slice() else {
        panic!("production deployment must own one exact base config literal")
    };
    assert_eq!(base_config.path, "app.token");
    assert_eq!(
        base_config.value,
        skiff_artifact_model::MetadataValue::String("owned-by-base".to_string())
    );
    assert_eq!(
        test_deployment.config_literals, production_deployment.config_literals,
        "test deployment must inherit the production deployment's typed config binding"
    );
    assert!(production_deployment
        .package_bindings
        .iter()
        .any(|binding| {
            binding.key.caller_package_build_id == project.package.artifact.package_build_id
                && binding.key.package_requirement_alias == package_requirement.alias
                && binding.package == helper_package
        }));
    assert!(test_deployment.package_bindings.iter().any(|binding| {
        binding.key.caller_package_build_id == fixture.overlay.package_build_id
            && binding.key.package_requirement_alias == package_requirement.alias
            && binding.package == helper_package
    }));
    assert!(test_deployment.service_selectors.iter().any(|selector| {
        selector.key.caller_package_build_id == fixture.overlay.package_build_id
            && selector.key.service_requirement_slot == service_requirement.service_binding_slot
            && selector.contract == payments_contract
    }));
    assert!(fixture
        .records
        .assembly
        .service_binding_templates
        .iter()
        .flat_map(|template| &template.bindings)
        .any(|binding| {
            binding.key.caller_package_build_id == fixture.overlay.package_build_id
                && binding.contract == payments_contract
                && binding.provider == provider_deployment
        }));
    fixture.records.publish(&artifacts, &runtime).unwrap();
    assert_eq!(read_tree(&artifacts), source_before_publish);
    let runtime_store = CanonicalArtifactStore::open(&runtime).unwrap();
    runtime_store
        .read_service_deployment(&provider_deployment)
        .expect("provider closure copied to runtime root");
    runtime_store
        .read_package_artifact(&helper_package)
        .expect("exact helper closure copied to runtime root");
    runtime_store
        .read_runtime_assembly(&base_assembly_ref)
        .expect("base assembly copied to runtime root");
}

#[test]
fn overlay_is_a_separate_build_and_external_store_remains_read_only() {
    let root = TestRoot::new("overlay");
    let artifacts = root.child("artifacts");
    let runtime = root.child("runtime-artifacts");
    let package = root.child("package");
    create_store(&artifacts);
    write_package(
        &package,
        "id: example.com/overlay-package\nversion: 1.0.0\n",
        None,
        Some("function helper() -> bool { return true }\n"),
    );
    fs::write(
        package.join("main.test.skiff"),
        "test \"overlay executes\" { assert true }\n",
    )
    .unwrap();
    let before = read_tree(&artifacts);
    let project = compile_package_project(&package, &artifacts).expect("production package");
    let production = skiff_artifact_identity::package_artifact_ref(&project.package.artifact)
        .expect("production ref");
    let cases = discover_package_test_cases(&package, &package, false).expect("test cases");
    let overlay = compile_package_test_overlay(&package, &project, &cases).expect("overlay");
    assert_ne!(
        overlay.overlay.artifact.package_build_id,
        production.package_build_id
    );
    assert!(CanonicalTestRecords::assert_production_package_unchanged(
        &production,
        &project.package,
    )
    .is_ok());
    let fixture =
        assemble_package_test_fixture(&project, overlay, CanonicalBaseAssembly::default())
            .expect("test deployment and assembly");
    fixture
        .records
        .publish(&artifacts, &runtime)
        .expect("isolated runtime publication");
    assert_eq!(
        read_tree(&artifacts),
        before,
        "external store must be read-only"
    );
    assert!(read_tree(&runtime).contains("runtime-assemblies"));
}

#[test]
fn non_live_runtime_root_cannot_be_nested_under_the_external_store() {
    let root = TestRoot::new("nested-runtime-root");
    let artifacts = root.child("artifacts");
    let runtime = artifacts.join("runtime-owned");
    let package = root.child("package");
    create_store(&artifacts);
    fs::create_dir_all(&runtime).unwrap();
    write_package(
        &package,
        "id: example.com/nested-runtime-root\nversion: 1.0.0\n",
        None,
        None,
    );
    fs::write(
        package.join("main.test.skiff"),
        "test \"nested runtime root\" { assert true }\n",
    )
    .unwrap();
    let before = read_tree(&artifacts);

    let error = run_skiff_tests_with_options(
        &package,
        &SkiffTestOptions {
            live: false,
            artifact_root: Some(artifacts.clone()),
            runtime_artifact_root: Some(runtime),
            base_assembly: None,
            activation_url: Some("http://127.0.0.1:9/__skiff/activate-assembly".to_string()),
            ingress_url: Some("http://127.0.0.1:9".to_string()),
            environment: "nested-runtime-root".to_string(),
            expected_generation: 0,
        },
    )
    .unwrap_err();
    assert!(matches!(error, SkiffTestError::MissingIsolatedRuntimeRoot));
    assert_eq!(read_tree(&artifacts), before);
}

#[test]
fn runtime_service_requirement_without_base_assembly_fails_before_activation() {
    let root = TestRoot::new("missing-base");
    let artifacts = root.child("artifacts");
    let package = root.child("consumer");
    create_store(&artifacts);
    publish_contract(&artifacts);
    write_package(
        &package,
        r#"id: example.com/base-required
version: 1.0.0
contracts:
  - alias: payments
    serviceId: example.com/payments
    contractVersion: 1.0.0
"#,
        Some("run: main.run\n"),
        Some("function run(input: string) -> string { return payments/echo(input) }\n"),
    );
    fs::write(
        package.join("main.test.skiff"),
        "test \"needs provider\" { assert true }\n",
    )
    .unwrap();
    let project = compile_package_project(&package, &artifacts).expect("consumer package");
    let cases = discover_package_test_cases(&package, &package, false).unwrap();
    let overlay = compile_package_test_overlay(&package, &project, &cases).unwrap();
    let error = assemble_package_test_fixture(&project, overlay, CanonicalBaseAssembly::default())
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("needs exactly one --base-assembly contract"));
}

fn assert_helper_mutation_semantics(
    project: &skiff_test_runner::canonical_package::CanonicalPackageProject,
    helper_package: &PackageArtifactRef,
) {
    let helper = project
        .dependency_packages
        .iter()
        .find(|package| package.package_id == helper_package.package_id)
        .expect("exact helper artifact in canonical dependency closure");
    assert_eq!(
        skiff_artifact_identity::package_artifact_ref(helper).unwrap(),
        *helper_package
    );
    let PackageLocalAbiSymbol::Callable {
        callable_id: mutate_id,
        ..
    } = helper
        .package_local_abi
        .public_symbols
        .get("tools.mutate")
        .expect("helper mutation public path")
    else {
        panic!("helper mutation must be callable")
    };
    assert_eq!(
        helper.callable_semantic_facts[mutate_id].effects,
        CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                writes_caller_reachable: true,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: false,
            }
        }
    );
    assert!(matches!(
        helper.callable_semantic_facts[mutate_id].provenance,
        CallableProvenanceSummary::Analyzed { .. }
    ));
    let BoundaryCallableProjection::Unavailable { reasons } =
        &helper.boundary_projections[mutate_id]
    else {
        panic!("mutating helper must remain unavailable at a detached boundary")
    };
    assert!(reasons.contains(&BoundaryUnavailableReason::WritesCallerReachable));
    assert!(!reasons.contains(&BoundaryUnavailableReason::UnknownEffect));
    assert!(!reasons.contains(&BoundaryUnavailableReason::UnknownCallTarget));
    assert!(!reasons.contains(&BoundaryUnavailableReason::RequiresSameHeapIdentity));
}

#[test]
fn fresh_helper_mutation_then_detached_service_call_projects_and_assembles() {
    let BaseAssemblyScenario {
        _root,
        artifacts,
        consumer,
        helper_package,
        payments_contract,
        provider_deployment,
        base,
        ..
    } = create_base_assembly_scenario();
    let project = compile_package_project(&consumer, &artifacts).unwrap();
    assert_helper_mutation_semantics(&project, &helper_package);

    let PackageLocalAbiSymbol::Callable { callable_id, .. } = project
        .package
        .artifact
        .package_local_abi
        .public_symbols
        .get("run")
        .expect("public helper-and-service consumer")
    else {
        panic!("run must be callable")
    };
    let consumer_projection = project
        .package
        .artifact
        .boundary_projections
        .get(callable_id)
        .expect("service-calling boundary projection");
    assert!(
        matches!(
            consumer_projection,
            BoundaryCallableProjection::Available { .. }
        ),
        "fresh helper mutation plus detached service call must project available, got {consumer_projection:?}"
    );

    let cases = discover_package_test_cases(&consumer, &consumer, false).unwrap();
    assert_eq!(cases.len(), 1);
    let overlay = compile_package_test_overlay(&consumer, &project, &cases).unwrap();
    assert!(matches!(
        overlay
            .overlay
            .artifact
            .boundary_projections
            .get(&overlay.bindings[0].callable_id)
            .expect("final business assertion boundary projection"),
        BoundaryCallableProjection::Available { .. }
    ));
    let fixture = assemble_package_test_fixture(&project, overlay, base).unwrap();
    assert!(fixture
        .records
        .assembly
        .service_binding_templates
        .iter()
        .flat_map(|template| &template.bindings)
        .any(|binding| {
            binding.key.caller_package_build_id == fixture.overlay.package_build_id
                && binding.contract == payments_contract
                && binding.provider == provider_deployment
        }));
    assert!(fixture
        .records
        .assembly
        .package_link_plan
        .package_links
        .iter()
        .any(|binding| {
            binding.key.caller_package_build_id == fixture.overlay.package_build_id
                && binding.package == helper_package
        }));
    assert_eq!(
        fixture.entrypoints[0].case.name,
        "provider observes helper mutation"
    );
}

#[test]
fn ecosystem_fixture_has_no_artifact_rewrite_or_synthetic_stream_bridge() {
    let root = TestRoot::new("smoke-unary");
    let artifacts = root.child("artifacts");
    let package = root.child("package");
    create_store(&artifacts);
    write_package(
        &package,
        "id: example.com/smoke\nversion: 1.0.0\n",
        Some("marker: main.marker\n"),
        Some("function marker() -> string { return \"A\" }\n"),
    );
    fs::write(
        package.join("main.test.skiff"),
        "test \"smoke\" { assert true }\n",
    )
    .unwrap();
    let project = compile_package_project(&package, &artifacts).unwrap();
    let production =
        skiff_artifact_identity::package_artifact_ref(&project.package.artifact).unwrap();
    let cases = discover_package_test_cases(&package, &package, false).unwrap();
    let overlay = compile_package_test_overlay(&package, &project, &cases).unwrap();
    let fixture = assemble_ecosystem_smoke_fixture(&project, overlay).unwrap();
    assert_eq!(fixture.production, production);
    assert_eq!(fixture.unary.selector.path, "/probe");
    assert_eq!(fixture.records.assembly.roots.len(), 2);
}

struct BaseAssemblyScenario {
    _root: TestRoot,
    artifacts: PathBuf,
    runtime: PathBuf,
    consumer: PathBuf,
    helper_package: PackageArtifactRef,
    payments_contract: ServiceContractRef,
    provider_deployment: ServiceDeploymentRef,
    consumer_deployment: ServiceDeploymentRef,
    base_assembly_ref: RuntimeAssemblyRef,
    base: CanonicalBaseAssembly,
}

fn create_base_assembly_scenario() -> BaseAssemblyScenario {
    let root = TestRoot::new("base-assembly");
    let artifacts = root.child("artifacts");
    let runtime = root.child("runtime-artifacts");
    create_store(&artifacts);
    let payments_contract = publish_contract(&artifacts);
    let consumer_contract = publish_contract_named(&artifacts, "example.com/consumer", "consumer");
    let helper_package = publish_base_helper(&root, &artifacts);
    let (_, provider_deployment) = publish_base_provider(&root, &artifacts, &payments_contract);
    let (consumer, consumer_deployment) = publish_base_consumer(
        &root,
        &artifacts,
        &helper_package,
        &payments_contract,
        &consumer_contract,
    );
    let base_assembly_ref = publish_assembly(
        &root.child("base-assembly"),
        &artifacts,
        "base-test",
        std::slice::from_ref(&consumer_deployment),
    );
    let base = CanonicalBaseAssembly::load(
        &artifacts,
        Some(base_assembly_ref.assembly_identity.as_str()),
    )
    .unwrap();
    assert!(base.deployments.iter().any(
        |deployment| skiff_artifact_identity::service_deployment_ref(deployment)
            == provider_deployment
    ));
    BaseAssemblyScenario {
        _root: root,
        artifacts,
        runtime,
        consumer,
        helper_package,
        payments_contract,
        provider_deployment,
        consumer_deployment,
        base_assembly_ref,
        base,
    }
}

fn publish_base_helper(root: &TestRoot, artifacts: &Path) -> PackageArtifactRef {
    let helper = root.child("helper");
    write_package(
        &helper,
        "id: example.com/helper\nversion: 1.0.0\n",
        Some("Box: main.Box\ntools:\n  mutate: main.mutate\n"),
        Some(
            r#"type Box { value: string }

function mutate(input: Box) -> void {
  input.value = "helper-mutated"
}
"#,
        ),
    );
    publish_package(&helper, artifacts)
}

fn publish_base_provider(
    root: &TestRoot,
    artifacts: &Path,
    payments_contract: &ServiceContractRef,
) -> (PackageArtifactRef, ServiceDeploymentRef) {
    let provider = root.child("provider");
    write_package(
        &provider,
        "id: example.com/provider\nversion: 1.0.0\n",
        Some("handle: main.handle\n"),
        Some(
            r#"function handle(input: string) -> string {
  if input == "helper-mutated" { return "provider-observed-helper-mutated" }
  return "provider-rejected-unmutated"
}
"#,
        ),
    );
    let provider_package = publish_package(&provider, artifacts);
    let provider_operation = contract_operation(artifacts, payments_contract);
    let provider_deployment = publish_deployment(
        &root.child("provider-deployment"),
        artifacts,
        json!({
            "schemaVersion": "skiff-service-deployment-input-v1",
            "contract": payments_contract,
            "deploymentRevision": "provider-r1",
            "implementation": provider_package,
            "operationBindings": [{
                "contractOperationId": provider_operation,
                "packagePublicPath": "handle"
            }],
            "packageBindings": [],
            "serviceSelectors": [],
            "ingress": [],
            "configLiterals": [],
            "secretRefs": [],
            "stateBindings": [],
            "resourceBindings": [],
            "runtimeCapabilityBindings": [],
            "policy": deployment_policy("service:provider"),
            "diagnosticText": { "displayName": "Provider", "notes": {} }
        }),
    );
    (provider_package, provider_deployment)
}

fn publish_base_consumer(
    root: &TestRoot,
    artifacts: &Path,
    helper_package: &PackageArtifactRef,
    payments_contract: &ServiceContractRef,
    consumer_contract: &ServiceContractRef,
) -> (PathBuf, ServiceDeploymentRef) {
    let consumer = root.child("consumer");
    write_package(
        &consumer,
        r#"id: example.com/consumer
version: 1.0.0
packages:
  - id: example.com/helper
    version: 1.0.0
    alias: helper
contracts:
  - alias: payments
    serviceId: example.com/payments
    contractVersion: 1.0.0
"#,
        Some("owner: main.owner\nrun: main.run\n"),
        Some(
            r#"import helper

type Box { value: string }

function owner(input: string) -> string { return "owner" }

function configured() -> string {
  return config.require<string>("app.token")
}

function run() -> string {
  const box = Box { value: "consumer" }
  helper/tools.mutate(box)
  return payments/echo(box.value)
}
"#,
        ),
    );
    fs::write(
        consumer.join("main.test.skiff"),
        r#"test "provider observes helper mutation" {
  assert root.main.run() == "provider-observed-helper-mutated"
}
"#,
    )
    .unwrap();
    let consumer_package = publish_package(&consumer, artifacts);
    let artifact = CanonicalArtifactStore::open(artifacts)
        .unwrap()
        .read_package_artifact(&consumer_package)
        .unwrap();
    let requirement = artifact
        .service_requirements
        .first()
        .expect("published consumer service requirement");
    let package_requirement = artifact
        .package_requirements
        .first()
        .expect("published consumer helper requirement");
    let consumer_operation = contract_operation(artifacts, consumer_contract);
    let consumer_deployment = publish_deployment(
        &root.child("consumer-deployment"),
        artifacts,
        json!({
            "schemaVersion": "skiff-service-deployment-input-v1",
            "contract": consumer_contract,
            "deploymentRevision": "consumer-r1",
            "implementation": consumer_package,
            "operationBindings": [{
                "contractOperationId": consumer_operation,
                "packagePublicPath": "owner"
            }],
            "packageBindings": [{
                "key": {
                    "callerPackageBuildId": consumer_package.package_build_id,
                    "packageRequirementAlias": package_requirement.alias
                },
                "package": helper_package
            }],
            "serviceSelectors": [{
                "key": {
                    "callerPackageBuildId": consumer_package.package_build_id,
                    "serviceRequirementSlot": requirement.service_binding_slot
                },
                "contract": payments_contract
            }],
            "ingress": [],
            "configLiterals": [{ "path": "app.token", "value": "owned-by-base" }],
            "secretRefs": [],
            "stateBindings": [],
            "resourceBindings": [],
            "runtimeCapabilityBindings": [],
            "policy": deployment_policy("service:consumer"),
            "diagnosticText": { "displayName": "Consumer", "notes": {} }
        }),
    );
    (consumer, consumer_deployment)
}

fn publish_package(root: &Path, artifacts: &Path) -> PackageArtifactRef {
    let output = build_authoring_object(AuthoringObject::Package, root, artifacts, true)
        .expect("production package authoring should publish pointer and records");
    serde_json::from_value(output["packageArtifactReceipt"]["artifact"].clone())
        .expect("typed package authoring receipt")
}

fn publish_contract(artifacts: &Path) -> ServiceContractRef {
    publish_contract_named(artifacts, "example.com/payments", "payments")
}

fn publish_contract_named(
    artifacts: &Path,
    service_id: &str,
    diagnostic_name: &str,
) -> ServiceContractRef {
    let contract = skiff_compiler::compile_contract(ServiceContractDefinition {
        service_id: service_id.to_string(),
        contract_version: "1.0.0".to_string(),
        operations: BTreeMap::from([("echo".to_string(), string_operation())]),
        boundary_schema: BTreeMap::new(),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: diagnostic_name.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    })
    .unwrap();
    let store = CanonicalArtifactStore::open(artifacts).unwrap();
    store.write_service_contract(&contract).unwrap();
    let reference = skiff_artifact_identity::service_contract_ref(&contract).unwrap();
    let pointer = ServiceContractPointer::new(reference.clone()).unwrap();
    store
        .compare_and_swap_service_contract_pointer(None, &pointer)
        .unwrap();
    reference
}

fn contract_operation(
    artifacts: &Path,
    contract: &ServiceContractRef,
) -> skiff_artifact_model::ContractOperationId {
    CanonicalArtifactStore::open(artifacts)
        .unwrap()
        .read_service_contract(contract)
        .unwrap()
        .operations
        .keys()
        .next()
        .expect("contract operation")
        .clone()
}

fn publish_deployment(
    root: &Path,
    artifacts: &Path,
    deployment: serde_json::Value,
) -> ServiceDeploymentRef {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("deployment.yml"),
        serde_json::to_string_pretty(&deployment).unwrap(),
    )
    .unwrap();
    let output = build_authoring_object(AuthoringObject::Deployment, root, artifacts, true)
        .expect("production deployment authoring");
    serde_json::from_value(output["serviceDeploymentReceipt"]["deployment"].clone())
        .expect("typed deployment authoring receipt")
}

fn publish_assembly(
    root: &Path,
    artifacts: &Path,
    environment: &str,
    roots: &[ServiceDeploymentRef],
) -> RuntimeAssemblyRef {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("assembly.yml"),
        serde_json::to_string_pretty(&json!({
            "environment": environment,
            "rootDeployments": roots
        }))
        .unwrap(),
    )
    .unwrap();
    let output = build_authoring_object(AuthoringObject::Assembly, root, artifacts, true)
        .expect("production assembly authoring");
    serde_json::from_value(output["runtimeAssemblyReceipt"]["assembly"].clone())
        .expect("typed assembly authoring receipt")
}

fn deployment_policy(principal: &str) -> serde_json::Value {
    json!({
        "timeoutMs": 1_000,
        "resources": { "cpuMillis": 100, "memoryBytes": 1_048_576 },
        "activation": { "maxConcurrency": 1, "idleTimeoutMs": null },
        "principal": principal
    })
}

fn string_operation() -> BoundaryOperationContract {
    BoundaryOperationContract {
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
    }
}

fn detached_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn write_package(root: &Path, manifest: &str, api: Option<&str>, source: Option<&str>) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("package.yml"), manifest).unwrap();
    if let Some(api) = api {
        fs::write(root.join("api.yml"), api).unwrap();
    }
    if let Some(source) = source {
        fs::write(root.join("main.skiff"), source).unwrap();
    }
}

fn create_store(path: &Path) {
    CanonicalArtifactStore::create(path).unwrap();
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

    fn child(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read_tree(root: &Path) -> String {
    fn visit(root: &Path, path: &Path, output: &mut String) {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        let mut entries = entries
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for entry in entries {
            if entry.is_dir() {
                visit(root, &entry, output);
            } else {
                output.push_str(entry.strip_prefix(root).unwrap().to_str().unwrap());
                output.push('\n');
                output.push_str(&fs::read_to_string(&entry).unwrap_or_default());
            }
        }
    }
    let mut output = String::new();
    visit(root, root, &mut output);
    output
}
