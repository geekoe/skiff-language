use std::fs;

use serde_json::{json, Value};
use skiff_artifact_identity::EnvironmentActivationStatePath;

use super::{run_ecosystem_store_adapter, RouterSnapshot};

#[path = "tests/bootstrap.rs"]
mod bootstrap;
#[path = "tests/fixtures.rs"]
mod fixtures;
#[path = "tests/snapshot.rs"]
mod snapshot;

#[test]
fn ecosystem_store_shared_workflow_and_negative_corpus() {
    let root = TestRoot::new("shared-corpus");
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../cross-system-fixtures/package-service-ecosystem/ecosystem-store-cases.json"
    ))
    .expect("shared store corpus");
    let expected_empty = &corpus["emptyAssembly"];
    let mut responses = Vec::new();
    for request in corpus["workflow"].as_array().unwrap() {
        responses.push(invoke(root.path(), request.clone()).expect("workflow operation"));
    }
    assert_eq!(responses[0]["committed"]["assembly"], *expected_empty);
    assert_eq!(responses[5]["committed"]["generation"], 1);
    assert_eq!(
        responses[6], responses[5],
        "bootstrap must not reset existing state"
    );
    let snapshot: RouterSnapshot =
        serde_json::from_value(responses[7].clone()).expect("typed snapshot response");
    assert_eq!(
        snapshot.assembly.assembly_identity.as_str(),
        expected_empty["assemblyIdentity"]
    );
    assert!(snapshot.service_contracts.is_empty());

    for case in corpus["invalidRequests"].as_array().unwrap() {
        assert!(
            invoke(root.path(), case["request"].clone()).is_err(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn ecosystem_store_bootstrap_is_idempotent_and_snapshot_is_typed() {
    let root = TestRoot::new("bootstrap");
    let first = invoke(
        root.path(),
        json!({
            "operation": "ensureEnvironmentBootstrap",
            "environment": "test"
        }),
    )
    .expect("first bootstrap");
    let state_path = root.path().join(
        EnvironmentActivationStatePath::new("test")
            .unwrap()
            .as_relative_path()
            .as_path(),
    );
    let first_bytes = fs::read(&state_path).unwrap();
    let second = invoke(
        root.path(),
        json!({
            "operation": "ensureEnvironmentBootstrap",
            "environment": "test"
        }),
    )
    .expect("second bootstrap");
    assert_eq!(first, second);
    assert_eq!(fs::read(&state_path).unwrap(), first_bytes);
    assert_eq!(first["committed"]["generation"], 0);
    assert!(first["pending"].is_null());

    let snapshot = invoke(
        root.path(),
        json!({
            "operation": "readRouterSnapshot",
            "assembly": first["committed"]["assembly"].clone()
        }),
    )
    .expect("snapshot");
    let typed: RouterSnapshot = serde_json::from_value(snapshot.clone()).expect("typed snapshot");
    assert!(typed.assembly.roots.is_empty());
    assert!(typed.service_contracts.is_empty());
    assert!(snapshot.get("path").is_none());
    assert!(snapshot.get("latest").is_none());
}

#[test]
fn ecosystem_store_state_transitions_delegate_exact_cas() {
    let root = TestRoot::new("cas");
    let initial = invoke(
        root.path(),
        json!({
            "operation": "ensureEnvironmentBootstrap",
            "environment": "test"
        }),
    )
    .unwrap();
    let assembly = initial["committed"]["assembly"].clone();
    let pending = invoke(
        root.path(),
        json!({
            "operation": "prepareEnvironment",
            "environment": "test",
            "activationId": "activation-1",
            "expectedGeneration": 0,
            "candidateGeneration": 1,
            "assembly": assembly,
            "participantReplicaIds": ["runtime-b", "runtime-a"]
        }),
    )
    .expect("prepare");
    assert_eq!(
        pending["pending"]["participantReplicaIds"],
        json!(["runtime-a", "runtime-b"])
    );
    let state_path = root.path().join(
        EnvironmentActivationStatePath::new("test")
            .unwrap()
            .as_relative_path()
            .as_path(),
    );
    let pending_bytes = fs::read(&state_path).unwrap();
    let pending_bootstrap = invoke(
        root.path(),
        json!({
            "operation": "ensureEnvironmentBootstrap",
            "environment": "test"
        }),
    )
    .expect("bootstrap must preserve pending state");
    assert_eq!(pending_bootstrap, pending);
    assert_eq!(fs::read(&state_path).unwrap(), pending_bytes);

    let committed = invoke(
        root.path(),
        json!({
            "operation": "commitEnvironment",
            "environment": "test",
            "activationId": "activation-1",
            "expectedGeneration": 0,
            "candidateGeneration": 1,
            "assembly": initial["committed"]["assembly"].clone(),
            "connectedReplicaIds": ["runtime-a", "runtime-b"],
            "preparedReplicaIds": ["runtime-a", "runtime-b"]
        }),
    )
    .expect("commit");
    assert_eq!(committed["committed"]["generation"], 1);
    assert!(committed["pending"].is_null());

    let committed_bytes = fs::read(&state_path).unwrap();

    let stale = invoke(
        root.path(),
        json!({
            "operation": "abortEnvironment",
            "environment": "test",
            "activationId": "activation-1",
            "expectedGeneration": 0
        }),
    );
    assert!(stale.is_err(), "typed store must reject stale CAS");
    assert_eq!(
        fs::read(&state_path).unwrap(),
        committed_bytes,
        "failed abort must not mutate committed state bytes"
    );

    let no_pending = invoke(
        root.path(),
        json!({
            "operation": "commitEnvironment",
            "environment": "test",
            "activationId": "activation-2",
            "expectedGeneration": 1,
            "candidateGeneration": 2,
            "assembly": initial["committed"]["assembly"].clone(),
            "connectedReplicaIds": [],
            "preparedReplicaIds": []
        }),
    );
    assert!(
        no_pending.is_err(),
        "typed store must reject commit without pending"
    );
    assert_eq!(
        fs::read(&state_path).unwrap(),
        committed_bytes,
        "failed commit must not mutate committed state bytes"
    );
}

#[test]
fn ecosystem_store_rejects_unknown_requests_and_does_not_replace_tampered_state() {
    let root = TestRoot::new("negative");
    assert!(invoke(
        root.path(),
        json!({
            "operation": "readEnvironment",
            "environment": "test",
            "artifactKind": "RuntimeAssembly"
        })
    )
    .is_err());

    let state_path = root.path().join("environments/test/activation.json");
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    fs::write(&state_path, b"{\"partial\":true}").unwrap();
    assert!(invoke(
        root.path(),
        json!({
            "operation": "ensureEnvironmentBootstrap",
            "environment": "test"
        })
    )
    .is_err());
    assert_eq!(fs::read(&state_path).unwrap(), b"{\"partial\":true}");
}

#[test]
fn ecosystem_store_requires_exactly_one_stdin_json_value() {
    let root = TestRoot::new("single-json");
    let mut output = Vec::new();
    let error = run_ecosystem_store_adapter(
        root.path(),
        b"{\"operation\":\"readEnvironment\",\"environment\":\"test\"}\n{}".as_slice(),
        &mut output,
    )
    .expect_err("second JSON value must fail");
    assert!(error.to_string().contains("trailing characters"));
    assert!(output.is_empty());
}

pub(super) fn invoke(root: &std::path::Path, request: Value) -> Result<Value, String> {
    let input = serde_json::to_vec(&request).unwrap();
    let mut output = Vec::new();
    run_ecosystem_store_adapter(root, input.as_slice(), &mut output)
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&output).map_err(|error| error.to_string())
}

pub(super) struct TestRoot(std::path::PathBuf);

impl TestRoot {
    pub(super) fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "skiff-compiler-ecosystem-store-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    pub(super) fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
