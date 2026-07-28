// These end-to-end contract scenarios intentionally keep each setup, mutation
// matrix, and assertion receipt together so failures retain their full owner
// context.
#![allow(clippy::too_many_lines)]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use skiff_artifact_identity::{service_contract_ref, service_deployment_ref};
use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryUnavailableReason, CallableEffectSummary,
    CallableMayEffects, CallableProvenanceSummary, GatewayAdapterKind, GatewayAdapterSource,
    GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey, GatewayExternalSchema,
    GatewayProtocolSurface, IngressProtocol, PackageArtifact, PackageArtifactRef,
    PackageLocalAbiSymbol, RuntimeAssemblyRef, ServiceAuthoringKind, ServiceContract,
    ServiceContractRef, ServiceDeployment, ServiceDeploymentRef, StateBindingKind,
};
use skiff_compiler::{
    authoring::{build_authoring_object, publish_package_artifact_records, AuthoringObject},
    compile_service_package, generate_service_deployment, CompilerPlatformSources,
    GeneratedServiceDeploymentInput, ManifestOwner, ManifestProvenance, PackageCompileInput,
    PackageSourceInput, PublicationManifest, PublicationSourceGraph, SourceTree,
};
use skiff_compiler_input::{
    package_config::read_user_package_manifest, package_sources::read_package_sources,
    read_publication_resources, read_service_package_root,
};
use skiff_deployment::{assembly::resolve_runtime_assembly, storage::CanonicalArtifactStore};
use skiff_test_runner::{
    canonical_fixture::{
        assemble_package_test_fixture, discover_package_test_cases, CanonicalBaseAssembly,
        CanonicalTestRecords,
    },
    canonical_package::{compile_package_project, compile_package_project_for_test},
    canonical_std_seed::seed_canonical_std,
    ecosystem_smoke_fixture::assemble_ecosystem_smoke_fixture,
    package_service_host_fixture::{
        prepare_package_service_host_fixture, PACKAGE_SERVICE_HOST_FIXTURE_SCHEMA_VERSION,
    },
    package_test_assembly::{assemble_package_test_fixture_for_run, CanonicalPackageTestFixture},
    run_skiff_tests_with_options,
    test_overlay::compile_package_test_overlay,
    SkiffTestError, SkiffTestOptions,
};

// Explicit identity regressions refreshed with the current canonical std
// source. Production code derives these identities from the F27A authoring
// receipt rather than these test pins.
const EXPECTED_PRELUDE_IDENTITY: &str =
    "skiff-prelude-v1:sha256:5166ba3c306e94624094e0736da821a1b653da5aace1ef8cee2fb654f4106699";
const EXPECTED_STD_PACKAGE_BUILD_ID: &str =
    "skiff-package-build-v10:sha256:0dec996a2d6388245539fb000a0284a1561dc21ac3cc6e88ed3fbe0eadfe3d43";
const EXPECTED_RAW_HTTP_UNARY_GATEWAY_IDENTITY: &str =
    "skiff-gateway-entry-v2:sha256:0fd289d7eec4e03b01e9e8f5633aedd7e1cc64158fa7932f99a9686e559c02f2";
const EXPECTED_RAW_HTTP_STREAM_GATEWAY_IDENTITY: &str =
    "skiff-gateway-entry-v2:sha256:1aef41f397b7c817110cb0cc74a7b472ba9732c5ac6bcfe6e219e3ac51ab6bd0";

