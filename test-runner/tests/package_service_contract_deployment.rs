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

use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryUnavailableReason, CallableEffectSummary,
    CallableMayEffects, CallableProvenanceSummary, GatewayAdapterKind, GatewayAdapterSource,
    GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey, GatewayExternalSchema,
    GatewayProtocolSurface, IngressProtocol, PackageArtifactRef, PackageLocalAbiSymbol,
    RuntimeAssemblyRef, ServiceContractRef, ServiceDeploymentRef, StateBindingKind,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources, ManifestOwner, ManifestProvenance, PackageSourceInput,
    PublicationManifest, PublicationSourceGraph, SourceTree,
};
use skiff_deployment::storage::CanonicalArtifactStore;
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
    let project = compile_package_project_for_test(
        &platform_sources,
        &test_service,
        &artifacts,
        "skiff-test",
    )
    .unwrap();
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
    let project = compile_package_project_for_test(
        &platform_sources(),
        &test_service,
        &artifacts,
        "skiff-test",
    )
    .unwrap();
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
fn test_service_environment_profile_projects_over_the_exact_package_closure() {
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
    access: topLevel
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
        service.join("main.test.skiff"),
        "test \"profile is projected\" { assert root.main.ownConfig() == \"test-value\" }\n",
    )
    .unwrap();

    let missing_profile =
        compile_package_project_for_test(&platform_sources(), &service, &artifacts, "other")
            .unwrap_err()
            .to_string();
    assert!(
        missing_profile.contains("requires config.other.yml"),
        "{missing_profile}"
    );

    let project =
        compile_package_project_for_test(&platform_sources(), &service, &artifacts, "skiff-test")
            .expect("kind:test must authorize topLevel and bind its environment profile");
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
        "skiff-gateway-entry-v1:sha256:",
        "cfcfced94f984612809ce837f81e975016b09f206925389d95e925e087fc32d4"
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
    assert_eq!(
        entrypoint.selector.host,
        "case-0.package-test.skiff.localhost"
    );
    assert_eq!(entrypoint.selector.method.as_deref(), Some("POST"));
    assert_eq!(entrypoint.selector.path, "/__skiff/package-test/0");
    let entry = &deployment.gateway_entries[&entrypoint.gateway_entry_key];
    assert_eq!(
        entry.gateway_entry_identity,
        entrypoint.gateway_entry_identity
    );
    assert_eq!(entry.handler, binding.gateway_callable_id);
    assert_eq!(entry.pre, None);
    assert_eq!(entry.guard, None);
    assert_eq!(entry.adapter_plan.kind, GatewayAdapterKind::TypedJson);
    assert_eq!(entry.adapter_plan.args.len(), 1);
    assert_eq!(entry.adapter_plan.args[0].param, "body");
    assert_eq!(
        entry.adapter_plan.args[0].source,
        GatewayAdapterSource::HttpBody
    );
    let GatewayProtocolSurface::Http(surface) = &entry.protocol_surface.protocol;
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
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v1:sha256:{}", "a".repeat(64)))
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

    let test_project = compile_package_project_for_test(
        &platform_sources(),
        &test_service,
        &artifacts,
        "skiff-test",
    )
    .unwrap();
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
        "skiff-gateway-entry-v1:sha256:",
        "cfcfced94f984612809ce837f81e975016b09f206925389d95e925e087fc32d4"
    );
    const PROBE_IDENTITY: &str = concat!(
        "skiff-gateway-entry-v1:sha256:",
        "adfaa17c077af0388f2b5751bbe4b9ba392ec647f5ce33022c8e8ec83eaf6653"
    );

    let root = TestRoot::new("smoke-unary");
    let artifacts = root.child("artifacts");
    let package = root.child("package");
    create_store(&artifacts);
    write_package(
        &package,
        "id: example.com/smoke\nversion: 1.0.0\n",
        Some("marker: main.marker\nwebsocket: main.websocket\n"),
        Some(
            r#"import std

function marker() -> string { return "A" }

function __skiffHttpProbe(body: null) -> string {
  return marker()
}

function acceptConnection() -> std.websocket.WebSocketConnectResult<null> {
  return {
    tag: "accept",
    context: null,
    businessIdentity: null,
    connectionPolicy: null
  }
}

function websocket(event: std.websocket.WebSocketIngressEvent<null>) -> std.websocket.WebSocketConnectResult<null>? {
  if event.tag == "connect" {
    return acceptConnection()
  }
  if event.tag == "receive" {
    std.websocket.sendTextToConnection(event.receiveEvent.connection.id, marker())
  }
  return null
}
"#,
        ),
    );
    fs::write(
        package.join("main.test.skiff"),
        "test \"smoke\" { assert true }\n",
    )
    .unwrap();
    let missing_std =
        compile_package_project(&platform_sources(), &package, &artifacts).unwrap_err();
    assert!(
        missing_std.to_string().contains(
            "references platform std, but the same compile graph has no canonical PackageArtifact"
        ),
        "{missing_std}"
    );
    let std = seed_canonical_std(&platform_sources(), &artifacts).unwrap();
    assert_eq!(
        std.package.artifact.package_build_id.as_str(),
        EXPECTED_STD_PACKAGE_BUILD_ID
    );
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
    assert_eq!(fixture.records.assembly.resolved_packages.len(), 3);
    assert!(fixture
        .records
        .assembly
        .resolved_packages
        .contains(&std.package.artifact));
}

#[test]
fn i02_submit_probe_is_private_http_gateway_not_service_operation() {
    const PROBE_IDENTITY: &str = concat!(
        "skiff-gateway-entry-v1:sha256:",
        "adfaa17c077af0388f2b5751bbe4b9ba392ec647f5ce33022c8e8ec83eaf6653"
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
    let project = compile_package_project(&platform_sources(), &package, &artifacts).unwrap();
    let production =
        skiff_artifact_identity::package_artifact_ref(&project.package.artifact).unwrap();

    assert_eq!(
        project
            .package
            .artifact
            .package_local_abi
            .public_symbols
            .len(),
        2
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
    assert_eq!(handler, wrapper);
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
    seed_canonical_std(&platform_sources(), &artifacts).unwrap();

    for (fixture_name, expected_call) in [
        ("package-service-websocket-smoke", "return marker()"),
        ("package-service-websocket-generation-a", "return marker()"),
        ("package-service-websocket-generation-b", "return marker()"),
        (
            "package-service-i02-spawn-submit",
            "return submitSpawnReceipt()",
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
        assert!(
            !fs::read_to_string(package.join("api.yml"))
                .unwrap()
                .contains("__skiffHttpProbe"),
            "{fixture_name} must not publish the private HTTP wrapper"
        );
        let project = compile_package_project(&platform_sources(), &package, &artifacts).unwrap();
        let production =
            skiff_artifact_identity::package_artifact_ref(&project.package.artifact).unwrap();
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
                "skiff-gateway-entry-v1:sha256:",
                "adfaa17c077af0388f2b5751bbe4b9ba392ec647f5ce33022c8e8ec83eaf6653"
            )
        );
    }
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
