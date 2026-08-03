//! Real-boundary Mongo probe for `MongoTaskStore`.
//!
//! Ignored by default. The harness sets `SKIFF_TASK_CONTROL_MONGO_URL`
//! (temporary replica set) and optionally `SKIFF_TASK_CONTROL_MONGO_DB`,
//! then runs this test with `--ignored`. The probe runs the same shared
//! contract matrix as the in-memory fake, plus an index existence check.

mod support;

#[cfg(test)]
mod tests {
    use futures_util::TryStreamExt;
    use mongodb::{options::ClientOptions, Client};

    use skiff_artifact_model::{
        ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity,
        RecoverableExpectedTypePlan, RecoverableExpectedTypeRoot, RecoverableTypeIdentityRef,
    };
    use skiff_deployment::projection::actor_routing::ActorRoutingRef;
    use skiff_task_control::model::{
        ActorActivationSnapshot, ActorDeclarationOwner, ActorDeclarationOwnerFile,
        ActorDeclarationOwnerUnit, DetachedCallTarget, DurableDuration, DurableUtcTimestamp,
        RecoverablePayload, TaskId, TaskState, TaskStatusKind,
    };
    use skiff_task_control::store::{
        ClaimInput, ClaimOutcome, DueScanInput, LeaseRecoveryInput, LeaseRecoveryOutcome,
        ReleaseInput, ReleaseOutcome, ScanExpiredLeasesInput, StatusInput,
    };
    use skiff_task_control::{
        MongoTaskStore, MongoTaskStoreOptions, TaskStore, TASK_STATE_DUE_AT_INDEX,
    };

    use super::support::{contract, fixtures, TestTime};

