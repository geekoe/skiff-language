//! MongoDB-backed production implementation of the trusted registry boundary.
//!
//! This adapter deliberately does not depend on `CanonicalArtifactStore`: the
//! platform database is the sole durable source of truth in production.

use mongodb::{
    bson::{self, doc, Bson, Document},
    error::{ErrorKind, WriteFailure},
    options::IndexOptions,
    Client, Collection, Database, IndexModel,
};
use serde::{de::DeserializeOwned, Serialize};
use skiff_artifact_identity::{
    package_artifact_ref, runtime_assembly_ref, service_contract_ref, service_deployment_ref,
    validate_package_artifact_identities, validate_runtime_assembly_identity,
    validate_service_contract_identities, validate_service_deployment_ref,
};
use skiff_artifact_model::{
    validate_activation_environment, validate_activation_token, validate_runtime_assembly_ref,
    validate_transition_generations, PackageArtifact, PackageArtifactRef, RuntimeAssembly,
    RuntimeAssemblyRef, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentRef,
};

use crate::router_activation_backend::{
    ActivationBackendCommit, ActivationBackendCommitted, ActivationBackendError,
    ActivationBackendFuture, ActivationBackendPending, ActivationBackendPrepare,
    ActivationBackendRef, ActivationBackendSnapshot, ReadActivation, ReadAssembly,
    RouterActivationBackend, RouterBackendSnapshot,
};
use skiff_trusted_registry_contract::{
    ActivationReceipt, ActivationRequest, PackageArtifactPointer, PackageArtifactPointerCas,
    PackageArtifactPointerHistoryQuery, PackageArtifactPointerKey, PackageArtifactPointerReceipt,
    PointerHistorySelector, RuntimeAssemblyPointer, RuntimeAssemblyPointerCas,
    RuntimeAssemblyPointerHistoryQuery, RuntimeAssemblyPointerKey, RuntimeAssemblyPointerReceipt,
    ServiceContractPointer, ServiceContractPointerCas, ServiceContractPointerHistoryQuery,
    ServiceContractPointerKey, ServiceContractPointerReceipt, ServiceDeploymentPointer,
    ServiceDeploymentPointerCas, ServiceDeploymentPointerHistoryQuery, ServiceDeploymentPointerKey,
    ServiceDeploymentPointerReceipt, TrustedRegistryError, TrustedRegistryFuture,
    TrustedRegistryResult, TrustedRegistryStoreApi,
};

const RECORDS: &str = "trusted_registry_records";
const POINTERS: &str = "trusted_registry_pointers";
const POINTER_HISTORY: &str = "trusted_registry_pointer_history";
const ACTIVATIONS: &str = "trusted_registry_activations";
const ACTIVATION_AUDIT: &str = "trusted_registry_activation_audit";

#[derive(Clone)]
pub struct PlatformDbTrustedRegistry {
    client: Client,
    database: Database,
}

impl PlatformDbTrustedRegistry {
    pub fn new(client: Client, database_name: &str) -> TrustedRegistryResult<Self> {
        if database_name.trim().is_empty() {
            return Err(TrustedRegistryError::InvalidRequest(
                "platform registry database name must be non-empty".into(),
            ));
        }
        Ok(Self {
            database: client.database(database_name),
            client,
        })
    }

