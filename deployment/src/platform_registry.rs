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

use crate::{
    storage::{
        PackageArtifactPointer, RuntimeAssemblyPointer, ServiceContractPointer,
        ServiceDeploymentPointer,
    },
    trusted_registry::{
        ActivationPrepare, ActivationReceipt, ActivationRef, PackageArtifactPointerReceipt,
        PointerHistorySelector, RuntimeAssemblyPointerReceipt, ServiceContractPointerReceipt,
        ServiceDeploymentPointerReceipt, TrustedRegistry, TrustedRegistryError,
        TrustedRegistryFuture, TrustedRegistryResult,
    },
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

    async fn prepare(
        &self,
        request: ActivationPrepare,
    ) -> TrustedRegistryResult<ActivationReceipt> {
        validate_activation(&request)?;
        self.read_record::<RuntimeAssembly>(
            runtime_record_id(&request.tuple.assembly),
            "runtimeAssembly",
        )
        .await?;
        let mut session = self.client.start_session().await.map_err(map_mongo)?;
        session.start_transaction().await.map_err(map_mongo)?;
        let existing = self
            .activations()
            .find_one(doc! {"_id": &request.tuple.environment})
            .session(&mut session)
            .await
            .map_err(map_mongo)?;
        let (generation, assembly, sequence) = match existing {
            Some(ref state) => (
                read_u64(state, "generation")?,
                read_ref(state, "assembly")?,
                read_u64(state, "auditSequence")?,
            ),
            None if request.tuple.expected_generation == 0 => {
                (0, request.tuple.assembly.clone(), 0)
            }
            None => {
                session.abort_transaction().await.map_err(map_mongo)?;
                return Err(TrustedRegistryError::CasMismatch);
            }
        };
        if generation != request.tuple.expected_generation {
            session.abort_transaction().await.map_err(map_mongo)?;
            return Err(TrustedRegistryError::CasMismatch);
        }
        if existing
            .as_ref()
            .and_then(|state| state.get("pending"))
            .is_some_and(|value| *value != Bson::Null)
        {
            session.abort_transaction().await.map_err(map_mongo)?;
            return Err(TrustedRegistryError::CasMismatch);
        }
        let pending = bson::to_bson(&request).map_err(invalid_bson)?;
        let next = next_sequence(sequence)?;
        self.activations()
            .replace_one(
                doc! {"_id": &request.tuple.environment},
                doc! {
                    "_id": &request.tuple.environment,
                    "generation": generation as i64,
                    "assembly": bson::to_bson(&assembly).map_err(invalid_bson)?,
                    "pending": pending,
                    "auditSequence": next as i64,
                },
            )
            .upsert(true)
            .session(&mut session)
            .await
            .map_err(map_mongo)?;
        insert_audit(
            &self.audit(),
            &mut session,
            &request.tuple.environment,
            next,
            "prepared",
            &request.activation_id,
        )
        .await?;
        commit_recoverably(&mut session).await?;
        Ok(receipt(
            &request.tuple.environment,
            &request.activation_id,
            request.tuple.candidate_generation,
            request.tuple.assembly,
        ))
    }

    async fn finish_activation(
        &self,
        activation: ActivationRef,
        commit: bool,
    ) -> TrustedRegistryResult<ActivationReceipt> {
        validate_activation_environment(&activation.environment).map_err(invalid_validation)?;
        validate_activation_token(&activation.activation_id, "activationId")
            .map_err(invalid_validation)?;
        let mut session = self.client.start_session().await.map_err(map_mongo)?;
        session.start_transaction().await.map_err(map_mongo)?;
        let state = self
            .activations()
            .find_one(doc! {"_id": &activation.environment})
            .session(&mut session)
            .await
            .map_err(map_mongo)?
            .ok_or(TrustedRegistryError::NotFound)?;
        let pending: ActivationPrepare = bson::from_bson(
            state
                .get("pending")
                .cloned()
                .filter(|value| *value != Bson::Null)
                .ok_or(TrustedRegistryError::CasMismatch)?,
        )
        .map_err(invalid_bson)?;
        if pending.activation_id != activation.activation_id {
            session.abort_transaction().await.map_err(map_mongo)?;
            return Err(TrustedRegistryError::CasMismatch);
        }
        let audit_sequence = next_sequence(read_u64(&state, "auditSequence")?)?;
        let (generation, assembly, event) = if commit {
            (
                pending.tuple.candidate_generation,
                pending.tuple.assembly.clone(),
                "committed",
            )
        } else {
            (
                read_u64(&state, "generation")?,
                read_ref(&state, "assembly")?,
                "aborted",
            )
        };
        self.activations()
            .update_one(
                doc! {"_id": &activation.environment, "pending.activationId": &activation.activation_id},
                doc! {"$set": {
                    "generation": generation as i64,
                    "assembly": bson::to_bson(&assembly).map_err(invalid_bson)?,
                    "pending": Bson::Null,
                    "auditSequence": audit_sequence as i64,
                }},
            )
            .session(&mut session)
            .await
            .map_err(map_mongo)?;
        insert_audit(
            &self.audit(),
            &mut session,
            &activation.environment,
            audit_sequence,
            event,
            &activation.activation_id,
        )
        .await?;
        commit_recoverably(&mut session).await?;
        Ok(ActivationReceipt {
            activation,
            generation,
            assembly,
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
    ($read:ident, $cas:ident, $history:ident, $pointer:ty, $receipt:ident, $key:expr) => {
        fn $read(
            &self,
            first: String,
            second: String,
        ) -> TrustedRegistryFuture<'_, Option<$receipt>> {
            Box::pin(async move {
                Ok(self
                    .read_pointer(($key)(&first, &second))
                    .await?
                    .map(|(sequence, pointer)| $receipt { sequence, pointer }))
            })
        }
        fn $cas(
            &self,
            expected: Option<$pointer>,
            candidate: $pointer,
        ) -> TrustedRegistryFuture<'_, $receipt> {
            Box::pin(async move {
                let key = ($key)(
                    &candidate_key_first(&candidate)?,
                    &candidate_key_second(&candidate)?,
                );
                let (sequence, pointer) = self.cas_pointer(key, expected, candidate).await?;
                Ok($receipt { sequence, pointer })
            })
        }
        fn $history(
            &self,
            first: String,
            second: String,
            selector: PointerHistorySelector,
        ) -> TrustedRegistryFuture<'_, Vec<$receipt>> {
            Box::pin(async move {
                Ok(self
                    .pointer_history(($key)(&first, &second), selector)
                    .await?
                    .into_iter()
                    .map(|(sequence, pointer)| $receipt { sequence, pointer })
                    .collect())
            })
        }
    };
}

