//! Mongo adapter for [`TaskStore`].
//!
//! Every transition is a conditional write / CAS on the current
//! state / lease, executed with the same semantics as the pure reducer. Due
//! visibility, lease expiry and expiry competition use the Mongo server clock
//! (`$$NOW` inside `$expr`), never the client wall clock: a stale settlement
//! cannot sneak in after expiry, and a future task cannot be revealed early by
//! client clock rollback / skew.
//!
//! Indexes: TaskId uniqueness is the `_id` key; the due scanner uses a
//! non-unique `(state, dueAt)` index (authoritative design option 1, no ready
//! queue). Driver / connection failures map to the transient error class.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::TryStreamExt;
use mongodb::bson::{
    doc, from_document, spec::BinarySubtype, to_document, Binary, Bson, DateTime, Document,
};
use mongodb::error::{Error as MongoError, ErrorKind, WriteFailure};
use mongodb::options::{ClientOptions, IndexOptions, ReturnDocument};
use mongodb::{Client, Collection, IndexModel};
use uuid::Uuid;

use crate::clock::{SystemClock, TaskClock};
use crate::error::{invalid_record, TaskStoreError};
use crate::model::{
    ActorActivationSnapshot, AttemptId, DetachedCallTarget, DurableUtcTimestamp, LeaseId,
    RecoverablePayload, ServiceOwner, TaskCancelResult, TaskCancelResultKind, TaskId, TaskLease,
    TaskOutcome, TaskRecord, TaskState, TaskStatus, TaskTerminal, TaskTraceContext,
};
use crate::retry::TaskRetryPolicy;
use crate::store::{
    CancelInput, ClaimInput, ClaimOutcome, ClaimRejection, DueScanInput, LeaseRecoveryInput,
    LeaseRecoveryOutcome, RenewInput, RenewOutcome, RenewRejection, SettleInput, SettleOutcome,
    StatusInput, TaskStore,
};

pub const DEFAULT_TASK_DATABASE: &str = "skiff-router";
pub const DEFAULT_TASK_COLLECTION: &str = "tasks";
pub const TASK_STATE_DUE_AT_INDEX: &str = "task_state_due_at";
const DUPLICATE_KEY_CODE: i32 = 11_000;

#[derive(Debug, Clone)]
pub struct MongoTaskStoreOptions {
    pub database: String,
    pub collection: String,
    pub connect_timeout_ms: u64,
    pub server_selection_timeout_ms: u64,
    pub retry: TaskRetryPolicy,
}

impl Default for MongoTaskStoreOptions {
    fn default() -> Self {
        Self {
            database: DEFAULT_TASK_DATABASE.to_string(),
            collection: DEFAULT_TASK_COLLECTION.to_string(),
            connect_timeout_ms: 10_000,
            server_selection_timeout_ms: 10_000,
            retry: TaskRetryPolicy::default(),
        }
    }
}

#[derive(Clone)]
pub struct MongoTaskStore {
    // Held for the adapter's lifetime; keeps the connection pool alive even
    // though all operations go through the typed `tasks` collection.
    #[allow(dead_code)]
    client: Client,
    tasks: Collection<Document>,
    options: MongoTaskStoreOptions,
    clock: Arc<dyn TaskClock>,
    closed: Arc<AtomicBool>,
}

pub fn task_state_due_at_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "state": 1, "dueAt": 1 })
        .options(
            IndexOptions::builder()
                .name(TASK_STATE_DUE_AT_INDEX.to_string())
                .build(),
        )
        .build()
}

impl MongoTaskStore {
    pub async fn connect(
        mongo_url: &str,
        options: MongoTaskStoreOptions,
    ) -> Result<Self, TaskStoreError> {
        let mut client_options = ClientOptions::parse(mongo_url)
            .await
            .map_err(map_driver_error)?;
        client_options.connect_timeout = Some(Duration::from_millis(options.connect_timeout_ms));
        client_options.server_selection_timeout =
            Some(Duration::from_millis(options.server_selection_timeout_ms));
        client_options.retry_writes = Some(false);
        client_options.app_name = Some("skiff-task-control".to_string());
        let client = Client::with_options(client_options).map_err(map_driver_error)?;
        let tasks = client
            .database(&options.database)
            .collection::<Document>(&options.collection);
        Ok(Self {
            client,
            tasks,
            options,
            clock: Arc::new(SystemClock),
            closed: Arc::new(AtomicBool::new(false)),
        })
    }

