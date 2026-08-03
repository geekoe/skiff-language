//! E-actor-rust lane unit tests: lease scheduler forget semantics and the
//! session outbound budget default. Full-chain behavior is covered by the
//! real two-replica
//! `actor_live_probe` (driven by `scripts/check-router-actor-live.mjs`).

use std::sync::Arc;

use skiff_router::actor::{
    ActorLeaseExpiryScheduler, ActorLogicalKey, ActorOwnershipRegistry, IdleEvictControlPort,
    LeaseSchedulerOptions,
};
use skiff_router::actor::{ActorOwnerFence, OwnerReleaseReason};
use skiff_router::session::budget::SessionBudgets;

#[cfg(test)]
mod tests {
    use super::*;

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
            &[],
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
                    owner_lease_id: "owner-lease-live".to_string(),
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
    fn session_outbound_budget_default_is_256_and_bounded() {
        let budgets = SessionBudgets::default();
        assert_eq!(budgets.outbound_frames, 256);
        assert_eq!(budgets.outbound_bytes, 4 * 1024 * 1024);
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