#[test]
fn platform_source_context_contract() {
    let runner = env!("CARGO_BIN_EXE_skiff-test-runner");
    let help = Command::new(runner).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    for option in [
        "--artifact-root",
        "--platform-source-root",
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
        "--test-config-literals",
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

    let fixture = env!("CARGO_BIN_EXE_skiff-package-service-smoke-fixture");
    assert_platform_context_rejections(runner, fixture, &platform_source_root());

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
fn runtime_target_environment_cli_and_non_live_harness_rules_are_unchanged() {
    let root = TestRoot::new("target-environment-cli");
    let missing_input = root.child("missing-input");
    let artifact_root = root.child("artifacts");
    let runner = env!("CARGO_BIN_EXE_skiff-test-runner");
    let platform_root = platform_source_root();
    let common_args = [
        missing_input.as_os_str(),
        std::ffi::OsStr::new("--artifact-root"),
        artifact_root.as_os_str(),
        std::ffi::OsStr::new("--platform-source-root"),
        platform_root.as_os_str(),
    ];
    let live_target_args = [
        "--live",
        "--activation-url",
        "http://127.0.0.1:9/__skiff/activate-assembly",
        "--ingress-url",
        "http://127.0.0.1:9",
        "--expected-generation",
        "0",
    ];

    let missing_live_environment = Command::new(runner)
        .args(common_args)
        .args(live_target_args)
        .output()
        .unwrap();
    assert!(!missing_live_environment.status.success());
    assert!(String::from_utf8(missing_live_environment.stderr)
        .unwrap()
        .contains("--live requires --activation-url, --ingress-url, --environment"));

    let explicit_live_target = Command::new(runner)
        .args(common_args)
        .args(live_target_args)
        .args(["--environment", "dev"])
        .output()
        .unwrap();
    assert!(!explicit_live_target.status.success());
    assert!(String::from_utf8(explicit_live_target.stderr)
        .unwrap()
        .contains("failed to inspect input"));

    let forbidden_non_live_cli_target = Command::new(runner)
        .args(common_args)
        .args(["--environment", "dev"])
        .output()
        .unwrap();
    assert!(!forbidden_non_live_cli_target.status.success());
    assert!(String::from_utf8(forbidden_non_live_cli_target.stderr)
        .unwrap()
        .contains("non-live targets are supplied only by the isolated runtime harness"));

    let harness_target = Command::new(runner)
        .args(common_args)
        .env("SKIFF_TEST_ENVIRONMENT", "dev")
        .output()
        .unwrap();
    assert!(!harness_target.status.success());
    assert!(String::from_utf8(harness_target.stderr)
        .unwrap()
        .contains("failed to inspect input"));
}

fn assert_platform_context_rejections(runner: &str, fixture: &str, platform_root: &Path) {
    let cases = [
        (vec![], "missing --platform-source-root"),
        (
            vec![
                "--platform-source-root".to_string(),
                platform_root.display().to_string(),
                "--platform-source-root".to_string(),
                platform_root.display().to_string(),
            ],
            "--platform-source-root was provided more than once",
        ),
        (
            vec![
                "--platform-source-root".to_string(),
                "relative/platform".to_string(),
            ],
            "compiler platform source root must be absolute",
        ),
        (
            vec![
                "--platform-source-root".to_string(),
                platform_root
                    .join("missing-platform-root")
                    .display()
                    .to_string(),
            ],
            "failed to inspect compiler platform source path",
        ),
    ];
    for (platform_args, expected) in cases {
        let runner_output = Command::new(runner)
            .arg(platform_root.join("std"))
            .args(["--artifact-root", "/missing"])
            .args(&platform_args)
            .output()
            .unwrap();
        assert!(!runner_output.status.success());
        assert!(String::from_utf8(runner_output.stderr)
            .unwrap()
            .contains(expected));

        let fixture_output = Command::new(fixture)
            .args([
                "--bootstrap-only",
                "--artifact-root",
                "/missing",
                "--environment",
                "context-contract",
            ])
            .args(&platform_args)
            .output()
            .unwrap();
        assert!(!fixture_output.status.success());
        assert!(String::from_utf8(fixture_output.stderr)
            .unwrap()
            .contains(expected));
    }
}

#[test]
fn host_fixture_cli_rejects_ambiguous_prepare_modes() {
    let binary = env!("CARGO_BIN_EXE_skiff-package-service-smoke-fixture");
    for (args, expected) in [
        (
            vec![
                "consumer",
                "--prepare-host-base",
                "fixture",
                "--work-root",
                "work",
                "--receipt",
                "receipt.json",
            ],
            "--prepare-host-base is mutually exclusive",
        ),
        (
            vec!["--work-root", "work", "--receipt", "receipt.json"],
            "--work-root and --receipt require --prepare-host-base",
        ),
        (
            vec!["--prepare-host-base", "fixture", "--work-root", "work"],
            "--prepare-host-base requires --work-root and --receipt",
        ),
    ] {
        let output = Command::new(binary)
            .args(args)
            .args(["--artifact-root", "artifacts", "--environment", "host-test"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr).unwrap().contains(expected));
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
        &platform_sources(),
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
services:
  - id: example.com/payments
    version: 1.0.0
    alias: payments
"#,
        Some("run: main.run\n"),
        Some(
            r#"function run(input: string) -> string {
  return payments/echo(input)
}
"#,
        ),
    );

    let project = compile_package_project(&platform_sources(), &package, &artifacts)
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
    write_package(
        &leaf,
        "id: example.com/leaf\nversion: 1.0.0\nstate:\n  leaf-db:\n    kind: database\n",
        None,
        Some("type LeafRecord { id: string }\ndb object LeafRecord { primary key(id) }\n"),
    );
    publish_package(&leaf, &artifacts);
    let helper = root.child("helper");
    write_package(
        &helper,
        r#"id: example.com/helper
version: 1.0.0
state:
  helper-db:
    kind: database
packages:
  - id: example.com/leaf
    version: 1.0.0
    alias: leaf
"#,
        Some("run: main.run\n"),
        Some(
            "function run(input: string) -> string { return input }\n\
             type HelperRecord { id: string }\n\
             db object HelperRecord { primary key(id) }\n",
        ),
    );
    publish_package(&helper, &artifacts);

    let consumer = root.child("consumer");
    write_package(
        &consumer,
        r#"id: example.com/consumer
version: 1.0.0
state:
  consumer-db:
    kind: database
packages:
  - id: example.com/helper
    version: 1.0.0
    alias: helper
"#,
        None,
        Some(
            "type ConsumerRecord { id: string }\n\
             db object ConsumerRecord { primary key(id) }\n",
        ),
    );
    let decoy = consumer.join(".skiff-packages/example.com/helper/1.0.0");
    fs::create_dir_all(&decoy).unwrap();
    fs::write(
        decoy.join("package.yml"),
        "this is not valid package source",
    )
    .unwrap();

    let project = compile_package_project(&platform_sources(), &consumer, &artifacts)
        .expect("only canonical dependency records should be consulted");
    assert_eq!(project.dependency_packages.len(), 2);
    assert!(project
        .dependency_packages
        .iter()
        .any(|package| package.package_id == "example.com/leaf"));

    fs::write(
        consumer.join("main.test.skiff"),
        "test \"transitive closure\" effects {\n\
           helper/run { expect: \"input\", respond: \"mock\" }\n\
         } { assert helper/run(\"input\") == \"mock\" }\n",
    )
    .unwrap();
    let source_before_publish = read_tree(&artifacts);
    let cases = discover_package_test_cases(&consumer, &consumer, false).unwrap();
    let overlay =
        compile_package_test_overlay(&platform_sources(), &consumer, &artifacts, &project, &cases)
            .unwrap();
    let fixture =
        assemble_package_test_fixture(&project, overlay, CanonicalBaseAssembly::default()).unwrap();
    let [deployment] = fixture.records.deployments.as_slice() else {
        panic!("one case must produce one deployment")
    };
    assert_eq!(
        deployment
            .state_bindings
            .iter()
            .map(|binding| binding.requirement_key.as_str())
            .collect::<Vec<_>>(),
        ["consumer-db", "helper-db", "leaf-db"]
    );
    assert_eq!(
        deployment
            .state_bindings
            .iter()
            .map(|binding| binding.namespace.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        1,
        "all cross-package database calls in one case must share the caller case namespace"
    );
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
    let error =
        compile_package_project(&platform_sources(), &consumer, &missing_store).unwrap_err();
    assert!(error.to_string().contains("no published canonical pointer"));
}

#[test]
fn official_platform_package_is_compiled_as_the_selected_source_root() {
    let root = TestRoot::new("platform-source");
    let artifacts = root.child("artifacts");
    create_store(&artifacts);

    let platform_sources = platform_sources();
    let platform_root = platform_sources.std_dir().to_path_buf();
    let project = compile_package_project(&platform_sources, &platform_root, &artifacts).unwrap();
    assert_eq!(project.package.artifact.package_id, "skiff.run/std");
    assert!(project.dependency_packages.is_empty());
    assert!(
        discover_package_test_cases(&platform_root, &platform_root, false)
            .unwrap()
            .is_empty()
    );

    let fake_root = root.child("fake-reserved");
    fs::create_dir_all(&fake_root).unwrap();
    fs::copy(
        platform_root.join("package.yml"),
        fake_root.join("package.yml"),
    )
    .unwrap();
    fs::copy(platform_root.join("api.yml"), fake_root.join("api.yml")).unwrap();
    let error = compile_package_project(&platform_sources, &fake_root, &artifacts).unwrap_err();
    assert!(error
        .to_string()
        .contains("package id skiff.run/std is reserved"));
}

#[test]
fn std_test_service_overlay_uses_its_exact_compiler_owned_std_closure() {
    let root = TestRoot::new("std-test-service");
    let artifacts = root.child("artifacts");
    create_store(&artifacts);
    let platform_sources = platform_sources();
    let std = seed_canonical_std(&platform_sources, &artifacts).unwrap();
    let test_service = platform_source_root().join("test-services/std");
    let project =
        compile_package_project_for_test(&platform_sources, &test_service, &artifacts).unwrap();
    assert!(
        project.dependency_packages.is_empty(),
        "the empty production test service must not gain a synthetic std dependency"
    );

    let cases = discover_package_test_cases(&test_service, &test_service, false).unwrap();
    assert_eq!(
        cases.len(),
        11,
        "the migrated std test service must stay complete"
    );
    let overlay = compile_package_test_overlay(
        &platform_sources,
        &test_service,
        &artifacts,
        &project,
        &cases,
    )
    .unwrap();
    assert_eq!(overlay.dependency_packages.len(), 1);
    assert_eq!(
        overlay.dependency_packages[0].package_build_id,
        std.package.artifact.package_build_id
    );
    assert!(overlay.bindings.iter().all(|binding| matches!(
        overlay
            .overlay
            .artifact
            .boundary_projections
            .get(&binding.callable_id),
        Some(BoundaryCallableProjection::Available { .. })
    )));

    let fixture =
        assemble_package_test_fixture(&project, overlay, CanonicalBaseAssembly::default()).unwrap();
    assert_eq!(fixture.entrypoints.len(), cases.len());
    assert!(fixture
        .records
        .assembly
        .resolved_packages
        .contains(&std.package.artifact));
}

#[test]
#[ignore = "I16/G16 shared-target identity probe only"]
fn platform_source_identity_probe() {
    let root = std::env::var_os("SKIFF_TEST_PLATFORM_SOURCE_ROOT")
        .map(PathBuf::from)
        .expect("SKIFF_TEST_PLATFORM_SOURCE_ROOT is required by the ignored identity probe");
    let platform_sources = CompilerPlatformSources::new(&root).unwrap();
    skiff_compiler_source::prelude_registry::initialize_prelude_registry(&platform_sources)
        .unwrap();

    let temp = TestRoot::new("platform-identity-probe");
    let artifacts = temp.child("artifacts");
    create_store(&artifacts);
    let project =
        compile_package_project(&platform_sources, platform_sources.std_dir(), &artifacts).unwrap();
    let prelude_identity = skiff_compiler_source::prelude_registry::prelude_identity();
    let std_package_build_id = project.package.artifact.package_build_id.as_str();
    assert_eq!(prelude_identity, EXPECTED_PRELUDE_IDENTITY);
    assert_eq!(std_package_build_id, EXPECTED_STD_PACKAGE_BUILD_ID);
    println!("PLATFORM_SOURCE_PRELUDE_IDENTITY={prelude_identity}");
    println!("PLATFORM_SOURCE_STD_PACKAGE_BUILD_ID={std_package_build_id}");
}

#[test]
fn base_assembly_supplies_provider_selectors_and_real_owner_bindings() {
    let BaseAssemblyScenario {
        _root,
        artifacts,
        runtime,
        consumer: _,
        test_service,
        helper_package,
        payments_contract,
        provider_deployment,
        consumer_deployment,
        base_assembly_ref,
        base,
    } = create_base_assembly_scenario();

    let source_before_publish = read_tree(&artifacts);
    let project =
        compile_package_project_for_test(&platform_sources(), &test_service, &artifacts).unwrap();
    let subject = project
        .dependency_packages
        .iter()
        .find(|package| package.package_id == "example.com/consumer")
        .expect("exact subject package");
    let package_requirement = subject
        .package_requirements
        .first()
        .expect("consumer helper requirement");
    let service_requirement = subject
        .service_requirements
        .first()
        .expect("consumer service requirement");
    let cases = discover_package_test_cases(&test_service, &test_service, false).unwrap();
    let overlay = compile_package_test_overlay(
        &platform_sources(),
        &test_service,
        &artifacts,
        &project,
        &cases,
    )
    .unwrap();
    let fixture = assemble_package_test_fixture(&project, overlay, base).unwrap();
    let test_deployment = fixture
        .records
        .deployments
        .first()
        .expect("test-owned deployment");
    let subject_requirements = project
        .package
        .artifact
        .package_requirements
        .iter()
        .filter(|requirement| requirement.package_id == "example.com/consumer")
        .collect::<Vec<_>>();
    let [subject_requirement] = subject_requirements.as_slice() else {
        panic!(
            "public alias plus topLevelAlias must produce exactly one subject requirement, found {}",
            subject_requirements.len()
        )
    };
    assert_eq!(subject_requirement.alias, "subject");
    assert_eq!(
        subject_requirement.expected_package_build.as_ref(),
        Some(&subject.package_build_id)
    );
    let subject_bindings = test_deployment
        .package_bindings
        .iter()
        .filter(|binding| binding.key.package_requirement_alias == "subject")
        .collect::<Vec<_>>();
    let [subject_binding] = subject_bindings.as_slice() else {
        panic!(
            "the second local alias must leave exactly one subject binding and collection projection, found {}",
            subject_bindings.len()
        )
    };
    assert_eq!(
        subject_binding.package,
        skiff_artifact_identity::package_artifact_ref(subject).unwrap()
    );
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
        test_deployment.config_literals,
        vec![skiff_artifact_model::ConfigLiteralBinding {
            path: "app.token".to_string(),
            value: skiff_artifact_model::MetadataValue::String("owned-by-test-service".to_string()),
        }],
        "the independent test service must own its ordinary profile config"
    );
    assert!(production_deployment
        .package_bindings
        .iter()
        .any(|binding| {
            binding.key.caller_package_build_id == subject.package_build_id
                && binding.key.package_requirement_alias == package_requirement.alias
                && binding.package == helper_package
        }));
    assert!(test_deployment.package_bindings.iter().any(|binding| {
        binding.key.caller_package_build_id == subject.package_build_id
            && binding.key.package_requirement_alias == package_requirement.alias
            && binding.package == helper_package
    }));
    assert!(test_deployment.service_selectors.iter().any(|selector| {
        selector.key.caller_package_build_id == subject.package_build_id
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
            binding.key.caller_package_build_id == subject.package_build_id
                && binding.contract == payments_contract
                && binding.provider == provider_deployment
        }));
    assert!(
        fixture
            .records
            .assembly
            .resolved_deployments
            .contains(&provider_deployment),
        "the selected dependency provider remains reachable from the test roots"
    );
    assert!(
        !fixture
            .records
            .assembly
            .resolved_deployments
            .contains(&consumer_deployment),
        "the production subject is a binding/config source, not a second test root"
    );
    assert!(
        fixture
            .records
            .assembly
            .resolved_packages
            .iter()
            .all(|package| package != &fixture.production),
        "the test overlay replaces the production subject in the execution closure"
    );
    fixture.records.publish(&artifacts, &runtime).unwrap();
    assert_eq!(read_tree(&artifacts), source_before_publish);
    let runtime_store = CanonicalArtifactStore::open(&runtime).unwrap();
    runtime_store
        .read_service_deployment(&provider_deployment)
        .expect("provider closure copied to runtime root");
    let helper = runtime_store
        .read_package_artifact(&helper_package)
        .expect("exact helper closure copied to runtime root");
    let helper_schema = runtime_store
        .resolve_package_artifact_schema(&helper)
        .expect("helper schema index and exact type-record closure copied to runtime root");
    assert_eq!(
        helper_schema.records.len(),
        helper.package_schema_type_records.len()
    );
    assert!(
        !helper_schema.records.is_empty(),
        "the host helper fixture must exercise copied schema records"
    );
    runtime_store
        .read_runtime_assembly(&base_assembly_ref)
        .expect("base assembly copied to runtime root");
}

#[test]
fn inline_effect_sequence_rejects_common_step_and_outcome_type_mismatches() {
    let scenario = create_base_assembly_scenario();
    let project =
        compile_package_project(&platform_sources(), &scenario.consumer, &scenario.artifacts)
            .unwrap();
    let cases = [
        (
            r#"
test "invalid common expect" effects {
  helper/tools.lookup {
    expect: { method: 7 },
    respond: helper.EffectResponse { value: "ok" },
  }
} { assert true }
"#,
            "test effect expect subset",
        ),
        (
            r#"
test "unknown common expect field" effects {
  helper/tools.lookup {
    expect: { missing: "not-a-request-field" },
    respond: helper.EffectResponse { value: "ok" },
  }
} { assert true }
"#,
            "unknown request field `missing`",
        ),
        (
            r#"
test "invalid step expect" effects {
  helper/tools.lookup {
    sequence: [{
      expect: { url: 7 },
      respond: helper.EffectResponse { value: "ok" },
    }],
  }
} { assert true }
"#,
            "test effect expect subset",
        ),
        (
            r#"
test "invalid response" effects {
  helper/tools.lookup {
    respond: { value: 7 },
  }
} { assert true }
"#,
            "test effect respond",
        ),
        (
            r#"
test "invalid stream event" effects {
  helper/tools.events {
    sequence: [{
      stream: [{ value: 7 }],
    }],
  }
} { assert true }
"#,
            "test effect stream event",
        ),
        (
            r#"
test "unary target cannot use stream outcome" effects {
  helper/tools.lookup {
    sequence: [{
      stream: [helper.EffectResponse { value: "event" }],
    }],
  }
} { assert true }
"#,
            "stream requires Stream<T> return",
        ),
        (
            r#"
test "stream target cannot use unary response outcome" effects {
  helper/tools.events {
    sequence: [{
      respond: helper/tools.events(helper.EffectRequest {
        method: "GET",
        url: "https://example.test/not-a-response",
        detail: "type-correct-stream-value",
      }),
    }],
  }
} { assert true }
"#,
            "cannot use respond for a direct Stream<T> target",
        ),
        (
            r#"
test "undeclared throw" effects {
  helper/tools.events {
    sequence: [{
      throw: "not-a-nominal-error",
    }],
  }
} { assert true }
"#,
            "throw has invalid catch payload",
        ),
    ];

    for (index, (source, expected)) in cases.into_iter().enumerate() {
        let package = scenario
            ._root
            .child(&format!("invalid-inline-effect-{index}"));
        copy_tree(&scenario.consumer, &package);
        fs::write(package.join("main.test.skiff"), source).unwrap();
        let discovered = discover_package_test_cases(&package, &package, false).unwrap();
        let error = compile_package_test_overlay(
            &platform_sources(),
            &package,
            &scenario.artifacts,
            &project,
            &discovered,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(expected),
            "expected {expected:?} for case {index}, got {error}"
        );
    }
}

#[test]
fn inline_effects_reject_aliases_that_resolve_to_the_same_exact_target() {
    let scenario = create_base_assembly_scenario();
    let duplicate_package = scenario._root.child("duplicate-package-aliases");
    copy_tree(&scenario.consumer, &duplicate_package);
    fs::write(
        duplicate_package.join("package.yml"),
        r#"id: example.com/consumer
version: 1.0.0
packages:
  - id: example.com/helper
    version: 1.0.0
    alias: helper
  - id: example.com/helper
    version: 1.0.0
    alias: helperTwin
services:
  - id: example.com/payments
    version: 1.0.0
    alias: payments
"#,
    )
    .unwrap();
    let package_error =
        compile_package_project(&platform_sources(), &duplicate_package, &scenario.artifacts)
            .unwrap_err()
            .to_string();
    assert!(
        package_error.contains("duplicate direct dependency declarations"),
        "{package_error}"
    );

    let package = scenario._root.child("duplicate-service-effect-aliases");
    copy_tree(&scenario.consumer, &package);
    fs::write(
        package.join("package.yml"),
        r#"id: example.com/consumer
version: 1.0.0
packages:
  - id: example.com/helper
    version: 1.0.0
    alias: helper
services:
  - id: example.com/payments
    version: 1.0.0
    alias: payments
  - id: example.com/payments
    version: 1.0.0
    alias: paymentsTwin
"#,
    )
    .unwrap();
    fs::write(
        package.join("main.test.skiff"),
        r#"
test "duplicate service target through aliases" effects {
  payments/echo { respond: "first" },
  paymentsTwin/echo { respond: "second" },
} { assert true }
"#,
    )
    .unwrap();

    let project = compile_package_project(&platform_sources(), &package, &scenario.artifacts)
        .expect("both aliases resolve through canonical dependencies");
    let discovered = discover_package_test_cases(&package, &package, false).unwrap();
    let error = compile_package_test_overlay(
        &platform_sources(),
        &package,
        &scenario.artifacts,
        &project,
        &discovered,
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("payments/echo"), "{error}");
    assert!(error.contains("paymentsTwin/echo"), "{error}");
    assert!(error.contains("use one explicit sequence"), "{error}");
}

#[test]
fn test_service_fixed_profile_projects_over_the_exact_package_closure() {
    let root = TestRoot::new("test-service-profile");
    let artifacts = root.child("artifacts");
    let dependency = root.child("dependency");
    let service = root.child("tests");
    create_store(&artifacts);

    write_package(
        &dependency,
        r#"id: example.com/test-subject
version: 1.0.0
state:
  dependency-db:
    kind: database
"#,
        None,
        Some(
            "function hiddenConfig() -> string {\n\
               return config.require<string>(\"dependency.token\")\n\
             }\n\
             function hiddenSecret() -> string {\n\
               return config.require<string>(\"dependency.secret\")\n\
             }\n\
             type DependencyRecord { id: string }\n\
             db object DependencyRecord { primary key(id) }\n",
        ),
    );
    publish_package(&dependency, &artifacts);

    write_package(
        &service,
        r#"id: example.com/test-subject-tests
version: 1.0.0
packages:
  - id: example.com/test-subject
    version: 1.0.0
    alias: subject
    topLevelAlias: subjectImpl
"#,
        Some("{}\n"),
        Some(
            "function ownConfig() -> string {\n\
               return config.require<string>(\"test.token\")\n\
             }\n",
        ),
    );
    fs::write(
        service.join("service.yml"),
        "id: example.com/test-subject-tests\nkind: test\n",
    )
    .unwrap();
    fs::write(
        service.join("config.skiff-test.yml"),
        r#"config:
  dependency.token: dependency-value
  test.token: test-value
secrets:
  dependency.secret: test/dependency-secret
state:
  dependency-db:
    kind: database
    namespace: authored-shared-name
timeout: 25000
quota:
  cpuMillis: 250
  memoryBytes: 134217728
principal: service:example.com/test-subject-tests
lifecycle:
  maxConcurrency: 2
  idleTimeoutMs: 5000
"#,
    )
    .unwrap();
    fs::write(
        service.join("config.dev.yml"),
        r#"config:
  dependency.token: dev-dependency-value
  test.token: dev-test-value
secrets:
  dependency.secret: dev/dependency-secret
state:
  dependency-db:
    kind: database
    namespace: dev-authored-name
timeout: 5000
quota:
  cpuMillis: 50
  memoryBytes: 67108864
principal: service:example.com/dev-target
lifecycle:
  maxConcurrency: 1
  idleTimeoutMs: 1000
"#,
    )
    .unwrap();
    fs::write(
        service.join("main.test.skiff"),
        "test \"profile is projected\" { assert root.main.ownConfig() == \"test-value\" }\n",
    )
    .unwrap();

    let project = compile_package_project_for_test(&platform_sources(), &service, &artifacts)
        .expect("kind:test must authorize topLevel and bind its fixed profile");
    let profile = project
        .test_service_profile
        .as_ref()
        .expect("kind:test project must retain the selected profile");
    assert_eq!(profile.service_id, "example.com/test-subject-tests");
    assert_eq!(profile.profile_name, "skiff-test");

    let cases = discover_package_test_cases(&service, &service, false).unwrap();
    let overlay =
        compile_package_test_overlay(&platform_sources(), &service, &artifacts, &project, &cases)
            .expect("test-service overlay must retain compiler test-service authority");
    let fixture =
        assemble_package_test_fixture(&project, overlay, CanonicalBaseAssembly::default())
            .expect("normal deployment projection must validate the exact package closure");
    assert_eq!(
        fixture.records.deployments[0].config_literals,
        vec![
            skiff_artifact_model::ConfigLiteralBinding {
                path: "dependency.token".to_string(),
                value: skiff_artifact_model::MetadataValue::String("dependency-value".to_string()),
            },
            skiff_artifact_model::ConfigLiteralBinding {
                path: "test.token".to_string(),
                value: skiff_artifact_model::MetadataValue::String("test-value".to_string()),
            },
        ]
    );
    assert_eq!(
        fixture.records.deployments[0].secret_refs,
        vec![skiff_artifact_model::SecretRefBinding {
            path: "dependency.secret".to_string(),
            secret_ref: "test/dependency-secret".to_string(),
        }]
    );
    assert_eq!(
        fixture.records.deployments[0].policy.timeout_ms,
        Some(25_000)
    );
    assert_eq!(
        fixture.records.deployments[0].policy.resources.cpu_millis,
        250
    );
    assert_eq!(
        fixture.records.deployments[0]
            .policy
            .activation
            .max_concurrency,
        2
    );
    assert_eq!(
        fixture.records.deployments[0].policy.principal,
        "service:example.com/test-subject-tests"
    );
    assert_eq!(fixture.records.deployments[0].state_bindings.len(), 1);
    assert_eq!(
        fixture.records.deployments[0].state_bindings[0].requirement_key,
        "dependency-db"
    );
    assert_ne!(
        fixture.records.deployments[0].state_bindings[0].namespace, "authored-shared-name",
        "the normal profile proves key/kind intent, but each case owns its namespace"
    );

    let mut wrong_dependency_type = project.clone();
    wrong_dependency_type
        .test_service_profile
        .as_mut()
        .unwrap()
        .authoring
        .config = serde_json::json!({
        "dependency.token": true,
        "test.token": "test-value",
    });
    let overlay = compile_package_test_overlay(
        &platform_sources(),
        &service,
        &artifacts,
        &wrong_dependency_type,
        &cases,
    )
    .unwrap();
    let error = assemble_package_test_fixture(
        &wrong_dependency_type,
        overlay,
        CanonicalBaseAssembly::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("dependency.token"), "{error}");
    assert!(error.contains("literal is not string"), "{error}");

    let mut missing_state = project.clone();
    missing_state
        .test_service_profile
        .as_mut()
        .unwrap()
        .authoring
        .state = serde_json::json!({});
    let overlay = compile_package_test_overlay(
        &platform_sources(),
        &service,
        &artifacts,
        &missing_state,
        &cases,
    )
    .unwrap();
    let error =
        assemble_package_test_fixture(&missing_state, overlay, CanonicalBaseAssembly::default())
            .unwrap_err()
            .to_string();
    assert!(
        error.contains("missing state binding dependency-db"),
        "{error}"
    );
}

#[test]
fn test_service_only_target_environment_profile_fails_without_fixed_profile() {
    let root = TestRoot::new("test-service-missing-fixed-profile");
    let artifacts = root.child("artifacts");
    let service = root.child("tests");
    create_store(&artifacts);
    write_package(
        &service,
        "id: example.com/missing-fixed-profile-tests\nversion: 1.0.0\n",
        Some("{}\n"),
        None,
    );
    fs::write(
        service.join("service.yml"),
        "id: example.com/missing-fixed-profile-tests\nkind: test\n",
    )
    .unwrap();
    fs::write(service.join("config.dev.yml"), "timeout: 5000\n").unwrap();

    let error = compile_package_project_for_test(&platform_sources(), &service, &artifacts)
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "test service example.com/missing-fixed-profile-tests requires config.skiff-test.yml"
    );
}

