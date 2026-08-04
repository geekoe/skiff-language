//! W-bootstrap × W-session seam test: the `ActiveRoutingEpochStore` supplies
//! the committed epoch source to `SessionLayer` (bootstrap bytes + register
//! validation context) without changing session internal logic. Static
//! `committed_epoch` fallback remains intact for the session test seam.

use std::sync::Arc;

use skiff_artifact_model::{
    AssemblyIdentity, RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
};
use skiff_deployment::fixtures::empty_runtime_assembly_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::artifact::ActorRoutingCatalog;
use skiff_router::bootstrap::{ActiveRoutingEpochStore, RoutingEpoch};
use skiff_router::config::{RouterConfig, ServiceDbConfig};
use skiff_router::session::identity::RegisteredAssemblyTuple;
use skiff_router::session::{SessionLayer, SessionLayerOptions};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;
use skiff_runtime_transport::protocol::decode_router_bootstrap_frame;

fn config() -> RouterConfig {
    RouterConfig {
        activation_prepare_timeout_ms: 120_000,
        artifacts_path: "/opt/skiff/artifacts".into(),
        dev_reload: None,
        host: "127.0.0.1".to_string(),
        http_max_request_bytes: 1,
        http_max_response_bytes: 8_388_608,
        http_port: 4000,
        manifests: vec![],
        profile: "dev".to_string(),
        release_mode: None,
        request_timeout_ms: 20_000,
        rewrite: vec![],
        runtime_path: "/runtime".to_string(),
        runtime_port: 4001,
        runtime_max_concurrency: 4,
        file_backend: None,
        service_db: ServiceDbConfig {
            mongo_url: "mongodb://127.0.0.1:27017/?replicaSet=rs0".to_string(),
        },
        telemetry: None,
        websocket_path: "/ws".to_string(),
    }
}

fn snapshot_ref() -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(
            "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("snapshot id"),
    }
}

fn epoch(profile: &str, generation: u64) -> Arc<RoutingEpoch> {
    let assembly = Arc::new(empty_runtime_assembly_fixture().expect("assembly fixture"));
    let snapshot = Arc::new(
        RuntimeConfigSnapshot::new(profile, snapshot_ref(), Vec::new()).expect("snapshot fixture"),
    );
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let catalog = Arc::new(ActorRoutingCatalog::from_projection(Arc::new(projection)));
    Arc::new(
        RoutingEpoch::new(profile, generation, assembly, snapshot, catalog).expect("epoch fixture"),
    )
}

fn tuple(profile: &str, generation: u64) -> RegisteredAssemblyTuple {
    RegisteredAssemblyTuple {
        profile: profile.to_string(),
        generation,
        assembly: RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(format!(
                "skiff-runtime-assembly-v3:sha256:{}",
                "a".repeat(64)
            )),
        },
        config_snapshot: snapshot_ref(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn epoch_store_drives_bootstrap_bytes_and_epoch_context() {
        let store = Arc::new(ActiveRoutingEpochStore::new());
        let layer = SessionLayer::with_options(
            config(),
            SessionLayerOptions {
                pending_epoch: Some(tuple("prod", 99)),
                ..SessionLayerOptions::default()
            },
        )
        .expect("layer options");
        layer.attach_epoch_store(Arc::clone(&store));

        assert!(
            layer.bootstrap_bytes().is_none(),
            "no epoch must fail closed"
        );
        assert_eq!(layer.epoch_context().current, None);

        let first = epoch("prod", 42);
        store.publish(Arc::clone(&first));
        let bytes = layer.bootstrap_bytes().expect("bootstrap bytes");
        let header = decode_router_bootstrap_frame(&bytes).expect("bootstrap frame");
        assert_eq!(header.activation.profile, "prod");
        assert_eq!(header.activation.generation, 42);
        assert_eq!(
            header.activation.assembly,
            first.registered_tuple().assembly
        );
        assert_eq!(
            header.activation.config_snapshot,
            first.registered_tuple().config_snapshot
        );

        let context = layer.epoch_context();
        assert_eq!(context.current, Some(first.registered_tuple()));
        assert_eq!(context.pending, Some(tuple("prod", 99)));
    }

    #[tokio::test]
    async fn epoch_store_replacement_is_visible_to_new_bootstrap_captures() {
        let store = Arc::new(ActiveRoutingEpochStore::new());
        let layer = SessionLayer::with_options(config(), SessionLayerOptions::default())
            .expect("layer options");
        layer.attach_epoch_store(Arc::clone(&store));

        store.publish(epoch("prod", 1));
        let first = layer.bootstrap_bytes().expect("first bootstrap");
        let first_header = decode_router_bootstrap_frame(&first).expect("first bootstrap frame");
        assert_eq!(first_header.activation.generation, 1);

        store.publish(epoch("prod", 2));
        let second = layer.bootstrap_bytes().expect("second bootstrap");
        let second_header = decode_router_bootstrap_frame(&second).expect("second bootstrap frame");
        assert_eq!(second_header.activation.generation, 2);
        assert_eq!(
            layer.epoch_context().current.expect("current").generation,
            2
        );
        assert_eq!(store.publish_count(), 2);
    }

    #[tokio::test]
    async fn static_committed_epoch_fallback_still_works() {
        let layer = SessionLayer::with_options(
            config(),
            SessionLayerOptions {
                committed_epoch: Some(tuple("prod", 7)),
                ..SessionLayerOptions::default()
            },
        )
        .expect("layer options");
        let bytes = layer.bootstrap_bytes().expect("bootstrap bytes");
        let header = decode_router_bootstrap_frame(&bytes).expect("bootstrap frame");
        assert_eq!(header.activation.generation, 7);
        assert_eq!(layer.epoch_context().current, Some(tuple("prod", 7)));
    }

    #[tokio::test]
    async fn default_session_options_keep_the_static_test_seam() {
        assert_eq!(SessionLayerOptions::default().committed_epoch, None);
        let layer = SessionLayer::with_options(config(), SessionLayerOptions::default())
            .expect("layer options");
        assert!(layer.bootstrap_bytes().is_none());
    }
}
