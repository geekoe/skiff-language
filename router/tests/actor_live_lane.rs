//! E-actor-rust lane unit tests: spawn wire store correlation, lease
//! scheduler forget semantics and the corrected session inbound budget
//! default. Full-chain behavior is covered by the real two-replica
//! `actor_live_probe` (driven by `scripts/check-router-actor-live.mjs`).

use std::sync::Arc;

use skiff_router::actor::{
    ActorLeaseExpiryScheduler, ActorLogicalKey, ActorOwnershipRegistry, IdleEvictControlPort,
    LeaseSchedulerOptions, SpawnSubmitError, SpawnWireStore,
};
use skiff_router::actor::{ActorOwnerFence, OwnerReleaseReason};
use skiff_router::session::budget::SessionBudgets;
use skiff_runtime_transport::protocol::{
    SpawnCallerKind, SpawnSubmitRequestFrame, SpawnSubmitRequestFrameHeaderV2, SpawnTargetKind,
    RUNTIME_FRAME_SCHEMA_VERSION,
};

fn wire_frame() -> SpawnSubmitRequestFrame {
    SpawnSubmitRequestFrame {
        header: SpawnSubmitRequestFrameHeaderV2 {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "spawn.submit.request".to_string(),
            rpc_id: "rpc-1".to_string(),
            runtime_id: "runtime-a".to_string(),
            caller_kind: SpawnCallerKind::ActorInvocation,
            caller_request_id: "invocation-1".to_string(),
            target_kind: SpawnTargetKind::ActorMethod,
            service_id: "example.com/service".to_string(),
            service_version: "1.0.0".to_string(),
            service_protocol_identity: "protocol".to_string(),
            target: "actorMethod".to_string(),
            spawn_id: None,
            build_id: None,
            activation_identity:
                skiff_runtime_transport::protocol::ActivationIdentityFrameMetadata {
                    assembly_identity: "assembly".to_string(),
                    generation: 1,
                    runtime_replica_id: "runtime-a".to_string(),
                    deployment_revision: "revision".to_string(),
                },
            trace_id: None,
            caller_target: None,
            max_queue_wait_ms: None,
            actor_method: None,
        },
        payload: vec![1, 2, 3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_wire_store_lifecycle_is_zero_after_response() {
        let store = Arc::new(SpawnWireStore::new());
        let frame = wire_frame();
        store.register("spawn-1", frame.clone());
        assert_eq!(store.pending_count(), 1);
        assert_eq!(store.get("spawn-1").expect("wire").frame, frame);
        assert!(store.get("spawn-1").expect("wire").outcome.is_none());

        store.set_outcome("spawn-1", Ok(()));
        assert!(store.get("spawn-1").expect("wire").outcome.is_some());
        store.remove("spawn-1");

        let health = store.health();
        assert_eq!(health.pending, 0);
        assert_eq!(health.registered, 1);
        assert_eq!(health.consumed, 1);
        assert_eq!(health.orphan_accepts, 0);
    }

    #[test]
    fn spawn_wire_store_rejects_unknown_outcome_and_removes() {
        let store = Arc::new(SpawnWireStore::new());
        store.set_outcome(
            "missing",
            Err(SpawnSubmitError::new(
                skiff_router::actor::SpawnErrorCode::ParentNotFound,
            )),
        );
        assert!(store.get("missing").is_none());
        assert_eq!(store.health().orphan_accepts, 0);
        store.remove("missing");
        assert_eq!(store.health().pending, 0);
    }

    #[test]
    fn lease_scheduler_forget_clears_stale_eviction_after_release() {
        let registry = Arc::new(ActorOwnershipRegistry::new());
        let scheduler = Arc::new(ActorLeaseExpiryScheduler::new(
            Arc::clone(&registry),
            Arc::new(FakeEvictControl),
            LeaseSchedulerOptions::default(),
        ));
        let key = ActorLogicalKey {
            service_id: "example.com/service".to_string(),
            actor_type_identity: "type".to_string(),
            actor_id_type_identity: "id-type".to_string(),
            actor_id_encoding_version: "v1".to_string(),
            canonical_actor_id_key_bytes_base64: "a2V5".to_string(),
            actor_id_hash: "sha256:abc".to_string(),
        };
        let facts = registry.ensure_present(
            &key,
            skiff_artifact_model::ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:abc"),
            skiff_artifact_model::ActorImplementationIdentity::new(
                "skiff-actor-implementation-v1:sha256:abc",
            ),
            skiff_runtime_transport::actor_method::ActorDeclarationOwnerFrameHeader {
                unit: skiff_runtime_transport::actor_method::ActorOwnerUnitFrameHeader::Service,
                file: skiff_runtime_transport::actor_method::ActorOwnerFileFrameHeader::LoadedFileIndex(0),
                actor_symbol: "Actor".to_string(),
            },
        );
        let token = registry
            .reserve(
                &key,
                facts.epoch,
                "runtime-a",
                &skiff_router::actor::ActorOwnerRouteAuthority {
                    assembly_identity: "assembly".to_string(),
                    assembly_generation: 1,
                },
                0,
            )
            .expect("reserve");
        let fence = registry
            .commit(
                &token,
                &skiff_router::actor::CommitFenceFacts {
                    actor_abi_identity: skiff_artifact_model::ActorAbiIdentity::new(
                        "skiff-actor-abi-v1:sha256:abc",
                    ),
                    actor_implementation_identity: skiff_artifact_model::ActorImplementationIdentity::new(
                        "skiff-actor-implementation-v1:sha256:abc",
                    ),
                    declaration_owner:
                        skiff_runtime_transport::actor_method::ActorDeclarationOwnerFrameHeader {
                            unit: skiff_runtime_transport::actor_method::ActorOwnerUnitFrameHeader::Service,
                            file: skiff_runtime_transport::actor_method::ActorOwnerFileFrameHeader::LoadedFileIndex(0),
                            actor_symbol: "Actor".to_string(),
                        },
                },
                0,
                100_000,
            )
            .expect("commit");
        scheduler.mark_live(&key, 0, "runtime-a#1");
        scheduler.sweep(40_000);
        assert!(scheduler.health().eviction_pending >= 1);

        registry
            .release(&key, &fence, OwnerReleaseReason::Disconnected)
            .expect("release");
        scheduler.forget(&key);
        assert_eq!(scheduler.health().eviction_pending, 0);
        assert_eq!(registry.current_owner(&key), None);

        let _: ActorOwnerFence = fence;
    }

    #[test]
    fn session_inbound_budget_default_is_4096_and_bounded() {
        let budgets = SessionBudgets::default();
        assert_eq!(budgets.inbound_frames, 4096);
        assert_eq!(budgets.inbound_bytes, 1024 * 1024);
        assert_eq!(budgets.outbound_frames, 256);
    }
}

#[derive(Debug, Default)]
struct FakeEvictControl;

impl IdleEvictControlPort for FakeEvictControl {
    fn send_idle_evict(
        &self,
        _key: &ActorLogicalKey,
        _fence: &ActorOwnerFence,
        _eviction_request_id: &str,
        _connection: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}