#[derive(Debug, Clone, Copy)]
struct ExpectedLiveHttpEntry {
    key: &'static str,
    method: &'static str,
    path: &'static str,
    handler: &'static str,
    guard: Option<&'static str>,
    dispatch: GatewayDispatchMode,
}

const LIVE_HTTP_MATRIX: &str = "\
default|default.insert|POST|/encrypted-live/default/insert|internal.live.insertOne|internal.live.guard|unary
default|default.insert-many|POST|/encrypted-live/default/insert-many|internal.live.insertMany|internal.live.guard|unary
default|default.insert-bulk|POST|/encrypted-live/default/insert-bulk|internal.live.insertBulk|internal.live.guard|unary
default|default.read|POST|/encrypted-live/default/read|internal.live.readOne|internal.live.guard|unary
default|default.project|POST|/encrypted-live/default/project|internal.live.projectOne|internal.live.guard|unary
default|default.replace-key|POST|/encrypted-live/default/replace-key|internal.live.replaceByKey|internal.live.guard|unary
default|default.replace-query|POST|/encrypted-live/default/replace-query|internal.live.replaceByQuery|internal.live.guard|unary
default|default.upsert|POST|/encrypted-live/default/upsert|internal.live.upsertOne|internal.live.guard|unary
default|default.update|POST|/encrypted-live/default/update|internal.live.updateOne|internal.live.guard|unary
default|default.scan|POST|/encrypted-live/default/scan|internal.live.scan|internal.live.guard|unary
default|default.rewrite|POST|/encrypted-live/default/rewrite|internal.live.rewrite|internal.live.guard|unary
default|default.rewrite-batch|POST|/encrypted-live/default/rewrite-batch|internal.live.rewriteBatch|internal.live.guard|unary
default|default.identity-date|POST|/encrypted-live/default/identity-date|internal.live.identityDate|internal.live.guard|unary
default|default.archive-insert|POST|/encrypted-live/default/archive-insert|internal.live.insertArchive|internal.live.guard|unary
default|default.archive-read|POST|/encrypted-live/default/archive-read|internal.live.readArchive|internal.live.guard|unary
default|default.archive-scan|POST|/encrypted-live/default/archive-scan|internal.live.scanArchive|internal.live.guard|unary
default|default.archive-rewrite|POST|/encrypted-live/default/archive-rewrite|internal.live.rewriteArchive|internal.live.guard|unary
default|default.archive-rewrite-batch|POST|/encrypted-live/default/archive-rewrite-batch|internal.live.rewriteArchiveBatch|internal.live.guard|unary
default|default.archive-restore|POST|/encrypted-live/default/archive-restore|internal.live.restoreArchive|internal.live.guard|unary
default|default.barrier|POST|/encrypted-live/default/barrier|internal.live.activateBarrier|internal.live.guard|unary
default|default.barrier-status|POST|/encrypted-live/default/barrier-status|internal.live.barrierStatus|internal.live.guard|unary
mapped|mapped.insert|POST|/encrypted-live/mapped/insert|internal.live.insertOne|internal.live.guard|unary
mapped|mapped.read|POST|/encrypted-live/mapped/read|internal.live.readOne|internal.live.guard|unary
mapped|mapped.scan|POST|/encrypted-live/mapped/scan|internal.live.scan|internal.live.guard|unary
mapped|mapped.rewrite|POST|/encrypted-live/mapped/rewrite|internal.live.rewrite|internal.live.guard|unary
mapped|mapped.rewrite-batch|POST|/encrypted-live/mapped/rewrite-batch|internal.live.rewriteBatch|internal.live.guard|unary
mapped|mapped.service-probe-insert|POST|/encrypted-live/mapped/service-probe-insert|internal.live.insertServiceContextProbe|internal.live.guard|unary
mapped|mapped.service-probe-read|POST|/encrypted-live/mapped/service-probe-read|internal.live.readServiceContextProbe|internal.live.guard|unary
mapped|mapped.service-probe-scan|POST|/encrypted-live/mapped/service-probe-scan|internal.live.scanServiceContextProbe|internal.live.guard|unary
mapped|mapped.service-probe-rewrite|POST|/encrypted-live/mapped/service-probe-rewrite|internal.live.rewriteServiceContextProbe|internal.live.guard|unary
mapped|mapped.service-probe-rewrite-batch|POST|/encrypted-live/mapped/service-probe-rewrite-batch|internal.live.rewriteServiceContextProbeBatch|internal.live.guard|unary
mapped|mapped.service-probe-restore|POST|/encrypted-live/mapped/service-probe-restore|internal.live.restoreServiceContextProbe|internal.live.guard|unary
mapped|mapped.barrier|POST|/encrypted-live/mapped/barrier|internal.live.activateBarrier|internal.live.guard|unary
mapped|mapped.barrier-status|POST|/encrypted-live/mapped/barrier-status|internal.live.barrierStatus|internal.live.guard|unary
runtime|runtime.echo|POST|/runtime-live/echo|internal.http_adapter.rawEcho|-|unary
runtime|runtime.json|POST|/runtime-live/json|internal.http_adapter.typedJsonEcho|-|unary
runtime|runtime.binary|POST|/runtime-live/binary|internal.http_adapter.binaryEcho|-|unary
runtime|runtime.guarded|GET|/runtime-live/guarded|internal.http_adapter.guardedPost|-|unary
runtime|runtime.stream|POST|/runtime-live/stream|internal.http_adapter.streamEcho|-|serverStream
runtime|runtime.package|POST|/runtime-live/package|internal.http_adapter.packageEcho|-|unary";

fn expected_live_http_entries(service: &str) -> Vec<ExpectedLiveHttpEntry> {
    LIVE_HTTP_MATRIX
        .lines()
        .filter_map(|line| {
            let fields = line.split('|').collect::<Vec<_>>();
            assert_eq!(fields.len(), 7, "invalid live HTTP matrix row {line}");
            (fields[0] == service).then(|| ExpectedLiveHttpEntry {
                key: fields[1],
                method: fields[2],
                path: fields[3],
                handler: fields[4],
                guard: (fields[5] != "-").then_some(fields[5]),
                dispatch: match fields[6] {
                    "unary" => GatewayDispatchMode::Unary,
                    "serverStream" => GatewayDispatchMode::ServerStream,
                    mode => panic!("invalid live HTTP dispatch {mode}"),
                },
            })
        })
        .collect()
}

struct CanonicalLiveServiceReceipt {
    package: PackageArtifact,
    contract: ServiceContract,
    deployment: ServiceDeployment,
}

