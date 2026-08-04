//! Router-owned activation state repository: port and Mongo adapter.
//!
//! Implements C-router-activation-state §3-§10: read, initialize,
//! prepare/commit/abort CAS, transactional audit append, bounded transient
//! retry, indexes, driver lifecycle, and health. The pure CAS semantics live
//! in `skiff-deployment::activation_state`; this adapter keeps the exact same
//! outcomes as the frozen file adapter while serializing concurrent mutations
//! with snapshot transactions plus CAS filters built from the derived tuple.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use async_trait::async_trait;
use mongodb::{
    bson::{doc, to_document, Bson, Document},
    error::{
        Error as MongoError, ErrorKind, WriteFailure, TRANSIENT_TRANSACTION_ERROR,
        UNKNOWN_TRANSACTION_COMMIT_RESULT,
    },
    options::{Acknowledgment, ClientOptions, ReadConcern, TransactionOptions, WriteConcern},
    Client, ClientSession, Collection,
};
use serde_json::Value as JsonValue;
use skiff_deployment::activation_state::{
    abort, commit, prepare, ActivationAuditEvent, ActivationAuditOperation, ProfileActivationState,
};

use super::{
    error::{cas_mismatch, invalid_record, map_reducer_error, RepositoryError},
    health::{ActivationRepositoryHealth, AuditHealth, RepositoryMutationOutcome, RetryHealth},
    index::{
        activation_audit_maintenance_index, activation_audit_query_index,
        activation_state_profile_index,
    },
    retry::{ActivationClock, RetryOutcome, RetryPolicy},
};

pub use skiff_deployment::activation_state::{AbortInput, CommitInput, PrepareInput};

pub const DEFAULT_ACTIVATION_DATABASE: &str = "skiff-router";
pub const DEFAULT_ACTIVATION_STATE_COLLECTION: &str = "activation_state";
pub const DEFAULT_ACTIVATION_AUDIT_COLLECTION: &str = "activation_audit";

#[async_trait]
pub trait ActivationStateRepository: Send + Sync {
    async fn read(&self, profile: &str) -> Result<ProfileActivationState, RepositoryError>;

    async fn initialize(
        &self,
        state: &ProfileActivationState,
    ) -> Result<ProfileActivationState, RepositoryError>;

    async fn prepare(&self, input: PrepareInput)
        -> Result<ProfileActivationState, RepositoryError>;

    async fn commit(&self, input: CommitInput) -> Result<ProfileActivationState, RepositoryError>;

    async fn abort(&self, input: AbortInput) -> Result<ProfileActivationState, RepositoryError>;

    async fn append_audit(&self, event: &ActivationAuditEvent) -> Result<(), RepositoryError>;

    async fn ensure_indexes(&self) -> Result<(), RepositoryError>;

    fn health(&self) -> ActivationRepositoryHealth;

    async fn close(&self) -> Result<(), RepositoryError>;
}

#[derive(Debug, Clone)]
pub struct MongoActivationStateRepositoryOptions {
    pub database: String,
    pub state_collection: String,
    pub audit_collection: String,
    pub retry: RetryPolicy,
    pub connect_timeout_ms: u64,
    pub server_selection_timeout_ms: u64,
}

impl Default for MongoActivationStateRepositoryOptions {
    fn default() -> Self {
        Self {
            database: DEFAULT_ACTIVATION_DATABASE.to_string(),
            state_collection: DEFAULT_ACTIVATION_STATE_COLLECTION.to_string(),
            audit_collection: DEFAULT_ACTIVATION_AUDIT_COLLECTION.to_string(),
            retry: RetryPolicy::default(),
            connect_timeout_ms: 10_000,
            server_selection_timeout_ms: 10_000,
        }
    }
}

#[derive(Clone)]
pub struct MongoActivationStateRepository {
    client: Client,
    states: Collection<Document>,
    audit: Collection<Document>,
    options: MongoActivationStateRepositoryOptions,
    clock: Arc<dyn ActivationClock>,
    health_state: Arc<Mutex<ActivationRepositoryHealth>>,
    closed: Arc<AtomicBool>,
}