    /// Idempotently installs the uniqueness constraints required by immutable
    /// records, pointer sequences, and activation audit ordering.
    pub async fn ensure_schema(&self) -> TrustedRegistryResult<()> {
        let unique = IndexOptions::builder().unique(true).build();
        self.database
            .collection::<Document>(POINTER_HISTORY)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"pointerKey": 1, "sequence": 1})
                    .options(unique.clone())
                    .build(),
            )
            .await
            .map_err(map_mongo)?;
        self.database
            .collection::<Document>(ACTIVATION_AUDIT)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"environment": 1, "sequence": 1})
                    .options(unique)
                    .build(),
            )
            .await
            .map_err(map_mongo)?;
        Ok(())
    }

    fn records(&self) -> Collection<Document> {
        self.database.collection(RECORDS)
    }

    fn pointers(&self) -> Collection<Document> {
        self.database.collection(POINTERS)
    }

    fn history(&self) -> Collection<Document> {
        self.database.collection(POINTER_HISTORY)
    }

    fn activations(&self) -> Collection<Document> {
        self.database.collection(ACTIVATIONS)
    }

    fn audit(&self) -> Collection<Document> {
        self.database.collection(ACTIVATION_AUDIT)
    }

    async fn put_record<T: Serialize>(
        &self,
        id: String,
        kind: &'static str,
        value: &T,
    ) -> TrustedRegistryResult<()> {
        let value = bson::to_bson(value).map_err(invalid_bson)?;
        let candidate = doc! {"_id": id.clone(), "kind": kind, "value": value};
        match self.records().insert_one(candidate.clone()).await {
            Ok(_) => Ok(()),
            Err(error) if duplicate_key(&error) => {
                let existing = self
                    .records()
                    .find_one(doc! {"_id": id})
                    .await
                    .map_err(map_mongo)?
                    .ok_or(TrustedRegistryError::BackendUnavailable)?;
                if existing == candidate {
                    Ok(())
                } else {
                    Err(TrustedRegistryError::ImmutableConflict)
                }
            }
            Err(error) => Err(map_mongo(error)),
        }
    }

    async fn read_record<T: DeserializeOwned>(
        &self,
        id: String,
        kind: &'static str,
    ) -> TrustedRegistryResult<T> {
        let record = self
            .records()
            .find_one(doc! {"_id": id, "kind": kind})
            .await
            .map_err(map_mongo)?
            .ok_or(TrustedRegistryError::NotFound)?;
        bson::from_bson(
            record
                .get("value")
                .cloned()
                .ok_or(TrustedRegistryError::BackendUnavailable)?,
        )
        .map_err(invalid_bson)
    }

    async fn read_pointer<P: DeserializeOwned + PointerCoordinates>(
        &self,
        key: String,
    ) -> TrustedRegistryResult<Option<(u64, P)>> {
        let value = self
            .pointers()
            .find_one(doc! {"_id": key})
            .await
            .map_err(map_mongo)?
            .map(pointer_from_document::<P>)
            .transpose()?;
        if let Some((_, pointer)) = &value {
            pointer.validate_typed()?;
        }
        Ok(value)
    }

    async fn cas_pointer<P>(
        &self,
        key: String,
        expected: Option<P>,
        candidate: P,
    ) -> TrustedRegistryResult<(u64, P)>
    where
        P: Clone + Eq + Serialize + DeserializeOwned + PointerCoordinates,
    {
        candidate.validate_typed()?;
        self.read_record::<Bson>(candidate.record_id(), candidate.record_kind())
            .await?;
        let expected_bson = expected
            .as_ref()
            .map(bson::to_bson)
            .transpose()
            .map_err(invalid_bson)?;
        let candidate_bson = bson::to_bson(&candidate).map_err(invalid_bson)?;
        let mut session = self.client.start_session().await.map_err(map_mongo)?;
        session.start_transaction().await.map_err(map_mongo)?;
        let current = self
            .pointers()
            .find_one(doc! {"_id": &key})
            .session(&mut session)
            .await
            .map_err(map_mongo)?;
        let current_pointer = current
            .as_ref()
            .and_then(|document| document.get("pointer"))
            .cloned();
        if current_pointer != expected_bson {
            session.abort_transaction().await.map_err(map_mongo)?;
            return Err(TrustedRegistryError::CasMismatch);
        }
        let sequence = current
            .as_ref()
            .map(|document| read_sequence(document).and_then(next_sequence))
            .transpose()?
            .unwrap_or(1);
        self.pointers()
            .replace_one(
                doc! {"_id": &key},
                doc! {"_id": &key, "sequence": sequence as i64, "pointer": candidate_bson.clone()},
            )
            .upsert(true)
            .session(&mut session)
            .await
            .map_err(map_mongo)?;
        self.history()
            .insert_one(doc! {
                "_id": format!("{key}:{sequence}"),
                "pointerKey": &key,
                "sequence": sequence as i64,
                "pointer": candidate_bson,
            })
            .session(&mut session)
            .await
            .map_err(map_mongo)?;
        commit_recoverably(&mut session).await?;
        Ok((sequence, candidate))
    }

    async fn pointer_history<P: DeserializeOwned + PointerCoordinates>(
        &self,
        key: String,
        selector: PointerHistorySelector,
    ) -> TrustedRegistryResult<Vec<(u64, P)>> {
        validate_selector(&selector)?;
        let after = selector.after_sequence.unwrap_or(0);
        let mut cursor = self
            .history()
            .find(doc! {"pointerKey": key, "sequence": {"$gt": after as i64}})
            .sort(doc! {"sequence": 1})
            .limit(selector.limit as i64)
            .await
            .map_err(map_mongo)?;
        let mut values = Vec::new();
        while cursor.advance().await.map_err(map_mongo)? {
            let value =
                pointer_from_document::<P>(cursor.deserialize_current().map_err(map_mongo)?)?;
            value.1.validate_typed()?;
            values.push(value);
        }
        Ok(values)
    }

    async fn read_backend_state(
        &self,
        environment: &str,
    ) -> Result<ActivationBackendSnapshot, ActivationBackendError> {
        validate_activation_environment(environment).map_err(backend_invalid)?;
        self.activations()
            .find_one(doc! {"_id": environment})
            .await
            .map_err(backend_mongo)?
            .map(|document| backend_snapshot(environment, &document))
            .transpose()
            .map(|snapshot| {
                snapshot.unwrap_or(ActivationBackendSnapshot {
                    environment: environment.to_owned(),
                    committed: None,
                    pending: None,
                })
            })
    }

    async fn mutate_backend_state(
        &self,
        operation: BackendMutation,
    ) -> Result<ActivationBackendSnapshot, ActivationBackendError> {
        operation.validate()?;
        let environment = operation.environment();
        let mut session = self.client.start_session().await.map_err(backend_mongo)?;
        session.start_transaction().await.map_err(backend_mongo)?;
        let current = self
            .activations()
            .find_one(doc! {"_id": environment})
            .session(&mut session)
            .await
            .map_err(backend_mongo)?;
        let current_snapshot = current
            .as_ref()
            .map(|document| backend_snapshot(environment, document))
            .transpose()?
            .unwrap_or(ActivationBackendSnapshot {
                environment: environment.to_owned(),
                committed: None,
                pending: None,
            });
        let Some((next, event, activation_id)) = operation.apply(current_snapshot.clone())? else {
            session.abort_transaction().await.map_err(backend_mongo)?;
            return Ok(current_snapshot);
        };
        let sequence = current
            .as_ref()
            .map(|value| backend_audit_sequence(value))
            .transpose()?
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| backend_failure("activation audit sequence overflow"))?;
        self.activations()
            .replace_one(
                doc! {"_id": environment},
                backend_document(&next, sequence)?,
            )
            .upsert(true)
            .session(&mut session)
            .await
            .map_err(backend_mongo)?;
        insert_audit(
            &self.audit(),
            &mut session,
            environment,
            sequence,
            event,
            &activation_id,
        )
        .await
        .map_err(backend_store)?;
        commit_recoverably(&mut session)
            .await
            .map_err(backend_store)?;
        Ok(next)
    }
}

