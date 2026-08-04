//! W-bootstrap epoch tests.

use std::sync::Arc;

use skiff_artifact_model::{RuntimeAssemblyRef, RuntimeConfigSnapshotRef};
use skiff_deployment::fixtures::empty_runtime_assembly_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::artifact::ActorRoutingCatalog;
use skiff_router::bootstrap::{ActiveRoutingEpochStore, RoutingEpoch};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;

fn assembly() -> Arc<skiff_artifact_model::RuntimeAssembly> {
    Arc::new(empty_runtime_assembly_fixture().expect("assembly fixture"))
}

fn snapshot_ref() -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
            "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("snapshot id"),
    }
}

fn snapshot(profile: &str) -> Arc<RuntimeConfigSnapshot> {
    Arc::new(
        RuntimeConfigSnapshot::new(profile, snapshot_ref(), Vec::new()).expect("snapshot fixture"),
    )
}

fn catalog() -> Arc<ActorRoutingCatalog> {
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    Arc::new(ActorRoutingCatalog::from_projection(Arc::new(projection)))
}

fn epoch(profile: &str, generation: u64) -> Arc<RoutingEpoch> {
    Arc::new(
        RoutingEpoch::new(
            profile,
            generation,
            assembly(),
            snapshot(profile),
            catalog(),
        )
        .expect("epoch fixture"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_exposes_frozen_corpus_fields() {
        let epoch = epoch("prod", 7);
        assert_eq!(epoch.profile(), "prod");
        assert_eq!(epoch.assembly_generation(), 7);
        assert_eq!(
            epoch.assembly_identity(),
            epoch.assembly().assembly_identity.as_str()
        );
        assert_eq!(
            epoch.config_snapshot_id(),
            epoch.snapshot().snapshot_ref().snapshot_id.as_str()
        );
        assert!(epoch.ingress_projection().is_empty());
        assert!(epoch.deployment_projection().is_empty());
        assert!(epoch.actor_catalog().is_empty());
    }

    #[test]
    fn epoch_rejects_snapshot_profile_mismatch() {
        let result = RoutingEpoch::new("prod", 1, assembly(), snapshot("stage"), catalog());
        let error = result.expect_err("profile mismatch must fail closed");
        assert!(error.to_string().contains("profile mismatch"), "{error}");
    }

    #[test]
    fn epoch_maps_to_registered_assembly_tuple_for_session_seam() {
        let epoch = epoch("prod", 7);
        let tuple = epoch.registered_tuple();
        assert_eq!(tuple.profile, "prod");
        assert_eq!(tuple.generation, 7);
        assert_eq!(tuple.assembly_identity(), epoch.assembly_identity());
        assert_eq!(tuple.snapshot_id(), epoch.config_snapshot_id());
    }

    #[test]
    fn store_publishes_whole_epoch_and_keeps_old_arcs_alive() {
        let store = ActiveRoutingEpochStore::new();
        assert_eq!(store.capture(), None);
        assert_eq!(store.publish_count(), 0);

        let first = epoch("prod", 1);
        store.publish(Arc::clone(&first));
        assert_eq!(store.capture().as_ref(), Some(&first));
        assert_eq!(store.publish_count(), 1);

        let second = epoch("prod", 2);
        store.publish(Arc::clone(&second));
        assert_eq!(store.capture().as_ref(), Some(&second));
        assert_eq!(store.publish_count(), 2);
        assert_eq!(first.assembly_generation(), 1);
        assert_eq!(second.assembly_generation(), 2);
    }

    #[test]
    fn store_health_exposes_active_epoch_and_publish_count() {
        let store = ActiveRoutingEpochStore::new();
        assert_eq!(store.health().publish_count, 0);
        assert_eq!(store.health().current, None);

        let epoch = epoch("prod", 42);
        store.publish(Arc::clone(&epoch));
        let health = store.health();
        assert_eq!(health.publish_count, 1);
        let current = health.current.expect("current epoch");
        assert_eq!(current.profile, "prod");
        assert_eq!(current.assembly_generation, 42);
        assert_eq!(current.assembly_identity, epoch.assembly_identity());
        assert_eq!(
            current.config_snapshot_id,
            "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn store_never_holds_pending_state() {
        let store = ActiveRoutingEpochStore::new();
        let epoch = epoch("prod", 1);
        store.publish(epoch);
        let captured = store.capture().expect("captured epoch");
        assert_eq!(captured.registered_tuple().generation, 1);
        assert_eq!(store.publish_count(), 1);
    }

    #[test]
    fn committed_bootstrap_refs_map_onto_wire_activation_header_fields() {
        let committed = skiff_deployment::storage::CommittedActivation {
            generation: 7,
            assembly: RuntimeAssemblyRef {
                assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                    "skiff-runtime-assembly-v3:sha256:".to_string() + &"a".repeat(64),
                ),
            },
            config_snapshot: snapshot_ref(),
        };
        let refs = skiff_router::bootstrap::CommittedBootstrapRefs::project_committed(&committed);
        assert_eq!(refs.generation, 7);
        assert_eq!(refs.assembly, committed.assembly);
        assert_eq!(refs.config_snapshot, committed.config_snapshot);
    }
}