#[test]
fn canonical_live_source_roots_compile_to_current_receipts() {
    let repository = platform_source_root();
    let default_service = repository.join("runtime/encrypted-storage-live/default-service");
    let mapped_service = repository.join("runtime/encrypted-storage-live/mapped-service");
    let runtime_live = repository.join("runtime/live-tests");
    assert_current_scope_source_artifact_receipt(&repository);

    for (root, profile) in [
        (&default_service, "config.dev.yml"),
        (&mapped_service, "config.dev.yml"),
        (&runtime_live, "config.skiff-test.yml"),
    ] {
        for control in ["package.yml", "api.yml", "service.yml", "http.yml", profile] {
            assert!(
                root.join(control).is_file(),
                "canonical live source root {} must own {control}",
                root.display()
            );
        }
        assert!(!root.join("websocket.yml").exists());
        read_user_package_manifest(&root.join("package.yml")).unwrap();
    }
    assert!(!runtime_live.join("config.dev.yml").exists());

    let test_root = TestRoot::new("canonical-live-source-roots");
    let artifacts = test_root.child("artifacts");
    create_store(&artifacts);
    seed_canonical_std(&platform_sources(), &artifacts).unwrap();

    let encrypted_store = repository.join(
        "runtime/encrypted-storage-live/package-store/example~com~~encrypted-live-store/1.0.0",
    );
    let runtime_kit = runtime_live.join(".skiff-packages/example~com~~runtime-live-kit/1.0.0");
    let encrypted_store_ref = publish_package(&encrypted_store, &artifacts);
    let runtime_kit_ref = publish_package(&runtime_kit, &artifacts);
    let store = CanonicalArtifactStore::open(&artifacts).unwrap();
    let encrypted_store_record = store.read_package_artifact(&encrypted_store_ref).unwrap();
    assert_eq!(
        (
            encrypted_store_record.package_id.as_str(),
            encrypted_store_record.package_version.as_str(),
        ),
        ("example.com/encrypted-live-store", "1.0.0")
    );
    assert_eq!(
        encrypted_store_record.runtime_requirements.state[0].key,
        "encrypted-live-store"
    );
    let runtime_kit_record = store.read_package_artifact(&runtime_kit_ref).unwrap();
    assert_eq!(
        (
            runtime_kit_record.package_id.as_str(),
            runtime_kit_record.package_version.as_str(),
        ),
        ("example.com/runtime-live-kit", "1.0.0")
    );
    assert!(runtime_kit_record
        .package_local_abi
        .public_symbols
        .contains_key("packageEcho"));

    let default_receipt = author_ordinary_live_service(&default_service, &artifacts);
    let mapped_receipt = author_ordinary_live_service(&mapped_service, &artifacts);
    let runtime_receipt = author_test_live_service(&runtime_live, &artifacts);
    let default_project =
        compile_package_project_for_test(&platform_sources(), &default_service, &artifacts)
            .expect("default live service production package must support its test overlay");
    let default_cases = discover_package_test_cases(
        &default_service.join("internal/encrypted.live.test.skiff"),
        &default_service,
        true,
    )
    .unwrap();
    assert_eq!(default_cases.len(), 1);
    compile_package_test_overlay(
        &platform_sources(),
        &default_service,
        &artifacts,
        &default_project,
        &default_cases,
    )
    .expect("default encrypted live test must consume the normal private config owner");

    assert_eq!(
        default_receipt
            .package
            .runtime_requirements
            .config
            .iter()
            .map(|requirement| requirement.path.as_str())
            .collect::<Vec<_>>(),
        ["encryptedLive.testRunnerSecret"]
    );
    assert_eq!(default_receipt.deployment.config_literals.len(), 1);
    assert_eq!(default_receipt.deployment.state_bindings.len(), 1);
    assert_eq!(mapped_receipt.deployment.state_bindings.len(), 2);
    assert!(mapped_receipt
        .deployment
        .state_bindings
        .iter()
        .all(|binding| binding.namespace == "encrypted-live-mapped"));
    assert_eq!(
        runtime_receipt
            .package
            .runtime_requirements
            .config
            .iter()
            .map(|requirement| requirement.path.as_str())
            .collect::<Vec<_>>(),
        [
            "runtimeLive.db",
            "runtimeLive.file",
            "runtimeLive.httpAdapter",
            "runtimeLive.operation",
        ]
    );
    assert_eq!(runtime_receipt.deployment.config_literals.len(), 4);
    assert_eq!(runtime_receipt.deployment.state_bindings.len(), 1);

    let default_entries = expected_live_http_entries("default");
    let mapped_entries = expected_live_http_entries("mapped");
    let runtime_entries = expected_live_http_entries("runtime");
    let mut identities = Vec::new();
    identities.extend(assert_live_service_receipt(
        &default_receipt,
        "example.com/encrypted-live-default",
        "0.1.0",
        &default_entries,
    ));
    identities.extend(assert_live_service_receipt(
        &mapped_receipt,
        "example.com/encrypted-live-mapped",
        "0.1.0",
        &mapped_entries,
    ));
    identities.extend(assert_live_service_receipt(
        &runtime_receipt,
        "skiff.run/runtime-live",
        "0.1.0",
        &runtime_entries,
    ));

    assert_eq!(identities.len(), 40);
    assert_eq!(
        identities
            .iter()
            .filter(|(mode, _)| *mode == GatewayDispatchMode::Unary)
            .count(),
        39
    );
    assert_eq!(
        identities
            .iter()
            .filter(|(mode, _)| *mode == GatewayDispatchMode::ServerStream)
            .count(),
        1
    );
    let unary_identities = identities
        .iter()
        .filter(|(mode, _)| *mode == GatewayDispatchMode::Unary)
        .map(|(_, identity)| identity.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let stream_identities = identities
        .iter()
        .filter(|(mode, _)| *mode == GatewayDispatchMode::ServerStream)
        .map(|(_, identity)| identity.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unary_identities.len(), 1);
    assert_eq!(stream_identities.len(), 1);
    assert_ne!(unary_identities, stream_identities);
    assert_eq!(
        unary_identities,
        std::collections::BTreeSet::from([EXPECTED_RAW_HTTP_UNARY_GATEWAY_IDENTITY])
    );
    assert_eq!(
        stream_identities,
        std::collections::BTreeSet::from([EXPECTED_RAW_HTTP_STREAM_GATEWAY_IDENTITY])
    );
    assert!(
        identities
            .iter()
            .all(|(_, identity)| identity.starts_with("skiff-gateway-entry-v2:sha256:")),
        "all gateway identities must be current v2 producer values: {identities:?}"
    );

    let mapped_requirement = mapped_receipt
        .package
        .package_requirements
        .iter()
        .find(|requirement| requirement.alias == "encryptedStore")
        .expect("mapped package must retain its exact dependency edge");
    assert_eq!(
        mapped_requirement.collection_name_mapping,
        BTreeMap::from([(
            "package_secret".to_string(),
            "mapped_package_secret".to_string(),
        )])
    );
    let mapped_binding = mapped_receipt
        .deployment
        .package_bindings
        .iter()
        .find(|binding| binding.key.package_requirement_alias == "encryptedStore")
        .expect("mapped deployment must bind the exact dependency edge");
    assert_eq!(
        mapped_binding.collection_name_mapping,
        mapped_requirement.collection_name_mapping
    );
    assert_eq!(
        runtime_receipt.deployment.diagnostic_text.display_name,
        "skiff.run/runtime-live@0.1.0 (skiff-test)"
    );
}

fn assert_current_scope_source_artifact_receipt(repository: &Path) {
    let fixture = repository.join("test-runner/fixtures/package-service-current-scope");
    let consumer_source = fixture.join("consumer/main.skiff");
    for control in [
        "consumer/package.yml",
        "consumer/api.yml",
        "consumer/service.yml",
        "consumer/http.yml",
        "consumer/websocket.yml",
        "consumer/main.skiff",
        "provider/package.yml",
        "provider/api.yml",
        "provider/service.yml",
        "provider/main.skiff",
    ] {
        assert!(
            fixture.join(control).is_file(),
            "current-scope fixture must own {control}"
        );
    }
    let source = fs::read_to_string(&consumer_source).unwrap();
    assert_eq!(
        source.matches("timeout(").count(),
        12,
        "six current-scope carriers must each use nested timeout expressions"
    );
    for required in [
        "std.http.request",
        "std.http.stream",
        "std.websocket.requestJsonToConnection<string, string>",
        "std.file.createText",
        "std.actor.getOrCreate<Counter>",
        "payments/echo",
    ] {
        assert!(
            source.contains(required),
            "current-scope source omitted {required}"
        );
    }
    for forbidden in [
        "$/cancelRequest",
        "-32800",
        "CancelError",
        "requestId",
        "ServiceTimeoutConfig",
    ] {
        assert!(
            !source.contains(forbidden),
            "current-scope source retained forbidden {forbidden}"
        );
    }

    let root = TestRoot::new("current-scope-source-artifact");
    let artifacts = root.child("artifacts");
    create_store(&artifacts);
    seed_canonical_std(&platform_sources(), &artifacts).unwrap();
    let receipt = prepare_package_service_host_fixture(
        &platform_sources(),
        &fixture,
        &root.child("authoring"),
        &artifacts,
        "current-scope",
    )
    .expect("checked-in current-scope source must produce canonical authoring receipts");
    let store = CanonicalArtifactStore::open(&artifacts).unwrap();
    let package = store
        .read_package_artifact(&receipt.consumer_package)
        .expect("consumer package round-trip");
    let contract = store
        .read_service_contract(&receipt.consumer_contract)
        .expect("consumer contract round-trip");
    let deployment = store
        .read_service_deployment(&receipt.consumer_deployment)
        .expect("consumer deployment round-trip");
    let assembly = store
        .read_runtime_assembly(&receipt.base_assembly)
        .expect("runtime assembly round-trip");
    let project =
        compile_package_project(&platform_sources(), &fixture.join("consumer"), &artifacts)
            .expect("checked-in current-scope consumer must compile from its real source root");
    let main = project
        .package
        .file_ir_units
        .iter()
        .find(|file| file.module_path == "main")
        .expect("current-scope main File IR");

    assert_eq!(main.unit.schema_version, "skiff-file-ir-v9");
    assert_eq!(main.unit.ir_format_version, "skiff-file-ir-format-v7");
    assert_eq!(main.unit.opcode_table_version, "skiff-opcode-table-v2");
    assert_eq!(package.schema_version, "skiff-package-artifact-v9");
    assert_eq!(contract.schema_version, "skiff-service-contract-v5");
    assert_eq!(deployment.schema_version, "skiff-service-deployment-v4");
    assert_eq!(assembly.schema_version, "skiff-runtime-assembly-v3");
    assert_eq!(
        skiff_artifact_identity::package_artifact_ref(&package).unwrap(),
        receipt.consumer_package
    );
    assert_eq!(
        service_contract_ref(&contract).unwrap(),
        receipt.consumer_contract
    );
    assert_eq!(
        service_deployment_ref(&deployment),
        receipt.consumer_deployment
    );
    assert_eq!(
        skiff_artifact_identity::runtime_assembly_ref(&assembly).unwrap(),
        receipt.base_assembly
    );
    assert_eq!(
        main.identity,
        skiff_artifact_identity::file_ir_identity(&main.unit).unwrap()
    );

    let main_json = serde_json::to_string(&main.unit).unwrap();
    assert_eq!(
        main_json.matches("\"kind\":\"timeout\"").count(),
        12,
        "all authored timeout expressions must reach File IR"
    );
    for operation in [
        "requestJsonToConnection",
        "createText",
        "getOrCreate",
        "\"kind\":\"serviceCall\"",
    ] {
        assert!(
            main_json.contains(operation),
            "File IR omitted current-scope operation {operation}"
        );
    }
    let service_call_refs =
        skiff_artifact_model::validated_file_ir_service_call_refs(&main.unit).unwrap();
    assert_eq!(service_call_refs.len(), 1);
    assert_eq!(
        service_call_refs[0].expected_protocol_identity,
        receipt.payments_contract.service_protocol_identity
    );

    assert!(contract.operations.is_empty());
    assert_eq!(deployment.service_selectors.len(), 1);
    assert_eq!(
        deployment.service_selectors[0].contract,
        receipt.payments_contract
    );
    assert_eq!(deployment.gateway_entries.len(), 3);
    assert_eq!(deployment.ingress.len(), 3);
    let unary =
        &deployment.gateway_entries[&GatewayEntryKey::parse("current-scope.unary").unwrap()];
    let stream =
        &deployment.gateway_entries[&GatewayEntryKey::parse("current-scope.stream").unwrap()];
    let websocket = &deployment.gateway_entries[&GatewayEntryKey::parse("websocket").unwrap()];
    let GatewayProtocolSurface::Http(unary_surface) = &unary.protocol_surface.protocol else {
        panic!("current-scope unary must remain HTTP")
    };
    assert_eq!(unary_surface.dispatch_mode, GatewayDispatchMode::Unary);
    let GatewayProtocolSurface::Http(stream_surface) = &stream.protocol_surface.protocol else {
        panic!("current-scope stream must remain HTTP")
    };
    assert_eq!(
        stream_surface.dispatch_mode,
        GatewayDispatchMode::ServerStream
    );
    assert!(matches!(
        websocket.protocol_surface.protocol,
        GatewayProtocolSurface::WebSocketConnect(_)
    ));
    assert_eq!(assembly.roots, [receipt.consumer_deployment.clone()]);
    assert!(assembly
        .resolved_deployments
        .contains(&receipt.provider_deployment));
    assert_eq!(assembly.gateway_ingress.len(), 3);
    assert_eq!(assembly.service_binding_templates.len(), 2);
    let consumer_bindings = assembly
        .service_binding_templates
        .iter()
        .find(|template| template.activation == receipt.consumer_deployment)
        .expect("consumer service binding template");
    assert_eq!(consumer_bindings.bindings.len(), 1);
    assert_eq!(
        consumer_bindings.bindings[0].provider,
        receipt.provider_deployment
    );

    let identity_tuple = (
        main.identity.as_str(),
        receipt.consumer_package.package_build_id.as_str(),
        receipt.consumer_package.package_local_abi_identity.as_str(),
        receipt.consumer_contract.service_protocol_identity.as_str(),
        receipt
            .consumer_deployment
            .deployment_artifact_identity
            .as_str(),
        unary.gateway_entry_identity.as_str(),
        stream.gateway_entry_identity.as_str(),
        websocket.gateway_entry_identity.as_str(),
        receipt.base_assembly.assembly_identity.as_str(),
    );
    assert_eq!(
        identity_tuple,
        (
            "skiff-file-ir-v9:sha256:9e0b0915efe308c05081320012f282ef81e37e9536c02f16af0a770a021f60f6",
            "skiff-package-build-v10:sha256:9b03476e93f5ccb66dc69ff899f4a8fb9c68593e70c5aeda94d4e865aab688ad",
            "skiff-package-local-abi-v7:sha256:605b18a2b130957f4b1feec499583334601b3788514ea851530b6623a017aed4",
            "skiff-service-protocol-v5:sha256:9ea7ac440bd594ef31632c1c1914b40f2e92957e7fb0f73f587f4cb4d8563fa5",
            "skiff-deployment-artifact-v4:sha256:bfa01d12d90d7a9e5af9da153b63862270a52eaffe59383a4563cff2a0dde2a4",
            "skiff-gateway-entry-v2:sha256:0fd289d7eec4e03b01e9e8f5633aedd7e1cc64158fa7932f99a9686e559c02f2",
            "skiff-gateway-entry-v2:sha256:1aef41f397b7c817110cb0cc74a7b472ba9732c5ac6bcfe6e219e3ac51ab6bd0",
            "skiff-gateway-entry-v2:sha256:f385624021966bab998385e1fd2c88804b51992f15f9c9d76c05d3e17a75018d",
            "skiff-runtime-assembly-v3:sha256:ce8c979de4c6786ee9c2fbf2ad01fbfa2271b33a074682e2e66f5a77654f6688",
        ),
        "checked-in current-scope source must retain its exact artifact identity tuple"
    );

    let mutation_root = root.child("mutations");
    copy_tree(&fixture, &mutation_root);
    fs::write(
        mutation_root.join("consumer/service.yml"),
        "id: example.com/current-scope-consumer\nhttp: {}\n",
    )
    .unwrap();
    let inline_error = read_service_package_root(&mutation_root.join("consumer"))
        .expect_err("inline ingress must fail closed")
        .to_string();
    assert!(
        inline_error.contains("unknown field `http`"),
        "{inline_error}"
    );

    copy_tree(&fixture, &mutation_root);
    let old_call_source = source.replace("payments/echo(value)", "payments.echo(value)");
    fs::write(mutation_root.join("consumer/main.skiff"), old_call_source).unwrap();
    let old_call_error = compile_package_project(
        &platform_sources(),
        &mutation_root.join("consumer"),
        &artifacts,
    )
    .expect_err("retired service call spelling must fail closed")
    .to_string();
    assert!(
        old_call_error.contains("payments.echo") || old_call_error.contains("payments"),
        "{old_call_error}"
    );

    copy_tree(&fixture, &mutation_root);
    let timeout_mutation = source.replace("timeout(250ms)", "timeout(251ms)");
    fs::write(mutation_root.join("consumer/main.skiff"), timeout_mutation).unwrap();
    let mutated = compile_package_project(
        &platform_sources(),
        &mutation_root.join("consumer"),
        &artifacts,
    )
    .expect("timeout fact mutation must remain valid source");
    let mutated_main = mutated
        .package
        .file_ir_units
        .iter()
        .find(|file| file.module_path == "main")
        .unwrap();
    assert_ne!(mutated_main.identity, main.identity);
    assert_ne!(
        mutated.package.artifact.package_build_id,
        project.package.artifact.package_build_id
    );
    assert_eq!(
        mutated
            .package
            .artifact
            .package_local_abi
            .local_abi_identity,
        project
            .package
            .artifact
            .package_local_abi
            .local_abi_identity
    );

    let old_assembly_ref = serde_json::json!({
        "assemblyIdentity": format!("skiff-runtime-assembly-v1:sha256:{}", "0".repeat(64))
    });
    let old_identity_error =
        serde_json::from_value::<RuntimeAssemblyRef>(old_assembly_ref).unwrap_err();
    assert!(
        old_identity_error
            .to_string()
            .contains("skiff-runtime-assembly-v3"),
        "{old_identity_error}"
    );
}

fn author_ordinary_live_service(root: &Path, artifacts: &Path) -> CanonicalLiveServiceReceipt {
    let output = build_authoring_object(
        &platform_sources(),
        AuthoringObject::Package,
        root,
        artifacts,
        "dev",
        false,
    )
    .unwrap_or_else(|error| {
        panic!(
            "ordinary canonical live source root {} must compile: {error}",
            root.display()
        )
    });
    let package_ref: PackageArtifactRef =
        serde_json::from_value(output["packageArtifactReceipt"]["artifact"].clone()).unwrap();
    let contract_ref: ServiceContractRef =
        serde_json::from_value(output["serviceContractReceipt"]["contract"].clone()).unwrap();
    let deployment_ref: ServiceDeploymentRef =
        serde_json::from_value(output["serviceDeploymentReceipt"]["deployment"].clone()).unwrap();
    let store = CanonicalArtifactStore::open(artifacts).unwrap();
    CanonicalLiveServiceReceipt {
        package: store
            .read_package_artifact(&package_ref)
            .unwrap()
            .as_ref()
            .clone(),
        contract: store
            .read_service_contract(&contract_ref)
            .unwrap()
            .as_ref()
            .clone(),
        deployment: store
            .read_service_deployment(&deployment_ref)
            .unwrap()
            .as_ref()
            .clone(),
    }
}

fn author_test_live_service(root: &Path, artifacts: &Path) -> CanonicalLiveServiceReceipt {
    let platform_sources = platform_sources();
    let project = compile_package_project_for_test(&platform_sources, root, artifacts)
        .expect("runtime-live test service must compile through the fixed test workflow");
    let profile = project
        .test_service_profile
        .as_ref()
        .expect("runtime-live must remain a kind:test service");
    assert_eq!(profile.profile_name, "skiff-test");
    for (source, obsolete) in [
        (
            "internal/operation.live.test.skiff",
            concat!("__skiff", "Payload"),
        ),
        (
            "internal/operation.live.test.skiff",
            concat!(
                "live operation dispatch crosses runtime ",
                "binary payload boundary"
            ),
        ),
        ("internal/operation.skiff", concat!("payload", "RoundTrip")),
        (
            "internal/file_live.live.test.skiff",
            concat!(
                "live file runtime rejects stream above ",
                "file guard limit"
            ),
        ),
        (
            "internal/file_live.skiff",
            concat!("liveFileOver", "LimitChunks"),
        ),
        (
            "internal/file_live.skiff",
            concat!("liveFileSixty", "FourMiBChunk"),
        ),
    ] {
        let contents = fs::read_to_string(root.join(source)).unwrap();
        assert!(
            !contents.contains(obsolete),
            "canonical runtime-live source {source} must not retain obsolete {obsolete}"
        );
    }

    let mut case_count = 0;
    for (source, expected_cases) in [
        ("internal/db_live.live.test.skiff", 4),
        ("internal/file_live.live.test.skiff", 3),
        ("internal/http_adapter.live.test.skiff", 4),
        ("internal/operation.live.test.skiff", 1),
    ] {
        let cases = discover_package_test_cases(&root.join(source), root, true)
            .unwrap_or_else(|error| panic!("runtime-live {source} discovery failed: {error}"));
        assert_eq!(
            cases.len(),
            expected_cases,
            "runtime-live {source} must retain its exact tracked case count"
        );
        case_count += cases.len();
        compile_package_test_overlay(&platform_sources, root, artifacts, &project, &cases)
            .unwrap_or_else(|error| panic!("runtime-live {source} compile failed: {error}"));
    }
    assert_eq!(case_count, 12);

    let manifest = read_user_package_manifest(&root.join("package.yml")).unwrap();
    let raw_sources = read_package_sources(&manifest, root).unwrap();
    let source_tree = raw_sources.source_tree();
    let source_graph =
        PublicationSourceGraph::parse_raw_publication_sources(&raw_sources.into_source_graph())
            .unwrap();
    let resources = read_publication_resources(root, &manifest.resources).unwrap();
    let source = PackageSourceInput::new(
        manifest.publication.clone(),
        source_tree,
        source_graph,
        resources,
    );
    let direct_dependencies = manifest
        .dependencies
        .iter()
        .map(|dependency| {
            project
                .dependency_packages
                .iter()
                .find(|artifact| {
                    artifact.package_id == dependency.id
                        && artifact.package_version == dependency.version
                })
                .unwrap_or_else(|| {
                    panic!(
                        "missing direct dependency {}@{}",
                        dependency.id, dependency.version
                    )
                })
                .clone()
        })
        .collect::<Vec<_>>();
    let aliases = manifest
        .dependencies
        .iter()
        .map(|dependency| {
            let artifact = direct_dependencies
                .iter()
                .find(|artifact| {
                    artifact.package_id == dependency.id
                        && artifact.package_version == dependency.version
                })
                .unwrap();
            let mut roots = artifact
                .package_local_abi
                .public_symbols
                .keys()
                .map(|path| path.split('.').take(2).collect::<Vec<_>>().join("."))
                .collect::<Vec<_>>();
            roots.sort();
            roots.dedup();
            (dependency.effective_alias().to_string(), roots)
        })
        .collect::<BTreeMap<_, _>>();
    let service = read_service_package_root(root).unwrap();
    assert_eq!(service.service.kind, ServiceAuthoringKind::Test);
    let store = CanonicalArtifactStore::open(artifacts).unwrap();
    let compiled = compile_service_package(
        PackageCompileInput::new(&platform_sources, &source, &aliases, manifest.id.as_str())
            .with_canonical_dependencies(&direct_dependencies, &project.contract_dependencies)
            .with_available_canonical_packages(&project.dependency_packages)
            .with_canonical_artifact_store(&store)
            .for_test_service(),
        &service,
    )
    .expect("runtime-live test service must use the real service compiler producer");
    assert_eq!(
        compiled.package.artifact.package_build_id,
        project.package.artifact.package_build_id
    );
    let deployment = generate_service_deployment(GeneratedServiceDeploymentInput {
        service: &service.service,
        http: service.http.as_ref(),
        websocket: service.websocket.as_ref(),
        profile_name: &profile.profile_name,
        profile: &profile.authoring,
        service_api: &compiled.service_api,
        implementation: &compiled.package.artifact,
        package_closure: &project.dependency_packages,
        package_schema_records: &compiled.package.resolved_package_schema_type_records,
    })
    .expect("runtime-live fixed profile must generate a real deployment");

    let package_receipt = publish_package_artifact_records(&store, &compiled.package).unwrap();
    store
        .write_service_contract(&compiled.service_api.contract)
        .unwrap();
    store.write_service_deployment(&deployment).unwrap();
    let contract_ref = service_contract_ref(&compiled.service_api.contract).unwrap();
    let deployment_ref = service_deployment_ref(&deployment);
    CanonicalLiveServiceReceipt {
        package: store
            .read_package_artifact(&package_receipt.artifact)
            .unwrap()
            .as_ref()
            .clone(),
        contract: store
            .read_service_contract(&contract_ref)
            .unwrap()
            .as_ref()
            .clone(),
        deployment: store
            .read_service_deployment(&deployment_ref)
            .unwrap()
            .as_ref()
            .clone(),
    }
}

fn assert_live_service_receipt(
    receipt: &CanonicalLiveServiceReceipt,
    service_id: &str,
    version: &str,
    expected: &[ExpectedLiveHttpEntry],
) -> Vec<(GatewayDispatchMode, String)> {
    assert_eq!(receipt.package.package_id, service_id);
    assert_eq!(receipt.package.package_version, version);
    assert_eq!(receipt.contract.service_id, service_id);
    assert_eq!(receipt.contract.contract_version, version);
    assert!(receipt.contract.operations.is_empty());
    assert!(receipt.contract.package_type_requirements.is_empty());
    assert_eq!(receipt.deployment.contract.service_id, service_id);
    assert_eq!(receipt.deployment.contract.contract_version, version);
    assert_eq!(
        receipt.deployment.implementation.package_build_id,
        receipt.package.package_build_id
    );
    assert!(receipt.deployment.operation_bindings.is_empty());
    assert_eq!(receipt.deployment.gateway_entries.len(), expected.len());
    assert_eq!(receipt.deployment.ingress.len(), expected.len());

    expected
        .iter()
        .map(|expected| {
            let key = GatewayEntryKey::parse(expected.key).unwrap();
            let ingress = receipt
                .deployment
                .ingress
                .iter()
                .find(|binding| binding.gateway_entry_key == key)
                .unwrap_or_else(|| panic!("missing ingress {}", expected.key));
            assert_eq!(ingress.selector.protocol, IngressProtocol::Http);
            assert_eq!(ingress.selector.method.as_deref(), Some(expected.method));
            assert_eq!(ingress.selector.path, expected.path);

            let gateway = &receipt.deployment.gateway_entries[&key];
            let expected_handler = callable_id(&receipt.package, expected.handler);
            assert_eq!(gateway.handler.as_ref(), Some(expected_handler));
            assert_eq!(
                gateway.guard.as_ref(),
                expected
                    .guard
                    .map(|selector| callable_id(&receipt.package, selector))
            );
            assert_eq!(gateway.pre, None);
            assert_eq!(gateway.adapter_plan.kind, GatewayAdapterKind::RawHttp);
            assert_eq!(gateway.adapter_plan.args.len(), 1);
            assert_eq!(gateway.adapter_plan.args[0].param, "request");
            assert_eq!(
                gateway.adapter_plan.args[0].source,
                GatewayAdapterSource::HttpRequest
            );
            let GatewayProtocolSurface::Http(surface) = &gateway.protocol_surface.protocol else {
                panic!("{} must remain an HTTP protocol surface", expected.key)
            };
            assert_eq!(surface.adapter_kind, GatewayAdapterKind::RawHttp);
            assert_eq!(surface.dispatch_mode, expected.dispatch);
            assert_eq!(
                surface.external_sources,
                vec![GatewayAdapterSource::HttpRequest]
            );
            (
                surface.dispatch_mode,
                gateway.gateway_entry_identity.as_str().to_string(),
            )
        })
        .collect()
}

fn callable_id<'a>(
    package: &'a PackageArtifact,
    selector: &str,
) -> &'a skiff_artifact_model::PackageCallableId {
    let PackageLocalAbiSymbol::Callable { callable_id, .. } = package
        .package_local_abi
        .implementation_symbols
        .get(selector)
        .unwrap_or_else(|| panic!("missing implementation callable {selector}"))
    else {
        panic!("implementation symbol {selector} must be callable")
    };
    callable_id
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
    let project = compile_package_project(&platform_sources(), &package, &artifacts)
        .expect("production package");
    let production = skiff_artifact_identity::package_artifact_ref(&project.package.artifact)
        .expect("production ref");
    let cases = discover_package_test_cases(&package, &package, false).expect("test cases");
    let overlay =
        compile_package_test_overlay(&platform_sources(), &package, &artifacts, &project, &cases)
            .expect("overlay");
    assert!(
        overlay
            .overlay
            .artifact
            .package_requirements
            .iter()
            .all(
                |requirement| requirement.package_id != production.package_id
                    || requirement.exact_version != production.package_version
            ),
        "the typed overlay coordinate must never become a self package requirement"
    );
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
fn package_test_http_fixture_is_zero_operation_reference_closed_and_fail_closed() {
    const NULL_GATEWAY_IDENTITY: &str = concat!(
        "skiff-gateway-entry-v2:sha256:",
        "b97af7d9ff0b9ddbfcb6ea8b19e6173722095c99f1566ccd6b1a6fd2ead3f305"
    );

    let root = TestRoot::new("package-test-http-gateway");
    let artifacts = root.child("artifacts");
    let package = root.child("package");
    create_store(&artifacts);
    write_package(
        &package,
        "id: example.com/package-test-http\nversion: 1.0.0\n",
        None,
        Some("function privateHelper() -> bool { return true }\n"),
    );
    fs::write(
        package.join("main.test.skiff"),
        "test \"HTTP gateway case\" { assert root.main.privateHelper() }\n",
    )
    .unwrap();

    let project = compile_package_project(&platform_sources(), &package, &artifacts).unwrap();
    let cases = discover_package_test_cases(&package, &package, false).unwrap();
    let overlay =
        compile_package_test_overlay(&platform_sources(), &package, &artifacts, &project, &cases)
            .unwrap();
    let binding = overlay.bindings[0].clone();

    let mut missing = overlay.clone();
    missing
        .overlay
        .artifact
        .package_local_abi
        .implementation_symbols
        .remove(&binding.gateway_selector);
    assert!(
        assemble_package_test_fixture(&project, missing, CanonicalBaseAssembly::default())
            .unwrap_err()
            .to_string()
            .contains("has no exact private gateway handler")
    );

    let mut wrong_signature = overlay.clone();
    let PackageLocalAbiSymbol::Callable { signature, .. } = wrong_signature
        .overlay
        .artifact
        .package_local_abi
        .implementation_symbols
        .get_mut(&binding.gateway_selector)
        .expect("generated gateway implementation symbol")
    else {
        panic!("generated gateway must be callable")
    };
    signature.parameters.clear();
    assert!(assemble_package_test_fixture(
        &project,
        wrong_signature,
        CanonicalBaseAssembly::default()
    )
    .unwrap_err()
    .to_string()
    .contains("must have exact signature"));

    let mut public_handler = overlay.clone();
    let leaked = public_handler
        .overlay
        .artifact
        .package_local_abi
        .implementation_symbols[&binding.gateway_selector]
        .clone();
    public_handler
        .overlay
        .artifact
        .package_local_abi
        .public_symbols
        .insert("leakedGateway".to_string(), leaked);
    assert!(assemble_package_test_fixture(
        &project,
        public_handler,
        CanonicalBaseAssembly::default()
    )
    .unwrap_err()
    .to_string()
    .contains("must not enter the package public API"));

    let fixture =
        assemble_package_test_fixture(&project, overlay, CanonicalBaseAssembly::default()).unwrap();
    let [contract] = fixture.records.contracts.as_slice() else {
        panic!("one case must produce one zero-operation contract")
    };
    let [deployment] = fixture.records.deployments.as_slice() else {
        panic!("one case must produce one deployment")
    };
    let [entrypoint] = fixture.entrypoints.as_slice() else {
        panic!("one case must produce one HTTP entrypoint")
    };
    assert!(contract.operations.is_empty());
    assert!(contract.package_type_requirements.is_empty());
    assert!(deployment.operation_bindings.is_empty());
    assert_eq!(deployment.gateway_entries.len(), 1);
    assert_eq!(deployment.ingress.len(), 1);
    assert_eq!(entrypoint.gateway_entry_key.as_str(), "run");
    assert_eq!(
        entrypoint.gateway_entry_identity.as_str(),
        NULL_GATEWAY_IDENTITY
    );
    assert_eq!(entrypoint.mode, GatewayDispatchMode::Unary);
    assert_eq!(entrypoint.selector.protocol, IngressProtocol::Http);
    assert_eq!(entrypoint.selector.method.as_deref(), Some("POST"));
    assert_eq!(entrypoint.selector.path, "/__skiff/package-test/0");
    let entry = &deployment.gateway_entries[&entrypoint.gateway_entry_key];
    assert_eq!(
        entry.gateway_entry_identity,
        entrypoint.gateway_entry_identity
    );
    assert_eq!(entry.handler.as_ref(), Some(&binding.gateway_callable_id));
    assert_eq!(entry.pre, None);
    assert_eq!(entry.guard, None);
    assert_eq!(entry.adapter_plan.kind, GatewayAdapterKind::TypedJson);
    assert_eq!(entry.adapter_plan.args.len(), 1);
    assert_eq!(entry.adapter_plan.args[0].param, "body");
    assert_eq!(
        entry.adapter_plan.args[0].source,
        GatewayAdapterSource::HttpBody
    );
    let GatewayProtocolSurface::Http(surface) = &entry.protocol_surface.protocol else {
        panic!("package-test HTTP fixture must use an HTTP protocol surface")
    };
    assert_eq!(surface.adapter_kind, GatewayAdapterKind::TypedJson);
    assert_eq!(surface.dispatch_mode, GatewayDispatchMode::Unary);
    assert_eq!(surface.external_sources, [GatewayAdapterSource::HttpBody]);
    assert_eq!(
        surface.request_body_schema,
        Some(GatewayExternalSchema::Null)
    );
    assert_eq!(surface.response_schema, Some(GatewayExternalSchema::Null));
    assert_eq!(surface.stream_item_schema, None);
    assert_eq!(
        deployment.ingress[0].gateway_entry_key,
        entrypoint.gateway_entry_key
    );
    assert_eq!(deployment.ingress[0].selector, entrypoint.selector);
    let [assembly_ingress] = fixture.records.assembly.gateway_ingress.as_slice() else {
        panic!("one case must project one assembly gateway ingress")
    };
    assert_eq!(assembly_ingress.selector, entrypoint.selector);
    assert_eq!(assembly_ingress.deployment, entrypoint.deployment);
    assert_eq!(
        assembly_ingress.gateway_entry_key,
        entrypoint.gateway_entry_key
    );
    assert_eq!(
        assembly_ingress.gateway_entry_identity,
        entrypoint.gateway_entry_identity
    );

    let mut wrong_key = deployment.clone();
    wrong_key.ingress[0].gateway_entry_key = GatewayEntryKey::parse("wrong").unwrap();
    assert!(skiff_artifact_identity::validate_service_deployment_surface(&wrong_key).is_err());

    let mut wrong_identity = deployment.clone();
    wrong_identity
        .gateway_entries
        .get_mut(&entrypoint.gateway_entry_key)
        .unwrap()
        .gateway_entry_identity =
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v2:sha256:{}", "a".repeat(64)))
            .unwrap();
    assert!(skiff_artifact_identity::validate_service_deployment_surface(&wrong_identity).is_err());

    let mut orphan = deployment.clone();
    orphan
        .gateway_entries
        .insert(GatewayEntryKey::parse("orphan").unwrap(), entry.clone());
    assert!(skiff_artifact_identity::validate_service_deployment_surface(&orphan).is_err());

    let mut duplicate_selector = deployment.clone();
    duplicate_selector
        .ingress
        .push(duplicate_selector.ingress[0].clone());
    assert!(
        skiff_artifact_identity::validate_service_deployment_surface(&duplicate_selector).is_err()
    );
}

