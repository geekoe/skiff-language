use std::collections::BTreeSet;

use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_test_runner::{
    canonical_fixture::discover_test_service_cases,
    canonical_package::compile_package_project_for_test,
    test_service_fixture::assemble_test_service_fixture_for_run_with_ingress,
};

use super::*;

#[test]
fn package_id_root_config_isolated_by_exact_case_deployment() {
    let root = TestRoot::new("config-snapshot");
    let artifacts = root.path().join("artifacts");
    let service = root.path().join("service");
    CanonicalArtifactStore::create(&artifacts).unwrap();
    write_configured_test_service(&service);

    let project = compile_package_project_for_test(&platform_sources(), &service, &artifacts)
        .expect("compile configured test service");
    let cases =
        discover_test_service_cases(&service, &service, false).expect("discover configured cases");
    assert_eq!(cases.len(), 3);

    let first = assemble_test_service_fixture_for_run_with_ingress(
        &project,
        &cases,
        Default::default(),
        "execution-one",
        "http://127.0.0.1:46100",
        "skiff-test",
    )
    .expect("assemble first execution");
    let second = assemble_test_service_fixture_for_run_with_ingress(
        &project,
        &cases,
        Default::default(),
        "execution-two",
        "http://127.0.0.1:46100",
        "skiff-test",
    )
    .expect("assemble second execution");

    assert_eq!(first.records.config_snapshot.deployments().len(), 3);
    assert_eq!(first.records.config_snapshot.environment(), "skiff-test");
    let first_deployments = first
        .records
        .config_snapshot
        .deployments()
        .iter()
        .map(|entry| entry.deployment().clone())
        .collect::<BTreeSet<_>>();
    let first_case_deployments = first
        .cases
        .iter()
        .map(|case| case.entrypoint.deployment.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(first_deployments, first_case_deployments);

    for deployment in first.records.config_snapshot.deployments() {
        let implementation = deployment
            .packages()
            .iter()
            .find(|package| {
                package.package_build_id() == &project.package.artifact.package_build_id
            })
            .expect("test implementation config partition");
        assert_eq!(
            implementation.config()["app"]["token"],
            serde_json::json!("profile-token")
        );
        assert_eq!(
            implementation.config()["app"]["baseOnly"],
            serde_json::json!("base-value")
        );
        assert_eq!(
            implementation.config()["skiff"]["test"]["ingressUrl"],
            serde_json::json!("http://127.0.0.1:46100")
        );
    }

    let first_service_ids = first
        .cases
        .iter()
        .map(|case| case.contract.service_id.as_str())
        .collect::<BTreeSet<_>>();
    let second_service_ids = second
        .cases
        .iter()
        .map(|case| case.contract.service_id.as_str())
        .collect::<BTreeSet<_>>();
    assert!(
        first_service_ids.is_disjoint(&second_service_ids),
        "repeated skiff test executions must derive different service identities"
    );
    assert!(first.records.deployments.iter().all(|deployment| {
        let wire = serde_json::to_value(deployment).unwrap();
        ["configLiterals", "secretRefs", "stateBindings", "policy"]
            .iter()
            .all(|field| wire.get(field).is_none())
    }));
}

#[test]
fn runner_owned_ingress_path_and_unknown_package_fail_closed() {
    let root = TestRoot::new("config-snapshot-negative");
    let artifacts = root.path().join("artifacts");
    let service = root.path().join("service");
    CanonicalArtifactStore::create(&artifacts).unwrap();
    write_configured_test_service(&service);

    write_profile(
        &service,
        r#""test.skiff/config-snapshot":
  skiff:
    test:
      ingressUrl: http://authored.invalid
"#,
    );
    let error = assemble_service(&service, &artifacts).unwrap_err();
    assert!(error.contains("skiff.test.ingressUrl"), "{error}");
    assert!(error.contains("reserved"), "{error}");

    write_profile(
        &service,
        r#""unknown.example/package":
  token: forbidden
"#,
    );
    let error = assemble_service(&service, &artifacts).unwrap_err();
    assert!(
        error.contains("outside the exact deployment closure"),
        "{error}"
    );
    assert!(error.contains("unknown.example/package"), "{error}");
}

fn assemble_service(service: &Path, artifacts: &Path) -> Result<(), String> {
    let project = compile_package_project_for_test(&platform_sources(), service, artifacts)
        .map_err(|error| error.to_string())?;
    let cases =
        discover_test_service_cases(service, service, false).map_err(|error| error.to_string())?;
    assemble_test_service_fixture_for_run_with_ingress(
        &project,
        &cases,
        Default::default(),
        "negative-execution",
        "http://127.0.0.1:46100",
        "skiff-test",
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn write_configured_test_service(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("package.yml"),
        "id: test.skiff/config-snapshot\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(root.join("api.yml"), "{}\n").unwrap();
    fs::write(
        root.join("service.yml"),
        "id: test.skiff/config-snapshot\nkind: test\n",
    )
    .unwrap();
    fs::write(
        root.join("main.skiff"),
        r#"function configured() -> string {
  const token = config.require<string>("app.token")
  const ingress = config.require<string>("skiff.test.ingressUrl")
  return token.concat(ingress)
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("alpha.test.skiff"),
        "test \"first\" { assert true }\ntest \"second\" { assert true }\n",
    )
    .unwrap();
    fs::write(
        root.join("beta.test.skiff"),
        "test \"third\" { assert true }\n",
    )
    .unwrap();
    fs::write(
        root.join("config.yml"),
        r#""test.skiff/config-snapshot":
  app:
    token: base-token
    baseOnly: base-value
"#,
    )
    .unwrap();
    write_profile(
        root,
        r#""test.skiff/config-snapshot":
  app:
    token: profile-token
"#,
    );
}

fn write_profile(root: &Path, profile: &str) {
    fs::write(root.join("config.skiff-test.yml"), profile).unwrap();
}