#[derive(Debug)]
enum MutationFailure {
    Repository(RepositoryError),
    Audit {
        error: RepositoryError,
        event: ActivationAuditEvent,
    },
}

#[derive(Debug, Default)]
struct MutationTrace {
    event: Option<ActivationAuditEvent>,
    audit_failed: bool,
}

impl MongoActivationStateRepository {
    pub async fn connect(
        mongo_url: &str,
        options: MongoActivationStateRepositoryOptions,
        clock: Arc<dyn ActivationClock>,
    ) -> Result<Self, RepositoryError> {
        let mut client_options = ClientOptions::parse(mongo_url)
            .await
            .map_err(map_driver_error)?;
        client_options.connect_timeout =
            Some(std::time::Duration::from_millis(options.connect_timeout_ms));
        client_options.server_selection_timeout = Some(std::time::Duration::from_millis(
            options.server_selection_timeout_ms,
        ));
        client_options.retry_writes = Some(false);
        client_options.app_name = Some("skiff-router-activation-state".to_string());
        let client = Client::with_options(client_options).map_err(map_driver_error)?;
        let database = client.database(&options.database);
        let repository = Self {
            client,
            states: database.collection::<Document>(&options.state_collection),
            audit: database.collection::<Document>(&options.audit_collection),
            options,
            clock,
            health_state: Arc::new(Mutex::new(ActivationRepositoryHealth::default())),
            closed: Arc::new(AtomicBool::new(false)),
        };
        repository.mark_driver_connected();
        Ok(repository)
    }

    fn check_open(&self) -> Result<(), RepositoryError> {
        if self.closed.load(Ordering::SeqCst) {
            Err(RepositoryError::Closed)
        } else {
            Ok(())
        }
    }

    async fn mutate<F>(
        &self,
        profile: &str,
        operation: ActivationAuditOperation,
        reduce: F,
    ) -> Result<ProfileActivationState, RepositoryError>
    where
        F: Fn(&ProfileActivationState) -> Result<ProfileActivationState, RepositoryError>
            + Send
            + Sync,
    {
        self.check_open()?;
        let audit_failure = Arc::new(AtomicBool::new(false));
        let audit_flag = audit_failure.clone();
        let (result, outcome) = self
            .options
            .retry
            .run(self.clock.as_ref(), || {
                self.mutate_attempt(profile, operation, &reduce, &audit_flag)
            })
            .await;
        let trace = MutationTrace {
            event: result.as_ref().ok().and_then(|(_, event)| event.clone()),
            audit_failed: audit_failure.load(Ordering::SeqCst),
        };
        let result = result.map(|(state, _)| state);
        self.record_mutation_health(profile, operation, &result, &outcome, &trace);
        result
    }

    async fn mutate_attempt<F>(
        &self,
        profile: &str,
        operation: ActivationAuditOperation,
        reduce: &F,
        audit_flag: &AtomicBool,
    ) -> Result<(ProfileActivationState, Option<ActivationAuditEvent>), RepositoryError>
    where
        F: Fn(&ProfileActivationState) -> Result<ProfileActivationState, RepositoryError> + Sync,
    {
        let mut session = self
            .client
            .start_session()
            .await
            .map_err(map_driver_error)?;
        let transaction_options = TransactionOptions::builder()
            .read_concern(Some(ReadConcern::snapshot()))
            .write_concern(Some(
                WriteConcern::builder().w(Acknowledgment::Majority).build(),
            ))
            .build();
        session
            .start_transaction()
            .with_options(transaction_options)
            .await
            .map_err(map_driver_error)?;
        let attempt = self
            .mutate_once(&mut session, profile, operation, reduce)
            .await;
        match attempt {
            Ok((next, event)) => match session.commit_transaction().await {
                Ok(()) => Ok((next, event)),
                Err(error) => {
                    let _ = session.abort_transaction().await;
                    Err(map_driver_error(error))
                }
            },
            Err(MutationFailure::Repository(error)) => {
                let _ = session.abort_transaction().await;
                Err(error)
            }
            Err(MutationFailure::Audit { error, event }) => {
                let _ = session.abort_transaction().await;
                audit_flag.store(true, Ordering::SeqCst);
                let _ = event;
                Err(RepositoryError::Transient {
                    message: format!(
                        "activation audit append failed and mutation was rolled back: {error}"
                    ),
                })
            }
        }
    }

