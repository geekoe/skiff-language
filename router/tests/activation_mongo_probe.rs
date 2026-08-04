//! P-activation-state real boundary probe (C-router-activation-state §11):
//! temporary Mongo replica set + real repository.
//!
//! Ignored by default. The harness (`scripts/run-router-activation-mongo-probe.mjs`)
//! starts an isolated mongod replica set (leased port, temp dbPath), sets
//! `SKIFF_ACTIVATION_MONGO_URL`/`SKIFF_ACTIVATION_MONGO_DB`, runs this test with
//! `--ignored`, and cleans up. Assertions: CAS conflicts, concurrent identical
//! prepare retry with a single audit, audit-failure rollback, indexes, and
//! reconnect (new repository instance) read consistency.

#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use mongodb::bson::doc;
    use skiff_artifact_model::{
        AssemblyIdentity, RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
    };
    use skiff_deployment::activation_state::{
        activation_audit_event_id, ActivationAuditOperation, ProfileActivationState,
    };
    use skiff_router::activation::{
        repository::CommitInput, repository::PrepareInput, ActivationStateRepository,
        MongoActivationStateRepository, MongoActivationStateRepositoryOptions, RetryPolicy,
        SystemClock,
    };

    fn assembly(byte: u8) -> RuntimeAssemblyRef {
        RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(format!(
                "skiff-runtime-assembly-v3:sha256:{}",
                char::from(b'a' + byte).to_string().repeat(64)
            )),
        }
    }

    fn config(byte: u8) -> RuntimeConfigSnapshotRef {
        RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(format!(
                "skiff-runtime-config-snapshot-v1:{}",
                char::from(b'a' + byte).to_string().repeat(32)
            ))
            .expect("config snapshot id"),
        }
    }

    fn initial_state() -> ProfileActivationState {
        ProfileActivationState::initial("probe", 7, assembly(0), config(0))
    }

    fn prepare_input(activation_id: &str) -> PrepareInput {
        PrepareInput {
            profile: "probe".to_string(),
            activation_id: activation_id.to_string(),
            expected_generation: 7,
            candidate_generation: 8,
            assembly: assembly(1),
            config_snapshot: config(1),
            participant_replica_ids: vec!["runtime-a".to_string(), "runtime-b".to_string()],
        }
    }

    fn commit_input(prepared: &ProfileActivationState) -> CommitInput {
        let pending = prepared.pending.as_ref().expect("prepared pending");
        CommitInput {
            profile: "probe".to_string(),
            activation_id: pending.activation_id.clone(),
            expected_generation: pending.expected_generation,
            candidate_generation: pending.candidate_generation,
            assembly: pending.assembly.clone(),
            config_snapshot: pending.config_snapshot.clone(),
            connected_replica_ids: pending.participant_replica_ids.clone(),
            prepared_replica_ids: pending.participant_replica_ids.clone(),
        }
    }

    fn probe_options(database: &str) -> MongoActivationStateRepositoryOptions {
        MongoActivationStateRepositoryOptions {
            database: database.to_string(),
            retry: RetryPolicy {
                max_attempts: 6,
                base_delay: std::time::Duration::from_millis(25),
                max_delay: std::time::Duration::from_millis(250),
                total_deadline: std::time::Duration::from_secs(10),
            },
            ..Default::default()
        }
    }

    async fn connect(database: &str) -> MongoActivationStateRepository {
        let mongo_url = std::env::var("SKIFF_ACTIVATION_MONGO_URL")
            .expect("SKIFF_ACTIVATION_MONGO_URL must be set by the probe harness");
        MongoActivationStateRepository::connect(
            &mongo_url,
            probe_options(database),
            Arc::new(SystemClock),
        )
        .await
        .expect("connect repository")
    }

    #[tokio::test]
    #[ignore = "requires SKIFF_ACTIVATION_MONGO_URL temporary replica set managed by the probe harness"]
    async fn activation_mongo_probe_cas_retry_audit_and_reconnect() {
        let database = std::env::var("SKIFF_ACTIVATION_MONGO_DB")
            .unwrap_or_else(|_| "skiff_router_activation_probe".to_string());
        let repository = connect(&database).await;
        repository.ensure_indexes().await.expect("ensure indexes");

        repository
            .initialize(&initial_state())
            .await
            .expect("initialize");

        // CAS: stale generation prepare is rejected without durable effect.
        let mut stale = prepare_input("activation-stale");
        stale.expected_generation = 6;
        stale.candidate_generation = 7;
        assert!(
            repository.prepare(stale).await.is_err(),
            "stale prepare must fail CAS"
        );
        assert_eq!(repository.read("probe").await.expect("read").pending, None);

        // Concurrent identical prepare: exactly one transaction wins; the loser
        // retries through the transient path and replays idempotently. Exactly one
        // audit event must exist afterwards.
        let first = connect(&database).await;
        let second = connect(&database).await;
        let input = prepare_input("activation-8");
        let (left, right) =
            tokio::join!(first.prepare(input.clone()), second.prepare(input.clone()));
        let prepared_left = left.expect("first concurrent prepare");
        let prepared_right = right.expect("second concurrent prepare");
        assert_eq!(
            prepared_left, prepared_right,
            "concurrent identical prepare converges"
        );
        assert!(prepared_left.pending.is_some());

        // CAS: a different pending tuple for the same slot is rejected.
        let mut conflicting = prepare_input("activation-8x");
        conflicting.participant_replica_ids = vec!["runtime-a".to_string()];
        assert!(
            repository.prepare(conflicting).await.is_err(),
            "different pending tuple must fail CAS"
        );

        let audit_count = count_audit_documents(&database, "probe", "activation-8").await;
        assert_eq!(
            audit_count, 1,
            "concurrent identical prepare retries must not duplicate audit"
        );

        // Audit failure rollback: pre-insert the exact event id, then prepare; the
        // state update succeeds inside the transaction but the audit append fails,
        // so the whole mutation rolls back (bounded retries then transient error).
        // The pending slot must be free first, so abort the concurrent prepare.
        repository
            .abort(skiff_router::activation::repository::AbortInput {
                profile: "probe".to_string(),
                activation_id: "activation-8".to_string(),
                expected_generation: 7,
            })
            .await
            .expect("abort before audit failure probe");
        let event_id = activation_audit_event_id(
            "probe",
            "activation-failure",
            ActivationAuditOperation::Prepare,
            7,
            8,
        );
        preinsert_audit_event(&database, &event_id).await;
        let failure_options = MongoActivationStateRepositoryOptions {
            retry: RetryPolicy {
                max_attempts: 2,
                base_delay: std::time::Duration::from_millis(10),
                max_delay: std::time::Duration::from_millis(20),
                total_deadline: std::time::Duration::from_secs(5),
            },
            ..probe_options(&database)
        };
        let mongo_url = std::env::var("SKIFF_ACTIVATION_MONGO_URL").expect("mongo url");
        let failing = MongoActivationStateRepository::connect(
            &mongo_url,
            failure_options,
            Arc::new(SystemClock),
        )
        .await
        .expect("connect failing repository");
        let error = failing
            .prepare(prepare_input("activation-failure"))
            .await
            .expect_err("audit conflict must fail");
        assert!(
            format!("{error}").contains("audit append failed"),
            "audit failure must be reported: {error}"
        );
        let after_failure = failing.read("probe").await.expect("read after failure");
        assert_eq!(
            after_failure.pending, None,
            "audit failure must roll back the failed prepare"
        );
        assert_eq!(
            count_audit_documents_by_id(&database, &event_id).await,
            1,
            "only the pre-inserted conflicting event exists"
        );

        // Commit the real pending tuple (re-prepare after the rollback probe), then
        // reconnect through a fresh repository instance and verify committed/pending
        // read consistency.
        let re_prepared = repository
            .prepare(prepare_input("activation-9"))
            .await
            .expect("re-prepare");
        let committed = repository
            .commit(commit_input(&re_prepared))
            .await
            .expect("commit");
        assert_eq!(committed.committed.generation, 8);
        let reconnected = connect(&database).await;
        let read = reconnected.read("probe").await.expect("reconnected read");
        assert_eq!(
            read, committed,
            "fresh driver must read the same committed tuple"
        );

        // Shutdown terminal: closed repository refuses operations; the new
        // repository remains usable.
        repository.close().await.expect("close");
        assert!(
            repository.read("probe").await.is_err(),
            "closed repository must fail reads"
        );
        assert_eq!(
            reconnected
                .read("probe")
                .await
                .expect("still usable")
                .committed
                .generation,
            8
        );

        // Cleanup of the probe namespace is best-effort (harness drops the temp
        // mongod entirely), but the unique indexes must have been created.
        assert_probe_indexes(&database).await;
    }

    async fn preinsert_audit_event(database: &str, event_id: &str) {
        let mongo_url = std::env::var("SKIFF_ACTIVATION_MONGO_URL").expect("mongo url");
        let client = mongodb::Client::with_uri_str(&mongo_url)
            .await
            .expect("raw client");
        client
            .database(database)
            .collection::<mongodb::bson::Document>("activation_audit")
            .insert_one(doc! { "_id": event_id })
            .await
            .expect("pre-insert conflicting audit event");
    }

    async fn count_audit_documents(database: &str, profile: &str, activation_id: &str) -> usize {
        let mongo_url = std::env::var("SKIFF_ACTIVATION_MONGO_URL").expect("mongo url");
        let client = mongodb::Client::with_uri_str(&mongo_url)
            .await
            .expect("raw client");
        client
            .database(database)
            .collection::<mongodb::bson::Document>("activation_audit")
            .count_documents(doc! {
                "profile": profile,
                "activationId": activation_id
            })
            .await
            .expect("count audit documents") as usize
    }

    async fn count_audit_documents_by_id(database: &str, event_id: &str) -> usize {
        let mongo_url = std::env::var("SKIFF_ACTIVATION_MONGO_URL").expect("mongo url");
        let client = mongodb::Client::with_uri_str(&mongo_url)
            .await
            .expect("raw client");
        client
            .database(database)
            .collection::<mongodb::bson::Document>("activation_audit")
            .count_documents(doc! { "_id": event_id })
            .await
            .expect("count audit documents by id") as usize
    }

    async fn assert_probe_indexes(database: &str) {
        let mongo_url = std::env::var("SKIFF_ACTIVATION_MONGO_URL").expect("mongo url");
        let client = mongodb::Client::with_uri_str(&mongo_url)
            .await
            .expect("raw client");
        let states = client
            .database(database)
            .collection::<mongodb::bson::Document>("activation_state")
            .list_index_names()
            .await
            .expect("state index names");
        assert!(states
            .iter()
            .any(|name| name == "activation_state_profile_unique"));
        let audit = client
            .database(database)
            .collection::<mongodb::bson::Document>("activation_audit")
            .list_index_names()
            .await
            .expect("audit index names");
        assert!(audit
            .iter()
            .any(|name| name == "activation_audit_query_key"));
        assert!(audit
            .iter()
            .any(|name| name == "activation_audit_maintenance"));
    }
}