#[test]
fn package_test_generates_typed_state_bindings_in_run_isolated_namespaces() {
    let root = TestRoot::new("package-test-state-bindings");
    let artifacts = root.child("artifacts");
    let package = root.child("package");
    create_store(&artifacts);
    write_package(
        &package,
        r#"id: example.com/stateful-package
version: 1.0.0
state:
  app-db:
    kind: database
  jobs:
    kind: queue
"#,
        None,
        Some(
            "type Stored { id: string }\n\
             db object Stored { primary key(id) }\n",
        ),
    );
    fs::write(
        package.join("main.test.skiff"),
        "test \"state binding first\" { assert true }\n\
         test \"state binding second\" { assert true }\n",
    )
    .unwrap();
    fs::write(
        package.join("other.test.skiff"),
        "test \"state binding other file\" { assert true }\n",
    )
    .unwrap();
    let project = compile_package_project(&platform_sources(), &package, &artifacts).unwrap();
    let cases = discover_package_test_cases(&package, &package, false).unwrap();

    let assemble = |run_scope: &str| {
        let overlay = compile_package_test_overlay(
            &platform_sources(),
            &package,
            &artifacts,
            &project,
            &cases,
        )
        .unwrap();
        assemble_package_test_fixture_for_run(
            &project,
            overlay,
            CanonicalBaseAssembly::default(),
            run_scope,
        )
        .unwrap()
    };
    let run_a = assemble("run-a");
    let run_a_repeat = assemble("run-a");
    let run_b = assemble("run-b");
    assert_eq!(run_a.records.deployments.len(), 3);
    assert_eq!(run_a.entrypoints.len(), 3);
    let namespaces = |fixture: &CanonicalPackageTestFixture| {
        fixture
            .records
            .deployments
            .iter()
            .map(|deployment| {
                assert_eq!(deployment.state_bindings.len(), 2);
                assert_eq!(deployment.state_bindings[0].requirement_key, "app-db");
                assert_eq!(
                    deployment.state_bindings[0].kind,
                    StateBindingKind::Database
                );
                assert_eq!(deployment.state_bindings[1].requirement_key, "jobs");
                assert_eq!(deployment.state_bindings[1].kind, StateBindingKind::Queue);
                deployment
                    .state_bindings
                    .iter()
                    .map(|binding| binding.namespace.clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let namespaces_a = namespaces(&run_a);
    let namespaces_a_repeat = namespaces(&run_a_repeat);
    let namespaces_b = namespaces(&run_b);
    assert_eq!(
        namespaces_a
            .iter()
            .flatten()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        6,
        "same-file and cross-file cases must all receive distinct state namespaces"
    );
    assert!(namespaces_a
        .iter()
        .flatten()
        .all(|namespace| namespace.starts_with("skiff_pt_")));
    assert!(
        namespaces_a
            .iter()
            .flatten()
            .all(|namespace| !namespaces_a_repeat
                .iter()
                .flatten()
                .any(|next| next == namespace)),
        "reusing a diagnostic run scope must not reuse a database namespace"
    );
    assert!(namespaces_a
        .iter()
        .flatten()
        .all(|namespace| !namespaces_b.iter().flatten().any(|next| next == namespace)));
    assert_eq!(
        run_a
            .entrypoints
            .iter()
            .map(|entrypoint| {
                (
                    &entrypoint.gateway_entry_key,
                    &entrypoint.gateway_entry_identity,
                    &entrypoint.selector,
                    entrypoint.mode,
                )
            })
            .collect::<Vec<_>>(),
        run_a_repeat
            .entrypoints
            .iter()
            .map(|entrypoint| {
                (
                    &entrypoint.gateway_entry_key,
                    &entrypoint.gateway_entry_identity,
                    &entrypoint.selector,
                    entrypoint.mode,
                )
            })
            .collect::<Vec<_>>(),
        "case gateway key, identity, selector and mode remain deterministic"
    );
}

#[test]
fn test_overlay_resolves_public_private_and_test_local_roots_in_one_compilation() {
    let root = TestRoot::new("overlay-root-paths");
    let artifacts = root.child("artifacts");
    let package = root.child("package");
    create_store(&artifacts);
    write_package(
        &package,
        "id: example.com/overlay-roots\nversion: 1.0.0\n",
        Some("public: main.publicHelper\n"),
        Some(
            "function publicHelper() -> bool { return true }\n\
             function privateHelper() -> bool { return true }\n",
        ),
    );
    fs::write(
        package.join("main.test.skiff"),
        "function testLocalHelper() -> bool { return true }\n\
         test \"root visibility\" {\n\
           assert root.main.publicHelper()\n\
           assert root.main.privateHelper()\n\
           assert root.main.__test.testLocalHelper()\n\
         }\n",
    )
    .unwrap();
    let project = compile_package_project(&platform_sources(), &package, &artifacts).unwrap();
    let production =
        skiff_artifact_identity::package_artifact_ref(&project.package.artifact).unwrap();
    let cases = discover_package_test_cases(&package, &package, false).unwrap();
    let overlay =
        compile_package_test_overlay(&platform_sources(), &package, &artifacts, &project, &cases)
            .unwrap();

    assert_eq!(overlay.production, production);
    assert!(overlay.overlay.artifact.package_requirements.is_empty());
}

#[test]
fn test_overlay_missing_root_target_fails_closed_without_self_dependency_fallback() {
    let root = TestRoot::new("overlay-missing-root");
    let artifacts = root.child("artifacts");
    let package = root.child("package");
    create_store(&artifacts);
    write_package(
        &package,
        "id: example.com/overlay-missing\nversion: 1.0.0\n",
        None,
        Some("function privateHelper() -> bool { return true }\n"),
    );
    fs::write(
        package.join("main.test.skiff"),
        "test \"missing root\" { assert root.main.missingHelper() }\n",
    )
    .unwrap();
    let project = compile_package_project(&platform_sources(), &package, &artifacts).unwrap();
    let cases = discover_package_test_cases(&package, &package, false).unwrap();
    let error =
        compile_package_test_overlay(&platform_sources(), &package, &artifacts, &project, &cases)
            .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("root.main.missingHelper"), "{message}");
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
            platform_sources: platform_sources(),
            runtime_artifact_root: Some(runtime),
            base_assembly: None,
            activation_url: Some("http://127.0.0.1:9/__skiff/activate-assembly".to_string()),
            ingress_url: Some("http://127.0.0.1:9".to_string()),
            target_environment: "nested-runtime-root".to_string(),
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
services:
  - id: example.com/payments
    version: 1.0.0
    alias: payments
"#,
        Some("run: main.run\n"),
        Some("function run(input: string) -> string { return payments/echo(input) }\n"),
    );
    fs::write(
        package.join("main.test.skiff"),
        "test \"needs provider\" { assert true }\n",
    )
    .unwrap();
    let project = compile_package_project(&platform_sources(), &package, &artifacts)
        .expect("consumer package");
    let cases = discover_package_test_cases(&package, &package, false).unwrap();
    let overlay =
        compile_package_test_overlay(&platform_sources(), &package, &artifacts, &project, &cases)
            .unwrap();
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
    assert!(!reasons.contains(&BoundaryUnavailableReason::RequiresSameHeapIdentity));
    assert!(!reasons.contains(&BoundaryUnavailableReason::UnknownEffect));
    assert!(!reasons.contains(&BoundaryUnavailableReason::UnknownCallTarget));
}

#[test]
fn caller_identity_comparisons_remain_boundary_unavailable_but_fresh_comparison_is_available() {
    let root = TestRoot::new("same-heap-identity");
    let artifacts = root.child("artifacts");
    let package = root.child("package");
    create_store(&artifacts);
    write_package(
        &package,
        "id: example.com/same-heap\nversion: 1.0.0\n",
        Some("Box: main.Box\nsame: main.same\nnotSame: main.notSame\nfresh: main.fresh\n"),
        Some(
            r#"type Box { value: string }

function same(input: Box) -> bool {
  return input == input
}

function notSame(input: Box) -> bool {
  return input != input
}

function fresh() -> bool {
  const left = Box { value: "left" }
  const right = Box { value: "right" }
  return left == right
}
"#,
        ),
    );
    let project = compile_package_project(&platform_sources(), &package, &artifacts).unwrap();

    for public_path in ["same", "notSame"] {
        let PackageLocalAbiSymbol::Callable { callable_id, .. } =
            &project.package.artifact.package_local_abi.public_symbols[public_path]
        else {
            panic!("{public_path} must be callable")
        };
        assert_eq!(
            project.package.artifact.callable_semantic_facts[callable_id].effects,
            CallableEffectSummary::Analyzed {
                effects: CallableMayEffects {
                    writes_caller_reachable: false,
                    returns_caller_alias: false,
                    throws_caller_alias: false,
                    escapes_caller_value: false,
                    requires_same_heap_identity: true,
                    invokes_unknown_target: false,
                    may_suspend: false,
                }
            },
            "{public_path}"
        );
        let BoundaryCallableProjection::Unavailable { reasons } =
            &project.package.artifact.boundary_projections[callable_id]
        else {
            panic!("{public_path} must remain boundary unavailable")
        };
        assert!(
            reasons.contains(&BoundaryUnavailableReason::RequiresSameHeapIdentity),
            "{public_path}: {reasons:?}"
        );
        assert!(
            !reasons.contains(&BoundaryUnavailableReason::UnknownEffect)
                && !reasons.contains(&BoundaryUnavailableReason::UnknownCallTarget),
            "{public_path}: {reasons:?}"
        );
    }

    public_operation_projection(&project, "fresh");
}

#[test]
fn fresh_helper_mutation_then_detached_service_call_projects_and_assembles() {
    let BaseAssemblyScenario {
        _root,
        artifacts,
        consumer,
        test_service,
        helper_package,
        payments_contract,
        provider_deployment,
        base,
        ..
    } = create_base_assembly_scenario();
    let project = compile_package_project(&platform_sources(), &consumer, &artifacts).unwrap();
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

    let test_project =
        compile_package_project_for_test(&platform_sources(), &test_service, &artifacts).unwrap();
    let cases = discover_package_test_cases(&test_service, &test_service, false).unwrap();
    assert_eq!(cases.len(), 4);
    let overlay = compile_package_test_overlay(
        &platform_sources(),
        &test_service,
        &artifacts,
        &test_project,
        &cases,
    )
    .unwrap();
    assert!(matches!(
        overlay
            .overlay
            .artifact
            .boundary_projections
            .get(&overlay.bindings[0].callable_id)
            .expect("final business assertion boundary projection"),
        BoundaryCallableProjection::Available { .. }
    ));
    let fixture = assemble_package_test_fixture(&test_project, overlay, base).unwrap();
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
fn ecosystem_http_fixture_uses_two_gateway_entries_without_ws_compat() {
    const PACKAGE_TEST_IDENTITY: &str = concat!(
        "skiff-gateway-entry-v2:sha256:",
        "b97af7d9ff0b9ddbfcb6ea8b19e6173722095c99f1566ccd6b1a6fd2ead3f305"
    );
    const PROBE_IDENTITY: &str = concat!(
        "skiff-gateway-entry-v2:sha256:",
        "94d4fb9ed499a8e4717ac6a46eb716a4595445573808f2543b7ea5aeefe83705"
    );

    let root = TestRoot::new("smoke-unary");
    let artifacts = root.child("artifacts");
    let package = root.child("package");
    create_store(&artifacts);
    write_package(
        &package,
        "id: example.com/smoke\nversion: 1.0.0\n",
        Some("marker: main.marker\n"),
        Some(
            r#"function marker() -> string { return "A" }

function __skiffHttpProbe(body: null) -> string {
  return marker()
}
"#,
        ),
    );
    fs::write(
        package.join("main.test.skiff"),
        "test \"smoke\" { assert true }\n",
    )
    .unwrap();
    let project = compile_package_project(&platform_sources(), &package, &artifacts).unwrap();
    let production =
        skiff_artifact_identity::package_artifact_ref(&project.package.artifact).unwrap();
    let cases = discover_package_test_cases(&package, &package, false).unwrap();
    let overlay =
        compile_package_test_overlay(&platform_sources(), &package, &artifacts, &project, &cases)
            .unwrap();
    let fixture = assemble_ecosystem_smoke_fixture(&project, overlay).unwrap();
    assert_eq!(fixture.production, production);
    assert_eq!(fixture.package_test.gateway_entry_key.as_str(), "run");
    assert_eq!(
        fixture.package_test.gateway_entry_identity.as_str(),
        PACKAGE_TEST_IDENTITY
    );
    assert_eq!(fixture.unary.selector.path, "/probe");
    assert_eq!(fixture.unary.gateway_entry_key.as_str(), "probe");
    assert_eq!(
        fixture.unary.gateway_entry_identity.as_str(),
        PROBE_IDENTITY
    );
    assert_eq!(fixture.unary.mode, GatewayDispatchMode::Unary);
    assert_eq!(fixture.records.assembly.roots.len(), 2);
    assert_eq!(fixture.records.deployments.len(), 2);
    assert_eq!(fixture.records.contracts.len(), 2);
    assert_eq!(fixture.records.assembly.gateway_ingress.len(), 2);
    assert!(fixture
        .records
        .contracts
        .iter()
        .all(|contract| contract.operations.is_empty()
            && contract.package_type_requirements.is_empty()));
    assert!(fixture.records.deployments.iter().all(|deployment| {
        deployment.operation_bindings.is_empty()
            && deployment.gateway_entries.len() == 1
            && deployment.ingress.len() == 1
    }));
    assert!(fixture
        .records
        .assembly
        .gateway_ingress
        .iter()
        .all(|binding| binding.selector.protocol == IngressProtocol::Http));
    assert!(!fixture
        .records
        .assembly
        .gateway_ingress
        .iter()
        .any(|binding| binding.selector.path == "/socket"));
    assert_eq!(fixture.records.assembly.resolved_packages.len(), 2);
}

#[test]
fn i02_submit_probe_is_private_http_gateway_not_service_operation() {
    const PROBE_IDENTITY: &str = concat!(
        "skiff-gateway-entry-v2:sha256:",
        "94d4fb9ed499a8e4717ac6a46eb716a4595445573808f2543b7ea5aeefe83705"
    );

    let root = TestRoot::new("i02-spawn-submit-effects");
    let artifacts = root.child("artifacts");
    create_store(&artifacts);
    seed_canonical_std(&platform_sources(), &artifacts).unwrap();
    let package =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/package-service-i02-spawn-submit");
    let source = fs::read_to_string(package.join("main.skiff")).unwrap();
    assert!(
        source.contains(
            "function __skiffHttpProbe(body: null) -> string {\n  return submitSpawnReceipt()\n}"
        ),
        "I02 private HTTP wrapper must call the callable selected by marker: main.submitSpawnReceipt"
    );
    assert!(!fs::read_to_string(package.join("api.yml"))
        .unwrap()
        .contains("__skiffHttpProbe"));
    assert_current_websocket_test_service(&package, "test.skiff/package-service-i02-spawn-submit");
    let project =
        compile_package_project_for_test(&platform_sources(), &package, &artifacts).unwrap();
    let production =
        skiff_artifact_identity::package_artifact_ref(&project.package.artifact).unwrap();

    assert_eq!(
        project
            .package
            .artifact
            .package_local_abi
            .public_symbols
            .len(),
        1
    );
    let marker = public_operation_projection(&project, "marker");
    assert!(marker.effect_guarantee.no_caller_reachable_mutation);
    let PackageLocalAbiSymbol::Callable {
        callable_id: marker_callable_id,
        ..
    } = &project.package.artifact.package_local_abi.public_symbols["marker"]
    else {
        panic!("marker must be a concrete callable")
    };
    let CallableEffectSummary::Analyzed {
        effects: marker_effects,
    } = &project.package.artifact.callable_semantic_facts[marker_callable_id].effects
    else {
        panic!("marker effects must be analyzed")
    };
    assert!(marker_effects.may_suspend);

    let cases = discover_package_test_cases(&package, &package, false).unwrap();
    assert_eq!(cases.len(), 1);
    let overlay =
        compile_package_test_overlay(&platform_sources(), &package, &artifacts, &project, &cases)
            .unwrap();
    let fixture = assemble_ecosystem_smoke_fixture(&project, overlay).unwrap();
    assert_eq!(fixture.production, production);
    assert_eq!(fixture.unary.gateway_entry_key.as_str(), "probe");
    assert_eq!(
        fixture.unary.gateway_entry_identity.as_str(),
        PROBE_IDENTITY
    );
    assert_eq!(fixture.records.assembly.gateway_ingress.len(), 2);
    assert!(fixture
        .records
        .assembly
        .gateway_ingress
        .iter()
        .all(|binding| binding.selector.protocol == IngressProtocol::Http));
    let smoke_deployment = fixture
        .records
        .deployments
        .iter()
        .find(|deployment| {
            skiff_artifact_identity::service_deployment_ref(deployment) == fixture.unary.deployment
        })
        .expect("I02 smoke deployment");
    assert!(smoke_deployment.operation_bindings.is_empty());
    assert_eq!(smoke_deployment.gateway_entries.len(), 1);
    let handler = &smoke_deployment.gateway_entries[&fixture.unary.gateway_entry_key].handler;
    let PackageLocalAbiSymbol::Callable {
        callable_id: wrapper,
        ..
    } = &project
        .package
        .artifact
        .package_local_abi
        .implementation_symbols["main.__skiffHttpProbe"]
    else {
        panic!("I02 private HTTP wrapper must compile as a callable")
    };
    assert_eq!(handler.as_ref(), Some(wrapper));
    assert!(!project
        .package
        .artifact
        .package_local_abi
        .public_symbols
        .values()
        .any(|symbol| matches!(
            symbol,
            PackageLocalAbiSymbol::Callable { callable_id, .. } if callable_id == wrapper
        )));
    let smoke_contract = fixture
        .records
        .contracts
        .iter()
        .find(|contract| {
            skiff_artifact_identity::service_contract_ref(contract).unwrap()
                == smoke_deployment.contract
        })
        .expect("I02 zero-operation smoke contract");
    assert!(smoke_contract.operations.is_empty());
    assert!(smoke_contract.package_type_requirements.is_empty());
}

#[test]
fn ecosystem_http_private_wrappers_compile_for_all_owned_source_fixtures() {
    let root = TestRoot::new("ecosystem-http-source-wrappers");
    let artifacts = root.child("artifacts");
    create_store(&artifacts);
    let std = seed_canonical_std(&platform_sources(), &artifacts).unwrap();
    let std_artifact = CanonicalArtifactStore::open(&artifacts)
        .unwrap()
        .read_package_artifact(&std.package.artifact)
        .unwrap()
        .as_ref()
        .clone();
    for (
        fixture_name,
        service_id,
        expected_api,
        expected_call,
        expected_build,
        expected_abi,
        expected_deployment,
        expected_assembly,
    ) in [
        (
            "package-service-websocket-smoke",
            "test.skiff/package-service-websocket-smoke",
            "marker: main.marker\n",
            "return marker()",
            "skiff-package-build-v10:sha256:8b2040cc626b711035fb1981698af641960a1a61eba8e4a788a1da22cc0c0c32",
            "skiff-package-local-abi-v7:sha256:d5627a25f7edd95d81505910f4d86f89434f2eff3837475ebf9e2b31f257b9ba",
            "skiff-deployment-artifact-v4:sha256:66c3b0dec0b771edf36d6f5a51e800989d2d46449722723beff128843da516e9",
            "skiff-runtime-assembly-v3:sha256:8b2d4a7f67ac024598fca0c7e8cd8f7a06a7cb05eaf88db8509be822ecc2bbfa",
        ),
        (
            "package-service-websocket-generation-a",
            "test.skiff/package-service-websocket-smoke",
            "marker: main.marker\n",
            "return marker()",
            "skiff-package-build-v10:sha256:edd5d2e760040d3a63ea776e461de1c62d38fe013467f9c417945d1f2a94d472",
            "skiff-package-local-abi-v7:sha256:d5627a25f7edd95d81505910f4d86f89434f2eff3837475ebf9e2b31f257b9ba",
            "skiff-deployment-artifact-v4:sha256:cdca3dc670c0099ca89d94f45d2a879f8b660147a87db222cd91eb7b3b361605",
            "skiff-runtime-assembly-v3:sha256:5f92066b34ff1eb13ada3fcf18506b9a1a622927397456ac2561dbf52c043a84",
        ),
        (
            "package-service-websocket-generation-b",
            "test.skiff/package-service-websocket-smoke",
            "marker: main.marker\n",
            "return marker()",
            "skiff-package-build-v10:sha256:1e75d027d0703be8296f8525c7d7fed60543d57c76852036e5427dbb2acc62cb",
            "skiff-package-local-abi-v7:sha256:d5627a25f7edd95d81505910f4d86f89434f2eff3837475ebf9e2b31f257b9ba",
            "skiff-deployment-artifact-v4:sha256:a2fef6e0bdbb22873797b80cf81e711ed50faaa219d09d1b48d81d6bed3e9057",
            "skiff-runtime-assembly-v3:sha256:8ba30a03d659f95ee0c6a20a1f5bccfaf3fd25068592a62fc175c4080e6174c9",
        ),
        (
            "package-service-i02-spawn-submit",
            "test.skiff/package-service-i02-spawn-submit",
            "marker: main.submitSpawnReceipt\n",
            "return submitSpawnReceipt()",
            "skiff-package-build-v10:sha256:66c1d911eedb821c27e4c8433189d432652ea875999ccb245dee55a20595d08d",
            "skiff-package-local-abi-v7:sha256:3db7056f815676834489b34a069b5016f05973b3be9379eb55736a545d7dcdf9",
            "skiff-deployment-artifact-v4:sha256:8041663d03286a3819d7d87ddff8d83f80dc8e5bbfc82ea61cf243ec05aa3690",
            "skiff-runtime-assembly-v3:sha256:d85a50a47063508f1e90548a6b6373e49cf129ac20118a50d679c9b382548998",
        ),
    ] {
        let package = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(fixture_name);
        let source = fs::read_to_string(package.join("main.skiff")).unwrap();
        assert!(
            source.contains("function __skiffHttpProbe(body: null) -> string")
                && source.contains(expected_call),
            "{fixture_name} must carry the exact private HTTP wrapper"
        );
        assert_eq!(
            fs::read_to_string(package.join("api.yml")).unwrap(),
            expected_api,
            "{fixture_name} must publish only its real business marker surface"
        );
        assert_current_websocket_test_service(&package, service_id);
        let project = compile_package_project_for_test(
            &platform_sources(),
            &package,
            &artifacts,
        )
        .unwrap();
        let production =
            skiff_artifact_identity::package_artifact_ref(&project.package.artifact).unwrap();
        let (generated_package, generated_contract, generated_deployment) =
            generate_current_websocket_service_fixture(&package, &artifacts, &std_artifact);
        assert_eq!(
            skiff_artifact_identity::package_artifact_ref(&generated_package).unwrap(),
            production,
            "{fixture_name} service and test compilation must select the same package artifact"
        );
        assert!(generated_contract.operations.is_empty());
        assert!(generated_deployment.operation_bindings.is_empty());
        assert_eq!(generated_deployment.gateway_entries.len(), 1);
        assert_eq!(generated_deployment.ingress.len(), 1);
        let websocket_key = GatewayEntryKey::parse("websocket").unwrap();
        let websocket = &generated_deployment.gateway_entries[&websocket_key];
        assert_eq!(
            websocket.gateway_entry_identity.as_str(),
            concat!(
                "skiff-gateway-entry-v2:sha256:",
                "f385624021966bab998385e1fd2c88804b51992f15f9c9d76c05d3e17a75018d"
            )
        );
        assert!(matches!(
            websocket.protocol_surface.protocol,
            GatewayProtocolSurface::WebSocketConnect(_)
        ));
        assert_eq!(
            websocket.adapter_plan.args,
            [
                skiff_artifact_model::GatewayAdapterArg {
                    param: "request".to_string(),
                    source: GatewayAdapterSource::WebSocketConnectRequest,
                },
                skiff_artifact_model::GatewayAdapterArg {
                    param: "connectionId".to_string(),
                    source: GatewayAdapterSource::WebSocketConnectionId,
                },
            ]
        );
        let PackageLocalAbiSymbol::Callable {
            callable_id: connect_callable,
            ..
        } = &generated_package.package_local_abi.implementation_symbols["main.websocketConnect"]
        else {
            panic!("{fixture_name} connect target must compile as a private callable")
        };
        assert_eq!(websocket.handler.as_ref(), Some(connect_callable));
        assert_eq!(
            generated_deployment.ingress[0].selector.protocol,
            IngressProtocol::WebSocket
        );
        assert_eq!(generated_deployment.ingress[0].selector.path, "/socket");
        assert_eq!(
            generated_deployment.ingress[0].gateway_entry_key,
            websocket_key
        );
        let deployment = skiff_artifact_identity::service_deployment_ref(&generated_deployment);
        let assembly = resolve_runtime_assembly(
            std::slice::from_ref(&deployment),
            std::slice::from_ref(&generated_deployment),
            std::slice::from_ref(&generated_contract),
            &[generated_package, std_artifact.clone()],
        )
        .unwrap();
        assert_eq!(
            (
                production.package_build_id.as_str(),
                production.package_local_abi_identity.as_str(),
                deployment.deployment_artifact_identity.as_str(),
                skiff_artifact_identity::runtime_assembly_ref(&assembly)
                    .unwrap()
                    .assembly_identity
                    .as_str(),
            ),
            (
                expected_build,
                expected_abi,
                expected_deployment,
                expected_assembly,
            ),
            "{fixture_name} compiler-generated identity tuple must remain exact",
        );
        assert!(project
            .package
            .artifact
            .package_local_abi
            .implementation_symbols
            .contains_key("main.__skiffHttpProbe"));
        assert!(!project
            .package
            .artifact
            .package_local_abi
            .public_symbols
            .contains_key("__skiffHttpProbe"));
        let cases = discover_package_test_cases(&package, &package, false).unwrap();
        let overlay = compile_package_test_overlay(
            &platform_sources(),
            &package,
            &artifacts,
            &project,
            &cases,
        )
        .unwrap();
        let fixture = assemble_ecosystem_smoke_fixture(&project, overlay).unwrap();
        assert_eq!(fixture.production, production);
        assert_eq!(
            fixture.unary.gateway_entry_identity.as_str(),
            concat!(
                "skiff-gateway-entry-v2:sha256:",
                "94d4fb9ed499a8e4717ac6a46eb716a4595445573808f2543b7ea5aeefe83705"
            )
        );
    }
}

#[test]
fn canonical_websocket_fixtures_use_split_external_manifests() {
    const WEBSOCKET: &str = r#"path: /socket
connect:
  handler: main.websocketConnect
  adapterArgs:
    - param: request
      source: { kind: websocket.connectRequest }
    - param: connectionId
      source: { kind: websocket.connectionId }
"#;
    for (fixture_name, service_id) in [
        (
            "package-service-websocket-smoke",
            "test.skiff/package-service-websocket-smoke",
        ),
        (
            "package-service-websocket-generation-a",
            "test.skiff/package-service-websocket-smoke",
        ),
        (
            "package-service-websocket-generation-b",
            "test.skiff/package-service-websocket-smoke",
        ),
        (
            "package-service-i02-spawn-submit",
            "test.skiff/package-service-i02-spawn-submit",
        ),
    ] {
        let package = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(fixture_name);
        assert_eq!(
            fs::read_to_string(package.join("service.yml")).unwrap(),
            format!("id: {service_id}\nkind: test\n"),
            "{fixture_name} service.yml must only declare service role"
        );
        assert_eq!(
            fs::read_to_string(package.join("websocket.yml")).unwrap(),
            WEBSOCKET,
            "{fixture_name} websocket.yml must own the exact connect surface"
        );
        assert_current_websocket_test_service(&package, service_id);
    }
}

fn assert_current_websocket_test_service(package: &Path, expected_service_id: &str) {
    let root = read_service_package_root(package)
        .unwrap_or_else(|error| panic!("{} service authoring: {error}", package.display()));
    assert_eq!(root.service.id, expected_service_id);
    assert_eq!(root.service.kind, ServiceAuthoringKind::Test);
    let websocket = root.websocket.expect("current singleton WebSocket entry");
    assert_eq!(websocket.path, "/socket");
    assert!(
        websocket.json_rpc.is_empty(),
        "connect-only canonical fixtures must not invent JSON-RPC methods"
    );
    let connect = websocket.connect.expect("private connect target");
    assert_eq!(connect.handler, "main.websocketConnect");
    assert_eq!(
        connect
            .adapter_args
            .iter()
            .map(|argument| (argument.param.as_str(), argument.source))
            .collect::<Vec<_>>(),
        [
            ("request", GatewayAdapterSource::WebSocketConnectRequest),
            ("connectionId", GatewayAdapterSource::WebSocketConnectionId),
        ]
    );
}

fn generate_current_websocket_service_fixture(
    package: &Path,
    artifacts: &Path,
    std: &PackageArtifact,
) -> (PackageArtifact, ServiceContract, ServiceDeployment) {
    let manifest = read_user_package_manifest(&package.join("package.yml")).unwrap();
    let raw_sources = read_package_sources(&manifest, package).unwrap();
    let source_tree = raw_sources.source_tree();
    let source_graph =
        PublicationSourceGraph::parse_raw_publication_sources(&raw_sources.into_source_graph())
            .unwrap();
    let resources = read_publication_resources(package, &manifest.resources).unwrap();
    let source = PackageSourceInput::new(
        manifest.publication.clone(),
        source_tree,
        source_graph,
        resources,
    );
    let service = read_service_package_root(package).unwrap();
    let aliases = BTreeMap::new();
    let available = [std.clone()];
    let store = CanonicalArtifactStore::open(artifacts).unwrap();
    let compiled = compile_service_package(
        PackageCompileInput::new(&platform_sources(), &source, &aliases, manifest.id.as_str())
            .with_available_canonical_packages(&available)
            .with_canonical_artifact_store(&store)
            .for_test_service(),
        &service,
    )
    .unwrap();
    let profile = &service.config_profiles["skiff-test"].authoring;
    let deployment = generate_service_deployment(GeneratedServiceDeploymentInput {
        service: &service.service,
        http: service.http.as_ref(),
        websocket: service.websocket.as_ref(),
        profile_name: "skiff-test",
        profile,
        service_api: &compiled.service_api,
        implementation: &compiled.package.artifact,
        package_closure: &available,
        package_schema_records: &compiled.package.resolved_package_schema_type_records,
    })
    .unwrap();
    (
        compiled.package.artifact,
        compiled.service_api.contract,
        deployment,
    )
}

fn public_operation_projection<'a>(
    project: &'a skiff_test_runner::canonical_package::CanonicalPackageProject,
    public_path: &str,
) -> &'a skiff_artifact_model::BoundaryOperationContract {
    let PackageLocalAbiSymbol::Callable { callable_id, .. } = project
        .package
        .artifact
        .package_local_abi
        .public_symbols
        .get(public_path)
        .unwrap_or_else(|| panic!("missing public callable {public_path}"))
    else {
        panic!("{public_path} must be callable")
    };
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = &project.package.artifact.boundary_projections[callable_id]
    else {
        panic!("{public_path} must remain boundary available")
    };
    operation_contract
}

struct BaseAssemblyScenario {
    _root: TestRoot,
    artifacts: PathBuf,
    runtime: PathBuf,
    consumer: PathBuf,
    test_service: PathBuf,
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
    let fixture_root = root.child("package-service-host");
    copy_tree(&package_service_host_fixture_root(), &fixture_root);
    let consumer = fixture_root.join("consumer");
    let test_service = fixture_root.join("consumer-tests");
    let receipt = prepare_package_service_host_fixture(
        &platform_sources(),
        &fixture_root,
        &root.child("authoring"),
        &artifacts,
        "base-test",
    )
    .unwrap();
    let receipt_json = receipt.to_json();
    assert_eq!(
        receipt_json["schemaVersion"],
        PACKAGE_SERVICE_HOST_FIXTURE_SCHEMA_VERSION
    );
    assert_json_keys(
        &receipt_json,
        &[
            "baseAssembly",
            "contracts",
            "deployments",
            "environment",
            "packages",
            "schemaVersion",
        ],
    );
    assert_json_keys(&receipt_json["contracts"], &["consumer", "payments"]);
    assert_json_keys(
        &receipt_json["packages"],
        &["consumer", "helper", "provider"],
    );
    assert_json_keys(&receipt_json["deployments"], &["consumer", "provider"]);
    assert_json_keys(&receipt_json["baseAssembly"], &["assemblyIdentity"]);
    assert_eq!(
        receipt.provider_package.package_id, "example.com/payments",
        "a service implementation is published by the package with the same canonical id"
    );
    assert_eq!(
        receipt.payments_contract.service_id, "example.com/payments",
        "the provider package and its service contract must share their canonical id"
    );
    let payments_contract = CanonicalArtifactStore::open(&artifacts)
        .unwrap()
        .read_service_contract(&receipt.payments_contract)
        .unwrap();
    assert_eq!(
        payments_contract.operations.len(),
        1,
        "the direct provider fixture must publish exactly one contract operation"
    );
    let operation_id = payments_contract
        .operations
        .keys()
        .next()
        .expect("the exact echo contract operation");
    assert_eq!(
        payments_contract
            .diagnostic_text
            .operations
            .get(operation_id)
            .map(String::as_str),
        Some("echo")
    );
    let base = CanonicalBaseAssembly::load(
        &artifacts,
        Some(receipt.base_assembly.assembly_identity.as_str()),
    )
    .unwrap();
    assert!(base.deployments.iter().any(
        |deployment| skiff_artifact_identity::service_deployment_ref(deployment)
            == receipt.provider_deployment
    ));
    BaseAssemblyScenario {
        _root: root,
        artifacts,
        runtime,
        consumer,
        test_service,
        helper_package: receipt.helper_package,
        payments_contract: receipt.payments_contract,
        provider_deployment: receipt.provider_deployment,
        consumer_deployment: receipt.consumer_deployment,
        base_assembly_ref: receipt.base_assembly,
        base,
    }
}

fn publish_package(root: &Path, artifacts: &Path) -> PackageArtifactRef {
    let output = build_authoring_object(
        &platform_sources(),
        AuthoringObject::Package,
        root,
        artifacts,
        "dev",
        true,
    )
    .expect("production package authoring should publish pointer and records");
    serde_json::from_value(output["packageArtifactReceipt"]["artifact"].clone())
        .expect("typed package authoring receipt")
}

fn publish_contract(artifacts: &Path) -> ServiceContractRef {
    let work = TestRoot::new("service-contract");
    let source = package_service_host_fixture_root().join("provider");
    let provider = work.child("provider");
    copy_tree(&source, &provider);
    fs::write(
        provider.join("config.dev.yml"),
        "timeout: 1000\nquota: { cpuMillis: 100, memoryBytes: 1048576 }\nlifecycle: { maxConcurrency: 1 }\nprincipal: service:provider\n",
    )
    .unwrap();
    let output = build_authoring_object(
        &platform_sources(),
        AuthoringObject::Package,
        &provider,
        artifacts,
        "dev",
        true,
    )
    .expect("production contract authoring should publish pointer and record");
    serde_json::from_value(output["serviceContractReceipt"]["contract"].clone())
        .expect("typed contract authoring receipt")
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&source_path, &target_path);
        } else {
            fs::copy(source_path, target_path).unwrap();
        }
    }
}