    async fn mutate_once<F>(
        &self,
        session: &mut ClientSession,
        profile: &str,
        operation: ActivationAuditOperation,
        reduce: &F,
    ) -> Result<(ProfileActivationState, Option<ActivationAuditEvent>), MutationFailure>
    where
        F: Fn(&ProfileActivationState) -> Result<ProfileActivationState, RepositoryError>,
    {
        let document = self
            .states
            .find_one(doc! { "_id": profile })
            .session(&mut *session)
            .await
            .map_err(map_driver_error)
            .map_err(MutationFailure::Repository)?;
        let Some(document) = document else {
            return Err(MutationFailure::Repository(cas_mismatch(
                profile,
                "activation state does not exist",
            )));
        };
        let current =
            decode_state_document(&document, profile).map_err(MutationFailure::Repository)?;
        let next = reduce(&current).map_err(MutationFailure::Repository)?;
        if next == current {
            return Ok((next, None));
        }
        let cas_filter =
            cas_filter(profile, operation, &current).map_err(MutationFailure::Repository)?;
        let next_document = state_document(&next).map_err(MutationFailure::Repository)?;
        let update = self
            .states
            .update_one(cas_filter, doc! { "$set": { "state": &next_document } })
            .session(&mut *session)
            .await
            .map_err(map_driver_error)
            .map_err(MutationFailure::Repository)?;
        if update.matched_count != 1 {
            return Err(MutationFailure::Repository(cas_mismatch(
                profile,
                format!(
                    "activation state CAS conflict during {}",
                    operation.as_str()
                ),
            )));
        }
        let event = audit_event_for(operation, &current, &next, self.clock.now_millis());
        let audit_document = audit_document(&event).map_err(MutationFailure::Repository)?;
        self.audit
            .insert_one(audit_document)
            .session(&mut *session)
            .await
            .map_err(|error| MutationFailure::Audit {
                error: map_driver_error(error),
                event: event.clone(),
            })?;
        Ok((next, Some(event)))
    }

    fn record_mutation_health(
        &self,
        profile: &str,
        operation: ActivationAuditOperation,
        result: &Result<ProfileActivationState, RepositoryError>,
        outcome: &RetryOutcome,
        trace: &MutationTrace,
    ) {
        let mut health = self.health_state.lock().expect("health lock");
        health.retry = RetryHealth {
            attempts: outcome.attempts,
            retried: outcome.retried,
            next_backoff_ms: outcome.next_backoff_ms,
            deadline_remaining_ms: outcome.deadline_remaining_ms,
        };
        health.last_outcome_operation = Some(operation.as_str().to_string());
        match result {
            Ok(state) => {
                health.profile = Some(profile.to_string());
                health.committed_generation = Some(state.committed.generation);
                health.pending_activation_id = state
                    .pending
                    .as_ref()
                    .map(|pending| pending.activation_id.clone());
                health.last_outcome = Some(RepositoryMutationOutcome::Ok);
                health.driver.connected = true;
                health.driver.reconnecting = false;
                if let Some(event) = &trace.event {
                    record_audit_health(&mut health.audit, event);
                }
            }
            Err(RepositoryError::CasMismatch { .. }) => {
                health.last_outcome = Some(RepositoryMutationOutcome::CasMismatch);
            }
            Err(RepositoryError::InvalidRecord { .. }) => {
                health.last_outcome = Some(RepositoryMutationOutcome::InvalidRecord);
            }
            Err(RepositoryError::Transient { .. }) => {
                health.last_outcome = Some(RepositoryMutationOutcome::Transient);
                health.driver.reconnecting = true;
                if trace.audit_failed {
                    health.audit.failed_writes += 1;
                }
            }
            Err(RepositoryError::Closed) => {}
        }
    }

    fn mark_driver_connected(&self) {
        let mut health = self.health_state.lock().expect("health lock");
        health.driver.connected = true;
        health.driver.reconnecting = false;
    }
}