enum BackendMutation {
    Prepare(ActivationBackendPrepare),
    Commit(ActivationBackendCommit),
    Abort(ActivationBackendRef),
}

impl BackendMutation {
    fn environment(&self) -> &str {
        match self {
            Self::Prepare(value) => &value.environment,
            Self::Commit(value) => &value.environment,
            Self::Abort(value) => &value.environment,
        }
    }
    fn validate(&self) -> Result<(), ActivationBackendError> {
        validate_activation_environment(self.environment()).map_err(backend_invalid)?;
        let (activation_id, assembly) = match self {
            Self::Prepare(value) => {
                validate_transition_generations(
                    value.expected_generation,
                    value.candidate_generation,
                )
                .map_err(backend_invalid)?;
                if value.candidate_generation
                    != value
                        .expected_generation
                        .checked_add(1)
                        .ok_or_else(|| backend_failure("activation generation overflow"))?
                {
                    return Err(backend_invalid(
                        "candidate generation must equal expected generation + 1",
                    ));
                }
                validate_runtime_assembly_ref(&value.assembly).map_err(backend_invalid)?;
                canonical_replica_ids(&value.participant_replica_ids)?;
                (&value.activation_id, Some(&value.assembly))
            }
            Self::Commit(value) => {
                canonical_replica_ids(&value.connected_replica_ids)?;
                canonical_replica_ids(&value.prepared_replica_ids)?;
                (&value.activation_id, None)
            }
            Self::Abort(value) => (&value.activation_id, None),
        };
        validate_activation_token(activation_id, "activationId").map_err(backend_invalid)?;
        let _ = assembly;
        Ok(())
    }
    fn apply(
        &self,
        mut state: ActivationBackendSnapshot,
    ) -> Result<Option<(ActivationBackendSnapshot, &'static str, String)>, ActivationBackendError>
    {
        match self {
            Self::Prepare(request) => {
                let generation = state.committed.as_ref().map_or(0, |value| value.generation);
                if generation != request.expected_generation {
                    return Err(backend_conflict(
                        "activation prepare expected generation is stale",
                    ));
                }
                let pending = ActivationBackendPending {
                    activation_id: request.activation_id.clone(),
                    expected_generation: request.expected_generation,
                    candidate_generation: request.candidate_generation,
                    assembly: request.assembly.clone(),
                    participant_replica_ids: canonical_replica_ids(
                        &request.participant_replica_ids,
                    )?,
                };
                if let Some(current) = &state.pending {
                    if current == &pending {
                        return Ok(None);
                    }
                    return Err(backend_conflict(
                        "a different activation is already pending",
                    ));
                }
                state.pending = Some(pending);
                Ok(Some((state, "prepared", request.activation_id.clone())))
            }
            Self::Commit(request) => {
                let Some(pending) = state.pending.clone() else {
                    return Err(backend_conflict(
                        "activation commit has no durable pending tuple",
                    ));
                };
                if pending.activation_id != request.activation_id {
                    return Err(backend_conflict(
                        "activation commit does not match durable pending tuple",
                    ));
                }
                let participants = canonical_replica_ids(&pending.participant_replica_ids)?;
                let connected = canonical_replica_ids(&request.connected_replica_ids)?;
                let prepared = canonical_replica_ids(&request.prepared_replica_ids)?;
                if prepared != participants || connected != participants {
                    return Err(backend_conflict("activation commit requires every frozen participant connected and prepared"));
                }
                state.committed = Some(ActivationBackendCommitted {
                    generation: pending.candidate_generation,
                    assembly: pending.assembly,
                });
                state.pending = None;
                Ok(Some((state, "committed", request.activation_id.clone())))
            }
            Self::Abort(request) => {
                let Some(pending) = state.pending.as_ref() else {
                    return Ok(None);
                };
                if pending.activation_id != request.activation_id {
                    return Err(backend_conflict(
                        "activation abort does not match durable pending tuple",
                    ));
                }
                state.pending = None;
                Ok(Some((state, "aborted", request.activation_id.clone())))
            }
        }
    }
}

impl RouterActivationBackend for PlatformDbTrustedRegistry {
    fn read(
        &self,
        request: ReadActivation,
    ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot> {
        Box::pin(async move { self.read_backend_state(&request.environment).await })
    }
    fn read_snapshot(
        &self,
        request: ReadAssembly,
    ) -> ActivationBackendFuture<'_, RouterBackendSnapshot> {
        Box::pin(async move {
            let assembly = self
                .read_record::<RuntimeAssembly>(
                    runtime_record_id(&request.assembly),
                    "runtimeAssembly",
                )
                .await
                .map_err(backend_store)?;
            let mut contracts = Vec::with_capacity(assembly.resolved_contracts.len());
            for reference in &assembly.resolved_contracts {
                contracts.push(
                    self.read_record(contract_record_id(reference), "serviceContract")
                        .await
                        .map_err(backend_store)?,
                );
            }
            RouterBackendSnapshot::from_canonical(assembly, contracts)
        })
    }
    fn prepare(
        &self,
        request: ActivationBackendPrepare,
    ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot> {
        Box::pin(async move {
            self.read_record::<RuntimeAssembly>(
                runtime_record_id(&request.assembly),
                "runtimeAssembly",
            )
            .await
            .map_err(backend_store)?;
            self.mutate_backend_state(BackendMutation::Prepare(request))
                .await
        })
    }
    fn commit(
        &self,
        request: ActivationBackendCommit,
    ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot> {
        Box::pin(async move {
            self.mutate_backend_state(BackendMutation::Commit(request))
                .await
        })
    }
    fn abort(
        &self,
        request: ActivationBackendRef,
    ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot> {
        Box::pin(async move {
            self.mutate_backend_state(BackendMutation::Abort(request))
                .await
        })
    }
}

macro_rules! record_methods {
    ($put:ident, $read:ident, $value:ty, $reference:ty, $validate:expr, $make_ref:expr, $id:expr, $kind:literal) => {
        fn $put(&self, value: $value) -> TrustedRegistryFuture<'_, $reference> {
            Box::pin(async move {
                ($validate)(&value).map_err(invalid_validation)?;
                let reference = ($make_ref)(&value).map_err(invalid_validation)?;
                self.put_record(($id)(&reference), $kind, &value).await?;
                Ok(reference)
            })
        }
        fn $read(&self, reference: $reference) -> TrustedRegistryFuture<'_, $value> {
            Box::pin(async move {
                let value = self.read_record(($id)(&reference), $kind).await?;
                ($validate)(&value).map_err(invalid_validation)?;
                let actual = ($make_ref)(&value).map_err(invalid_validation)?;
                if actual != reference {
                    return Err(TrustedRegistryError::ImmutableConflict);
                }
                Ok(value)
            })
        }
    };
}

macro_rules! pointer_methods {
    (
        $read:ident($key_ty:ty),
        $cas:ident($cas_ty:ty),
        $history:ident($query_ty:ty),
        $pointer:ty,
        $receipt:ident,
        $key:expr,
        $key_fields:expr
    ) => {
        fn $read(&self, request: $key_ty) -> TrustedRegistryFuture<'_, Option<$receipt>> {
            Box::pin(async move {
                let (first, second) = ($key_fields)(&request);
                Ok(self
                    .read_pointer(($key)(&first, &second))
                    .await?
                    .map(|(sequence, pointer)| $receipt { sequence, pointer }))
            })
        }
        fn $cas(&self, request: $cas_ty) -> TrustedRegistryFuture<'_, $receipt> {
            Box::pin(async move {
                let expected = request.expected;
                let candidate = request.candidate;
                let key = ($key)(
                    &candidate_key_first(&candidate)?,
                    &candidate_key_second(&candidate)?,
                );
                let (sequence, pointer) = self.cas_pointer(key, expected, candidate).await?;
                Ok($receipt { sequence, pointer })
            })
        }
        fn $history(&self, query: $query_ty) -> TrustedRegistryFuture<'_, Vec<$receipt>> {
            Box::pin(async move {
                let (first, second) = ($key_fields)(&query.key);
                Ok(self
                    .pointer_history(($key)(&first, &second), query.selector)
                    .await?
                    .into_iter()
                    .map(|(sequence, pointer)| $receipt { sequence, pointer })
                    .collect())
            })
        }
    };
}

