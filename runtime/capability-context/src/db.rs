use std::{any::Any, collections::HashMap, error::Error, fmt, future::Future, pin::Pin, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use skiff_artifact_model::DbMetadataIr;
use skiff_runtime_boundary::recoverable::RecoverableBehaviorHooks;
use skiff_runtime_model::{
    error::{RuntimeErrorPayload, TypeIdentity, WirePayload},
    recoverable::{RuntimeRecoverableBoundaryContext, RuntimeRecoverableExpectedTypePlan},
    request_heap::RequestHeap,
    runtime_value::RuntimeValue,
};

macro_rules! db_wire_newtype {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
        #[serde(transparent)]
        pub struct $name(Value);

        impl $name {
            pub fn new(value: Value) -> Self {
                Self(value)
            }

            pub fn as_value(&self) -> &Value {
                &self.0
            }

            pub fn into_value(self) -> Value {
                self.0
            }
        }

        impl From<Value> for $name {
            fn from(value: Value) -> Self {
                Self::new(value)
            }
        }

        impl From<$name> for Value {
            fn from(value: $name) -> Self {
                value.into_value()
            }
        }
    };
}

db_wire_newtype!(DbKey);
db_wire_newtype!(DbQuery);
db_wire_newtype!(DbDocument);
db_wire_newtype!(DbWriteResult);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FieldPath {
    pub text: String,
    pub segments: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DbOrderEntry {
    pub field: FieldPath,
    pub direction: DbOrderDirection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DbOrderDirection {
    Asc,
    Desc,
}

#[derive(Clone, Debug, Default)]
pub struct ServiceDbFindOptions {
    pub order: Vec<DbOrderEntry>,
    pub limit: Option<i64>,
    pub offset: Option<u64>,
}

#[derive(Clone, Debug)]
pub enum DbOneSelector {
    Key(DbKey),
    Query {
        query: DbQuery,
        order: Vec<DbOrderEntry>,
    },
}

impl DbOneSelector {
    pub fn key(key: impl Into<DbKey>) -> Self {
        Self::Key(key.into())
    }

    pub fn query(query: impl Into<DbQuery>, order: Vec<DbOrderEntry>) -> Self {
        Self::Query {
            query: query.into(),
            order,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DbPageResult {
    pub values: Vec<DbDocument>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ServiceDbChangeOp {
    Set { field: String, value: DbDocument },
    Inc { field: String, value: DbDocument },
    Unset { field: String },
    AddToSet { field: String, value: DbDocument },
    Pull { field: String, value: DbDocument },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceDbChange {
    ops: Vec<ServiceDbChangeOp>,
}

impl ServiceDbChangeOp {
    pub fn field(&self) -> &str {
        match self {
            ServiceDbChangeOp::Set { field, .. }
            | ServiceDbChangeOp::Inc { field, .. }
            | ServiceDbChangeOp::Unset { field }
            | ServiceDbChangeOp::AddToSet { field, .. }
            | ServiceDbChangeOp::Pull { field, .. } => field,
        }
    }

    pub fn value(&self) -> Option<&DbDocument> {
        match self {
            ServiceDbChangeOp::Set { value, .. }
            | ServiceDbChangeOp::Inc { value, .. }
            | ServiceDbChangeOp::AddToSet { value, .. }
            | ServiceDbChangeOp::Pull { value, .. } => Some(value),
            ServiceDbChangeOp::Unset { .. } => None,
        }
    }
}

impl ServiceDbChange {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn set(&mut self, field: impl Into<String>, value: impl Into<DbDocument>) {
        self.ops.push(ServiceDbChangeOp::Set {
            field: field.into(),
            value: value.into(),
        });
    }

    pub fn inc(&mut self, field: impl Into<String>, value: impl Into<DbDocument>) {
        self.ops.push(ServiceDbChangeOp::Inc {
            field: field.into(),
            value: value.into(),
        });
    }

    pub fn unset(&mut self, field: impl Into<String>) {
        self.ops.push(ServiceDbChangeOp::Unset {
            field: field.into(),
        });
    }

    pub fn add_to_set(&mut self, field: impl Into<String>, value: impl Into<DbDocument>) {
        self.ops.push(ServiceDbChangeOp::AddToSet {
            field: field.into(),
            value: value.into(),
        });
    }

    pub fn pull(&mut self, field: impl Into<String>, value: impl Into<DbDocument>) {
        self.ops.push(ServiceDbChangeOp::Pull {
            field: field.into(),
            value: value.into(),
        });
    }

    pub fn touched_fields(&self) -> impl Iterator<Item = &str> {
        self.ops.iter().map(ServiceDbChangeOp::field)
    }

    pub fn ops(&self) -> &[ServiceDbChangeOp] {
        &self.ops
    }

    pub fn operations(&self) -> impl Iterator<Item = &ServiceDbChangeOp> {
        self.ops.iter()
    }

    pub fn into_ops(self) -> Vec<ServiceDbChangeOp> {
        self.ops
    }

    pub fn unset_contains(&self, field: &str) -> bool {
        self.ops
            .iter()
            .any(|op| matches!(op, ServiceDbChangeOp::Unset { field: unset } if unset == field))
    }

    pub fn unset_fields(&self) -> impl Iterator<Item = &str> {
        self.ops.iter().filter_map(|op| match op {
            ServiceDbChangeOp::Unset { field } => Some(field.as_str()),
            _ => None,
        })
    }

    pub fn set_value(&self, field: &str) -> Option<&DbDocument> {
        self.ops.iter().rev().find_map(|op| match op {
            ServiceDbChangeOp::Set { field: set, value } if set == field => Some(value),
            _ => None,
        })
    }

    pub fn set_entries(&self) -> Vec<(&str, &DbDocument)> {
        let mut entries = Vec::new();
        for op in &self.ops {
            let ServiceDbChangeOp::Set { field, value } = op else {
                continue;
            };
            if let Some((_, existing_value)) = entries
                .iter_mut()
                .find(|(existing_field, _)| *existing_field == field)
            {
                *existing_value = value;
            } else {
                entries.push((field.as_str(), value));
            }
        }
        entries
    }
}

pub type DbCapabilityResult<T> = Result<T, DbCapabilityError>;
pub type DbCapabilityFuture<'a, T> =
    Pin<Box<dyn Future<Output = DbCapabilityResult<T>> + Send + 'a>>;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(transparent)]
pub struct DbProviderConfig {
    value: Value,
}

impl DbProviderConfig {
    pub fn opaque(value: Value) -> Self {
        Self { value }
    }

    pub fn as_value(&self) -> &Value {
        &self.value
    }

    pub fn into_value(self) -> Value {
        self.value
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DbProviderBuildInput {
    pub service_id: String,
    pub config: DbProviderConfig,
    pub runtime_program_db: Vec<DbMetadataIr>,
}

#[derive(Debug)]
pub enum DbCapabilityError {
    Decode(String),
    ProviderUnavailable { target: String, reason: String },
    Opaque(Box<dyn WirePayload>),
}

impl DbCapabilityError {
    pub fn decode(message: impl Into<String>) -> Self {
        Self::Decode(message.into())
    }

    pub fn provider_unavailable(target: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ProviderUnavailable {
            target: target.into(),
            reason: reason.into(),
        }
    }

    pub fn opaque(error: impl WirePayload) -> Self {
        Self::Opaque(Box::new(error))
    }
}

impl fmt::Display for DbCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(message) => formatter.write_str(message),
            Self::ProviderUnavailable { target, reason } => {
                write!(formatter, "provider unavailable for {target}: {reason}")
            }
            Self::Opaque(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for DbCapabilityError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Opaque(error) => Some(error.as_ref()),
            Self::Decode(_) | Self::ProviderUnavailable { .. } => None,
        }
    }
}

impl WirePayload for DbCapabilityError {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            Self::Decode(message) => RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::ProviderUnavailable { target, reason } => RuntimeErrorPayload {
                code: "std.service.ProviderUnavailableError".to_string(),
                message: reason.clone(),
                status: None,
                details: Some(json!({
                    "target": target,
                    "reason": reason,
                })),
            },
            Self::Opaque(error) => error.payload(),
        }
    }

    fn catch_projection(&self) -> Option<(TypeIdentity, Value)> {
        match self {
            Self::ProviderUnavailable { target, reason } => Some((
                TypeIdentity::builtin("std.service.ProviderUnavailableError"),
                json!({
                    "target": target,
                    "reason": reason,
                }),
            )),
            Self::Opaque(error) => error.catch_projection(),
            Self::Decode(_) => None,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub trait DbCapabilityContextApi: Send + Sync {
    fn require_store(
        &self,
        target: &str,
        unavailable_reason: &str,
    ) -> DbCapabilityResult<DbCapabilityStore>;
}

#[derive(Clone)]
pub struct DbCapabilityContext {
    inner: Option<Arc<dyn DbCapabilityContextApi>>,
}

impl DbCapabilityContext {
    pub fn new<T>(inner: T) -> Self
    where
        T: DbCapabilityContextApi + 'static,
    {
        Self {
            inner: Some(Arc::new(inner)),
        }
    }

    pub fn from_arc(inner: Arc<dyn DbCapabilityContextApi>) -> Self {
        Self { inner: Some(inner) }
    }

    pub fn unavailable() -> Self {
        Self { inner: None }
    }

    pub fn from_handle<T>(handle: T) -> Self
    where
        T: DbCapabilityContextApi + 'static,
    {
        Self::new(handle)
    }

    pub fn require_store(
        &self,
        target: &str,
        unavailable_reason: &str,
    ) -> DbCapabilityResult<DbCapabilityStore> {
        let Some(inner) = &self.inner else {
            return Err(DbCapabilityError::provider_unavailable(
                target,
                unavailable_reason,
            ));
        };
        inner.require_store(target, unavailable_reason)
    }
}

pub trait DbCapabilityFactory: Send + Sync {
    fn context_for_request(&self, owner: String, request_id: String) -> DbCapabilityContext;
}

#[derive(Clone)]
pub struct DbCapabilitySource {
    factory: Option<Arc<dyn DbCapabilityFactory>>,
}

impl DbCapabilitySource {
    pub fn new<T>(factory: Option<T>) -> Self
    where
        T: DbCapabilityFactory + 'static,
    {
        Self {
            factory: factory.map(|factory| Arc::new(factory) as Arc<dyn DbCapabilityFactory>),
        }
    }

    pub fn from_arc(factory: Option<Arc<dyn DbCapabilityFactory>>) -> Self {
        Self { factory }
    }

    pub fn unavailable() -> Self {
        Self { factory: None }
    }

    pub fn context_for_request(
        &self,
        owner: impl Into<String>,
        request_id: impl Into<String>,
    ) -> DbCapabilityContext {
        self.factory
            .as_ref()
            .map(|factory| factory.context_for_request(owner.into(), request_id.into()))
            .unwrap_or_else(DbCapabilityContext::unavailable)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn is_some(&self) -> bool {
        self.factory.is_some()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn is_none(&self) -> bool {
        self.factory.is_none()
    }
}

impl Default for DbCapabilitySource {
    fn default() -> Self {
        Self::unavailable()
    }
}

pub trait DbProviderFactory: Send + Sync {
    fn build(&self, input: DbProviderBuildInput) -> DbCapabilityResult<DbCapabilitySource>;
}

#[derive(Clone)]
pub struct DbProviderSource {
    factory: Option<Arc<dyn DbProviderFactory>>,
}

impl DbProviderSource {
    pub fn new<T>(factory: T) -> Self
    where
        T: DbProviderFactory + 'static,
    {
        Self {
            factory: Some(Arc::new(factory)),
        }
    }

    pub fn from_arc(factory: Option<Arc<dyn DbProviderFactory>>) -> Self {
        Self { factory }
    }

    pub fn unavailable() -> Self {
        Self { factory: None }
    }

    pub fn build(&self, input: DbProviderBuildInput) -> DbCapabilityResult<DbCapabilitySource> {
        let Some(factory) = &self.factory else {
            return Err(DbCapabilityError::provider_unavailable(
                input.service_id,
                "serviceDb provider is not configured for this runtime host",
            ));
        };
        factory.build(input)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn is_some(&self) -> bool {
        self.factory.is_some()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn is_none(&self) -> bool {
        self.factory.is_none()
    }
}

impl Default for DbProviderSource {
    fn default() -> Self {
        Self::unavailable()
    }
}

#[derive(Clone, Debug, Default)]
pub struct DbRecoverableRuntimeExpectedPlans {
    fields: HashMap<String, RuntimeRecoverableExpectedTypePlan>,
}

impl DbRecoverableRuntimeExpectedPlans {
    pub fn new(fields: HashMap<String, RuntimeRecoverableExpectedTypePlan>) -> Self {
        Self { fields }
    }

    pub fn insert_field(
        &mut self,
        field: impl Into<String>,
        expected: RuntimeRecoverableExpectedTypePlan,
    ) {
        self.fields.insert(field.into(), expected);
    }

    pub fn field(&self, field: &str) -> Option<&RuntimeRecoverableExpectedTypePlan> {
        let top = field.split('.').next().unwrap_or(field);
        self.fields.get(top)
    }

    pub fn fields(&self) -> &HashMap<String, RuntimeRecoverableExpectedTypePlan> {
        &self.fields
    }
}

#[derive(Clone)]
pub struct DbRecoverableRuntimeContext {
    pub behavior_hooks: Arc<dyn RecoverableBehaviorHooks + Send + Sync>,
    pub expected_plans: DbRecoverableRuntimeExpectedPlans,
    pub artifact_identity: String,
    pub build_id: String,
    pub boundary_context: RuntimeRecoverableBoundaryContext,
    pub retention_expires_at_epoch_millis: Option<i64>,
}

impl fmt::Debug for DbRecoverableRuntimeContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DbRecoverableRuntimeContext")
            .field("expected_plans", &self.expected_plans)
            .field("artifact_identity", &self.artifact_identity)
            .field("build_id", &self.build_id)
            .field("boundary_context", &self.boundary_context)
            .field(
                "retention_expires_at_epoch_millis",
                &self.retention_expires_at_epoch_millis,
            )
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct DbRuntimeSetOp {
    pub field: String,
    pub value: RuntimeValue,
}

#[derive(Clone, Debug, Default)]
pub struct DbRuntimeChange {
    pub wire_change: ServiceDbChange,
    pub set_ops: Vec<DbRuntimeSetOp>,
}

impl DbRuntimeChange {
    pub fn is_empty(&self) -> bool {
        self.wire_change.is_empty() && self.set_ops.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileCapabilityRecord {
    pub id: String,
    pub sha256: String,
    pub size: i64,
    pub content_type: Option<String>,
    pub purpose: Option<String>,
    pub blob_key: String,
    pub created_at: String,
}

pub trait DbCapabilityStoreApi: Send + Sync {
    fn begin_transaction(&self) -> DbCapabilityFuture<'_, ()>;
    fn commit_transaction(&self) -> DbCapabilityFuture<'_, ()>;
    fn abort_transaction(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    fn find_one_by_key<'a>(
        &'a self,
        type_name: &'a str,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>>;

    fn find_one_by_key_runtime<'a>(
        &'a self,
        type_name: &'a str,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
        heap: &'a mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>>;

    fn find_one_by_query<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
        order: Vec<DbOrderEntry>,
        projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>>;

    fn find_one_by_query_runtime<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
        order: Vec<DbOrderEntry>,
        projection: Option<Vec<FieldPath>>,
        heap: &'a mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>>;

    fn find_many_page<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
        options: ServiceDbFindOptions,
        projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, DbPageResult>;

    fn find_many_page_runtime<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
        options: ServiceDbFindOptions,
        projection: Option<Vec<FieldPath>>,
        heap: &'a mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Vec<RuntimeValue>>;

    fn create<'a>(
        &'a self,
        type_name: &'a str,
        value: DbDocument,
    ) -> DbCapabilityFuture<'a, DbDocument>;

    fn create_runtime<'a>(
        &'a self,
        type_name: &'a str,
        value: &'a RuntimeValue,
        heap: &'a RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, RuntimeValue>;

    fn insert_many_result<'a>(
        &'a self,
        type_name: &'a str,
        values: Vec<DbDocument>,
    ) -> DbCapabilityFuture<'a, DbWriteResult>;

    fn update_one<'a>(
        &'a self,
        type_name: &'a str,
        selector: DbOneSelector,
        change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>>;

    fn update_one_runtime<'a>(
        &'a self,
        type_name: &'a str,
        selector: DbOneSelector,
        change: DbRuntimeChange,
        heap: &'a mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>>;

    fn update_many<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
        change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult>;

    fn upsert_by_key<'a>(
        &'a self,
        type_name: &'a str,
        key: DbKey,
        insert: DbDocument,
        change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult>;

    fn replace_one<'a>(
        &'a self,
        type_name: &'a str,
        selector: DbOneSelector,
        value: DbDocument,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>>;

    fn replace_one_runtime<'a>(
        &'a self,
        type_name: &'a str,
        selector: DbOneSelector,
        value: &'a RuntimeValue,
        heap: &'a mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>>;

    fn delete_one<'a>(
        &'a self,
        type_name: &'a str,
        selector: DbOneSelector,
    ) -> DbCapabilityFuture<'a, bool>;

    fn delete_many<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
    ) -> DbCapabilityFuture<'a, DbWriteResult>;

    fn count<'a>(&'a self, type_name: &'a str, query: DbQuery) -> DbCapabilityFuture<'a, u64>;

    fn exists_by_key<'a>(&'a self, type_name: &'a str, key: DbKey) -> DbCapabilityFuture<'a, bool>;

    fn exists_by_query<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
    ) -> DbCapabilityFuture<'a, bool>;

    fn claim_lease<'a>(
        &'a self,
        type_name: &'a str,
        key: DbKey,
        slot: &'a str,
    ) -> DbCapabilityFuture<'a, Option<DbCapabilityLeaseHandle>>;

    fn renew_lease<'a>(&'a self, hold: &'a DbCapabilityLeaseHold) -> DbCapabilityFuture<'a, bool>;

    fn release_lease<'a>(&'a self, hold: &'a DbCapabilityLeaseHold) -> DbCapabilityFuture<'a, ()>;

    fn read_lease<'a>(
        &'a self,
        type_name: &'a str,
        key: DbKey,
        slot: &'a str,
    ) -> DbCapabilityFuture<'a, Option<Value>>;

    fn lease_lost(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;

    fn insert_skiff_file_record<'a>(
        &'a self,
        record: FileCapabilityRecord,
    ) -> DbCapabilityFuture<'a, ()>;

    fn find_skiff_file_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> DbCapabilityFuture<'a, Option<FileCapabilityRecord>>;

    fn delete_skiff_file_by_id<'a>(&'a self, id: &'a str) -> DbCapabilityFuture<'a, u64>;
}

#[derive(Clone)]
pub struct DbCapabilityStore {
    inner: Arc<dyn DbCapabilityStoreApi>,
}

impl DbCapabilityStore {
    pub fn new<T>(inner: T) -> Self
    where
        T: DbCapabilityStoreApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn from_arc(inner: Arc<dyn DbCapabilityStoreApi>) -> Self {
        Self { inner }
    }

    pub fn as_api(&self) -> &dyn DbCapabilityStoreApi {
        self.inner.as_ref()
    }

    pub async fn begin_transaction(&self) -> DbCapabilityResult<()> {
        self.inner.begin_transaction().await
    }

    pub async fn commit_transaction(&self) -> DbCapabilityResult<()> {
        self.inner.commit_transaction().await
    }

    pub async fn abort_transaction(&self) {
        self.inner.abort_transaction().await;
    }

    pub async fn find_one_by_key(
        &self,
        type_name: &str,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityResult<Option<DbDocument>> {
        self.inner.find_one_by_key(type_name, key, projection).await
    }

    pub async fn find_one_by_key_runtime(
        &self,
        type_name: &str,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<Option<RuntimeValue>> {
        self.inner
            .find_one_by_key_runtime(type_name, key, projection, heap, context)
            .await
    }

    pub async fn find_one_by_query(
        &self,
        type_name: &str,
        query: DbQuery,
        order: Vec<DbOrderEntry>,
        projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityResult<Option<DbDocument>> {
        self.inner
            .find_one_by_query(type_name, query, order, projection)
            .await
    }

    pub async fn find_one_by_query_runtime(
        &self,
        type_name: &str,
        query: DbQuery,
        order: Vec<DbOrderEntry>,
        projection: Option<Vec<FieldPath>>,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<Option<RuntimeValue>> {
        self.inner
            .find_one_by_query_runtime(type_name, query, order, projection, heap, context)
            .await
    }

    pub async fn find_many_page(
        &self,
        type_name: &str,
        query: DbQuery,
        options: ServiceDbFindOptions,
        projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityResult<DbPageResult> {
        self.inner
            .find_many_page(type_name, query, options, projection)
            .await
    }

    pub async fn find_many_page_runtime(
        &self,
        type_name: &str,
        query: DbQuery,
        options: ServiceDbFindOptions,
        projection: Option<Vec<FieldPath>>,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<Vec<RuntimeValue>> {
        self.inner
            .find_many_page_runtime(type_name, query, options, projection, heap, context)
            .await
    }

    pub async fn create(
        &self,
        type_name: &str,
        value: DbDocument,
    ) -> DbCapabilityResult<DbDocument> {
        self.inner.create(type_name, value).await
    }

    pub async fn create_runtime(
        &self,
        type_name: &str,
        value: &RuntimeValue,
        heap: &RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<RuntimeValue> {
        self.inner
            .create_runtime(type_name, value, heap, context)
            .await
    }

    pub async fn insert_many_result(
        &self,
        type_name: &str,
        values: Vec<DbDocument>,
    ) -> DbCapabilityResult<DbWriteResult> {
        self.inner.insert_many_result(type_name, values).await
    }

    pub async fn update_one(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        change: ServiceDbChange,
    ) -> DbCapabilityResult<Option<DbDocument>> {
        self.inner.update_one(type_name, selector, change).await
    }

    pub async fn update_one_runtime(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        change: DbRuntimeChange,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<Option<RuntimeValue>> {
        self.inner
            .update_one_runtime(type_name, selector, change, heap, context)
            .await
    }

    pub async fn update_many(
        &self,
        type_name: &str,
        query: DbQuery,
        change: ServiceDbChange,
    ) -> DbCapabilityResult<DbWriteResult> {
        self.inner.update_many(type_name, query, change).await
    }

    pub async fn upsert_by_key(
        &self,
        type_name: &str,
        key: DbKey,
        insert: DbDocument,
        change: ServiceDbChange,
    ) -> DbCapabilityResult<DbWriteResult> {
        self.inner
            .upsert_by_key(type_name, key, insert, change)
            .await
    }

    pub async fn replace_one(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        value: DbDocument,
    ) -> DbCapabilityResult<Option<DbDocument>> {
        self.inner.replace_one(type_name, selector, value).await
    }

    pub async fn replace_one_runtime(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<Option<RuntimeValue>> {
        self.inner
            .replace_one_runtime(type_name, selector, value, heap, context)
            .await
    }

    pub async fn delete_one(
        &self,
        type_name: &str,
        selector: DbOneSelector,
    ) -> DbCapabilityResult<bool> {
        self.inner.delete_one(type_name, selector).await
    }

    pub async fn delete_many(
        &self,
        type_name: &str,
        query: DbQuery,
    ) -> DbCapabilityResult<DbWriteResult> {
        self.inner.delete_many(type_name, query).await
    }

    pub async fn count(&self, type_name: &str, query: DbQuery) -> DbCapabilityResult<u64> {
        self.inner.count(type_name, query).await
    }

    pub async fn exists_by_key(&self, type_name: &str, key: DbKey) -> DbCapabilityResult<bool> {
        self.inner.exists_by_key(type_name, key).await
    }

    pub async fn exists_by_query(
        &self,
        type_name: &str,
        query: DbQuery,
    ) -> DbCapabilityResult<bool> {
        self.inner.exists_by_query(type_name, query).await
    }

    pub async fn claim_lease(
        &self,
        type_name: &str,
        key: DbKey,
        slot: &str,
    ) -> DbCapabilityResult<Option<DbCapabilityLeaseHandle>> {
        self.inner.claim_lease(type_name, key, slot).await
    }

    pub async fn renew_lease(&self, hold: &DbCapabilityLeaseHold) -> DbCapabilityResult<bool> {
        self.inner.renew_lease(hold).await
    }

    pub async fn release_lease(&self, hold: &DbCapabilityLeaseHold) -> DbCapabilityResult<()> {
        self.inner.release_lease(hold).await
    }

    pub async fn read_lease(
        &self,
        type_name: &str,
        key: DbKey,
        slot: &str,
    ) -> DbCapabilityResult<Option<Value>> {
        self.inner.read_lease(type_name, key, slot).await
    }

    pub async fn lease_lost(&self) -> bool {
        self.inner.lease_lost().await
    }

    pub async fn insert_skiff_file_record(
        &self,
        record: FileCapabilityRecord,
    ) -> DbCapabilityResult<()> {
        self.inner.insert_skiff_file_record(record).await
    }

    pub async fn find_skiff_file_by_id(
        &self,
        id: &str,
    ) -> DbCapabilityResult<Option<FileCapabilityRecord>> {
        self.inner.find_skiff_file_by_id(id).await
    }

    pub async fn delete_skiff_file_by_id(&self, id: &str) -> DbCapabilityResult<u64> {
        self.inner.delete_skiff_file_by_id(id).await
    }
}

pub trait DbCapabilityLeaseHoldHandle: fmt::Debug + Send + Sync {
    fn as_any(&self) -> &dyn Any;

    fn eq_handle(&self, other: &dyn DbCapabilityLeaseHoldHandle) -> bool;
}

#[derive(Clone)]
pub struct DbCapabilityLeaseHold {
    hold: Arc<dyn DbCapabilityLeaseHoldHandle>,
}

impl DbCapabilityLeaseHold {
    pub fn new(hold: Arc<dyn DbCapabilityLeaseHoldHandle>) -> Self {
        Self { hold }
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.hold.as_any().downcast_ref()
    }
}

impl fmt::Debug for DbCapabilityLeaseHold {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.hold.fmt(formatter)
    }
}

impl PartialEq for DbCapabilityLeaseHold {
    fn eq(&self, other: &Self) -> bool {
        self.hold.eq_handle(other.hold.as_ref())
    }
}

#[derive(Clone, Debug)]
pub struct DbCapabilityLeaseHandle {
    pub hold: DbCapabilityLeaseHold,
    pub value: DbDocument,
    pub ttl_ms: u64,
}

impl DbCapabilityLeaseHandle {
    pub fn new(hold: DbCapabilityLeaseHold, value: DbDocument, ttl_ms: u64) -> Self {
        Self {
            hold,
            value,
            ttl_ms,
        }
    }
}