fn record_audit_health(audit: &mut AuditHealth, event: &ActivationAuditEvent) {
    audit.last_event_id = Some(event.event_id.clone());
    audit.last_event_operation = Some(event.operation.as_str().to_string());
    audit.last_event_timestamp = Some(event.timestamp);
}

fn audit_event_for(
    operation: ActivationAuditOperation,
    current: &ProfileActivationState,
    next: &ProfileActivationState,
    timestamp_millis: i64,
) -> ActivationAuditEvent {
    let (activation_id, expected_generation, candidate_generation, participants) = match operation {
        ActivationAuditOperation::Prepare => {
            let pending = next.pending.as_ref().expect("prepare writes pending");
            (
                pending.activation_id.clone(),
                current.committed.generation,
                pending.candidate_generation,
                Some(pending.participant_replica_ids.clone()),
            )
        }
        ActivationAuditOperation::Commit => {
            let pending = current.pending.as_ref().expect("commit reads pending");
            (
                pending.activation_id.clone(),
                current.committed.generation,
                next.committed.generation,
                Some(pending.participant_replica_ids.clone()),
            )
        }
        ActivationAuditOperation::Abort => {
            let pending = current.pending.as_ref().expect("abort reads pending");
            (
                pending.activation_id.clone(),
                current.committed.generation,
                pending.candidate_generation,
                Some(pending.participant_replica_ids.clone()),
            )
        }
    };
    ActivationAuditEvent::new(
        current.profile.clone(),
        activation_id,
        operation,
        expected_generation,
        candidate_generation,
        skiff_deployment::activation_state::ActivationAuditOutcome::Ok,
        participants,
        timestamp_millis,
    )
}

fn cas_filter(
    profile: &str,
    operation: ActivationAuditOperation,
    current: &ProfileActivationState,
) -> Result<Document, RepositoryError> {
    let generation = Bson::Int64(i64::try_from(current.committed.generation).unwrap_or(i64::MAX));
    let base = doc! { "_id": profile, "state.committed.generation": generation };
    match operation {
        ActivationAuditOperation::Prepare => {
            let mut filter = base;
            filter.insert("state.pending", Bson::Null);
            Ok(filter)
        }
        ActivationAuditOperation::Abort => {
            let pending = current.pending.as_ref().ok_or_else(|| {
                invalid_record(profile, "abort CAS filter requires a pending activation")
            })?;
            let mut filter = base;
            filter.insert("state.pending.activationId", pending.activation_id.clone());
            Ok(filter)
        }
        ActivationAuditOperation::Commit => {
            let pending = current.pending.as_ref().ok_or_else(|| {
                invalid_record(profile, "commit CAS filter requires a pending activation")
            })?;
            let mut filter = base;
            filter.insert(
                "state.pending.activationId",
                Bson::String(pending.activation_id.clone()),
            );
            filter.insert(
                "state.pending.expectedGeneration",
                Bson::Int64(i64::try_from(pending.expected_generation).unwrap_or(i64::MAX)),
            );
            filter.insert(
                "state.pending.candidateGeneration",
                Bson::Int64(i64::try_from(pending.candidate_generation).unwrap_or(i64::MAX)),
            );
            filter.insert(
                "state.pending.assembly.assemblyIdentity",
                Bson::String(pending.assembly.assembly_identity.as_str().to_string()),
            );
            filter.insert(
                "state.pending.configSnapshot.snapshotId",
                Bson::String(pending.config_snapshot.snapshot_id.to_string()),
            );
            filter.insert(
                "state.pending.participantReplicaIds",
                Bson::Array(
                    pending
                        .participant_replica_ids
                        .iter()
                        .cloned()
                        .map(Bson::String)
                        .collect(),
                ),
            );
            Ok(filter)
        }
    }
}

fn state_document(state: &ProfileActivationState) -> Result<Document, RepositoryError> {
    let value = serde_json::to_value(state).map_err(|error| {
        invalid_record(
            &state.profile,
            format!("serialize activation state: {error}"),
        )
    })?;
    to_document(&value).map_err(|error| {
        invalid_record(
            &state.profile,
            format!("convert activation state to BSON: {error}"),
        )
    })
}