impl TrustedRegistryStoreApi for PlatformDbTrustedRegistry {
    record_methods!(
        put_package_artifact,
        read_package_artifact,
        PackageArtifact,
        PackageArtifactRef,
        validate_package_artifact_identities,
        package_ref_result,
        package_record_id,
        "packageArtifact"
    );
    record_methods!(
        put_service_contract,
        read_service_contract,
        ServiceContract,
        ServiceContractRef,
        validate_service_contract_identities,
        contract_ref_result,
        contract_record_id,
        "serviceContract"
    );
    record_methods!(
        put_service_deployment,
        read_service_deployment,
        ServiceDeployment,
        ServiceDeploymentRef,
        validate_deployment,
        deployment_ref_result,
        deployment_record_id,
        "serviceDeployment"
    );
    record_methods!(
        put_runtime_assembly,
        read_runtime_assembly,
        RuntimeAssembly,
        RuntimeAssemblyRef,
        validate_runtime_assembly_identity,
        assembly_ref_result,
        runtime_record_id,
        "runtimeAssembly"
    );

    pointer_methods!(
        read_package_artifact_pointer(PackageArtifactPointerKey),
        cas_package_artifact_pointer(PackageArtifactPointerCas),
        package_artifact_pointer_history(PackageArtifactPointerHistoryQuery),
        PackageArtifactPointer,
        PackageArtifactPointerReceipt,
        |a: &str, b: &str| format!("package:{a}:{b}"),
        |key: &PackageArtifactPointerKey| (key.package_id.clone(), key.package_version.clone())
    );
    pointer_methods!(
        read_service_contract_pointer(ServiceContractPointerKey),
        cas_service_contract_pointer(ServiceContractPointerCas),
        service_contract_pointer_history(ServiceContractPointerHistoryQuery),
        ServiceContractPointer,
        ServiceContractPointerReceipt,
        |a: &str, b: &str| format!("contract:{a}:{b}"),
        |key: &ServiceContractPointerKey| (key.service_id.clone(), key.contract_version.clone())
    );
    pointer_methods!(
        read_service_deployment_pointer(ServiceDeploymentPointerKey),
        cas_service_deployment_pointer(ServiceDeploymentPointerCas),
        service_deployment_pointer_history(ServiceDeploymentPointerHistoryQuery),
        ServiceDeploymentPointer,
        ServiceDeploymentPointerReceipt,
        |a: &str, b: &str| format!("deployment:{a}:{b}"),
        |key: &ServiceDeploymentPointerKey| (key.service_id.clone(), key.contract_version.clone())
    );

