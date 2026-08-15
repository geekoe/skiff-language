#![allow(clippy::too_many_lines)]

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use serde_json::Value;
use skiff_runtime_capability_context::{
    DbCapabilityContext, DbCapabilityContextApi, DbCapabilityError, DbCapabilityFactory,
    DbCapabilityFuture, DbCapabilityLeaseHandle, DbCapabilityLeaseHold, DbCapabilityResult,
    DbCapabilitySource, DbCapabilityStore, DbCapabilityStoreApi, DbDocument, DbKey, DbOneSelector,
    DbOrderEntry, DbPageResult, DbProviderBuildInput, DbProviderFactory, DbQuery,
    DbRecoverableRuntimeContext, DbRuntimeChange, DbRuntimeFinalizer, DbWriteResult, FieldPath,
    FileCapabilityRecord, PreparedDbValueRuntimeOperation, ServiceDbChange, ServiceDbFindOptions,
};
use skiff_runtime_model::{
    request_heap::{deep_clone_runtime_value_between_heaps, RequestHeap, RequestHeapLimits},
    runtime_value::{
        HeapNode, RuntimeMap, RuntimeObject, RuntimeObjectFields, RuntimeValue, RuntimeValueKey,
    },
};

const EXACT_DB_TARGET_KEY_PREFIX: &str = "skiff-db-object-target-v1";
const IN_MEMORY_PROVIDER: &str = "in-memory-service-db";
const MAX_LOGICAL_DB_VALUE_DEPTH: usize = 256;

/// In-process serviceDb provider for host/unit harnesses.
///
/// The provider uses the same capability-context and prepared-runtime APIs as
/// the Mongo provider. It stores logical runtime values in memory and keeps
/// transaction admission behind `DbCapabilityStore`'s request guard.
#[derive(Clone, Default)]
pub struct InMemoryDbProviderFactory {
    store: InMemoryDbStore,
    admitted_targets: Option<Arc<HashSet<String>>>,
}

impl InMemoryDbProviderFactory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Restricts admission to the exact `DbObjectTargetId` lookup keys.
    pub fn with_admitted_targets<I, S>(mut self, targets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let admitted = Arc::new(targets.into_iter().map(Into::into).collect::<HashSet<_>>());
        self.admitted_targets = Some(admitted);
        self
    }

    pub fn store(&self) -> &InMemoryDbStore {
        &self.store
    }
}

impl DbProviderFactory for InMemoryDbProviderFactory {
    fn build(&self, input: DbProviderBuildInput) -> DbCapabilityResult<DbCapabilitySource> {
        let store = if let Some(explicit) = &self.admitted_targets {
            let mut merged = explicit.as_ref().clone();
            merged.extend(
                input
                    .runtime_program_db
                    .iter()
                    .map(|metadata| metadata.target.lookup_key().to_string()),
            );
            InMemoryDbStore::new(Some(Arc::new(merged)))
        } else {
            self.store.clone()
        };
        Ok(DbCapabilitySource::new(Some(
            InMemoryDbCapabilityFactory::new(store),
        )))
    }
}

#[derive(Clone)]
pub struct InMemoryDbCapabilityFactory {
    store: InMemoryDbStore,
}

impl InMemoryDbCapabilityFactory {
    pub fn new(store: InMemoryDbStore) -> Self {
        Self { store }
    }
}

impl DbCapabilityFactory for InMemoryDbCapabilityFactory {
    fn context_for_request(&self, _owner: String, _request_id: String) -> DbCapabilityContext {
        DbCapabilityContext::new(InMemoryDbCapabilityContext {
            store: self.store.clone(),
        })
    }
}

#[derive(Clone)]
struct InMemoryDbCapabilityContext {
    store: InMemoryDbStore,
}

impl DbCapabilityContextApi for InMemoryDbCapabilityContext {
    fn require_store(
        &self,
        target: &str,
        unavailable_reason: &str,
    ) -> DbCapabilityResult<DbCapabilityStore> {
        self.store.admit_target(target, unavailable_reason)?;
        Ok(DbCapabilityStore::new(self.store.clone()))
    }
}

#[derive(Clone, Default)]
pub struct InMemoryDbStore {
    admitted_targets: Option<Arc<HashSet<String>>>,
    records: Arc<Mutex<HashMap<String, Vec<RuntimeValue>>>>,
}