    fn check_open(&self) -> Result<(), TaskStoreError> {
        if self.closed.load(Ordering::SeqCst) {
            Err(TaskStoreError::Closed)
        } else {
            Ok(())
        }
    }

    async fn with_retry<F, Fut, T>(&self, operation: F) -> Result<T, TaskStoreError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, TaskStoreError>>,
    {
        let (result, _outcome) = self.options.retry.run(self.clock.as_ref(), operation).await;
        result
    }

    async fn find_record(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, TaskStoreError> {
        let document = self
            .tasks
            .find_one(doc! { "_id": task_id.as_str() })
            .await
            .map_err(map_driver_error)?;
        document.map(document_to_record).transpose()
    }

    async fn create_once(&self, record: TaskRecord) -> Result<TaskRecord, TaskStoreError> {
        self.check_open()?;
        record
            .validate_create()
            .map_err(|message| invalid_record(&record.task_id, message))?;
        let document = record_document(&record)?;
        match self.tasks.insert_one(document).await {
            Ok(_) => Ok(record),
            Err(error) if is_duplicate_key(&error) => {
                match self.find_record(&record.task_id).await? {
                    Some(existing) if existing == record => Ok(existing),
                    Some(_) => Err(TaskStoreError::DuplicateTaskId {
                        task_id: record.task_id.clone(),
                        message: "same TaskId with a different canonical record".to_string(),
                    }),
                    None => Err(TaskStoreError::Transient {
                        message: "duplicate key observed but the record is not readable"
                            .to_string(),
                    }),
                }
            }
            Err(error) => Err(map_driver_error(error)),
        }
    }

    async fn claim_once(&self, input: ClaimInput) -> Result<ClaimOutcome, TaskStoreError> {
        self.check_open()?;
        if !input.image_activatable {
            return Ok(ClaimOutcome::Rejected(ClaimRejection::NotActivatable));
        }
        let attempt_id = AttemptId::new(Uuid::new_v4().to_string());
        let lease_id = LeaseId::new(Uuid::new_v4().to_string());
        let lease = TaskLease {
            lease_id: lease_id.clone(),
            attempt_id: attempt_id.clone(),
            owner: input.owner.clone(),
            expiry: input.lease_expiry,
        };
        let filter = doc! {
            "_id": input.task_id.as_str(),
            "state": "ready",
            "$and": [
                { "$expr": { "$lte": [ "$dueAt", "$$NOW" ] } },
                { "$expr": { "$gt": [ DateTime::from_millis(input.lease_expiry.millis()), "$$NOW" ] } },
            ],
        };
        let update = doc! {
            "$set": { "state": "leased", "activeLease": lease_document(&lease) },
            "$inc": { "attemptGeneration": 1 },
        };
        let updated = self
            .tasks
            .find_one_and_update(filter, update)
            .return_document(ReturnDocument::After)
            .await
            .map_err(map_driver_error)?;
        match updated {
            Some(document) => Ok(ClaimOutcome::Claimed(document_to_record(document)?)),
            None => self.classify_claim_rejection(&input).await,
        }
    }

    async fn classify_claim_rejection(
        &self,
        input: &ClaimInput,
    ) -> Result<ClaimOutcome, TaskStoreError> {
        let Some(record) = self.find_record(&input.task_id).await? else {
            return Ok(ClaimOutcome::Rejected(ClaimRejection::NotFound));
        };
        let rejection = match record.state {
            TaskState::Scheduled => ClaimRejection::NotReady,
            TaskState::Ready => {
                let due = self
                    .tasks
                    .find_one(doc! {
                        "_id": input.task_id.as_str(),
                        "state": "ready",
                        "$expr": { "$lte": [ "$dueAt", "$$NOW" ] },
                    })
                    .await
                    .map_err(map_driver_error)?;
                if due.is_some() {
                    ClaimRejection::InvalidLeaseExpiry
                } else {
                    ClaimRejection::NotDue
                }
            }
            TaskState::Leased => ClaimRejection::AlreadyLeased,
            _ => ClaimRejection::Terminal,
        };
        Ok(ClaimOutcome::Rejected(rejection))
    }

    async fn renew_once(&self, input: RenewInput) -> Result<RenewOutcome, TaskStoreError> {
        self.check_open()?;
        let new_expiry = DateTime::from_millis(input.new_expiry.millis());
        let filter = doc! {
            "_id": input.task_id.as_str(),
            "state": "leased",
            "activeLease.leaseId": input.lease_id.as_str(),
            "$and": [
                { "$expr": { "$gt": [ "$activeLease.expiry", "$$NOW" ] } },
                { "$expr": { "$gt": [ new_expiry, "$$NOW" ] } },
            ],
        };
        let update = doc! { "$set": { "activeLease.expiry": new_expiry } };
        let updated = self
            .tasks
            .find_one_and_update(filter, update)
            .return_document(ReturnDocument::After)
            .await
            .map_err(map_driver_error)?;
        match updated {
            Some(document) => Ok(RenewOutcome::Renewed(document_to_record(document)?)),
            None => self.classify_renew_rejection(&input).await,
        }
    }

    async fn classify_renew_rejection(
        &self,
        input: &RenewInput,
    ) -> Result<RenewOutcome, TaskStoreError> {
        let Some(record) = self.find_record(&input.task_id).await? else {
            return Ok(RenewOutcome::Rejected(RenewRejection::NotFound));
        };
        let rejection = match &record.state {
            TaskState::Leased => {
                let lease = record.active_lease.as_ref().expect("leased lease");
                if lease.lease_id != input.lease_id {
                    RenewRejection::StaleLease
                } else {
                    let still_valid = self
                        .tasks
                        .find_one(doc! {
                            "_id": input.task_id.as_str(),
                            "state": "leased",
                            "activeLease.leaseId": input.lease_id.as_str(),
                            "$expr": { "$gt": [ "$activeLease.expiry", "$$NOW" ] },
                        })
                        .await
                        .map_err(map_driver_error)?;
                    if still_valid.is_some() {
                        RenewRejection::InvalidExpiry
                    } else {
                        RenewRejection::ExpiredLease
                    }
                }
            }
            TaskState::Scheduled | TaskState::Ready => RenewRejection::NotLeased,
            _ => RenewRejection::Terminal,
        };
        Ok(RenewOutcome::Rejected(rejection))
    }

    async fn settle_once(&self, input: SettleInput) -> Result<SettleOutcome, TaskStoreError> {
        self.check_open()?;
        let terminal_state = input.terminal.state().as_str();
        let outcome = terminal_outcome_document(&input.terminal.outcome);
        let filter = doc! {
            "_id": input.task_id.as_str(),
            "state": "leased",
            "activeLease.leaseId": input.lease_id.as_str(),
            "$expr": { "$gt": [ "$activeLease.expiry", "$$NOW" ] },
        };
        let update = vec![
            doc! {
                "$set": {
                    "state": terminal_state,
                    "terminal": { "outcome": outcome, "settledAt": "$$NOW" },
                }
            },
            doc! { "$unset": "activeLease" },
        ];
        let updated = self
            .tasks
            .find_one_and_update(filter, update)
            .return_document(ReturnDocument::After)
            .await
            .map_err(map_driver_error)?;
        match updated {
            Some(document) => Ok(SettleOutcome::Settled(document_to_record(document)?)),
            None => self.classify_settle_rejection(&input).await,
        }
    }

    async fn classify_settle_rejection(
        &self,
        input: &SettleInput,
    ) -> Result<SettleOutcome, TaskStoreError> {
        let Some(record) = self.find_record(&input.task_id).await? else {
            return Ok(SettleOutcome::NotFound);
        };
        match &record.state {
            TaskState::Leased => {
                let lease = record.active_lease.as_ref().expect("leased lease");
                if lease.lease_id != input.lease_id {
                    Ok(SettleOutcome::StaleLease)
                } else {
                    let still_valid = self
                        .tasks
                        .find_one(doc! {
                            "_id": input.task_id.as_str(),
                            "state": "leased",
                            "activeLease.leaseId": input.lease_id.as_str(),
                            "$expr": { "$gt": [ "$activeLease.expiry", "$$NOW" ] },
                        })
                        .await
                        .map_err(map_driver_error)?;
                    if still_valid.is_some() {
                        Ok(SettleOutcome::NotLeased)
                    } else {
                        Ok(SettleOutcome::ExpiredLease)
                    }
                }
            }
            TaskState::Scheduled | TaskState::Ready => Ok(SettleOutcome::NotLeased),
            _ => {
                let existing = record
                    .terminal
                    .as_ref()
                    .expect("terminal state has terminal");
                if existing.same_outcome(&input.terminal) {
                    Ok(SettleOutcome::AlreadySettled(record))
                } else {
                    Ok(SettleOutcome::Conflict(record))
                }
            }
        }
    }

    async fn cancel_once(&self, input: CancelInput) -> Result<TaskCancelResult, TaskStoreError> {
        self.check_open()?;
        let filter = doc! {
            "_id": input.task_id.as_str(),
            "state": { "$in": ["scheduled", "ready"] },
        };
        let update = vec![doc! {
            "$set": {
                "state": "canceled",
                "terminal": { "outcome": { "kind": "canceled" }, "settledAt": "$$NOW" },
            }
        }];
        let updated = self
            .tasks
            .find_one_and_update(filter, update)
            .return_document(ReturnDocument::After)
            .await
            .map_err(map_driver_error)?;
        match updated {
            Some(_) => Ok(TaskCancelResult {
                kind: TaskCancelResultKind::Canceled,
            }),
            None => {
                let Some(record) = self.find_record(&input.task_id).await? else {
                    return Ok(TaskCancelResult {
                        kind: TaskCancelResultKind::Expired,
                    });
                };
                let kind = match record.state {
                    TaskState::Leased => TaskCancelResultKind::AlreadyStarted,
                    _ => TaskCancelResultKind::AlreadyTerminal,
                };
                Ok(TaskCancelResult { kind })
            }
        }
    }

    async fn recover_once(
        &self,
        input: LeaseRecoveryInput,
    ) -> Result<LeaseRecoveryOutcome, TaskStoreError> {
        self.check_open()?;
        let filter = doc! {
            "_id": input.task_id.as_str(),
            "state": "leased",
            "$expr": { "$lte": [ "$activeLease.expiry", "$$NOW" ] },
        };
        let update = doc! {
            "$set": { "state": "ready" },
            "$unset": { "activeLease": "" },
        };
        let updated = self
            .tasks
            .find_one_and_update(filter, update)
            .return_document(ReturnDocument::After)
            .await
            .map_err(map_driver_error)?;
        match updated {
            Some(document) => Ok(LeaseRecoveryOutcome::Recovered(document_to_record(
                document,
            )?)),
            None => {
                let Some(record) = self.find_record(&input.task_id).await? else {
                    return Ok(LeaseRecoveryOutcome::NotFound);
                };
                match &record.state {
                    TaskState::Leased => {
                        let still_valid = self
                            .tasks
                            .find_one(doc! {
                                "_id": input.task_id.as_str(),
                                "state": "leased",
                                "$expr": { "$gt": [ "$activeLease.expiry", "$$NOW" ] },
                            })
                            .await
                            .map_err(map_driver_error)?;
                        if still_valid.is_some() {
                            Ok(LeaseRecoveryOutcome::NotExpired)
                        } else {
                            Err(TaskStoreError::Transient {
                                message: "expired lease recovery CAS did not match".to_string(),
                            })
                        }
                    }
                    TaskState::Scheduled | TaskState::Ready => Ok(LeaseRecoveryOutcome::NotLeased),
                    _ => Ok(LeaseRecoveryOutcome::Terminal),
                }
            }
        }
    }

    async fn scan_due_once(&self, input: DueScanInput) -> Result<Vec<TaskRecord>, TaskStoreError> {
        self.check_open()?;
        self.tasks
            .update_many(
                doc! {
                    "state": "scheduled",
                    "$expr": { "$lte": [ "$dueAt", "$$NOW" ] },
                },
                doc! { "$set": { "state": "ready" } },
            )
            .await
            .map_err(map_driver_error)?;
        let mut cursor = self
            .tasks
            .find(doc! {
                "state": "ready",
                "$expr": { "$lte": [ "$dueAt", "$$NOW" ] },
            })
            .sort(doc! { "dueAt": 1 })
            .limit(input.limit as i64)
            .await
            .map_err(map_driver_error)?;
        let mut records = Vec::new();
        while let Some(document) = cursor.try_next().await.map_err(map_driver_error)? {
            records.push(document_to_record(document)?);
        }
        Ok(records)
    }

    async fn status_once(&self, input: StatusInput) -> Result<TaskStatus, TaskStoreError> {
        self.check_open()?;
        let expired = self
            .tasks
            .find_one(doc! {
                "_id": input.task_id.as_str(),
                "$expr": {
                    "$lt": [
                        { "$add": [ "$createdAt", input.retention.millis() ] },
                        "$$NOW",
                    ]
                },
            })
            .await
            .map_err(map_driver_error)?;
        if expired.is_some() {
            return Ok(TaskStatus {
                kind: crate::model::TaskStatusKind::Expired,
            });
        }
        let Some(record) = self.find_record(&input.task_id).await? else {
            return Ok(TaskStatus {
                kind: crate::model::TaskStatusKind::Expired,
            });
        };
        Ok(TaskStatus {
            kind: record.status_kind(),
        })
    }
}

#[async_trait]
impl TaskStore for MongoTaskStore {
    async fn create(&self, record: TaskRecord) -> Result<TaskRecord, TaskStoreError> {
        self.with_retry(|| self.create_once(record.clone())).await
    }