fn decode_state_document(
    document: &Document,
    profile: &str,
) -> Result<ProfileActivationState, RepositoryError> {
    let state_document = document.get_document("state").map_err(|error| {
        invalid_record(
            profile,
            format!("activation state document has no state member: {error}"),
        )
    })?;
    let value: JsonValue =
        mongodb::bson::from_document(state_document.clone()).map_err(|error| {
            invalid_record(
                profile,
                format!("strict BSON decode of activation state failed: {error}"),
            )
        })?;
    let state: ProfileActivationState =
        serde_json::from_value(normalize_non_negative_integers(value)).map_err(|error| {
            invalid_record(
                profile,
                format!("strict activation state decode failed: {error}"),
            )
        })?;
    state.validate().map_err(|error| {
        invalid_record(
            profile,
            format!("activation state validation failed: {error}"),
        )
    })?;
    if state.profile != profile {
        return Err(invalid_record(
            profile,
            "activation state profile/_id mismatch",
        ));
    }
    Ok(state)
}

fn audit_document(event: &ActivationAuditEvent) -> Result<Document, RepositoryError> {
    let value = serde_json::to_value(event).map_err(|error| {
        invalid_record(&event.profile, format!("serialize audit event: {error}"))
    })?;
    let mut document = to_document(&value).map_err(|error| {
        invalid_record(
            &event.profile,
            format!("convert audit event to BSON: {error}"),
        )
    })?;
    document.insert("_id", Bson::String(event.event_id.clone()));
    Ok(document)
}

fn normalize_non_negative_integers(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Number(number) => match number.as_i64() {
            Some(integer) if integer >= 0 => JsonValue::Number(integer.into()),
            _ => JsonValue::Number(number),
        },
        JsonValue::Array(values) => JsonValue::Array(
            values
                .into_iter()
                .map(normalize_non_negative_integers)
                .collect(),
        ),
        JsonValue::Object(map) => JsonValue::Object(
            map.into_iter()
                .map(|(key, value)| (key, normalize_non_negative_integers(value)))
                .collect(),
        ),
        other => other,
    }
}

fn map_driver_error(error: MongoError) -> RepositoryError {
    let message = error.to_string();
    let transient = error.contains_label(TRANSIENT_TRANSACTION_ERROR)
        || error.contains_label(UNKNOWN_TRANSACTION_COMMIT_RESULT)
        || matches!(
            &*error.kind,
            ErrorKind::ServerSelection { .. }
                | ErrorKind::ConnectionPoolCleared { .. }
                | ErrorKind::Io(_)
                | ErrorKind::DnsResolve { .. }
                | ErrorKind::Transaction { .. }
                | ErrorKind::Command { .. }
                | ErrorKind::Write(_)
                | ErrorKind::Authentication { .. }
                | ErrorKind::Shutdown
        );
    if transient {
        RepositoryError::Transient { message }
    } else {
        RepositoryError::InvalidRecord {
            profile: "<mongo-driver>".to_string(),
            message,
        }
    }
}