    fn read_runtime_assembly_pointer(
        &self,
        key: RuntimeAssemblyPointerKey,
    ) -> TrustedRegistryFuture<'_, Option<RuntimeAssemblyPointerReceipt>> {
        Box::pin(async move {
            Ok(self
                .read_pointer(format!("assembly:{}", key.release))
                .await?
                .map(|(sequence, pointer)| RuntimeAssemblyPointerReceipt { sequence, pointer }))
        })
    }
    fn cas_runtime_assembly_pointer(
        &self,
        request: RuntimeAssemblyPointerCas,
    ) -> TrustedRegistryFuture<'_, RuntimeAssemblyPointerReceipt> {
        Box::pin(async move {
            let expected = request.expected;
            let candidate = request.candidate;
            validate_runtime_pointer(&candidate)?;
            let (sequence, pointer) = self
                .cas_pointer(
                    format!("assembly:{}", candidate.release),
                    expected,
                    candidate,
                )
                .await?;
            Ok(RuntimeAssemblyPointerReceipt { sequence, pointer })
        })
    }
    fn runtime_assembly_pointer_history(
        &self,
        query: RuntimeAssemblyPointerHistoryQuery,
    ) -> TrustedRegistryFuture<'_, Vec<RuntimeAssemblyPointerReceipt>> {
        Box::pin(async move {
            Ok(self
                .pointer_history(format!("assembly:{}", query.key.release), query.selector)
                .await?
                .into_iter()
                .map(|(sequence, pointer)| RuntimeAssemblyPointerReceipt { sequence, pointer })
                .collect())
        })
    }
    fn activate(&self, request: ActivationRequest) -> TrustedRegistryFuture<'_, ActivationReceipt> {
        Box::pin(async move {
            let _ = request;
            Err(TrustedRegistryError::InvalidRequest(
                "production activation is Router-coordinated; direct registry activate is disabled"
                    .into(),
            ))
        })
    }
}

fn canonical_replica_ids(values: &[String]) -> Result<Vec<String>, ActivationBackendError> {
    if values.is_empty() {
        return Err(backend_invalid("activation replica set must not be empty"));
    }
    let mut canonical = values.to_vec();
    canonical.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for value in &canonical {
        validate_activation_token(value, "replicaId").map_err(backend_invalid)?;
    }
    if canonical.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(backend_invalid("activation replica ids must be unique"));
    }
    Ok(canonical)
}