#[test]
fn recursive_copy_tree_receipt_preserves_external_control_files() {
    let root = TestRoot::new("external-control-copy-receipt");
    let source = root.child("source");
    let target = root.child("target");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("http.yml"), "probe: { method: GET }\n").unwrap();
    fs::write(source.join("nested/websocket.yml"), "path: /socket\n").unwrap();
    fs::write(
        source.join("nested/source.skiff"),
        "function marker() -> bool { return true }\n",
    )
    .unwrap();

    copy_tree(&source, &target);

    let receipt = ["http.yml", "nested/websocket.yml"]
        .map(|path| {
            (
                path,
                fs::read(source.join(path)).unwrap(),
                fs::read(target.join(path)).unwrap(),
            )
        })
        .to_vec();
    assert!(
        receipt
            .iter()
            .all(|(_, source_bytes, copied_bytes)| source_bytes == copied_bytes),
        "recursive copy receipt must retain exact external control-file bytes"
    );
    assert_eq!(read_tree(&source), read_tree(&target));
}

fn package_service_host_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/package-service-host")
}

fn platform_source_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("test-runner must live directly below the Skiff root")
        .to_path_buf()
}

fn platform_sources() -> CompilerPlatformSources {
    CompilerPlatformSources::new(&platform_source_root()).unwrap()
}

fn assert_json_keys(value: &serde_json::Value, expected: &[&str]) {
    let mut actual = value
        .as_object()
        .expect("receipt section must be an object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

fn write_package(root: &Path, manifest: &str, api: Option<&str>, source: Option<&str>) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("package.yml"), manifest).unwrap();
    fs::write(root.join("api.yml"), api.unwrap_or("{}\n")).unwrap();
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