impl InMemoryDbStore {
    pub fn new(admitted_targets: Option<Arc<HashSet<String>>>) -> Self {
        Self {
            admitted_targets,
            records: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn inserted_values(&self) -> Vec<RuntimeValue> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .flatten()
            .cloned()
            .collect()
    }

    fn admit_target(&self, target: &str, unavailable_reason: &str) -> DbCapabilityResult<()> {
        if !exact_target_key(target) {
            return Err(DbCapabilityError::provider_unavailable(
                target,
                unavailable_reason,
            ));
        }
        if let Some(admitted) = &self.admitted_targets {
            if !admitted.contains(target) {
                return Err(DbCapabilityError::provider_unavailable(
                    target,
                    unavailable_reason,
                ));
            }
        }
        Ok(())
    }
}

fn exact_target_key(target: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(target) else {
        return false;
    };
    let Some(fields) = value.as_array() else {
        return false;
    };
    fields.len() == 4
        && fields[0].as_str() == Some(EXACT_DB_TARGET_KEY_PREFIX)
        && fields[1].is_object()
        && fields[2].is_object()
        && fields[3].is_u64()
}

macro_rules! unavailable_async {
    ($method:ident, $ret:ty $(, $arg:ident: $ty:ty)*) => {
        fn $method<'a>(&'a self, $($arg: $ty),*) -> $ret {
            let _ = ($($arg,)*);
            Box::pin(async move {
                Err(DbCapabilityError::provider_unavailable(
                    IN_MEMORY_PROVIDER,
                    "in-memory serviceDb provider does not implement this operation",
                ))
            })
        }
    };
}

impl DbCapabilityStoreApi for InMemoryDbStore {
    fn begin_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn commit_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn abort_transaction(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }

    unavailable_async!(find_one_by_key, DbCapabilityFuture<'a, Option<DbDocument>>, type_name: &'a str, key: DbKey, projection: Option<Vec<FieldPath>>);
    unavailable_async!(find_one_by_key_runtime, DbCapabilityFuture<'a, Option<RuntimeValue>>, type_name: &'a str, key: DbKey, projection: Option<Vec<FieldPath>>, heap: &'a mut RequestHeap, context: DbRecoverableRuntimeContext);
    unavailable_async!(find_one_by_query, DbCapabilityFuture<'a, Option<DbDocument>>, type_name: &'a str, query: DbQuery, order: Vec<DbOrderEntry>, projection: Option<Vec<FieldPath>>);
    unavailable_async!(find_one_by_query_runtime, DbCapabilityFuture<'a, Option<RuntimeValue>>, type_name: &'a str, query: DbQuery, order: Vec<DbOrderEntry>, projection: Option<Vec<FieldPath>>, heap: &'a mut RequestHeap, context: DbRecoverableRuntimeContext);
    unavailable_async!(find_many_page, DbCapabilityFuture<'a, DbPageResult>, type_name: &'a str, query: DbQuery, options: ServiceDbFindOptions, projection: Option<Vec<FieldPath>>);
    unavailable_async!(find_many_page_runtime, DbCapabilityFuture<'a, Vec<RuntimeValue>>, type_name: &'a str, query: DbQuery, options: ServiceDbFindOptions, projection: Option<Vec<FieldPath>>, heap: &'a mut RequestHeap, context: DbRecoverableRuntimeContext);
    unavailable_async!(create, DbCapabilityFuture<'a, DbDocument>, type_name: &'a str, value: DbDocument);

    fn create_runtime<'a>(
        &'a self,
        type_name: &'a str,
        value: &'a RuntimeValue,
        heap: &'a mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, RuntimeValue> {
        let prepared = match self.prepare_create_runtime(type_name, value, heap, context) {
            Ok(prepared) => prepared,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        Box::pin(async move { prepared.into_wait().await?.finalize(heap) })
    }

    fn prepare_create_runtime(
        &self,
        type_name: &str,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbValueRuntimeOperation> {
        self.admit_target(type_name, "in-memory DB target is not admitted")?;
        let type_name = type_name.to_string();
        let mut logical_heap = RequestHeap::new(RequestHeapLimits::default());
        let logical_value = logical_runtime_value(value, heap, &mut logical_heap, 0)?;
        let records = Arc::clone(&self.records);
        Ok(PreparedDbValueRuntimeOperation::new(async move {
            records
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entry(type_name)
                .or_default()
                .push(logical_value.clone());
            Ok(DbRuntimeFinalizer::new(move |heap| {
                deep_clone_runtime_value_between_heaps(&logical_heap, heap, &logical_value)
                    .map_err(|error| DbCapabilityError::decode(error.to_string()))
            }))
        }))
    }

    unavailable_async!(insert_many_result, DbCapabilityFuture<'a, DbWriteResult>, type_name: &'a str, values: Vec<DbDocument>);
    unavailable_async!(update_one, DbCapabilityFuture<'a, Option<DbDocument>>, type_name: &'a str, selector: DbOneSelector, change: ServiceDbChange);
    unavailable_async!(update_one_runtime, DbCapabilityFuture<'a, Option<RuntimeValue>>, type_name: &'a str, selector: DbOneSelector, change: DbRuntimeChange, heap: &'a mut RequestHeap, context: DbRecoverableRuntimeContext);
    unavailable_async!(update_many, DbCapabilityFuture<'a, DbWriteResult>, type_name: &'a str, query: DbQuery, change: ServiceDbChange);
    unavailable_async!(upsert_by_key, DbCapabilityFuture<'a, DbWriteResult>, type_name: &'a str, key: DbKey, insert: DbDocument, change: ServiceDbChange);
    unavailable_async!(replace_one, DbCapabilityFuture<'a, Option<DbDocument>>, type_name: &'a str, selector: DbOneSelector, value: DbDocument);
    unavailable_async!(replace_one_runtime, DbCapabilityFuture<'a, Option<RuntimeValue>>, type_name: &'a str, selector: DbOneSelector, value: &'a RuntimeValue, heap: &'a mut RequestHeap, context: DbRecoverableRuntimeContext);
    unavailable_async!(delete_one, DbCapabilityFuture<'a, bool>, type_name: &'a str, selector: DbOneSelector);
    unavailable_async!(delete_many, DbCapabilityFuture<'a, DbWriteResult>, type_name: &'a str, query: DbQuery);
    unavailable_async!(count, DbCapabilityFuture<'a, u64>, type_name: &'a str, query: DbQuery);
    unavailable_async!(exists_by_key, DbCapabilityFuture<'a, bool>, type_name: &'a str, key: DbKey);
    unavailable_async!(exists_by_query, DbCapabilityFuture<'a, bool>, type_name: &'a str, query: DbQuery);
    unavailable_async!(claim_lease, DbCapabilityFuture<'a, Option<DbCapabilityLeaseHandle>>, type_name: &'a str, key: DbKey, slot: &'a str);
    unavailable_async!(renew_lease, DbCapabilityFuture<'a, bool>, hold: &'a DbCapabilityLeaseHold);
    unavailable_async!(release_lease, DbCapabilityFuture<'a, ()>, hold: &'a DbCapabilityLeaseHold);
    unavailable_async!(read_lease, DbCapabilityFuture<'a, Option<Value>>, type_name: &'a str, key: DbKey, slot: &'a str);

    fn lease_lost(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async { false })
    }

    unavailable_async!(insert_skiff_file_record, DbCapabilityFuture<'a, ()>, record: FileCapabilityRecord);
    unavailable_async!(find_skiff_file_by_id, DbCapabilityFuture<'a, Option<FileCapabilityRecord>>, id: &'a str);
    unavailable_async!(delete_skiff_file_by_id, DbCapabilityFuture<'a, u64>, id: &'a str);
}

fn logical_runtime_value(
    value: &RuntimeValue,
    source: &RequestHeap,
    destination: &mut RequestHeap,
    depth: usize,
) -> DbCapabilityResult<RuntimeValue> {
    if depth > MAX_LOGICAL_DB_VALUE_DEPTH {
        return Err(DbCapabilityError::decode(
            "in-memory DB value exceeds logical decode depth",
        ));
    }
    match value {
        RuntimeValue::Null
        | RuntimeValue::Bool(_)
        | RuntimeValue::Number(_)
        | RuntimeValue::Date(_) => Ok(value.clone()),
        RuntimeValue::String(value) => Ok(RuntimeValue::String(value.clone())),
        RuntimeValue::ActorRef(_) => Err(DbCapabilityError::decode(
            "in-memory DB value cannot carry an actor ref",
        )),
        RuntimeValue::Heap(handle) => {
            if let Some(carrier) = source
                .local_carrier_cell(*handle)
                .map_err(|error| DbCapabilityError::decode(error.to_string()))?
            {
                return logical_runtime_value(carrier.value(), source, destination, depth + 1);
            }
            match source.get(*handle) {
                Ok(HeapNode::Bytes(bytes)) => Ok(RuntimeValue::String(
                    String::from_utf8_lossy(bytes.as_slice()).into_owned(),
                )),
                Ok(HeapNode::Array(items)) => {
                    let values = items
                        .iter()
                        .map(|item| logical_runtime_value(item, source, destination, depth + 1))
                        .collect::<DbCapabilityResult<Vec<_>>>()?;
                    destination
                        .alloc_array(values)
                        .map(RuntimeValue::Heap)
                        .map_err(|error| DbCapabilityError::decode(error.to_string()))
                }
                Ok(HeapNode::Object(object)) => {
                    let fields = object
                        .fields()
                        .iter()
                        .map(|(field, value)| {
                            Ok((
                                field.clone(),
                                logical_runtime_value(value, source, destination, depth + 1)?,
                            ))
                        })
                        .collect::<DbCapabilityResult<RuntimeObjectFields>>()?;
                    destination
                        .alloc_object(RuntimeObject::unshaped(fields))
                        .map(RuntimeValue::Heap)
                        .map_err(|error| DbCapabilityError::decode(error.to_string()))
                }
                Ok(HeapNode::Map(map)) => {
                    let entries = map
                        .iter()
                        .map(|(key, value)| {
                            Ok((
                                RuntimeValueKey::string(key.string_payload()),
                                logical_runtime_value(value, source, destination, depth + 1)?,
                            ))
                        })
                        .collect::<DbCapabilityResult<RuntimeMap>>()?;
                    destination
                        .alloc_map(entries)
                        .map(RuntimeValue::Heap)
                        .map_err(|error| DbCapabilityError::decode(error.to_string()))
                }
                Ok(HeapNode::Interface(_) | HeapNode::Exception(_)) => {
                    Err(DbCapabilityError::decode(
                        "in-memory DB value cannot carry a live capability or exception",
                    ))
                }
                Err(error) => Err(DbCapabilityError::decode(error.to_string())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use skiff_artifact_model::{
        FileIrRef, PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity,
    };
    use skiff_runtime_boundary::recoverable::FailClosedRecoverableBehaviorHooks;
    use skiff_runtime_capability_context::{
        DbCapabilityError, DbCapabilityTarget, DbCapabilityTargetId, DbProviderConfig,
        DbRecoverableRuntimeContext, DbRecoverableRuntimeExpectedPlans,
    };
    use skiff_runtime_model::recoverable::{
        RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableStorageLane, RuntimeRecoverableTrustBoundary,
    };
    use skiff_runtime_model::request_heap::RequestHeap;

    use super::*;

    fn fixture_target() -> DbCapabilityTarget {
        DbCapabilityTarget::new(
            DbCapabilityTargetId {
                package_artifact_ref: PackageArtifactRef {
                    package_id: "test.skiff/bytecode-vm-phase-6-db".to_string(),
                    package_version: "1.0.0".to_string(),
                    package_build_id: PackageBuildId::new("build:phase-6-db"),
                    package_local_abi_identity: PackageLocalAbiIdentity::new("abi:phase-6-db"),
                },
                file_ir_ref: FileIrRef::new("file:phase-6-db", "main.skiff"),
                type_index: 0,
            },
            "Doc",
        )
    }

    fn provider_input() -> DbProviderBuildInput {
        DbProviderBuildInput {
            environment: "skiff-test".to_string(),
            service_id: "test.skiff/bytecode-vm-phase-6-db".to_string(),
            config: DbProviderConfig::opaque(json!({})),
            runtime_program_db: Vec::new(),
        }
    }

    fn recoverable_context() -> DbRecoverableRuntimeContext {
        DbRecoverableRuntimeContext {
            behavior_hooks: Arc::new(FailClosedRecoverableBehaviorHooks),
            expected_plans: DbRecoverableRuntimeExpectedPlans::default(),
            artifact_identity: "artifact:phase-6-db".to_string(),
            build_id: "build:phase-6-db".to_string(),
            boundary_context: RuntimeRecoverableBoundaryContext::new(
                RuntimeRecoverableBoundaryKind::DbValue,
                RuntimeRecoverableTrustBoundary::OwnerInternal,
                RuntimeRecoverableStorageLane::RecoverableEnvelope,
            ),
            retention_expires_at_epoch_millis: None,
        }
    }

    #[test]
    fn in_memory_provider_admits_exact_fixture_target_and_rejects_unadmitted() {
        let exact = fixture_target();
        let provider = InMemoryDbProviderFactory::new()
            .with_admitted_targets([exact.lookup_key().to_string()]);
        let source = provider
            .build(provider_input())
            .expect("in-memory provider builds");
        let context = source.context_for_request("test.skiff/bytecode-vm-phase-6-db", "request");

        context
            .require_store(exact.lookup_key(), "exact fixture target must be admitted")
            .expect("exact fixture target should be admitted");

        let other = DbCapabilityTarget::new(
            DbCapabilityTargetId {
                package_artifact_ref: PackageArtifactRef {
                    package_id: "test.skiff/bytecode-vm-phase-6-recoverable".to_string(),
                    package_version: "1.0.0".to_string(),
                    package_build_id: PackageBuildId::new("build:phase-6-recoverable"),
                    package_local_abi_identity: PackageLocalAbiIdentity::new(
                        "abi:phase-6-recoverable",
                    ),
                },
                file_ir_ref: FileIrRef::new("file:phase-6-recoverable", "main.skiff"),
                type_index: 0,
            },
            "RecoverableDoc",
        );
        let error = match context.require_store(other.lookup_key(), "other target is not admitted")
        {
            Ok(_) => panic!("unadmitted exact target should fail closed"),
            Err(error) => error,
        };
        match error {
            DbCapabilityError::ProviderUnavailable { reason, .. } => {
                assert_eq!(reason, "other target is not admitted");
            }
            other => panic!("expected provider unavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn in_memory_provider_prepared_insert_persists_and_returns_value() {
        let provider = InMemoryDbProviderFactory::new();
        let source = provider
            .build(provider_input())
            .expect("in-memory provider builds");
        let context = source.context_for_request("test.skiff/bytecode-vm-phase-6-db", "request");
        let exact = fixture_target();
        let store = context
            .require_store(exact.lookup_key(), "exact fixture target must be admitted")
            .expect("exact fixture target should be admitted");
        let mut heap = RequestHeap::default();
        let value = RuntimeValue::String("phase6-db".to_string());

        let prepared = store
            .prepare_create_runtime(exact.lookup_key(), &value, &mut heap, recoverable_context())
            .expect("prepared insert should be accepted");

        let finalizer = prepared
            .into_wait()
            .await
            .expect("prepared insert should complete");
        let created = finalizer
            .finalize(&mut heap)
            .expect("prepared insert should finalize");

        assert_eq!(created, value);
        assert_eq!(provider.store().inserted_values(), vec![value]);
    }

    #[tokio::test]
    async fn in_memory_provider_prepared_insert_normalizes_vm_string_carriers() {
        let provider = InMemoryDbProviderFactory::new();
        let source = provider
            .build(provider_input())
            .expect("in-memory provider builds");
        let context = source.context_for_request("test.skiff/bytecode-vm-phase-6-db", "request");
        let exact = fixture_target();
        let store = context
            .require_store(exact.lookup_key(), "exact fixture target must be admitted")
            .expect("exact fixture target should be admitted");

        let mut source_heap = RequestHeap::default();
        let id_handle = source_heap
            .alloc_local_carrier_cell(RuntimeValue::String("phase6-db".to_string()).into())
            .expect("VM string carrier should allocate");
        let object_handle = source_heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
                "id".to_string(),
                RuntimeValue::Heap(id_handle),
            )])))
            .expect("VM record should allocate");
        let value = RuntimeValue::Heap(object_handle);

        let prepared = store
            .prepare_create_runtime(
                exact.lookup_key(),
                &value,
                &mut source_heap,
                recoverable_context(),
            )
            .expect("prepared insert should accept a VM record");
        let finalizer = prepared
            .into_wait()
            .await
            .expect("prepared insert should complete");
        let mut destination_heap = RequestHeap::default();
        let created = finalizer
            .finalize(&mut destination_heap)
            .expect("prepared insert should finalize a logical record");

        let RuntimeValue::Heap(created_handle) = created else {
            panic!("logical DB insert result must be a record");
        };
        let HeapNode::Object(created_object) = destination_heap
            .get(created_handle)
            .expect("created record")
        else {
            panic!("created DB result must be an object");
        };
        assert_eq!(
            created_object.fields().get("id"),
            Some(&RuntimeValue::String("phase6-db".to_string()))
        );
        assert_eq!(provider.store().inserted_values().len(), 1);
    }
}
