use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use skiff_artifact_identity::{package_artifact_ref, service_contract_ref};
use skiff_artifact_model::{GatewayEntryKey, ServiceAuthoringKind};
use skiff_compiler::CompilerPlatformSources;
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_test_runner::{
    canonical_fixture::discover_test_service_cases,
    canonical_package::compile_package_project_for_test, canonical_std_seed::seed_canonical_std,
    test_service_fixture::assemble_test_service_fixture,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-test-service-flow-{}-{label}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test root");
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

fn platform_sources() -> CompilerPlatformSources {
    CompilerPlatformSources::new(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root"),
    )
    .expect("platform source root")
}

#[test]
fn kind_test_compiles_and_assembles_only_ordinary_service_artifacts() {
    let root = TestRoot::new("ordinary");
    let artifacts = root.path().join("artifacts");
    seed_canonical_std(&platform_sources(), &artifacts).expect("seed std");
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("package-service-websocket-smoke");

    let project = compile_package_project_for_test(&platform_sources(), &fixture_root, &artifacts)
        .expect("compile ordinary test service");
    let profile = project
        .test_service_profile
        .as_ref()
        .expect("kind:test profile");
    assert_eq!(
        profile.service_root.service.kind,
        ServiceAuthoringKind::Test
    );
    let service_api = project.service_api.as_ref().expect("ordinary service API");
    assert!(service_api.contract.operations.is_empty());
    assert_eq!(
        service_contract_ref(&service_api.contract)
            .expect("contract ref")
            .service_id,
        profile.service_id
    );

    let cases = discover_test_service_cases(&fixture_root, &fixture_root, false)
        .expect("discover test cases");
    assert_eq!(cases.len(), 1);
    let gateway_selector = format!("{}.{}Gateway", cases[0].module_path, cases[0].function_name);
    assert!(
        project
            .package
            .artifact
            .package_local_abi
            .implementation_symbols
            .contains_key(&gateway_selector),
        "missing {gateway_selector}; available symbols: {:?}",
        project
            .package
            .artifact
            .package_local_abi
            .implementation_symbols
            .keys()
            .collect::<Vec<_>>()
    );

    let fixture = assemble_test_service_fixture(&project, &cases, Default::default())
        .expect("assemble ordinary service cases");
    assert_eq!(
        fixture.test_service,
        package_artifact_ref(&project.package.artifact).expect("package ref")
    );
    assert_eq!(fixture.cases.len(), 1);
    let case = &fixture.cases[0];
    assert_eq!(case.records.packages.len(), 1);
    assert_eq!(case.records.contracts.len(), 1);
    assert_eq!(case.records.deployments.len(), 1);
    assert_eq!(case.records.assembly.roots.len(), 1);
    assert_eq!(
        case.entrypoint.gateway_entry_key,
        GatewayEntryKey::parse("run").unwrap()
    );
    assert_eq!(case.entrypoint.selector.path, "/__skiff/test/0");
    let deployment = &case.records.deployments[0];
    assert_eq!(case.contract.service_id, deployment.contract.service_id);
    assert!(case.contract.service_id.starts_with("test.skiff/p-"));
    assert_ne!(case.contract.service_id, profile.service_id);
    assert!(deployment
        .ingress
        .iter()
        .any(|binding| binding.selector.path == "/probe"));
    assert!(deployment
        .ingress
        .iter()
        .any(|binding| binding.selector.path == "/socket"));
}

#[test]
fn multiple_cases_receive_separate_deployments_and_assemblies() {
    let root = TestRoot::new("case-isolation");
    let artifacts = root.path().join("artifacts");
    let runtime_artifacts = root.path().join("runtime-artifacts");
    seed_canonical_std(&platform_sources(), &artifacts).expect("seed std");
    let service = root.path().join("service");
    fs::create_dir_all(&service).unwrap();
    fs::write(
        service.join("package.yml"),
        "id: test.skiff/case-isolation\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(service.join("api.yml"), "{}\n").unwrap();
    fs::write(
        service.join("service.yml"),
        "id: test.skiff/case-isolation\nkind: test\n",
    )
    .unwrap();
    fs::write(
        service.join("config.skiff-test.yml"),
        "timeout: 30000\nquota:\n  cpuMillis: 100\n  memoryBytes: 67108864\nprincipal: service:test.skiff/case-isolation\n",
    )
    .unwrap();
    fs::write(
        service.join("alpha.test.skiff"),
        "test \"first\" { assert true }\ntest \"second\" { assert true }\n",
    )
    .unwrap();
    fs::write(
        service.join("beta.test.skiff"),
        "test \"third\" { assert true }\n",
    )
    .unwrap();

    let project = compile_package_project_for_test(&platform_sources(), &service, &artifacts)
        .expect("compile test service");
    let cases = discover_test_service_cases(&service, &service, false).expect("discover all cases");
    assert_eq!(
        cases
            .iter()
            .map(|case| case.function_name.as_str())
            .collect::<Vec<_>>(),
        ["skiffTestCase0", "skiffTestCase1", "skiffTestCase0"]
    );
    let single_case_fixture =
        assemble_test_service_fixture(&project, &cases[..1], Default::default())
            .expect("assemble one isolated case");
    let fixture = assemble_test_service_fixture(&project, &cases, Default::default())
        .expect("assemble isolated cases");
    assert_eq!(
        fixture.package_identity_admission_count(),
        single_case_fixture.package_identity_admission_count(),
        "full PackageArtifact identity admissions depend on the unique closure, not case count"
    );
    assert_eq!(
        fixture.package_identity_admission_count(),
        project.artifacts().count(),
        "each unique fixture package is fully admitted exactly once"
    );
    assert_eq!(fixture.cases.len(), 3);
    for (index, case) in fixture.cases.iter().enumerate() {
        assert_eq!(case.records.deployments.len(), 1);
        assert_eq!(case.records.assembly.roots.len(), 1);
        assert_eq!(
            case.entrypoint.selector.path,
            format!("/__skiff/test/{index}")
        );
    }
    assert_ne!(
        fixture.cases[0].entrypoint.deployment,
        fixture.cases[1].entrypoint.deployment
    );
    assert_ne!(fixture.cases[0].contract, fixture.cases[1].contract);
    assert_ne!(
        fixture.cases[0].records.assembly.assembly_identity,
        fixture.cases[1].records.assembly.assembly_identity
    );
    fixture
        .publish(&artifacts, &runtime_artifacts)
        .expect("multi-case publish reuses admitted external package records");
    let runtime_store =
        CanonicalArtifactStore::open(&runtime_artifacts).expect("runtime artifact store");
    for case in &fixture.cases {
        runtime_store
            .read_runtime_assembly(&skiff_artifact_model::RuntimeAssemblyRef {
                assembly_identity: case.records.assembly.assembly_identity.clone(),
            })
            .expect("every case assembly remains independently published");
    }
}

#[test]
fn source_tests_without_kind_test_service_are_not_an_execution_surface() {
    let root = TestRoot::new("no-overlay-fallback");
    let artifacts = root.path().join("artifacts");
    seed_canonical_std(&platform_sources(), &artifacts).expect("seed std");
    let package = root.path().join("package");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.yml"),
        "id: example.com/legacy-overlay-only\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(package.join("api.yml"), "{}\n").unwrap();
    fs::write(
        package.join("main.skiff"),
        "function helper() -> boolean { return true }\n",
    )
    .unwrap();
    fs::write(
        package.join("main.test.skiff"),
        "test \"legacy overlay\" { assert helper() }\n",
    )
    .unwrap();

    let project = compile_package_project_for_test(&platform_sources(), &package, &artifacts)
        .expect("ordinary package production compile");
    assert!(project.test_service_profile.is_none());
    let cases =
        discover_test_service_cases(&package, &package, false).expect("discover legacy source");
    let error = assemble_test_service_fixture(&project, &cases, Default::default())
        .expect_err("runner must not recreate a package overlay")
        .to_string();
    assert!(error.contains("service.yml kind: test"));
}

#[path = "test_service_flow/base_assembly.rs"]
mod base_assembly;

#[path = "test_service_flow/fixture_compilation.rs"]
mod fixture_compilation;

#[path = "test_service_flow/profile_state.rs"]
mod profile_state;

#[path = "test_service_flow/roots.rs"]
mod roots;

#[path = "test_service_flow/runner_cli_contract.rs"]
mod runner_cli_contract;