fn backend_snapshot(
    environment: &str,
    document: &Document,
) -> Result<ActivationBackendSnapshot, ActivationBackendError> {
    let committed = match (document.get("generation"), document.get("assembly")) {
        (Some(_), Some(assembly)) => Some(ActivationBackendCommitted {
            generation: read_u64(document, "generation").map_err(backend_store)?,
            assembly: bson::from_bson(assembly.clone()).map_err(backend_invalid)?,
        }),
        (None, None) => None,
        _ => {
            return Err(backend_failure(
                "activation state has a partial committed tuple",
            ))
        }
    };
    let pending = document
        .get("pending")
        .cloned()
        .map(bson::from_bson)
        .transpose()
        .map_err(backend_invalid)?;
    Ok(ActivationBackendSnapshot {
        environment: environment.to_owned(),
        committed,
        pending,
    })
}

fn backend_document(
    snapshot: &ActivationBackendSnapshot,
    audit_sequence: u64,
) -> Result<Document, ActivationBackendError> {
    let mut document = doc! {
        "_id": &snapshot.environment,
        "auditSequence": i64::try_from(audit_sequence).map_err(|_| backend_failure("activation audit sequence exceeds Mongo i64"))?,
    };
    if let Some(committed) = &snapshot.committed {
        document.insert(
            "generation",
            i64::try_from(committed.generation)
                .map_err(|_| backend_failure("activation generation exceeds Mongo i64"))?,
        );
        document.insert(
            "assembly",
            bson::to_bson(&committed.assembly).map_err(backend_invalid)?,
        );
    }
    if let Some(pending) = &snapshot.pending {
        document.insert("pending", bson::to_bson(pending).map_err(backend_invalid)?);
    }
    Ok(document)
}

fn backend_audit_sequence(document: &Document) -> Result<u64, ActivationBackendError> {
    read_u64(document, "auditSequence").map_err(backend_store)
}
fn backend_invalid(error: impl std::fmt::Display) -> ActivationBackendError {
    ActivationBackendError {
        code: "invalid-request".into(),
        message: error.to_string(),
    }
}
fn backend_conflict(message: impl Into<String>) -> ActivationBackendError {
    ActivationBackendError {
        code: "cas-mismatch".into(),
        message: message.into(),
    }
}
fn backend_failure(message: impl Into<String>) -> ActivationBackendError {
    ActivationBackendError {
        code: "backend-unavailable".into(),
        message: message.into(),
    }
}
fn backend_mongo(error: mongodb::error::Error) -> ActivationBackendError {
    backend_failure(error.to_string())
}
fn backend_store(error: TrustedRegistryError) -> ActivationBackendError {
    let code = match error {
        TrustedRegistryError::Unauthorized => "unauthorized",
        TrustedRegistryError::NotFound => "not-found",
        TrustedRegistryError::InvalidRequest(_) => "invalid-request",
        TrustedRegistryError::CasMismatch => "cas-mismatch",
        TrustedRegistryError::ImmutableConflict => "immutable-conflict",
        TrustedRegistryError::BackendUnavailable => "backend-unavailable",
    };
    ActivationBackendError {
        code: code.into(),
        message: format!("{error:?}"),
    }
}

fn package_ref_result(
    value: &PackageArtifact,
) -> Result<PackageArtifactRef, skiff_artifact_identity::ArtifactIdentityError> {
    package_artifact_ref(value)
}
fn contract_ref_result(
    value: &ServiceContract,
) -> Result<ServiceContractRef, skiff_artifact_identity::ArtifactIdentityError> {
    service_contract_ref(value)
}
fn deployment_ref_result(
    value: &ServiceDeployment,
) -> Result<ServiceDeploymentRef, skiff_artifact_identity::ArtifactIdentityError> {
    Ok(service_deployment_ref(value))
}
fn assembly_ref_result(
    value: &RuntimeAssembly,
) -> Result<RuntimeAssemblyRef, skiff_artifact_identity::ArtifactIdentityError> {
    runtime_assembly_ref(value)
}
fn validate_deployment(
    value: &ServiceDeployment,
) -> Result<(), skiff_artifact_identity::ArtifactIdentityError> {
    validate_service_deployment_ref(&service_deployment_ref(value), value)
}

fn package_record_id(r: &PackageArtifactRef) -> String {
    format!(
        "package:{}:{}:{}:{}",
        r.package_id, r.package_version, r.package_build_id, r.package_local_abi_identity
    )
}
fn contract_record_id(r: &ServiceContractRef) -> String {
    format!(
        "contract:{}:{}:{}",
        r.service_id, r.contract_version, r.service_protocol_identity
    )
}
fn deployment_record_id(r: &ServiceDeploymentRef) -> String {
    format!(
        "deployment:{}:{}:{}:{}",
        r.service_id, r.contract_version, r.deployment_revision, r.deployment_artifact_identity
    )
}
fn runtime_record_id(r: &RuntimeAssemblyRef) -> String {
    format!("assembly:{}", r.assembly_identity)
}