    fn required(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| {
            panic!("{name} must be set by the task-control Mongo probe harness")
        })
    }

    fn probe_options(database: &str) -> MongoTaskStoreOptions {
        MongoTaskStoreOptions {
            database: database.to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    #[ignore = "requires SKIFF_TASK_CONTROL_MONGO_URL temporary replica set managed by the probe harness"]
    async fn task_store_mongo_probe_contract_and_indexes() {
        let mongo_url = required("SKIFF_TASK_CONTROL_MONGO_URL");
        let database = std::env::var("SKIFF_TASK_CONTROL_MONGO_DB")
            .unwrap_or_else(|_| "skiff_task_control_probe".to_string());

        let store = MongoTaskStore::connect(&mongo_url, probe_options(&database))
            .await
            .expect("connect task store");
        store.ensure_indexes().await.expect("ensure indexes");

        let mut client_options = ClientOptions::parse(&mongo_url)
            .await
            .expect("parse probe client");
        client_options.app_name = Some("skiff-task-control-probe".to_string());
        let client = Client::with_options(client_options).expect("probe client");
        let mut indexes = client
            .database(&database)
            .collection::<mongodb::bson::Document>("tasks")
            .list_indexes()
            .await
            .expect("list indexes");
        let mut found = false;
        while let Some(index) = indexes.try_next().await.expect("index stream") {
            if index
                .options
                .as_ref()
                .and_then(|options| options.name.as_deref())
                == Some(TASK_STATE_DUE_AT_INDEX)
            {
                found = true;
                assert_eq!(
                    index.keys.get("state"),
                    Some(&mongodb::bson::Bson::Int32(1))
                );
                assert_eq!(
                    index.keys.get("dueAt"),
                    Some(&mongodb::bson::Bson::Int32(1))
                );
            }
        }
        assert!(found, "{TASK_STATE_DUE_AT_INDEX} index must exist");
        contract::run_contract(&store, &TestTime::WallClock).await;
        scheduler_store_extensions(&store).await;
        actor_record_round_trip(&store).await;
        store.close().await.expect("close");
    }

    /// Actor-method record round trip (E2b store extension): the runtime-form
    /// expected-type plan and declaration owner survive the Mongo DTO.
    async fn actor_record_round_trip(store: &MongoTaskStore) {
        let now = store.now().await.expect("store authority now");
        let mut record = fixtures::record(9_002, now.millis() - 1_000);
        record.task_id = TaskId::new("task-actor-mongo");
        record.created_at = now;
        let expected_type_plan = RecoverableExpectedTypePlan {
            root: RecoverableExpectedTypeRoot::TypeIdentityRef {
                type_identity_ref: RecoverableTypeIdentityRef("type-actor".to_string()),
            },
            root_type_identity_ref: None,
            runtime_carrier_check_required: false,
            interface_projection_refs: Vec::new(),
            interface_method_refs: Vec::new(),
            field_refs: Vec::new(),
            union_branch_refs: Vec::new(),
        };
        record.target = DetachedCallTarget::ActorMethod {
            actor: ActorRoutingRef {
                service_id: "svc-9002".to_string(),
                actor_abi_identity: ActorAbiIdentity::new(
                    "skiff-actor-abi-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                ),
            },
            activation: ActorActivationSnapshot {
                key: RecoverablePayload::new(vec![1, 2, 3]),
                create_input: RecoverablePayload::new(b"[]".to_vec()),
                expected_type_plan: expected_type_plan.clone(),
                expected_type_plan_runtime: Some(serde_json::json!({
                    "label": "record",
                    "node": { "kind": "record", "fields": [] }
                })),
            },
            implementation: ActorImplementationIdentity::new(
                "skiff-actor-implementation-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            method: ActorMethodIdentity::new(
                "skiff-actor-method-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            declaration_owner: ActorDeclarationOwner {
                unit: ActorDeclarationOwnerUnit::Service,
                file: ActorDeclarationOwnerFile::LoadedFileIndex(0),
                actor_symbol: "Actor".to_string(),
            },
        };
        store
            .create(record.clone())
            .await
            .expect("create actor task record");
        let status = store
            .status(StatusInput {
                task_id: record.task_id.clone(),
                retention: DurableDuration::from_millis(60_000),
            })
            .await
            .expect("actor task status");
        assert_eq!(status.kind, TaskStatusKind::Scheduled);
        let records = store
            .scan_due(DueScanInput { limit: 10 })
            .await
            .expect("scan actor task");
        let round = records
            .into_iter()
            .find(|candidate| candidate.task_id == record.task_id)
            .expect("actor task visible after round trip");
        assert_eq!(round.target, record.target, "actor target round trips");
    }

    /// Scheduler-owned store extensions driven by the real server clock:
    /// authority `now`, provable-rejection `release` with atomic retry
    /// not-before, and the expired-lease scan feeding recovery.
    async fn scheduler_store_extensions(store: &MongoTaskStore) {
        let task_id = TaskId::new("task-scheduler-ext");
        let now = store.now().await.expect("store authority now");
        let mut record = fixtures::record(9_001, now.millis() - 1_000);
        record.task_id = task_id.clone();
        store.create(record).await.expect("create extension task");
        store
            .scan_due(DueScanInput { limit: 10 })
            .await
            .expect("scan due extension task");

        let claimed = match store
            .claim(ClaimInput {
                task_id: task_id.clone(),
                owner: "probe-scheduler".to_string(),
                lease_expiry: now.checked_add_millis(2_000).expect("expiry"),
                image_activatable: true,
            })
            .await
            .expect("claim extension task")
        {
            ClaimOutcome::Claimed(record) => record,
            other => panic!("claim failed: {other:?}"),
        };
        let lease_id = claimed.active_lease.expect("lease").lease_id.clone();
        let retry = now.checked_add_millis(5_000).expect("retry");
        let released = match store
            .release(ReleaseInput {
                task_id: task_id.clone(),
                lease_id,
                retry_not_before: retry,
            })
            .await
            .expect("release")
        {
            ReleaseOutcome::Released(record) => record,
            other => panic!("release failed: {other:?}"),
        };
        assert_eq!(released.state, TaskState::Ready);
        assert_eq!(released.retry_not_before, Some(retry));

        // Expired-lease scan finds the lease after the server clock passes it.
        store
            .claim(ClaimInput {
                task_id: task_id.clone(),
                owner: "probe-scheduler".to_string(),
                lease_expiry: now.checked_add_millis(1_500).expect("expiry"),
                image_activatable: true,
            })
            .await
            .expect("reclaim for expiry scan");
        tokio::time::sleep(std::time::Duration::from_millis(2_000)).await;
        let expired = store
            .scan_expired_leases(ScanExpiredLeasesInput { limit: 10 })
            .await
            .expect("scan expired leases");
        assert!(
            expired.iter().any(|record| record.task_id == task_id),
            "expired lease must be visible to the recovery loop"
        );
        let recovered = store
            .recover_expired_lease(LeaseRecoveryInput {
                task_id: task_id.clone(),
                retry_not_before: retry,
            })
            .await
            .expect("recover expired extension task");
        match recovered {
            LeaseRecoveryOutcome::Recovered(record) => {
                assert_eq!(record.state, TaskState::Ready);
                assert_eq!(record.retry_not_before, Some(retry));
            }
            other => panic!("recovery failed: {other:?}"),
        }
    }
}
