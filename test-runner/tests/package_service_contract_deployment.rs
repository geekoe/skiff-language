use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryCancellationContract, BoundaryUnavailableReason,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, IngressProtocol,
    MetadataValue, PackageArtifactRef, PackageConfigRequirement, PackageLocalAbiSymbol,
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
    canonical_package::compile_package_project,
    canonical_std_seed::seed_canonical_std,
    ecosystem_smoke_fixture::assemble_ecosystem_smoke_fixture,
    package_service_host_fixture::{
        prepare_package_service_host_fixture, PACKAGE_SERVICE_HOST_FIXTURE_SCHEMA_VERSION,
    },
    package_test_assembly::{
        assemble_package_test_fixture_for_run_with_config,
        assemble_package_test_fixture_with_config, CanonicalPackageTestFixture,
        PackageTestConfigLiteral,
    },
    run_skiff_tests_with_options,
    test_overlay::compile_package_test_overlay,
    SkiffTestError, SkiffTestOptions,
};

// Explicit identity regressions refreshed when c277e45 added the canonical
// std.websocket.WebSocketIngressEvent surface. Production code derives these
// identities from the F27A authoring receipt rather than these test pins.
const EXPECTED_PRELUDE_IDENTITY: &str =
    "skiff-prelude-v1:sha256:5166ba3c306e94624094e0736da821a1b653da5aace1ef8cee2fb654f4106699";
