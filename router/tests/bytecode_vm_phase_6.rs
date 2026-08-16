#[path = "bytecode_vm_phase_6/production_path.rs"]
mod production_path;

use std::sync::Arc;

use skiff_artifact_identity::DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX;
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, DeploymentArtifactIdentity, DeploymentRevision,
    PackageCallableId, ServiceDeploymentRef,
};
use skiff_router::actor::{
    ActorLogicalKey, ActorOwnerFence, ActorOwnerRouteAuthority, ActorOwnershipRegistry,
    CommitFenceFacts,
};
use skiff_runtime_transport::actor_method::{
    ActorDeclarationOwnerFrameHeader, ActorOwnerFileFrameHeader, ActorOwnerUnitFrameHeader,
};
use skiff_task_control::model::{
    DetachedCallTarget, DurableUtcTimestamp, RecoverablePayload, ServiceOwner,
    TaskExecutionImageRef, TaskId, TaskRecord, TaskState, TaskTraceContext,
};
use skiff_task_control::store::{ClaimInput, DueScanInput, TaskStore};
use skiff_task_control::MemoryTaskStore;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_s1() {
        let store = MemoryTaskStore::new();
        let created = store
            .create(task_record("task-s1"))
            .await
            .expect("TaskStore accepts a durable task record");
        assert_phase6_image(&created);
    }

    #[tokio::test]
    async fn task_s2() {
        let store = MemoryTaskStore::new();
        let record = task_record("task-s2");
        store.create(record).await.expect("create");
        store
            .scan_due(DueScanInput { limit: 10 })
            .await
            .expect("scan due");
        let now = store.now().await.expect("store now");
        let claimed = store
            .claim(ClaimInput {
                task_id: TaskId::new("task-s2"),
                owner: "phase6-runtime".to_string(),
                lease_expiry: now.checked_add_millis(30_000).unwrap(),
                image_activatable: true,
            })
            .await
            .expect("claim");
        match claimed {
            skiff_task_control::store::ClaimOutcome::Claimed(record) => {
                assert_phase6_image(&record)
            }
            other => panic!("TaskStore rejected exact Phase 6 claim: {other:?}"),
        }
    }

    #[tokio::test]
    async fn task_s3() {
        let store = MemoryTaskStore::new();
        let record = task_record("task-s3");
        store.create(record).await.expect("create");
        store
            .scan_due(DueScanInput { limit: 10 })
            .await
            .expect("scan due");
        let now = store.now().await.expect("now");
        let claimed = store
            .claim(ClaimInput {
                task_id: TaskId::new("task-s3"),
                owner: "phase6-runtime".to_string(),
                lease_expiry: now.checked_add_millis(30_000).unwrap(),
                image_activatable: true,
            })
            .await
            .expect("claim");
        let record = match claimed {
            skiff_task_control::store::ClaimOutcome::Claimed(record) => record,
            other => panic!("TaskStore rejected claim: {other:?}"),
        };
        let lease_id = record
            .active_lease
            .as_ref()
            .expect("lease")
            .lease_id
            .clone();
        let renewed = store
            .renew(skiff_task_control::store::RenewInput {
                task_id: record.task_id.clone(),
                lease_id,
                new_expiry: now.checked_add_millis(60_000).unwrap(),
            })
            .await
            .expect("renew");
        let renewed = match renewed {
            skiff_task_control::store::RenewOutcome::Renewed(record) => record,
            other => panic!("TaskStore rejected exact Phase 6 lease renewal: {other:?}"),
        };
        assert_phase6_image(&renewed);
    }

    #[tokio::test]
    async fn task_s4() {
        let store = MemoryTaskStore::new();
        let record = task_record("task-s4");
        store.create(record).await.expect("create");
        let records = store.records().await;
        assert_phase6_image(&records[0]);
    }

    #[tokio::test]
    async fn task_s5() {
        let store = MemoryTaskStore::new();
        let record = task_record("task-s5");
        store.create(record).await.expect("create");
        let records = store.records().await;
        assert_eq!(records.len(), 1);
        assert_phase6_image(&records[0]);
    }

    #[tokio::test]
    async fn task_s6() {
        let store = MemoryTaskStore::new();
        let record = task_record("task-s6");
        let first = store.create(record).await.expect("create");
        let duplicate = store
            .create(task_record("task-s6"))
            .await
            .expect("idempotent duplicate create");
        assert_eq!(first, duplicate);
        assert_phase6_image(&duplicate);
    }

    #[test]
    fn actor_s1() {
        assert_phase6_owner(&actor_fence("actor-s1"));
    }

    #[test]
    fn actor_s2() {
        let registry = actor_registry();
        let key = actor_key();
        registry.ensure_present(&key, abi(), implementation(), declaration_owner(), &[]);
        let token = registry
            .reserve(&key, 1, "runtime-a", &route_authority(), 0)
            .expect("reserve");
        let fence = registry
            .commit(&token, &fence_facts(), 0, 30_000)
            .expect("commit");
        assert_phase6_owner(&fence);
    }

    #[test]
    fn actor_s3() {
        let fence = actor_fence("actor-s3");
        assert_phase6_owner(&fence);
    }

    #[test]
    fn actor_s4() {
        let fence = actor_fence("actor-s4");
        assert_eq!(fence.epoch, 1);
        assert_phase6_owner(&fence);
    }

    #[test]
    fn actor_s5() {
        let fence = actor_fence("actor-s5");
        assert!(!fence.owner_lease_id.is_empty());
        assert_phase6_owner(&fence);
    }

    #[test]
    fn actor_s6() {
        let registry = actor_registry();
        let key = actor_key();
        registry.ensure_present(&key, abi(), implementation(), declaration_owner(), &[]);
        let token = registry
            .reserve(&key, 1, "runtime-a", &route_authority(), 0)
            .expect("reserve");
        let fence = registry
            .commit(&token, &fence_facts(), 0, 30_000)
            .expect("commit");
        let current = registry.current_owner(&key).expect("current owner");
        assert_eq!(current, fence);
        assert_phase6_owner(&current);
    }

    fn task_record(id: &str) -> TaskRecord {
        let deployment = ServiceDeploymentRef {
            service_id: "example.com/phase6".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: DeploymentRevision::new("revision-1"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                "{DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX}:sha256:{}",
                "a".repeat(64)
            )),
        };
        TaskRecord {
            task_id: TaskId::new(id),
            owner: ServiceOwner::new("example.com/phase6"),
            execution: TaskExecutionImageRef {
                target_profile: "prod".to_string(),
                package_version: "1.0.0".to_string(),
                deployment,
            },
            target: DetachedCallTarget::Function {
                callable: PackageCallableId::new("example.com/phase6:run"),
            },
            payload: RecoverablePayload::new(br#"{"phase6":true}"#.to_vec()),
            due_at: DurableUtcTimestamp::from_millis(1_000),
            state: TaskState::Scheduled,
            attempt_generation: 0,
            active_lease: None,
            terminal: None,
            trace: TaskTraceContext {
                trace_id: "trace-phase6".to_string(),
                span_id: None,
            },
            created_at: DurableUtcTimestamp::from_millis(1),
            retry_not_before: None,
            test_case: None,
        }
    }

    fn assert_phase6_image(record: &TaskRecord) {
        let prefix = format!("{DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX}:sha256:");
        assert!(
            record
                .execution
                .deployment
                .deployment_artifact_identity
                .as_str()
                .starts_with(&prefix),
            "TaskStore record does not pin the Phase 6 atomic image identity: {}",
            record.execution.deployment.deployment_artifact_identity
        );
    }

    fn actor_fence(lease_id: &str) -> ActorOwnerFence {
        let registry = actor_registry();
        let key = actor_key();
        registry.ensure_present(&key, abi(), implementation(), declaration_owner(), &[]);
        let token = registry
            .reserve(&key, 1, "runtime-a", &route_authority(), 0)
            .expect("reserve");
        registry
            .commit(
                &token,
                &CommitFenceFacts {
                    actor_abi_identity: abi(),
                    actor_implementation_identity: implementation(),
                    declaration_owner: declaration_owner(),
                    owner_lease_id: lease_id.to_string(),
                },
                0,
                30_000,
            )
            .expect("commit")
    }

    fn actor_registry() -> Arc<ActorOwnershipRegistry> {
        Arc::new(ActorOwnershipRegistry::new())
    }

    fn actor_key() -> ActorLogicalKey {
        ActorLogicalKey {
            service_id: "example.com/phase6".to_string(),
            actor_type_identity: "Counter".to_string(),
            actor_id_type_identity: "CounterId".to_string(),
            actor_id_encoding_version: "skiff-actor-id-encoding-v1".to_string(),
            canonical_actor_id_key_bytes_base64: "AQID".to_string(),
            actor_id_hash: format!("sha256:{}", "1".repeat(64)),
        }
    }

    fn abi() -> ActorAbiIdentity {
        ActorAbiIdentity::new(format!("skiff-actor-abi-v1:sha256:{}", "a".repeat(64)))
    }

    fn implementation() -> ActorImplementationIdentity {
        ActorImplementationIdentity::new(format!(
            "skiff-actor-implementation-v1:sha256:{}",
            "b".repeat(64)
        ))
    }

    fn declaration_owner() -> ActorDeclarationOwnerFrameHeader {
        ActorDeclarationOwnerFrameHeader {
            unit: ActorOwnerUnitFrameHeader::Service,
            file: ActorOwnerFileFrameHeader::FileIrIdentity("file:1".to_string()),
            actor_symbol: "Counter".to_string(),
        }
    }

    fn route_authority() -> ActorOwnerRouteAuthority {
        ActorOwnerRouteAuthority {
            build_id: format!(
                "{DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX}:sha256:{}",
                "a".repeat(64)
            ),
        }
    }

    fn fence_facts() -> CommitFenceFacts {
        CommitFenceFacts {
            actor_abi_identity: abi(),
            actor_implementation_identity: implementation(),
            declaration_owner: declaration_owner(),
            owner_lease_id: "phase6-owner-lease".to_string(),
        }
    }

    fn assert_phase6_owner(fence: &ActorOwnerFence) {
        let prefix = format!("{DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX}:sha256:");
        assert!(
            fence.build_id.starts_with(&prefix),
            "Actor owner fence does not pin the Phase 6 atomic image identity: {}",
            fence.build_id
        );
    }
}