    async fn claim(&self, input: ClaimInput) -> Result<ClaimOutcome, TaskStoreError> {
        self.with_retry(|| self.claim_once(input.clone())).await
    }

    async fn renew(&self, input: RenewInput) -> Result<RenewOutcome, TaskStoreError> {
        self.with_retry(|| self.renew_once(input.clone())).await
    }

    async fn settle(&self, input: SettleInput) -> Result<SettleOutcome, TaskStoreError> {
        self.with_retry(|| self.settle_once(input.clone())).await
    }

    async fn cancel(&self, input: CancelInput) -> Result<TaskCancelResult, TaskStoreError> {
        self.with_retry(|| self.cancel_once(input.clone())).await
    }

    async fn recover_expired_lease(
        &self,
        input: LeaseRecoveryInput,
    ) -> Result<LeaseRecoveryOutcome, TaskStoreError> {
        self.with_retry(|| self.recover_once(input.clone())).await
    }

    async fn scan_due(&self, input: DueScanInput) -> Result<Vec<TaskRecord>, TaskStoreError> {
        self.with_retry(|| self.scan_due_once(input.clone())).await
    }

    async fn status(&self, input: StatusInput) -> Result<TaskStatus, TaskStoreError> {
        self.with_retry(|| self.status_once(input.clone())).await
    }