trait PointerCoordinates {
    fn first(&self) -> &str;
    fn second(&self) -> &str;
    fn validate_typed(&self) -> TrustedRegistryResult<()>;
    fn record_id(&self) -> String;
    fn record_kind(&self) -> &'static str;
}
impl PointerCoordinates for PackageArtifactPointer {
    fn first(&self) -> &str {
        &self.artifact.package_id
    }
    fn second(&self) -> &str {
        &self.artifact.package_version
    }
    fn validate_typed(&self) -> TrustedRegistryResult<()> {
        validate_non_empty(self.first(), "packageId")?;
        validate_non_empty(self.second(), "packageVersion")
    }
    fn record_id(&self) -> String {
        package_record_id(&self.artifact)
    }
    fn record_kind(&self) -> &'static str {
        "packageArtifact"
    }
}
impl PointerCoordinates for ServiceContractPointer {
    fn first(&self) -> &str {
        &self.contract.service_id
    }
    fn second(&self) -> &str {
        &self.contract.contract_version
    }
    fn validate_typed(&self) -> TrustedRegistryResult<()> {
        validate_non_empty(self.first(), "serviceId")?;
        validate_non_empty(self.second(), "contractVersion")
    }
    fn record_id(&self) -> String {
        contract_record_id(&self.contract)
    }
    fn record_kind(&self) -> &'static str {
        "serviceContract"
    }
}
impl PointerCoordinates for ServiceDeploymentPointer {
    fn first(&self) -> &str {
        &self.deployment.service_id
    }
    fn second(&self) -> &str {
        &self.deployment.contract_version
    }
    fn validate_typed(&self) -> TrustedRegistryResult<()> {
        validate_non_empty(self.first(), "serviceId")?;
        validate_non_empty(self.second(), "contractVersion")
    }
    fn record_id(&self) -> String {
        deployment_record_id(&self.deployment)
    }
    fn record_kind(&self) -> &'static str {
        "serviceDeployment"
    }
}
impl PointerCoordinates for RuntimeAssemblyPointer {
    fn first(&self) -> &str {
        &self.release
    }
    fn second(&self) -> &str {
        ""
    }
    fn validate_typed(&self) -> TrustedRegistryResult<()> {
        validate_runtime_pointer(self)
    }
    fn record_id(&self) -> String {
        runtime_record_id(&self.assembly)
    }
    fn record_kind(&self) -> &'static str {
        "runtimeAssembly"
    }
}
fn candidate_key_first<P: PointerCoordinates>(pointer: &P) -> TrustedRegistryResult<String> {
    pointer.validate_typed()?;
    Ok(pointer.first().to_owned())
}
fn candidate_key_second<P: PointerCoordinates>(pointer: &P) -> TrustedRegistryResult<String> {
    pointer.validate_typed()?;
    Ok(pointer.second().to_owned())
}
fn validate_runtime_pointer(pointer: &RuntimeAssemblyPointer) -> TrustedRegistryResult<()> {
    validate_non_empty(&pointer.release, "release")?;
    validate_runtime_assembly_ref(&pointer.assembly).map_err(invalid_validation)
}

