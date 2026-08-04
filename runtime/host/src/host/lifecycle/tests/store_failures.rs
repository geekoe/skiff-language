use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use skiff_artifact_identity::{ProfileActivationStatePath, RuntimeAssemblyRecordPath};
use skiff_artifact_model::RuntimeAssemblyRef;
use skiff_deployment::{fixtures::runtime_assembly_fixture, storage::ProfileActivationState};
use tokio::{net::TcpListener, time::timeout};

use super::*;

fn activation_path(root: &Path, profile: &str) -> PathBuf {
    root.join(
        ProfileActivationStatePath::new(profile)
            .unwrap()
            .as_relative_path()
            .as_path(),
    )
}

fn assembly_path(root: &Path, reference: &RuntimeAssemblyRef) -> PathBuf {
    root.join(
        RuntimeAssemblyRecordPath::new(reference)
            .unwrap()
            .as_relative_path()
            .as_path(),
    )
}

async fn assert_preconnect_failure(label: &str, host: &RuntimeHost) {
    match timeout(Duration::from_secs(1), host.run_router_session_once()).await {
        Ok(Err(_)) => {}
        Ok(Ok(())) => panic!("{label}: invalid durable state unexpectedly opened a session"),
        Err(_) => panic!("{label}: recovery did not fail before the connector"),
    }
}

async fn exercise_missing_and_invalid_state_failures(router_url: &str) {
    let missing_parent = TestArtifactRoot::new("missing-root");
    let missing_root = missing_parent.path.join("absent");
    assert_preconnect_failure(
        "missing artifact root",
        &runtime_host(&missing_root, router_url.to_string()),
    )
    .await;

    let missing_state_root = TestArtifactRoot::new("missing-state");
    let missing_state_store = CanonicalArtifactStore::create(&missing_state_root.path).unwrap();
    missing_state_store
        .write_runtime_assembly(&empty_assembly())
        .unwrap();
    assert_preconnect_failure(
        "missing activation state",
        &runtime_host(&missing_state_root.path, router_url.to_string()),
    )
    .await;

    let (malformed_root, _, _) = initialized_store("malformed-state", 0);
    fs::write(activation_path(&malformed_root.path, "test"), b"{").unwrap();
    assert_preconnect_failure(
        "malformed activation state",
        &runtime_host(&malformed_root.path, router_url.to_string()),
    )
    .await;

    let (noncanonical_root, _, _) = initialized_store("noncanonical-state", 0);
    let noncanonical_path = activation_path(&noncanonical_root.path, "test");
    let mut noncanonical = fs::read(&noncanonical_path).unwrap();
    noncanonical.push(b'\n');
    fs::write(noncanonical_path, noncanonical).unwrap();
    assert_preconnect_failure(
        "non-canonical activation state",
        &runtime_host(&noncanonical_root.path, router_url.to_string()),
    )
    .await;
}

async fn exercise_cross_profile_failure(router_url: &str) {
    let root = TestArtifactRoot::new("cross-profile");
    let store = CanonicalArtifactStore::create(&root.path).unwrap();
    let assembly = empty_assembly();
    store.write_runtime_assembly(&assembly).unwrap();
    store
        .initialize_profile_activation(&ProfileActivationState::initial(
            "other",
            0,
            runtime_assembly_ref(&assembly).unwrap(),
        ))
        .unwrap();
    let test_path = activation_path(&root.path, "test");
    fs::create_dir_all(test_path.parent().unwrap()).unwrap();
    fs::copy(activation_path(&root.path, "other"), &test_path).unwrap();
    assert_preconnect_failure(
        "activation profile/path mismatch",
        &runtime_host(&root.path, router_url.to_string()),
    )
    .await;
}

async fn exercise_record_reference_failures(router_url: &str) {
    let (missing_ref_root, _, missing_ref) = initialized_store("missing-ref", 0);
    fs::remove_file(assembly_path(&missing_ref_root.path, &missing_ref)).unwrap();
    assert_preconnect_failure(
        "missing committed ref",
        &runtime_host(&missing_ref_root.path, router_url.to_string()),
    )
    .await;

    let (tampered_record_root, _, tampered_ref) = initialized_store("tampered-record", 0);
    fs::write(
        assembly_path(&tampered_record_root.path, &tampered_ref),
        b"{}",
    )
    .unwrap();
    assert_preconnect_failure(
        "tampered committed record",
        &runtime_host(&tampered_record_root.path, router_url.to_string()),
    )
    .await;

    let (identity_mismatch_root, identity_mismatch_store, committed_ref) =
        initialized_store("identity-mismatch", 0);
    let other_assembly = runtime_assembly_fixture().unwrap();
    let other_ref = runtime_assembly_ref(&other_assembly).unwrap();
    identity_mismatch_store
        .write_runtime_assembly(&other_assembly)
        .unwrap();
    fs::copy(
        assembly_path(&identity_mismatch_root.path, &other_ref),
        assembly_path(&identity_mismatch_root.path, &committed_ref),
    )
    .unwrap();
    assert_preconnect_failure(
        "committed record identity mismatch",
        &runtime_host(&identity_mismatch_root.path, router_url.to_string()),
    )
    .await;

    let (pending_ref_root, pending_store, _) = initialized_store("pending-ref", 0);
    let pending_assembly = runtime_assembly_fixture().unwrap();
    let pending_ref = runtime_assembly_ref(&pending_assembly).unwrap();
    pending_store
        .write_runtime_assembly(&pending_assembly)
        .unwrap();
    pending_store
        .prepare_profile_activation(
            "test",
            "activation-1",
            0,
            1,
            pending_ref.clone(),
            vec!["runtime-a".to_string()],
        )
        .unwrap();
    fs::remove_file(assembly_path(&pending_ref_root.path, &pending_ref)).unwrap();
    assert_preconnect_failure(
        "missing pending ref",
        &runtime_host(&pending_ref_root.path, router_url.to_string()),
    )
    .await;
}

#[tokio::test]
async fn committed_recovery_failures_never_open_a_router_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let router_url = format!("ws://{}/runtime", listener.local_addr().unwrap());

    exercise_missing_and_invalid_state_failures(&router_url).await;
    exercise_cross_profile_failure(&router_url).await;
    exercise_record_reference_failures(&router_url).await;

    assert!(
        timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "no invalid durable state may reach the connector"
    );
}
