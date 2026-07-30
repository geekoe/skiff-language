use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{MetadataValue, StateBindingKind};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_test_runner::{
    canonical_fixture::discover_test_service_cases,
    canonical_package::compile_package_project_for_test,
    test_service_fixture::assemble_test_service_fixture,
};

use super::*;

#[test]
fn fixed_profile_projects_config_secret_policy_and_typed_isolated_state() {
    let root = TestRoot::new("profile-state");
    let artifacts = root.path().join("artifacts");
    let service = root.path().join("service");
    CanonicalArtifactStore::create(&artifacts).unwrap();
    write_stateful_test_service(&service);

    let project = compile_package_project_for_test(&platform_sources(), &service, &artifacts)
        .expect("compile stateful test service");
    let cases =
        discover_test_service_cases(&service, &service, false).expect("discover stateful cases");
    assert_eq!(cases.len(), 3);

    let first = assemble_test_service_fixture(&project, &cases, Default::default())
        .expect("assemble first run");
    let second = assemble_test_service_fixture(&project, &cases, Default::default())
        .expect("assemble second run");
    assert_eq!(first.cases.len(), 3);

    let mut first_namespaces = BTreeSet::new();
    for case in &first.cases {
        let [deployment] = case.records.deployments.as_slice() else {
            panic!("each case must own one deployment")
        };
        assert_eq!(
            deployment.config_literals,
            vec![skiff_artifact_model::ConfigLiteralBinding {
                path: "app.token".to_string(),
                value: MetadataValue::String("configured-token".to_string()),
            }]
        );
        assert_eq!(
            deployment.secret_refs,
            vec![skiff_artifact_model::SecretRefBinding {
                path: "app.secret".to_string(),
                secret_ref: "test/app-secret".to_string(),
            }]
        );
        assert_eq!(deployment.policy.timeout_ms, Some(25_000));
        assert_eq!(deployment.policy.resources.cpu_millis, 250);
        assert_eq!(deployment.policy.resources.memory_bytes, 134_217_728);
        assert_eq!(
            deployment.policy.principal,
            "service:test.skiff/profile-state"
        );

        let bindings = deployment
            .state_bindings
            .iter()
            .map(|binding| {
                assert!(binding.namespace.starts_with("skiff_pt_"));
                first_namespaces.insert(binding.namespace.clone());
                (
                    binding.requirement_key.as_str(),
                    binding.kind,
                    binding.namespace.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            bindings
                .iter()
                .map(|(key, kind, _)| (*key, *kind))
                .collect::<Vec<_>>(),
            [
                ("app-db", StateBindingKind::Database),
                ("jobs", StateBindingKind::Queue),
            ]
        );
        assert_ne!(bindings[0].2, bindings[1].2);
    }
    assert_eq!(
        first_namespaces.len(),
        6,
        "every case and state kind must own an isolated namespace"
    );

    let second_namespaces = second
        .cases
        .iter()
        .flat_map(|case| &case.records.deployments[0].state_bindings)
        .map(|binding| binding.namespace.as_str())
        .collect::<BTreeSet<_>>();
    assert!(
        first_namespaces
            .iter()
            .all(|namespace| !second_namespaces.contains(namespace.as_str())),
        "a repeated diagnostic run scope must still receive fresh state namespaces"
    );
}

#[test]
fn missing_fixed_profile_and_missing_state_binding_fail_closed() {
    let root = TestRoot::new("profile-state-negative");
    let artifacts = root.path().join("artifacts");
    let service = root.path().join("service");
    CanonicalArtifactStore::create(&artifacts).unwrap();
    write_stateful_test_service(&service);

    fs::remove_file(service.join("config.skiff-test.yml")).unwrap();
    let missing_profile =
        compile_package_project_for_test(&platform_sources(), &service, &artifacts)
            .unwrap_err()
            .to_string();
    assert!(
        missing_profile.contains("requires config.skiff-test.yml"),
        "{missing_profile}"
    );

    write_profile(
        &service,
        r#"config:
  app.token: configured-token
secrets:
  app.secret: test/app-secret
state:
  app-db:
    kind: database
    namespace: authored-db
timeout: 25000
quota:
  cpuMillis: 250
  memoryBytes: 134217728
principal: service:test.skiff/profile-state
"#,
    );
    let project = compile_package_project_for_test(&platform_sources(), &service, &artifacts)
        .expect("compile missing-state candidate");
    let cases = discover_test_service_cases(&service, &service, false).unwrap();
    let missing_state = assemble_test_service_fixture(&project, &cases, Default::default())
        .unwrap_err()
        .to_string();
    assert!(
        missing_state.contains("missing state binding jobs"),
        "{missing_state}"
    );
}

#[test]
fn http_profile_rejects_retired_lifecycle_and_reserved_ingress_override() {
    let root = TestRoot::new("http-profile-negative");
    let artifacts = root.path().join("artifacts");
    let service = root.path().join("service");
    CanonicalArtifactStore::create(&artifacts).unwrap();
    write_http_test_service(&service);

    write_http_profile(&service, true, BTreeMap::new());
    let error = assemble_service(&service, &artifacts).unwrap_err();
    assert!(error.contains("lifecycle"), "{error}");
    assert!(error.contains("unknown field"), "{error}");

    write_http_profile(
        &service,
        false,
        BTreeMap::from([("skiff.test.ingressUrl", "http://authored.invalid")]),
    );
    let error = assemble_service(&service, &artifacts).unwrap_err();
    assert!(error.contains("skiff.test.ingressUrl"), "{error}");
    assert!(error.contains("reserved"), "{error}");
}

fn assemble_service(service: &Path, artifacts: &Path) -> Result<(), String> {
    let project = compile_package_project_for_test(&platform_sources(), service, artifacts)
        .map_err(|error| error.to_string())?;
    let cases =
        discover_test_service_cases(service, service, false).map_err(|error| error.to_string())?;
    assemble_test_service_fixture(&project, &cases, Default::default())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn write_stateful_test_service(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("package.yml"),
        r#"id: test.skiff/profile-state
version: 1.0.0
state:
  app-db:
    kind: database
  jobs:
    kind: queue
"#,
    )
    .unwrap();
    fs::write(root.join("api.yml"), "{}\n").unwrap();
    fs::write(
        root.join("service.yml"),
        "id: test.skiff/profile-state\nkind: test\n",
    )
    .unwrap();
    fs::write(
        root.join("main.skiff"),
        r#"function configured() -> string {
  const token = config.require<string>("app.token")
  const secret = config.require<string>("app.secret")
  return token.concat(secret)
}

type Stored { id: string }
db object Stored { primary key(id) }
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
    write_profile(
        root,
        r#"config:
  app.token: configured-token
secrets:
  app.secret: test/app-secret
state:
  app-db:
    kind: database
    namespace: authored-db
  jobs:
    kind: queue
    namespace: authored-jobs
timeout: 25000
quota:
  cpuMillis: 250
  memoryBytes: 134217728
principal: service:test.skiff/profile-state
"#,
    );
}

fn write_http_test_service(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("package.yml"),
        "id: test.skiff/http-profile\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(root.join("api.yml"), "{}\n").unwrap();
    fs::write(
        root.join("service.yml"),
        "id: test.skiff/http-profile\nkind: test\n",
    )
    .unwrap();
    fs::write(
        root.join("main.skiff"),
        "function probe(body: null) -> null { return null }\n",
    )
    .unwrap();
    fs::write(
        root.join("main.test.skiff"),
        "test \"http profile\" { assert true }\n",
    )
    .unwrap();
    fs::write(
        root.join("http.yml"),
        r#"probe:
  method: POST
  path: /probe
  kind: typedJson
  handler: main.probe
  adapterArgs:
    - param: body
      source: { kind: http.body }
"#,
    )
    .unwrap();
}

fn write_http_profile(root: &Path, retired_lifecycle: bool, config: BTreeMap<&str, &str>) {
    let config = if config.is_empty() {
        " {}".to_string()
    } else {
        format!(
            "\n{}",
            config
                .into_iter()
                .map(|(key, value)| format!("  {key}: {value}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    let lifecycle = retired_lifecycle
        .then_some("lifecycle: {}\n")
        .unwrap_or_default();
    write_profile(
        root,
        &format!(
            "config:{config}\ntimeout: 30000\nquota:\n  cpuMillis: 100\n  memoryBytes: 67108864\nprincipal: service:test.skiff/http-profile\n{lifecycle}"
        ),
    );
}

fn write_profile(root: &Path, profile: &str) {
    fs::write(root.join("config.skiff-test.yml"), profile).unwrap();
}