fn validate_selector(selector: &PointerHistorySelector) -> TrustedRegistryResult<()> {
    if selector.limit == 0 || selector.limit > 1000 {
        Err(TrustedRegistryError::InvalidRequest(
            "history limit must be between 1 and 1000".into(),
        ))
    } else {
        Ok(())
    }
}
fn validate_non_empty(value: &str, field: &str) -> TrustedRegistryResult<()> {
    if value.trim().is_empty() {
        Err(TrustedRegistryError::InvalidRequest(format!(
            "{field} must be non-empty"
        )))
    } else {
        Ok(())
    }
}
fn pointer_from_document<P: DeserializeOwned>(
    document: Document,
) -> TrustedRegistryResult<(u64, P)> {
    let sequence = read_sequence(&document)?;
    let pointer = bson::from_bson(
        document
            .get("pointer")
            .cloned()
            .ok_or(TrustedRegistryError::BackendUnavailable)?,
    )
    .map_err(invalid_bson)?;
    Ok((sequence, pointer))
}
fn read_sequence(document: &Document) -> TrustedRegistryResult<u64> {
    read_u64(document, "sequence")
}
fn read_u64(document: &Document, field: &str) -> TrustedRegistryResult<u64> {
    document
        .get_i64(field)
        .map_err(|_| TrustedRegistryError::BackendUnavailable)
        .and_then(|v| u64::try_from(v).map_err(|_| TrustedRegistryError::BackendUnavailable))
}
fn next_sequence(value: u64) -> TrustedRegistryResult<u64> {
    value
        .checked_add(1)
        .ok_or(TrustedRegistryError::BackendUnavailable)
}
async fn insert_audit(
    collection: &Collection<Document>,
    session: &mut mongodb::ClientSession,
    environment: &str,
    sequence: u64,
    event: &str,
    activation_id: &str,
) -> TrustedRegistryResult<()> {
    collection.insert_one(doc! {"_id": format!("{environment}:{sequence}"), "environment": environment, "sequence": sequence as i64, "event": event, "activationId": activation_id}).session(session).await.map_err(map_mongo)?;
    Ok(())
}
async fn commit_recoverably(session: &mut mongodb::ClientSession) -> TrustedRegistryResult<()> {
    // The driver retries retryable transaction commands; an unknown commit
    // result is safe to retry because commitTransaction is idempotent.
    match session.commit_transaction().await {
        Ok(()) => Ok(()),
        Err(error) if error.contains_label("UnknownTransactionCommitResult") => {
            session.commit_transaction().await.map_err(map_mongo)
        }
        Err(error) => Err(map_mongo(error)),
    }
}
fn duplicate_key(error: &mongodb::error::Error) -> bool {
    matches!(error.kind.as_ref(), ErrorKind::Write(WriteFailure::WriteError(e)) if e.code == 11000)
}
fn map_mongo(error: mongodb::error::Error) -> TrustedRegistryError {
    if duplicate_key(&error) {
        TrustedRegistryError::ImmutableConflict
    } else {
        TrustedRegistryError::BackendUnavailable
    }
}
fn invalid_bson(error: impl std::fmt::Display) -> TrustedRegistryError {
    TrustedRegistryError::InvalidRequest(error.to_string())
}
fn invalid_validation(error: impl std::fmt::Display) -> TrustedRegistryError {
    TrustedRegistryError::InvalidRequest(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_history_selector_is_bounded() {
        assert!(validate_selector(&PointerHistorySelector {
            after_sequence: None,
            limit: 1
        })
        .is_ok());
        assert_eq!(
            validate_selector(&PointerHistorySelector {
                after_sequence: None,
                limit: 0
            }),
            Err(TrustedRegistryError::InvalidRequest(
                "history limit must be between 1 and 1000".into()
            ))
        );
        assert!(validate_selector(&PointerHistorySelector {
            after_sequence: Some(4),
            limit: 1001
        })
        .is_err());
    }

    #[test]
    fn sequence_is_strict_and_overflow_fails_closed() {
        assert_eq!(next_sequence(0), Ok(1));
        assert_eq!(next_sequence(41), Ok(42));
        assert_eq!(
            next_sequence(u64::MAX),
            Err(TrustedRegistryError::BackendUnavailable)
        );
    }

    #[test]
    fn transaction_collection_ownership_is_disjoint_from_file_store() {
        assert_eq!(
            [
                RECORDS,
                POINTERS,
                POINTER_HISTORY,
                ACTIVATIONS,
                ACTIVATION_AUDIT
            ]
            .len(),
            5
        );
        let forbidden = ["use crate::storage::Canonical", "ArtifactStore"].concat();
        assert!(!include_str!("platform_registry.rs").contains(&forbidden));
    }

    #[test]
    fn immutable_conflict_and_stale_cas_paths_fail_closed() {
        let source = include_str!("platform_registry.rs");
        assert!(source.contains("Err(error) if duplicate_key(&error)"));
        assert!(source.contains("Err(TrustedRegistryError::ImmutableConflict)"));
        assert!(source.contains("if current_pointer != expected_bson"));
        assert!(source.contains("session.abort_transaction().await"));
    }

    #[test]
    fn pointer_current_and_history_share_one_transaction() {
        let source = include_str!("platform_registry.rs");
        let body = source
            .split("async fn cas_pointer")
            .nth(1)
            .expect("CAS implementation")
            .split("async fn pointer_history")
            .next()
            .expect("bounded CAS implementation");
        assert!(body.contains("self.pointers()"));
        assert!(body.contains("self.history()"));
        assert!(body.matches(".session(&mut session)").count() >= 3);
        assert!(body.contains("commit_recoverably(&mut session).await"));
    }

    #[test]
    fn activation_state_and_audit_share_transaction_and_commit_is_recoverable() {
        let source = include_str!("platform_registry.rs");
        let body = source
            .split("async fn mutate_backend_state")
            .nth(1)
            .expect("atomic activation implementation")
            .split("enum BackendMutation")
            .next()
            .expect("bounded activation implementation");
        assert!(body.contains("self.activations()"));
        assert!(body.contains("insert_audit("));
        assert!(body.matches(".session(&mut session)").count() >= 2);
        assert!(source.contains("UnknownTransactionCommitResult"));
        assert!(source.contains("session.commit_transaction().await"));
        assert!(source.contains("direct registry activate is disabled"));
    }
}