fn is_duplicate_key(error: &MongoError) -> bool {
    matches!(
        &*error.kind,
        ErrorKind::Write(WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

#[async_trait]
impl ActivationStateRepository for MongoActivationStateRepository {
    async fn read(&self, profile: &str) -> Result<ProfileActivationState, RepositoryError> {
        self.check_open()?;
        let document = self
            .states
            .find_one(doc! { "_id": profile })
            .await
            .map_err(map_driver_error)?;
        let Some(document) = document else {
            return Err(cas_mismatch(profile, "activation state does not exist"));
        };
        let state = decode_state_document(&document, profile)?;
        let mut health = self.health_state.lock().expect("health lock");
        health.profile = Some(profile.to_string());
        health.committed_generation = Some(state.committed.generation);
        health.pending_activation_id = state
            .pending
            .as_ref()
            .map(|pending| pending.activation_id.clone());
        health.last_outcome = Some(RepositoryMutationOutcome::Ok);
        health.last_outcome_operation = Some("read".to_string());
        Ok(state)
    }

    async fn initialize(
        &self,
        state: &ProfileActivationState,
    ) -> Result<ProfileActivationState, RepositoryError> {
        self.check_open()?;
        if state.pending.is_some() {
            return Err(invalid_record(
                &state.profile,
                "initial activation state cannot contain pending",
            ));
        }
        state.validate().map_err(|error| {
            invalid_record(
                &state.profile,
                format!("initial activation state validation failed: {error}"),
            )
        })?;
        let profile = state.profile.clone();
        let document = state_document(state)?;
        let update = self
            .states
            .update_one(
                doc! { "_id": &profile },
                doc! { "$setOnInsert": { "_id": &profile, "state": &document } },
            )
            .upsert(true)
            .await
            .map_err(map_driver_error)?;
        if update.upserted_id.is_some() {
            let mut health = self.health_state.lock().expect("health lock");
            health.profile = Some(profile);
            health.committed_generation = Some(state.committed.generation);
            health.pending_activation_id = None;
            health.last_outcome = Some(RepositoryMutationOutcome::Ok);
            health.last_outcome_operation = Some("initialize".to_string());
            return Ok(state.clone());
        }
        let existing_document = self
            .states
            .find_one(doc! { "_id": &profile })
            .await
            .map_err(map_driver_error)?
            .ok_or_else(|| cas_mismatch(&profile, "activation state disappeared"))?;
        let existing = decode_state_document(&existing_document, &profile)?;
        if existing == *state {
            Ok(existing)
        } else {
            Err(cas_mismatch(
                &profile,
                "activation state already exists with a different tuple",
            ))
        }
    }

    async fn prepare(
        &self,
        input: PrepareInput,
    ) -> Result<ProfileActivationState, RepositoryError> {
        let profile = input.profile.clone();
        self.mutate(&profile, ActivationAuditOperation::Prepare, |current| {
            prepare(current, &input).map_err(map_reducer_error)
        })
        .await
    }

    async fn commit(&self, input: CommitInput) -> Result<ProfileActivationState, RepositoryError> {
        let profile = input.profile.clone();
        self.mutate(&profile, ActivationAuditOperation::Commit, |current| {
            commit(current, &input).map_err(map_reducer_error)
        })
        .await
    }

    async fn abort(&self, input: AbortInput) -> Result<ProfileActivationState, RepositoryError> {
        let profile = input.profile.clone();
        self.mutate(&profile, ActivationAuditOperation::Abort, |current| {
            abort(current, &input).map_err(map_reducer_error)
        })
        .await
    }

    async fn append_audit(&self, event: &ActivationAuditEvent) -> Result<(), RepositoryError> {
        self.check_open()?;
        let document = audit_document(event)?;
        match self.audit.insert_one(document).await {
            Ok(_) => {
                let mut health = self.health_state.lock().expect("health lock");
                record_audit_health(&mut health.audit, event);
                Ok(())
            }
            Err(error) if is_duplicate_key(&error) => Ok(()),
            Err(error) => Err(map_driver_error(error)),
        }
    }

    async fn ensure_indexes(&self) -> Result<(), RepositoryError> {
        self.check_open()?;
        self.states
            .create_index(activation_state_profile_index())
            .await
            .map_err(map_driver_error)?;
        self.audit
            .create_index(activation_audit_query_index())
            .await
            .map_err(map_driver_error)?;
        self.audit
            .create_index(activation_audit_maintenance_index())
            .await
            .map_err(map_driver_error)?;
        self.mark_driver_connected();
        Ok(())
    }

    fn health(&self) -> ActivationRepositoryHealth {
        self.health_state.lock().expect("health lock").clone()
    }

    async fn close(&self) -> Result<(), RepositoryError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.client.clone().shutdown().await;
        let mut health = self.health_state.lock().expect("health lock");
        health.driver.connected = false;
        health.driver.reconnecting = false;
        health.driver.closed = true;
        health.driver.shutdown_residue = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{AssemblyIdentity, RuntimeConfigSnapshotId};

    use super::*;

    fn test_state() -> ProfileActivationState {
        ProfileActivationState::initial(
            "test",
            7,
            skiff_artifact_model::RuntimeAssemblyRef {
                assembly_identity: AssemblyIdentity::new(format!(
                    "skiff-runtime-assembly-v3:sha256:{}",
                    "a".repeat(64)
                )),
            },
            skiff_artifact_model::RuntimeConfigSnapshotRef {
                snapshot_id: RuntimeConfigSnapshotId::parse(
                    "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                )
                .expect("config id"),
            },
        )
    }

    #[test]
    fn state_document_round_trips_through_strict_decode() {
        let state = test_state();
        let state_doc = state_document(&state).expect("encode state");
        let document = doc! { "_id": "test", "state": state_doc };
        let decoded = decode_state_document(&document, "test").expect("decode state");
        assert_eq!(decoded, state);
    }

    #[test]
    fn decode_rejects_unknown_state_fields() {
        let state_doc = state_document(&test_state()).expect("encode state");
        let mut document = doc! { "_id": "test", "state": state_doc };
        let mut state_member = document
            .get_document_mut("state")
            .expect("state member")
            .clone();
        state_member.insert("rogue", Bson::String("field".to_string()));
        document.insert("state", state_member);
        let error = decode_state_document(&document, "test").expect_err("unknown field");
        assert!(matches!(error, RepositoryError::InvalidRecord { .. }));
    }

    #[test]
    fn cas_filters_follow_frozen_anchors() {
        let state = test_state();
        let prepare_filter =
            cas_filter("test", ActivationAuditOperation::Prepare, &state).expect("prepare filter");
        assert_eq!(prepare_filter.get("state.pending"), Some(&Bson::Null));
        assert_eq!(
            prepare_filter.get("state.committed.generation"),
            Some(&Bson::Int64(7))
        );

        let mut pending_state = state.clone();
        pending_state.pending = Some(skiff_deployment::storage::PendingActivation {
            activation_id: "activation-8".to_string(),
            expected_generation: 7,
            candidate_generation: 8,
            assembly: pending_state.committed.assembly.clone(),
            config_snapshot: pending_state.committed.config_snapshot.clone(),
            participant_replica_ids: vec!["runtime-a".to_string(), "runtime-b".to_string()],
        });
        let abort_filter =
            cas_filter("test", ActivationAuditOperation::Abort, &pending_state).expect("abort");
        assert_eq!(
            abort_filter.get("state.pending.activationId"),
            Some(&Bson::String("activation-8".to_string()))
        );
        let commit_filter =
            cas_filter("test", ActivationAuditOperation::Commit, &pending_state).expect("commit");
        assert_eq!(
            commit_filter.get("state.pending.candidateGeneration"),
            Some(&Bson::Int64(8))
        );
        assert_eq!(
            commit_filter.get("state.pending.participantReplicaIds"),
            Some(&Bson::Array(vec![
                Bson::String("runtime-a".to_string()),
                Bson::String("runtime-b".to_string())
            ]))
        );
    }

    #[test]
    fn integer_normalization_preserves_positive_values() {
        let value = serde_json::json!({
            "generation": 7,
            "negative": -3,
            "nested": [9],
            "text": "7"
        });
        let normalized = normalize_non_negative_integers(value);
        assert_eq!(normalized["generation"].as_u64(), Some(7));
        assert_eq!(normalized["negative"].as_i64(), Some(-3));
        assert_eq!(normalized["nested"][0].as_u64(), Some(9));
        assert_eq!(normalized["text"], "7");
    }

    #[test]
    fn audit_event_fields_use_frozen_tuple() {
        let current = test_state();
        let mut next = current.clone();
        next.pending = Some(skiff_deployment::storage::PendingActivation {
            activation_id: "activation-8".to_string(),
            expected_generation: 7,
            candidate_generation: 8,
            assembly: current.committed.assembly.clone(),
            config_snapshot: current.committed.config_snapshot.clone(),
            participant_replica_ids: vec!["runtime-a".to_string()],
        });
        let event = audit_event_for(
            ActivationAuditOperation::Prepare,
            &current,
            &next,
            1_752_531_600_000,
        );
        assert_eq!(event.activation_id, "activation-8");
        assert_eq!(event.expected_generation, 7);
        assert_eq!(event.candidate_generation, 8);
        assert_eq!(
            event.participant_replica_ids,
            Some(vec!["runtime-a".to_string()])
        );
    }
}