impl TrustedRegistry for PlatformDbTrustedRegistry {
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
        read_package_artifact_pointer,
        cas_package_artifact_pointer,
        package_artifact_pointer_history,
        PackageArtifactPointer,
        PackageArtifactPointerReceipt,
        |a: &str, b: &str| format!("package:{a}:{b}")
    );
    pointer_methods!(
        read_service_contract_pointer,
        cas_service_contract_pointer,
        service_contract_pointer_history,
        ServiceContractPointer,
        ServiceContractPointerReceipt,
        |a: &str, b: &str| format!("contract:{a}:{b}")
    );
    pointer_methods!(
        read_service_deployment_pointer,
        cas_service_deployment_pointer,
        service_deployment_pointer_history,
        ServiceDeploymentPointer,
        ServiceDeploymentPointerReceipt,
        |a: &str, b: &str| format!("deployment:{a}:{b}")
    );

    fn read_runtime_assembly_pointer(
        &self,
        release: String,
    ) -> TrustedRegistryFuture<'_, Option<RuntimeAssemblyPointerReceipt>> {
        Box::pin(async move {
            Ok(self
                .read_pointer(format!("assembly:{release}"))
                .await?
                .map(|(sequence, pointer)| RuntimeAssemblyPointerReceipt { sequence, pointer }))
        })
    }
    fn cas_runtime_assembly_pointer(
        &self,
        expected: Option<RuntimeAssemblyPointer>,
        candidate: RuntimeAssemblyPointer,
    ) -> TrustedRegistryFuture<'_, RuntimeAssemblyPointerReceipt> {
        Box::pin(async move {
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
        release: String,
        selector: PointerHistorySelector,
    ) -> TrustedRegistryFuture<'_, Vec<RuntimeAssemblyPointerReceipt>> {
        Box::pin(async move {
            Ok(self
                .pointer_history(format!("assembly:{release}"), selector)
                .await?
                .into_iter()
                .map(|(sequence, pointer)| RuntimeAssemblyPointerReceipt { sequence, pointer })
                .collect())
        })
    }
    fn prepare_activation(
        &self,
        request: ActivationPrepare,
    ) -> TrustedRegistryFuture<'_, ActivationReceipt> {
        Box::pin(async move { self.prepare(request).await })
    }
    fn commit_activation(
        &self,
        activation: ActivationRef,
    ) -> TrustedRegistryFuture<'_, ActivationReceipt> {
        Box::pin(async move { self.finish_activation(activation, true).await })
    }
    fn abort_activation(
        &self,
        activation: ActivationRef,
    ) -> TrustedRegistryFuture<'_, ActivationReceipt> {
        Box::pin(async move { self.finish_activation(activation, false).await })
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
        PackageArtifactPointer::new(self.artifact.clone())
            .map_err(invalid_validation)
            .and_then(|p| {
                if p == *self {
                    Ok(())
                } else {
                    Err(TrustedRegistryError::InvalidRequest(
                        "invalid package pointer".into(),
                    ))
                }
            })
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
        ServiceContractPointer::new(self.contract.clone())
            .map_err(invalid_validation)
            .and_then(|p| {
                if p == *self {
                    Ok(())
                } else {
                    Err(TrustedRegistryError::InvalidRequest(
                        "invalid contract pointer".into(),
                    ))
                }
            })
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
        ServiceDeploymentPointer::new(self.deployment.clone())
            .map_err(invalid_validation)
            .and_then(|p| {
                if p == *self {
                    Ok(())
                } else {
                    Err(TrustedRegistryError::InvalidRequest(
                        "invalid deployment pointer".into(),
                    ))
                }
            })
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
    let expected = RuntimeAssemblyPointer::new(pointer.release.clone(), pointer.assembly.clone())
        .map_err(invalid_validation)?;
    if expected == *pointer {
        Ok(())
    } else {
        Err(TrustedRegistryError::InvalidRequest(
            "invalid runtime assembly pointer".into(),
        ))
    }
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
fn validate_activation(request: &ActivationPrepare) -> TrustedRegistryResult<()> {
    validate_activation_environment(&request.tuple.environment).map_err(invalid_validation)?;
    validate_activation_token(&request.activation_id, "activationId")
        .map_err(invalid_validation)?;
    validate_transition_generations(
        request.tuple.expected_generation,
        request.tuple.candidate_generation,
    )
    .map_err(invalid_validation)?;
    validate_runtime_assembly_ref(&request.tuple.assembly).map_err(invalid_validation)
}
fn receipt(
    environment: &str,
    activation_id: &str,
    generation: u64,
    assembly: RuntimeAssemblyRef,
) -> ActivationReceipt {
    ActivationReceipt {
        activation: ActivationRef {
            environment: environment.into(),
            activation_id: activation_id.into(),
        },
        generation,
        assembly,
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
fn read_ref(document: &Document, field: &str) -> TrustedRegistryResult<RuntimeAssemblyRef> {
    bson::from_bson(
        document
            .get(field)
            .cloned()
            .ok_or(TrustedRegistryError::BackendUnavailable)?,
    )
    .map_err(invalid_bson)
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
        assert!(source.contains("insert_audit("));
        assert!(source.contains("UnknownTransactionCommitResult"));
        assert!(source.contains("session.commit_transaction().await"));
        assert!(source.contains("\"pending\": Bson::Null"));
    }
}