    async fn ensure_indexes(&self) -> Result<(), TaskStoreError> {
        self.with_retry(|| async {
            self.check_open()?;
            self.tasks
                .create_index(task_state_due_at_index())
                .await
                .map_err(map_driver_error)?;
            Ok(())
        })
        .await
    }

    async fn close(&self) -> Result<(), TaskStoreError> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn is_duplicate_key(error: &MongoError) -> bool {
    matches!(
        error.kind.as_ref(),
        ErrorKind::Write(WriteFailure::WriteError(ref write_error))
            if write_error.code == DUPLICATE_KEY_CODE
    )
}

fn map_driver_error(error: MongoError) -> TaskStoreError {
    TaskStoreError::Transient {
        message: format!("mongo task store failure: {error}"),
    }
}

fn record_document(record: &TaskRecord) -> Result<Document, TaskStoreError> {
    Ok(doc! {
        "_id": record.task_id.as_str(),
        "owner": record.owner.as_str(),
        "execution": to_document(&record.execution)
            .map_err(|error| invalid_record(&record.task_id, format!("execution encode: {error}")))?,
        "target": target_document(&record.task_id, &record.target)?,
        "payload": Binary {
            subtype: BinarySubtype::Generic,
            bytes: record.payload.as_bytes().to_vec(),
        },
        "dueAt": DateTime::from_millis(record.due_at.millis()),
        "state": record.state.as_str(),
        "attemptGeneration": record.attempt_generation as i64,
        "activeLease": record.active_lease.as_ref().map(lease_document),
        "terminal": record.terminal.as_ref().map(terminal_document),
        "trace": to_document(&record.trace)
            .map_err(|error| invalid_record(&record.task_id, format!("trace encode: {error}")))?,
        "createdAt": DateTime::from_millis(record.created_at.millis()),
    })
}

fn document_to_record(document: Document) -> Result<TaskRecord, TaskStoreError> {
    let task_id = TaskId::new(
        document
            .get("_id")
            .and_then(Bson::as_str)
            .map(str::to_string)
            .ok_or_else(|| TaskStoreError::InvalidRecord {
                task_id: TaskId::new("?"),
                message: "missing _id".to_string(),
            })?,
    );
    let owner = ServiceOwner::new(get_string(&task_id, &document, "owner")?);
    let execution = from_document::<crate::model::TaskExecutionImageRef>(get_document(
        &task_id,
        &document,
        "execution",
    )?)
    .map_err(|error| invalid_record(&task_id, format!("execution decode: {error}")))?;
    let target = decode_target(&task_id, get_document(&task_id, &document, "target")?)?;
    let payload = RecoverablePayload::new(get_binary(&task_id, &document, "payload")?);
    let due_at = DurableUtcTimestamp::from_millis(
        get_datetime(&task_id, &document, "dueAt")?.timestamp_millis(),
    );
    let state = decode_state(&task_id, &document)?;
    let attempt_generation = get_i64(&task_id, &document, "attemptGeneration")? as u64;
    let active_lease = document
        .get("activeLease")
        .and_then(Bson::as_document)
        .map(|lease| decode_lease(&task_id, lease))
        .transpose()?;
    let terminal = document
        .get("terminal")
        .and_then(Bson::as_document)
        .map(|terminal| decode_terminal(&task_id, terminal))
        .transpose()?;
    let trace = from_document::<TaskTraceContext>(get_document(&task_id, &document, "trace")?)
        .map_err(|error| invalid_record(&task_id, format!("trace decode: {error}")))?;
    let created_at = DurableUtcTimestamp::from_millis(
        get_datetime(&task_id, &document, "createdAt")?.timestamp_millis(),
    );
    Ok(TaskRecord {
        task_id,
        owner,
        execution,
        target,
        payload,
        due_at,
        state,
        attempt_generation,
        active_lease,
        terminal,
        trace,
        created_at,
    })
}

fn target_document(
    task_id: &TaskId,
    target: &DetachedCallTarget,
) -> Result<Document, TaskStoreError> {
    match target {
        DetachedCallTarget::Function { callable } => Ok(doc! {
            "kind": "function",
            "callable": callable.as_str(),
        }),
        DetachedCallTarget::ActorMethod {
            actor,
            activation,
            implementation,
            method,
        } => Ok(doc! {
            "kind": "actorMethod",
            "actor": to_document(actor).map_err(|error| {
                invalid_record(task_id, format!("actor encode: {error}"))
            })?,
            "activation": {
                "key": payload_binary(&activation.key),
                "createInput": payload_binary(&activation.create_input),
                "expectedTypePlan": to_document(&activation.expected_type_plan).map_err(
                    |error| invalid_record(task_id, format!("expected type plan encode: {error}")),
                )?,
            },
            "implementation": implementation.as_str(),
            "method": method.as_str(),
        }),
    }
}

fn decode_target(
    task_id: &TaskId,
    document: Document,
) -> Result<DetachedCallTarget, TaskStoreError> {
    match get_string(task_id, &document, "kind")?.as_str() {
        "function" => Ok(DetachedCallTarget::Function {
            callable: skiff_artifact_model::PackageCallableId::new(get_string(
                task_id, &document, "callable",
            )?),
        }),
        "actorMethod" => {
            let actor =
                from_document::<skiff_deployment::projection::actor_routing::ActorRoutingRef>(
                    get_document(task_id, &document, "actor")?,
                )
                .map_err(|error| invalid_record(task_id, format!("actor decode: {error}")))?;
            let activation_document = get_document(task_id, &document, "activation")?;
            let activation = ActorActivationSnapshot {
                key: RecoverablePayload::new(get_binary(task_id, &activation_document, "key")?),
                create_input: RecoverablePayload::new(get_binary(
                    task_id,
                    &activation_document,
                    "createInput",
                )?),
                expected_type_plan: from_document(get_document(
                    task_id,
                    &activation_document,
                    "expectedTypePlan",
                )?)
                .map_err(|error| {
                    invalid_record(task_id, format!("expected type plan decode: {error}"))
                })?,
            };
            Ok(DetachedCallTarget::ActorMethod {
                actor,
                activation,
                implementation: skiff_artifact_model::ActorImplementationIdentity::new(get_string(
                    task_id,
                    &document,
                    "implementation",
                )?),
                method: skiff_artifact_model::ActorMethodIdentity::new(get_string(
                    task_id, &document, "method",
                )?),
            })
        }
        other => Err(invalid_record(
            task_id,
            format!("unknown target kind {other:?}"),
        )),
    }
}

fn lease_document(lease: &TaskLease) -> Document {
    doc! {
        "leaseId": lease.lease_id.as_str(),
        "attemptId": lease.attempt_id.as_str(),
        "owner": lease.owner.clone(),
        "expiry": DateTime::from_millis(lease.expiry.millis()),
    }
}

fn decode_lease(task_id: &TaskId, document: &Document) -> Result<TaskLease, TaskStoreError> {
    Ok(TaskLease {
        lease_id: LeaseId::new(get_string(task_id, document, "leaseId")?),
        attempt_id: AttemptId::new(get_string(task_id, document, "attemptId")?),
        owner: get_string(task_id, document, "owner")?,
        expiry: DurableUtcTimestamp::from_millis(
            get_datetime(task_id, document, "expiry")?.timestamp_millis(),
        ),
    })
}

fn terminal_document(terminal: &TaskTerminal) -> Document {
    doc! {
        "settledAt": DateTime::from_millis(terminal.settled_at.millis()),
        "outcome": terminal_outcome_document(&terminal.outcome),
    }
}

fn terminal_outcome_document(outcome: &TaskOutcome) -> Document {
    match outcome {
        TaskOutcome::Succeeded => doc! { "kind": "succeeded" },
        TaskOutcome::TargetFailed { error } => doc! { "kind": "targetFailed", "error": error },
        TaskOutcome::PlatformFailed { reason } => {
            doc! { "kind": "platformFailed", "reason": reason }
        }
        TaskOutcome::Canceled => doc! { "kind": "canceled" },
    }
}

fn decode_terminal(task_id: &TaskId, document: &Document) -> Result<TaskTerminal, TaskStoreError> {
    let outcome_document = get_document(task_id, document, "outcome")?;
    let outcome = match get_string(task_id, &outcome_document, "kind")?.as_str() {
        "succeeded" => TaskOutcome::Succeeded,
        "targetFailed" => TaskOutcome::TargetFailed {
            error: get_string(task_id, &outcome_document, "error")?,
        },
        "platformFailed" => TaskOutcome::PlatformFailed {
            reason: get_string(task_id, &outcome_document, "reason")?,
        },
        "canceled" => TaskOutcome::Canceled,
        other => {
            return Err(invalid_record(
                task_id,
                format!("unknown terminal outcome kind {other:?}"),
            ));
        }
    };
    Ok(TaskTerminal {
        settled_at: DurableUtcTimestamp::from_millis(
            get_datetime(task_id, document, "settledAt")?.timestamp_millis(),
        ),
        outcome,
    })
}

fn decode_state(task_id: &TaskId, document: &Document) -> Result<TaskState, TaskStoreError> {
    match get_string(task_id, document, "state")?.as_str() {
        "scheduled" => Ok(TaskState::Scheduled),
        "ready" => Ok(TaskState::Ready),
        "leased" => Ok(TaskState::Leased),
        "succeeded" => Ok(TaskState::Succeeded),
        "failed" => Ok(TaskState::Failed),
        "platformFailed" => Ok(TaskState::PlatformFailed),
        "canceled" => Ok(TaskState::Canceled),
        other => Err(invalid_record(
            task_id,
            format!("unknown task state {other:?}"),
        )),
    }
}

fn payload_binary(payload: &RecoverablePayload) -> Binary {
    Binary {
        subtype: BinarySubtype::Generic,
        bytes: payload.as_bytes().to_vec(),
    }
}

fn get_string(task_id: &TaskId, document: &Document, key: &str) -> Result<String, TaskStoreError> {
    document
        .get(key)
        .and_then(Bson::as_str)
        .map(str::to_string)
        .ok_or_else(|| invalid_record(task_id, format!("missing string {key}")))
}

fn get_i64(task_id: &TaskId, document: &Document, key: &str) -> Result<i64, TaskStoreError> {
    document
        .get(key)
        .and_then(Bson::as_i64)
        .ok_or_else(|| invalid_record(task_id, format!("missing i64 {key}")))
}

fn get_datetime(
    task_id: &TaskId,
    document: &Document,
    key: &str,
) -> Result<DateTime, TaskStoreError> {
    document
        .get(key)
        .and_then(Bson::as_datetime)
        .cloned()
        .ok_or_else(|| invalid_record(task_id, format!("missing datetime {key}")))
}

fn get_binary(task_id: &TaskId, document: &Document, key: &str) -> Result<Vec<u8>, TaskStoreError> {
    document
        .get(key)
        .and_then(|value| match value {
            Bson::Binary(binary) => Some(binary.bytes.clone()),
            _ => None,
        })
        .ok_or_else(|| invalid_record(task_id, format!("missing binary {key}")))
}

fn get_document(
    task_id: &TaskId,
    document: &Document,
    key: &str,
) -> Result<Document, TaskStoreError> {
    document
        .get(key)
        .and_then(Bson::as_document)
        .cloned()
        .ok_or_else(|| invalid_record(task_id, format!("missing document {key}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reducer::tests::fixtures;

    #[test]
    fn state_due_index_is_non_unique() {
        let index = task_state_due_at_index();
        assert_eq!(
            index
                .options
                .as_ref()
                .and_then(|options| options.name.as_deref()),
            Some(TASK_STATE_DUE_AT_INDEX)
        );
        assert_ne!(
            index.options.as_ref().and_then(|options| options.unique),
            Some(true)
        );
        assert_eq!(index.keys.get("state"), Some(&Bson::Int32(1)));
        assert_eq!(index.keys.get("dueAt"), Some(&Bson::Int32(1)));
    }

    #[test]
    fn record_document_round_trips_all_authority_fields() {
        let record = fixtures::actor_record(7, 123_456);
        let document = record_document(&record).expect("encode");
        let decoded = document_to_record(document).expect("decode");
        assert_eq!(decoded, record);
    }

    #[test]
    fn leased_record_round_trips_lease_and_terminal() {
        let mut record = fixtures::record(8, 123_456);
        record.state = TaskState::Leased;
        record.attempt_generation = 3;
        record.active_lease = Some(TaskLease {
            lease_id: LeaseId::new("lease-9"),
            attempt_id: AttemptId::new("attempt-9"),
            owner: "scheduler-a".to_string(),
            expiry: DurableUtcTimestamp::from_millis(999_999),
        });
        record.terminal = Some(TaskTerminal {
            settled_at: DurableUtcTimestamp::from_millis(500_000),
            outcome: TaskOutcome::TargetFailed {
                error: "boom".to_string(),
            },
        });
        let decoded =
            document_to_record(record_document(&record).expect("encode")).expect("decode");
        assert_eq!(decoded, record);
    }
}