const EXPECTED_STD_PACKAGE_BUILD_ID: &str =
    "skiff-package-build-v4:sha256:4cf082e69e7b95f16494319f1a74bd0c1d6499f75ee45092bcabcb12241be24e";

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
    let runtime = root.child("runtime-artifacts");
    create_store(&artifacts);

    let platform_sources = platform_sources();
    let platform_root = platform_sources.std_dir().to_path_buf();
    let project = compile_package_project(&platform_sources, &platform_root, &artifacts).unwrap();
    assert_eq!(project.package.artifact.package_id, "skiff.run/std");
    assert!(project.dependency_packages.is_empty());

    let cases = discover_package_test_cases(&platform_root, &platform_root, false).unwrap();
    assert_eq!(cases.len(), 11, "the canonical std root must stay complete");
    let overlay = compile_package_test_overlay(
        &platform_sources,
        &platform_root,
        &artifacts,
        &project,
        &cases,
    )
    .unwrap();
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

    let fake_root = root.child("fake-reserved");
    fs::create_dir_all(&fake_root).unwrap();
    fs::copy(
        platform_root.join("package.yml"),
        fake_root.join("package.yml"),
    )
    .unwrap();
    let error = compile_package_project(&platform_sources, &fake_root, &artifacts).unwrap_err();
    assert!(error
        .to_string()
        .contains("package id skiff.run/std is reserved"));
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
        consumer,
        helper_package,
        payments_contract,
        provider_deployment,
        consumer_deployment,
        base_assembly_ref,
        base,
    } = create_base_assembly_scenario();

    let source_before_publish = read_tree(&artifacts);
    let project = compile_package_project(&platform_sources(), &consumer, &artifacts).unwrap();
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
    let overlay =
        compile_package_test_overlay(&platform_sources(), &consumer, &artifacts, &project, &cases)
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
fn test_config_literals_are_exact_typed_and_test_deployment_owned() {
    let BaseAssemblyScenario {
        _root,
        artifacts,
        consumer,
        base,
        ..
    } = create_base_assembly_scenario();
    let mut project = compile_package_project(&platform_sources(), &consumer, &artifacts).unwrap();
    project.dependency_packages[0]
        .runtime_requirements
        .config
        .push(PackageConfigRequirement {
            path: "helper.token".to_string(),
            value_type: "number".to_string(),
            required: true,
        });
    project.dependency_packages[0]
        .runtime_requirements
        .config
        .push(PackageConfigRequirement {
            path: "helper.optional".to_string(),
            value_type: "bool".to_string(),
            required: false,
        });
    skiff_artifact_identity::assign_package_artifact_identities(
        &mut project.dependency_packages[0],
    )
    .unwrap();
    skiff_artifact_identity::assign_package_artifact_identities(&mut project.package.artifact)
        .unwrap();
    let exact_dependency =
        skiff_artifact_identity::package_artifact_ref(&project.dependency_packages[0]).unwrap();
    let exact_production =
        skiff_artifact_identity::package_artifact_ref(&project.package.artifact).unwrap();
    let cases = discover_package_test_cases(&consumer, &consumer, false).unwrap();
    let probe_overlay =
        compile_package_test_overlay(&platform_sources(), &consumer, &artifacts, &project, &cases)
            .unwrap();
    let exact_package =
        skiff_artifact_identity::package_artifact_ref(&probe_overlay.overlay.artifact).unwrap();

    let assemble = |literals: &[PackageTestConfigLiteral]| {
        let overlay = compile_package_test_overlay(
            &platform_sources(),
            &consumer,
            &artifacts,
            &project,
            &cases,
        )
        .unwrap();
        assemble_package_test_fixture_with_config(&project, overlay, base.clone(), literals)
    };
    let required = PackageTestConfigLiteral {
        package: exact_production.clone(),
        key: "app.token".to_string(),
        value: MetadataValue::String("owned-by-base".to_string()),
    };
    let dependency_required = PackageTestConfigLiteral {
        package: exact_dependency.clone(),
        key: "helper.token".to_string(),
        value: MetadataValue::Number(7.into()),
    };
    let fixture = assemble(&[required.clone(), dependency_required.clone()]).unwrap();
    assert_eq!(
        fixture.records.deployments[0].config_literals,
        vec![
            skiff_artifact_model::ConfigLiteralBinding {
                path: "app.token".to_string(),
                value: MetadataValue::String("owned-by-base".to_string()),
            },
            skiff_artifact_model::ConfigLiteralBinding {
                path: "helper.token".to_string(),
                value: MetadataValue::Number(7.into()),
            },
        ]
    );

    let missing = assemble(&[]).unwrap_err().to_string();
    assert!(missing.contains("required test config literal helper.token"));
    let wrong_type = assemble(&[
        PackageTestConfigLiteral {
            value: MetadataValue::Bool(true),
            ..required.clone()
        },
        dependency_required.clone(),
    ])
    .unwrap_err()
    .to_string();
    assert!(wrong_type.contains("must be string"));
    let unknown = assemble(&[
        PackageTestConfigLiteral {
            key: "app.unknown".to_string(),
            ..required.clone()
        },
        dependency_required.clone(),
    ])
    .unwrap_err()
    .to_string();
    assert!(unknown.contains("unknown requirement"));
    let duplicate = assemble(&[
        required.clone(),
        PackageTestConfigLiteral {
            package: exact_package.clone(),
            ..required.clone()
        },
        dependency_required.clone(),
    ])
    .unwrap_err()
    .to_string();
    assert!(duplicate.contains("repeats exact package requirement"));

    let mut wrong_production = exact_production.clone();
    wrong_production.package_build_id = skiff_artifact_model::PackageBuildId::new(
        "skiff-package-build-v4:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    let wrong_production = assemble(&[
        PackageTestConfigLiteral {
            package: wrong_production,
            ..required.clone()
        },
        dependency_required.clone(),
    ])
    .unwrap_err()
    .to_string();
    assert!(wrong_production.contains("outside the exact deployment closure"));

    let mut wrong_overlay = exact_package;
    wrong_overlay.package_build_id = skiff_artifact_model::PackageBuildId::new(
        "skiff-package-build-v4:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );
    let wrong_overlay = assemble(&[
        PackageTestConfigLiteral {
            package: wrong_overlay,
            ..required.clone()
        },
        dependency_required.clone(),
    ])
    .unwrap_err()
    .to_string();
    assert!(wrong_overlay.contains("outside the exact deployment closure"));

    let with_optional = assemble(&[
        required,
        dependency_required,
        PackageTestConfigLiteral {
            package: exact_dependency,
            key: "helper.optional".to_string(),
            value: MetadataValue::Bool(true),
        },
    ])
    .unwrap();
    assert_eq!(
        with_optional.records.deployments[0].config_literals.len(),
        3
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
        assemble_package_test_fixture_for_run_with_config(
            &project,
            overlay,
            CanonicalBaseAssembly::default(),
            &[],
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
            .map(|entrypoint| &entrypoint.contract)
            .collect::<Vec<_>>(),
        run_a_repeat
            .entrypoints
            .iter()
            .map(|entrypoint| &entrypoint.contract)
            .collect::<Vec<_>>(),
        "case contract identity remains deterministic for diagnostics"
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
            test_config_literals: Vec::new(),
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

    let cases = discover_package_test_cases(&consumer, &consumer, false).unwrap();
    assert_eq!(cases.len(), 5);
    let overlay =
        compile_package_test_overlay(&platform_sources(), &consumer, &artifacts, &project, &cases)
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
        Some("marker: main.marker\nwebsocket: main.websocket\n"),
        Some(
            r#"import std

function marker() -> string { return "A" }

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
    assert_eq!(fixture.unary.selector.path, "/probe");
    let websocket = fixture
        .websocket
        .as_ref()
        .expect("canonical websocket public ABI should enter the real smoke fixture");
    assert_eq!(websocket.selector.protocol, IngressProtocol::WebSocket);
    assert_eq!(websocket.selector.path, "/socket");
    assert_eq!(fixture.records.assembly.roots.len(), 2);
    assert_eq!(fixture.records.deployments.len(), 2);
    assert_eq!(fixture.records.contracts.len(), 2);
    assert_eq!(fixture.records.assembly.resolved_packages.len(), 3);
    assert!(fixture
        .records
        .assembly
        .resolved_packages
        .contains(&std.package.artifact));
}

#[test]
fn i02_spawn_submit_fixture_splits_unary_and_websocket_effects() {
    let root = TestRoot::new("i02-spawn-submit-effects");
    let artifacts = root.child("artifacts");
    create_store(&artifacts);
    seed_canonical_std(&platform_sources(), &artifacts).unwrap();
    let package =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/package-service-i02-spawn-submit");
    let project = compile_package_project(&platform_sources(), &package, &artifacts).unwrap();

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
    assert!(marker.may_suspend);
    assert_eq!(
        marker.cancellation,
        BoundaryCancellationContract::Cooperative
    );
    let websocket = public_operation_projection(&project, "websocket");
    assert!(!websocket.may_suspend);
    assert_eq!(
        websocket.cancellation,
        BoundaryCancellationContract::NotCancellable
    );

    let cases = discover_package_test_cases(&package, &package, false).unwrap();
    assert_eq!(cases.len(), 1);
    let overlay =
        compile_package_test_overlay(&platform_sources(), &package, &artifacts, &project, &cases)
            .unwrap();
    let fixture = assemble_ecosystem_smoke_fixture(&project, overlay).unwrap();
    let websocket = fixture
        .websocket
        .as_ref()
        .expect("I02 websocket entrypoint");
    assert_eq!(fixture.unary.contract, websocket.contract);
    assert_eq!(fixture.unary.deployment, websocket.deployment);
    let smoke_contract = fixture
        .records
        .contracts
        .iter()
        .find(|contract| {
            skiff_artifact_identity::service_contract_ref(contract).unwrap()
                == fixture.unary.contract
        })
        .expect("I02 smoke contract");
    assert_eq!(smoke_contract.operations.len(), 2);
    assert_eq!(
        smoke_contract
            .operations
            .values()
            .filter(|descriptor| descriptor.contract.may_suspend)
            .count(),
        1
    );
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
    let fixture_root = package_service_host_fixture_root();
    let consumer = fixture_root.join("consumer");
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
