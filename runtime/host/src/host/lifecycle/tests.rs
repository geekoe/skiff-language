use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use futures_util::StreamExt;
use skiff_artifact_identity::runtime_assembly_ref;
use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyIdentity, CanonicalPackageLinkPlan, RuntimeAssembly,
    RuntimeAssemblyRef, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_deployment::storage::{CanonicalArtifactStore, EnvironmentActivationState};
use skiff_runtime_transport::{
    assembly_activation::{decode_assembly_activation_frame, AssemblyActivationFrameDirection},
    protocol::{decode_typed_binary_frame, RuntimeCapabilitiesFrameHeader},
};
use tokio::{net::TcpListener, sync::mpsc, time::timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};

use super::*;
use crate::host::{DbProviderSource, RuntimeConfig};

mod store_failures;

struct TestArtifactRoot {
    path: PathBuf,
}

impl TestArtifactRoot {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "skiff-f10-{label}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TestArtifactRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn empty_assembly() -> RuntimeAssembly {
    let mut assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("unassigned"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        global_ingress: Vec::new(),
    };
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();
    assembly
}

fn runtime_host(artifact_root: &Path, router_url: String) -> RuntimeHost {
    RuntimeHost::new(RuntimeConfig {
        db_provider: DbProviderSource::unavailable(),
        services: Vec::new(),
        router_url,
        base_runtime_id: "runtime-a".to_string(),
        runtime_home: artifact_root.join("runtime-home"),
        artifact_roots: vec![artifact_root.to_path_buf()],
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
    })
    .unwrap()
}

fn initialized_store(
    label: &str,
    generation: u64,
) -> (TestArtifactRoot, CanonicalArtifactStore, RuntimeAssemblyRef) {
    let root = TestArtifactRoot::new(label);
    let store = CanonicalArtifactStore::create(&root.path).unwrap();
    let assembly = empty_assembly();
    store.write_runtime_assembly(&assembly).unwrap();
    let reference = runtime_assembly_ref(&assembly).unwrap();
    store
        .initialize_environment_activation(&EnvironmentActivationState::initial(
            "test",
            generation,
            reference.clone(),
        ))
        .unwrap();
    (root, store, reference)
}

async fn receive_connection_registration(
    listener: &TcpListener,
) -> (String, AssemblyActivationControl) {
    let (stream, _) = listener.accept().await.unwrap();
    let mut socket = accept_async(stream).await.unwrap();

    let first = socket.next().await.unwrap().unwrap();
    let Message::Binary(first) = first else {
        panic!("first runtime frame must be binary capabilities");
    };
    let (capabilities, payload) =
        decode_typed_binary_frame::<RuntimeCapabilitiesFrameHeader>(&first).unwrap();
    assert_eq!(capabilities.envelope_type, "runtime.capabilities");
    assert!(payload.is_empty());

    let second = socket.next().await.unwrap().unwrap();
    let Message::Binary(second) = second else {
        panic!("second runtime frame must be binary assembly registration");
    };
    let registration = decode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RuntimeToRouter,
        &second,
    )
    .unwrap();
    assert!(matches!(
        registration,
        AssemblyActivationControl::Register { .. }
    ));
    socket.close(None).await.unwrap();
    (capabilities.runtime_id, registration)
}

#[tokio::test]
async fn committed_recovery_valid_pending_is_ignored_until_router_replay() {
    let (root, store, reference) = initialized_store("pending", 0);
    store
        .prepare_environment_activation(
            "test",
            "activation-1",
            0,
            1,
            reference.clone(),
            vec!["runtime-a".to_string()],
        )
        .unwrap();
    let host = runtime_host(&root.path, "ws://127.0.0.1:1/runtime".to_string());

    host.recover_durable_committed().await.unwrap();

    assert!(store
        .read_environment_activation("test")
        .unwrap()
        .pending
        .is_some());
    assert!(matches!(
        host.active_assembly_registration().unwrap(),
        Some(AssemblyActivationControl::Register {
            generation: 0,
            assembly,
            replica_id,
            ..
        }) if assembly == reference && replica_id == "runtime-a"
    ));
}

#[tokio::test]
async fn reconnect_rereads_offline_generation_and_sends_capabilities_before_exact_register() {
    let (root, store, reference) = initialized_store("offline-advance", 5);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let router_url = format!("ws://{}/runtime", listener.local_addr().unwrap());
    let (registrations_tx, mut registrations_rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            registrations_tx
                .send(receive_connection_registration(&listener).await)
                .unwrap();
        }
    });
    let host = runtime_host(&root.path, router_url);

    timeout(Duration::from_secs(5), host.run_router_session_once())
        .await
        .expect("first session must close")
        .unwrap();
    let (first_runtime_id, first_registration) = registrations_rx.recv().await.unwrap();
    assert_eq!(first_runtime_id, "runtime-a");
    assert!(matches!(
        first_registration,
        AssemblyActivationControl::Register {
            generation: 5,
            ref assembly,
            ref replica_id,
            ..
        } if assembly == &reference && replica_id == "runtime-a"
    ));

    store
        .prepare_environment_activation(
            "test",
            "activation-6",
            5,
            6,
            reference.clone(),
            vec!["runtime-a".to_string()],
        )
        .unwrap();
    store
        .commit_environment_activation(
            "test",
            "activation-6",
            5,
            6,
            &reference,
            &["runtime-a".to_string()],
            &["runtime-a".to_string()],
        )
        .unwrap();

    timeout(Duration::from_secs(5), host.run_router_session_once())
        .await
        .expect("reconnected session must close")
        .unwrap();
    let (second_runtime_id, second_registration) = registrations_rx.recv().await.unwrap();
    assert_eq!(second_runtime_id, "runtime-a");
    assert!(matches!(
        second_registration,
        AssemblyActivationControl::Register {
            generation: 6,
            assembly,
            replica_id,
            ..
        } if assembly == reference && replica_id == "runtime-a"
    ));
    assert!(matches!(
        host.active_assembly_registration().unwrap(),
        Some(AssemblyActivationControl::Register { generation: 6, .. })
    ));
    server.await.unwrap();
}
