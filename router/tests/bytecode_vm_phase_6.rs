#[path = "bytecode_vm_phase_6/production_path.rs"]
mod production_path;

use std::sync::{Arc, Mutex};

use skiff_artifact_identity::DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX;
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, DeploymentArtifactIdentity,
    DeploymentRevision, PackageCallableId, RecoverableExpectedTypePlan,
    RecoverableExpectedTypeRoot, ServiceDeploymentRef, TypeRefIr,
};
use skiff_deployment::projection::actor_routing::ActorRoutingRef;
use skiff_router::actor::{
    ActorLogicalKey, ActorOwnerFence, ActorOwnerRouteAuthority, ActorOwnershipRegistry,
    CommitFenceFacts,
};
use skiff_runtime_transport::actor_method::{
    ActorDeclarationOwnerFrameHeader, ActorOwnerFileFrameHeader, ActorOwnerUnitFrameHeader,
};
use skiff_task_control::clock::TaskClock;
use skiff_task_control::model::{
    ActorActivationSnapshot, ActorDeclarationOwner, ActorDeclarationOwnerFile,
    ActorDeclarationOwnerUnit, DetachedCallTarget, DurableUtcTimestamp, LeaseId,
    RecoverablePayload, ServiceOwner, TaskExecutionImageRef, TaskId, TaskOutcome, TaskRecord,
    TaskState, TaskTerminal, TaskTraceContext,
};
use skiff_task_control::store::{
    ClaimInput, DueScanInput, LeaseRecoveryInput, RenewInput, ScanExpiredLeasesInput, SettleInput,
    TaskStore,
};
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

    #[tokio::test]
    async fn task_fresh_attempt_lease_renew_retry() {
        let clock = Arc::new(Mutex::new(1_000_i64));
        let store = MemoryTaskStore::with_clock(Arc::new(TestClock(Arc::clone(&clock))));
        store
            .create(task_record("task-fresh-retry"))
            .await
            .expect("create");

        let first = claim_ready(&store, "task-fresh-retry").await;
        assert_eq!(first.attempt_generation, 1);
        let first_lease = first
            .active_lease
            .as_ref()
            .expect("first attempt lease")
            .clone();
        assert!(!first_lease.attempt_id.as_str().is_empty());
        assert!(!first_lease.lease_id.as_str().is_empty());

        let renewed = store
            .renew(RenewInput {
                task_id: first.task_id.clone(),
                lease_id: first_lease.lease_id.clone(),
                new_expiry: DurableUtcTimestamp::from_millis(61_000),
            })
            .await
            .expect("renew");
        assert!(matches!(
            renewed,
            skiff_task_control::store::RenewOutcome::Renewed(_)
        ));

        *clock.lock().expect("clock") = 61_001;
        let expired = store
            .scan_expired_leases(ScanExpiredLeasesInput { limit: 10 })
            .await
            .expect("scan expired leases");
        assert!(
            expired
                .iter()
                .any(|record| record.task_id.as_str() == "task-fresh-retry"),
            "expired lease must be visible for recovery"
        );
        let recovered = store
            .recover_expired_lease(LeaseRecoveryInput {
                task_id: first.task_id.clone(),
                retry_not_before: DurableUtcTimestamp::from_millis(71_000),
            })
            .await
            .expect("recover expired lease");
        let recovered = match recovered {
            skiff_task_control::store::LeaseRecoveryOutcome::Recovered(record) => record,
            other => panic!("lease recovery rejected fresh retry: {other:?}"),
        };
        assert_eq!(recovered.state, TaskState::Ready);

        *clock.lock().expect("clock") = 80_000;
        let second = claim_ready(&store, "task-fresh-retry").await;
        assert_eq!(second.attempt_generation, 2);
        let second_lease = second.active_lease.clone().expect("second attempt lease");
        assert_ne!(second_lease.lease_id, first_lease.lease_id);
        assert_phase6_image(&second);
    }

    #[tokio::test]
    async fn task_settlement_idempotent_and_stale_lease_rejected() {
        let store = MemoryTaskStore::new();
        store
            .create(task_record("task-settlement"))
            .await
            .expect("create");
        let claimed = claim_ready(&store, "task-settlement").await;
        let task_id = claimed.task_id.clone();
        let lease_id = claimed
            .active_lease
            .as_ref()
            .expect("lease")
            .lease_id
            .clone();
        let terminal = TaskTerminal {
            settled_at: store.now().await.expect("now"),
            outcome: TaskOutcome::Succeeded,
        };
        let first = store
            .settle(SettleInput {
                task_id: task_id.clone(),
                lease_id: lease_id.clone(),
                terminal: terminal.clone(),
            })
            .await
            .expect("settle");
        assert!(matches!(
            first,
            skiff_task_control::store::SettleOutcome::Settled(_)
        ));
        let duplicate = store
            .settle(SettleInput {
                task_id: task_id.clone(),
                lease_id: lease_id.clone(),
                terminal: terminal.clone(),
            })
            .await
            .expect("settle duplicate");
        assert!(matches!(
            duplicate,
            skiff_task_control::store::SettleOutcome::AlreadySettled(_)
        ));
        let stale = store
            .settle(SettleInput {
                task_id,
                lease_id: LeaseId::new("stale-lease"),
                terminal,
            })
            .await
            .expect("settle stale");
        assert!(
            matches!(
                stale,
                skiff_task_control::store::SettleOutcome::StaleLease
                    | skiff_task_control::store::SettleOutcome::AlreadySettled(_)
                    | skiff_task_control::store::SettleOutcome::NotLeased
                    | skiff_task_control::store::SettleOutcome::Conflict(_)
            ),
            "stale settlement after terminal must not win: {stale:?}"
        );
    }

    #[tokio::test]
    async fn actor_task_fresh_attempt_and_lease() {
        let store = MemoryTaskStore::new();
        store
            .create(owner_actor_task_record("actor-task-fresh"))
            .await
            .expect("create actor task");
        let claimed = claim_ready(&store, "actor-task-fresh").await;
        assert_eq!(claimed.attempt_generation, 1);
        assert!(claimed.active_lease.is_some());
        assert_phase6_image(&claimed);

        let DetachedCallTarget::ActorMethod {
            activation,
            implementation: implementation_identity,
            method,
            declaration_owner,
            ..
        } = &claimed.target
        else {
            panic!("actor task must preserve the ActorMethod detached target");
        };
        assert_eq!(activation.create_input.as_bytes(), b"create");
        assert_eq!(implementation_identity, &implementation());
        assert_eq!(method, &method_identity());
        assert_eq!(declaration_owner.actor_symbol, "Counter");
    }

    #[test]
    fn actor_s1() {
        assert_phase6_owner(&owner_fence("actor-s1"));
    }

    #[test]
    fn actor_s2() {
        let registry = owner_registry();
        let key = owner_key();
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
        let fence = owner_fence("actor-s3");
        assert_phase6_owner(&fence);
    }

    #[test]
    fn actor_s4() {
        let fence = owner_fence("actor-s4");
        assert_eq!(fence.epoch, 1);
        assert_phase6_owner(&fence);
    }

    #[test]
    fn actor_s5() {
        let fence = owner_fence("actor-s5");
        assert!(!fence.owner_lease_id.is_empty());
        assert_phase6_owner(&fence);
    }

    #[test]
    fn actor_s6() {
        let registry = owner_registry();
        let key = owner_key();
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

    #[derive(Debug, Clone)]
    struct TestClock(Arc<Mutex<i64>>);

    impl TaskClock for TestClock {
        fn now_millis(&self) -> i64 {
            *self.0.lock().expect("clock")
        }
    }

    async fn claim_ready(store: &MemoryTaskStore, task_id: &str) -> TaskRecord {
        let records = store
            .scan_due(DueScanInput { limit: 10 })
            .await
            .expect("scan due");
        let record = records
            .into_iter()
            .find(|record| record.task_id.as_str() == task_id)
            .unwrap_or_else(|| panic!("task {task_id} is not due and ready"));
        let expiry = store
            .now()
            .await
            .expect("store now")
            .checked_add_millis(60_000)
            .expect("lease expiry");
        match store
            .claim(ClaimInput {
                task_id: record.task_id.clone(),
                owner: "phase6-runtime".to_string(),
                lease_expiry: expiry,
                image_activatable: true,
            })
            .await
            .expect("claim")
        {
            skiff_task_control::store::ClaimOutcome::Claimed(record) => record,
            other => panic!("TaskStore rejected fresh Phase 6 claim: {other:?}"),
        }
    }

    fn method_identity() -> ActorMethodIdentity {
        ActorMethodIdentity::new(format!("skiff-actor-method-v1:sha256:{}", "a".repeat(64)))
    }

    fn owner_actor_task_record(id: &str) -> TaskRecord {
        let mut record = task_record(id);
        record.target = DetachedCallTarget::ActorMethod {
            actor: ActorRoutingRef {
                service_id: "example.com/phase6".to_string(),
                actor_abi_identity: abi(),
            },
            activation: ActorActivationSnapshot {
                key: RecoverablePayload::new(b"actor-key".to_vec()),
                create_input: RecoverablePayload::new(b"create".to_vec()),
                expected_type_plan: RecoverableExpectedTypePlan {
                    root: RecoverableExpectedTypeRoot::TypeRef {
                        ty: TypeRefIr::builtin("number"),
                    },
                    root_type_identity_ref: None,
                    runtime_carrier_check_required: false,
                    interface_projection_refs: Vec::new(),
                    interface_method_refs: Vec::new(),
                    field_refs: Vec::new(),
                    union_branch_refs: Vec::new(),
                },
                expected_type_plan_runtime: Some(serde_json::json!({
                    "kind": "builtin",
                    "name": "number",
                    "args": [],
                })),
            },
            implementation: implementation(),
            method: method_identity(),
            declaration_owner: ActorDeclarationOwner {
                unit: ActorDeclarationOwnerUnit::Service,
                file: ActorDeclarationOwnerFile::FileIrIdentity("file:1".to_string()),
                actor_symbol: "Counter".to_string(),
            },
        };
        record
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

    fn owner_fence(lease_id: &str) -> ActorOwnerFence {
        let registry = owner_registry();
        let key = owner_key();
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

    fn owner_registry() -> Arc<ActorOwnershipRegistry> {
        Arc::new(ActorOwnershipRegistry::new())
    }

    fn owner_key() -> ActorLogicalKey {
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
