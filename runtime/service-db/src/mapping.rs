use std::collections::HashMap;

use mongodb::bson::{self, spec::BinarySubtype, Binary, Bson, DateTime, Document};
use serde_json::{Map, Value};
use skiff_runtime_capability_context::{
    DbDocument, DbKey, DbOneSelector, DbOrderDirection, DbOrderEntry, DbQuery, FieldPath,
    ServiceDbChange, ServiceDbChangeOp, ServiceDbFindOptions,
};
use skiff_runtime_model::{
    recoverable::{
        RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableExpectedTypeNode, RuntimeRecoverableExpectedTypePlan,
        RuntimeRecoverableStorageLane, RuntimeRecoverableTrustBoundary,
    },
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeObject, RuntimeObjectFields, RuntimeValue},
};

use crate::{DbEncryptedFieldContext, DbRuntimeChange, Result, ServiceDbError};
use skiff_artifact_model::DbFieldStorageIr;
use skiff_runtime_boundary::{
    date_value,
    json::{decode_untyped_wire_json, encode_untyped_wire_json},
    persistent,
    recoverable::{
        RecoverableArtifactRetentionRootStore, RecoverableArtifactStore, RecoverableBehaviorHooks,
        RecoverableBoundaryCodec, RecoverableDecodePolicy,
    },
};

use super::metadata::{DbCollectionMetadata, DbIndexMetadata};
use skiff_runtime_boundary::db as db_boundary;

const DB_DECODE_TARGET: &str = "std.db";

#[derive(Clone)]
pub struct EncryptedRecordContext {
    record_id: String,
}

pub struct NormalizedOneSelector {
    pub filter: Document,
    pub sort: Option<Document>,
    normalized_key: Option<Bson>,
    encrypted_context: Option<EncryptedRecordContext>,
}

impl NormalizedOneSelector {
    pub fn normalized_key(&self) -> Option<&Bson> {
        self.normalized_key.as_ref()
    }

    pub fn encrypted_context(&self) -> Option<&EncryptedRecordContext> {
        self.encrypted_context.as_ref()
    }
}

pub struct DbRecoverableRuntimeWriteContext<'a> {
    pub behavior_hooks: &'a dyn RecoverableBehaviorHooks,
    pub boundary_context: Option<&'a RuntimeRecoverableBoundaryContext>,
    pub recoverable_expected_override: Option<&'a RuntimeRecoverableExpectedTypePlan>,
    pub recoverable_expected_overrides:
        Option<&'a HashMap<String, RuntimeRecoverableExpectedTypePlan>>,
    pub artifact_store: Option<&'a dyn RecoverableArtifactStore>,
    pub retention_root_store: Option<&'a mut dyn RecoverableArtifactRetentionRootStore>,
    pub retention_expires_at_epoch_millis: Option<i64>,
}

pub struct DbRecoverableRuntimeReadContext<'a> {
    pub behavior_hooks: &'a dyn RecoverableBehaviorHooks,
    pub boundary_context: Option<&'a RuntimeRecoverableBoundaryContext>,
    pub recoverable_expected_override: Option<&'a RuntimeRecoverableExpectedTypePlan>,
    pub recoverable_expected_overrides:
        Option<&'a HashMap<String, RuntimeRecoverableExpectedTypePlan>>,
}

fn db_decode_error(message: impl Into<String>) -> ServiceDbError {
    ServiceDbError::db_decode(DB_DECODE_TARGET, message)
}

fn db_path_policy_error(error: db_boundary::DbFieldPathPolicyError) -> ServiceDbError {
    db_decode_error(error.to_string())
}

impl DbCollectionMetadata {
    pub fn document_from_runtime_business_value(
        &self,
        value: &RuntimeValue,
        heap: &RequestHeap,
        recoverable_context: Option<&mut DbRecoverableRuntimeWriteContext<'_>>,
    ) -> Result<Document> {
        self.runtime_document_from_business_value(value, heap, recoverable_context, None, None)
    }

    pub fn replacement_document_from_runtime_business_value(
        &self,
        value: &RuntimeValue,
        heap: &RequestHeap,
        recoverable_context: Option<&mut DbRecoverableRuntimeWriteContext<'_>>,
        expected_key: Option<&Bson>,
        encrypted_context: Option<&EncryptedRecordContext>,
    ) -> Result<Document> {
        self.runtime_document_from_business_value(
            value,
            heap,
            recoverable_context,
            expected_key,
            encrypted_context,
        )
    }

    fn runtime_document_from_business_value(
        &self,
        value: &RuntimeValue,
        heap: &RequestHeap,
        mut recoverable_context: Option<&mut DbRecoverableRuntimeWriteContext<'_>>,
        expected_key: Option<&Bson>,
        encrypted_context: Option<&EncryptedRecordContext>,
    ) -> Result<Document> {
        let fields = runtime_object_fields(value, heap, "db write value")?;
        let body_key = fields.get(&self.key_field);
        let key_bson = match body_key {
            Some(key) => self.bson_from_runtime_business_value(
                key,
                heap,
                self.key_write_projection_plan(),
                recoverable_context.as_deref_mut(),
                Some(&self.key_field),
            )?,
            None => {
                let expected_key = expected_key.ok_or_else(|| {
                    db_decode_error(format!("db value missing key field {}", self.key_field))
                })?;
                expected_key.clone()
            }
        };
        if body_key.is_some() {
            if let Some(expected_key) = expected_key {
                if &key_bson != expected_key {
                    return Err(db_decode_error(format!(
                        "db replace value key field {} must match selected object",
                        self.key_field
                    )));
                }
            }
        }
        let mut document = Document::new();
        let encrypted_context = match encrypted_context {
            Some(context) => Some(context.clone()),
            None => self.encrypted_context_from_key_bson(&key_bson)?,
        };
        document.insert("_id", key_bson);
        for (field, value) in fields {
            if field == &self.key_field {
                continue;
            }
            validate_db_business_field_name(field)?;
            let plaintext = self.bson_from_runtime_business_value(
                value,
                heap,
                self.field_write_projection_plan_for_path(field),
                recoverable_context.as_deref_mut(),
                Some(field),
            )?;
            document.insert(
                field.clone(),
                self.encode_field_storage(field, encrypted_context.as_ref(), plaintext)?,
            );
        }
        Ok(document)
    }

    pub fn runtime_business_value_from_document(
        &self,
        document: Document,
        heap: &mut RequestHeap,
        recoverable_context: Option<&DbRecoverableRuntimeReadContext<'_>>,
    ) -> Result<RuntimeValue> {
        let checkpoint = heap.checkpoint();
        match self.runtime_business_value_from_document_inner(document, heap, recoverable_context) {
            Ok(value) => Ok(value),
            Err(error) => {
                heap.rollback_to_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    pub fn document_from_business_value(
        &self,
        value: DbDocument,
    ) -> Result<(Document, DbDocument)> {
        let value = value.into_value();
        let object = value
            .as_object()
            .cloned()
            .ok_or_else(|| db_decode_error("db.create value must be an object".to_string()))?;
        validate_db_business_json_object(&object)?;
        let key = object.get(&self.key_field).cloned().ok_or_else(|| {
            db_decode_error(format!("db value missing key field {}", self.key_field))
        })?;
        let mut stored = object.clone();
        stored.remove(&self.key_field);
        stored.insert("_id".to_string(), key.clone());
        let document = self.document_from_stored_object(stored)?;
        let materialized = self.business_value_from_document(document.clone())?;
        Ok((document, materialized))
    }

    pub fn upsert_insert_value_with_key(
        &self,
        value: DbDocument,
        key: &DbKey,
    ) -> Result<DbDocument> {
        let value = value.into_value();
        let mut object = value.as_object().cloned().ok_or_else(|| {
            db_decode_error("db upsert insert value must be an object".to_string())
        })?;
        match object.get(&self.key_field) {
            Some(body_key) if body_key != key.as_value() => {
                return Err(db_decode_error(format!(
                    "db upsert insert value key field {} must match selector key",
                    self.key_field
                )));
            }
            Some(_) => {}
            None => {
                object.insert(self.key_field.clone(), key.as_value().clone());
            }
        }
        Ok(DbDocument::new(Value::Object(object)))
    }

    pub fn replacement_document_from_business_value(
        &self,
        value: DbDocument,
        expected_key: Option<&Bson>,
        encrypted_context: Option<&EncryptedRecordContext>,
    ) -> Result<Document> {
        let value = value.into_value();
        let object = value
            .as_object()
            .cloned()
            .ok_or_else(|| db_decode_error("db replace value must be an object".to_string()))?;
        validate_db_business_json_object(&object)?;
        let mut object = object;
        object.remove("_id");
        let body_key = object.remove(&self.key_field);
        let key_bson = match body_key {
            Some(body_key) => {
                let body_key =
                    self.bson_from_business_value(&body_key, self.key_write_projection_plan())?;
                if expected_key.is_some_and(|expected_key| expected_key != &body_key) {
                    return Err(db_decode_error(format!(
                        "db replace value key field {} must match selected object",
                        self.key_field
                    )));
                }
                body_key
            }
            None => expected_key.cloned().ok_or_else(|| {
                db_decode_error(format!("db value missing key field {}", self.key_field))
            })?,
        };
        let encrypted_context = match encrypted_context {
            Some(context) => Some(context.clone()),
            None => self.encrypted_context_from_key_bson(&key_bson)?,
        };
        self.document_from_business_object_with_key(object, key_bson, encrypted_context.as_ref())
    }

    pub fn documents_from_business_values(
        &self,
        values: Vec<DbDocument>,
    ) -> Result<(Vec<Document>, Vec<DbDocument>)> {
        let mut documents = Vec::with_capacity(values.len());
        let mut materialized = Vec::with_capacity(values.len());
        for value in values {
            let (document, business_value) = self.document_from_business_value(value)?;
            documents.push(document);
            materialized.push(business_value);
        }
        Ok((documents, materialized))
    }

    pub fn business_value_from_document(&self, mut document: Document) -> Result<DbDocument> {
        let key = document.remove("_id").ok_or_else(|| {
            db_decode_error(format!(
                "db document in {} missing _id",
                self.collection_name
            ))
        })?;
        let encrypted_context = self.encrypted_context_from_key_bson(&key)?;
        let mut object =
            self.business_object_from_stored_document(document, encrypted_context.as_ref())?;
        object.insert(
            self.key_field.clone(),
            self.business_value_from_bson(key, self.key_result_decode_plan())?,
        );
        Ok(DbDocument::new(Value::Object(object)))
    }

    pub fn key_filter(&self, key: &DbKey) -> Result<Document> {
        let key =
            self.bson_from_business_value(key.as_value(), self.key_write_projection_plan())?;
        Ok(mongodb::bson::doc! { "_id": key })
    }

    pub fn key_from_document(&self, document: &Document) -> Result<DbKey> {
        document
            .get("_id")
            .cloned()
            .map(|value| self.business_value_from_bson(value, self.key_result_decode_plan()))
            .transpose()?
            .map(DbKey::new)
            .ok_or_else(|| db_decode_error("db document missing _id".to_string()))
    }

    pub fn query_filter(&self, query: DbQuery) -> Result<Document> {
        match query.into_value() {
            Value::Null => Ok(Document::new()),
            Value::Object(object) if object.is_empty() => Ok(Document::new()),
            Value::Object(object) => self.query_document(object),
            _ => Err(db_decode_error(
                "db.findMany query must be an object or null".to_string(),
            )),
        }
    }

    pub fn selector_filter_sort(
        &self,
        selector: DbOneSelector,
    ) -> Result<(Document, Option<Document>)> {
        let normalized = self.normalize_one_selector(selector)?;
        Ok((normalized.filter, normalized.sort))
    }

    pub fn normalize_one_selector(&self, selector: DbOneSelector) -> Result<NormalizedOneSelector> {
        match selector {
            DbOneSelector::Key(key) => {
                let key = self
                    .bson_from_business_value(key.as_value(), self.key_write_projection_plan())?;
                let encrypted_context = self.encrypted_context_from_key_bson(&key)?;
                Ok(NormalizedOneSelector {
                    filter: mongodb::bson::doc! { "_id": key.clone() },
                    sort: None,
                    normalized_key: Some(key),
                    encrypted_context,
                })
            }
            DbOneSelector::Query { query, order } => Ok(NormalizedOneSelector {
                filter: self.query_filter(query)?,
                sort: self.order_document(&order)?,
                normalized_key: None,
                encrypted_context: None,
            }),
        }
    }

    pub fn order_document(&self, order: &[DbOrderEntry]) -> Result<Option<Document>> {
        if order.is_empty() {
            return Ok(None);
        }
        let mut sort = Document::new();
        for entry in order {
            let field = self.field_path_to_mongo_name(&entry.field, DbFieldUse::Order)?;
            let direction = match entry.direction {
                DbOrderDirection::Asc => 1,
                DbOrderDirection::Desc => -1,
            };
            sort.insert(field, Bson::Int32(direction));
        }
        Ok(Some(sort))
    }

    pub fn page_sort_document(&self, options: &ServiceDbFindOptions) -> Result<Option<Document>> {
        self.order_document(&options.order)
    }

    pub fn projection_document(
        &self,
        projection: Option<&[FieldPath]>,
    ) -> Result<Option<Document>> {
        let Some(fields) = projection else {
            return Ok(None);
        };
        let mut doc = Document::new();
        doc.insert("_id", Bson::Int32(1));
        for field in fields {
            doc.insert(
                self.field_path_to_mongo_name(field, DbFieldUse::Projection)?,
                Bson::Int32(1),
            );
        }
        Ok(Some(doc))
    }

    pub(crate) fn validate_indexes(&self) -> Result<()> {
        for index in &self.indexes {
            for field in &index.fields {
                self.field_path_to_mongo_name(&field.field, DbFieldUse::Index)?;
            }
        }
        Ok(())
    }

    pub(crate) fn index_key_document(&self, index: &DbIndexMetadata) -> Result<Document> {
        if index.fields.is_empty() {
            return Err(ServiceDbError::InvalidDbMetadata(format!(
                "runtime program db metadata index {} must declare at least one field",
                index.name
            )));
        }
        let mut keys = Document::new();
        for field in &index.fields {
            let mongo_name = self.field_path_to_mongo_name(&field.field, DbFieldUse::Index)?;
            if keys.contains_key(&mongo_name) {
                return Err(ServiceDbError::InvalidDbMetadata(format!(
                    "runtime program db metadata index {} repeats physical field {mongo_name}",
                    index.name
                )));
            }
            let direction = match field.direction {
                DbOrderDirection::Asc => 1,
                DbOrderDirection::Desc => -1,
            };
            keys.insert(mongo_name, direction);
        }
        Ok(keys)
    }

    pub fn validated_change_update(
        &self,
        type_name: &str,
        change: ServiceDbChange,
    ) -> Result<Document> {
        self.validated_change_update_with_context(type_name, change, None)
    }

    pub fn validated_change_update_with_context(
        &self,
        type_name: &str,
        change: ServiceDbChange,
        encrypted_context: Option<&EncryptedRecordContext>,
    ) -> Result<Document> {
        let change = self.validated_change(type_name, change)?;
        self.change_update_document(&change, encrypted_context)
    }

    pub fn validated_change(
        &self,
        type_name: &str,
        change: ServiceDbChange,
    ) -> Result<ServiceDbChange> {
        let _ = type_name;
        for field in change.touched_fields() {
            self.validate_mutable_field(field)?;
        }
        Ok(change)
    }

    pub fn change_update_document(
        &self,
        change: &ServiceDbChange,
        encrypted_context: Option<&EncryptedRecordContext>,
    ) -> Result<Document> {
        let mut set = Document::new();
        let mut inc = Document::new();
        let mut unset = Document::new();
        let mut add_to_set = Document::new();
        let mut pull = Document::new();

        for op in change.ops() {
            match op {
                ServiceDbChangeOp::Set { field, value } => {
                    let resolved = self.resolve_business_field_path(field)?;
                    self.validate_storage_field_use(
                        &resolved,
                        DbFieldUse::WholeSet,
                        encrypted_context,
                    )?;
                    let plaintext = self.bson_from_business_value(
                        value.as_value(),
                        self.field_write_projection_plan_for_path(field),
                    )?;
                    set.insert(
                        field.clone(),
                        self.encode_field_storage(
                            resolved.top_level(),
                            encrypted_context,
                            plaintext,
                        )?,
                    );
                }
                ServiceDbChangeOp::Inc { field, value } => {
                    let resolved = self.resolve_business_field_path(field)?;
                    self.validate_storage_field_use(&resolved, DbFieldUse::PartialChange, None)?;
                    inc.insert(field.clone(), bson::to_bson(value.as_value())?);
                }
                ServiceDbChangeOp::Unset { field } => {
                    let resolved = self.resolve_business_field_path(field)?;
                    self.validate_storage_field_use(&resolved, DbFieldUse::PartialChange, None)?;
                    unset.insert(field.clone(), Bson::Int32(1));
                }
                ServiceDbChangeOp::AddToSet { field, value } => {
                    let resolved = self.resolve_business_field_path(field)?;
                    self.validate_storage_field_use(&resolved, DbFieldUse::PartialChange, None)?;
                    add_to_set.insert(
                        field.clone(),
                        self.bson_from_business_value(
                            value.as_value(),
                            self.collection_item_plan_for_path(field),
                        )?,
                    );
                }
                ServiceDbChangeOp::Pull { field, value } => {
                    let resolved = self.resolve_business_field_path(field)?;
                    self.validate_storage_field_use(&resolved, DbFieldUse::PartialChange, None)?;
                    pull.insert(
                        field.clone(),
                        self.bson_from_business_value(
                            value.as_value(),
                            self.collection_item_plan_for_path(field),
                        )?,
                    );
                }
            }
        }

        let mut update = Document::new();
        if !set.is_empty() {
            update.insert("$set", Bson::Document(set));
        }
        if !inc.is_empty() {
            update.insert("$inc", Bson::Document(inc));
        }
        if !unset.is_empty() {
            update.insert("$unset", Bson::Document(unset));
        }
        if !add_to_set.is_empty() {
            update.insert("$addToSet", Bson::Document(add_to_set));
        }
        if !pull.is_empty() {
            update.insert("$pull", Bson::Document(pull));
        }
        Ok(update)
    }

    pub fn runtime_change_update_document(
        &self,
        type_name: &str,
        change: DbRuntimeChange,
        heap: &RequestHeap,
        mut recoverable_context: Option<&mut DbRecoverableRuntimeWriteContext<'_>>,
        encrypted_context: Option<&EncryptedRecordContext>,
    ) -> Result<Document> {
        let mut update = self.validated_change_update_with_context(
            type_name,
            change.wire_change,
            encrypted_context,
        )?;
        if change.set_ops.is_empty() {
            return Ok(update);
        }

        let mut set = match update.remove("$set") {
            Some(Bson::Document(set)) => set,
            Some(other) => {
                return Err(db_decode_error(format!(
                    "db runtime change generated invalid $set document {}",
                    bson_kind(&other)
                )));
            }
            None => Document::new(),
        };

        for op in change.set_ops {
            let resolved = self.resolve_business_field_path(&op.field)?;
            self.validate_storage_field_use(&resolved, DbFieldUse::WholeSet, encrypted_context)?;
            let plaintext = self.bson_from_runtime_business_value(
                &op.value,
                heap,
                self.field_write_projection_plan_for_path(&op.field),
                recoverable_context.as_deref_mut(),
                Some(&op.field),
            )?;
            set.insert(
                op.field.clone(),
                self.encode_field_storage(resolved.top_level(), encrypted_context, plaintext)?,
            );
        }
        if !set.is_empty() {
            update.insert("$set", Bson::Document(set));
        }
        Ok(update)
    }

    fn validate_mutable_field(&self, field: &str) -> Result<()> {
        self.field_path_policy()
            .resolve_mutable_business_field_path(field, &self.type_name, |top| {
                self.fields.contains_key(top)
            })
            .map(|_| ())
            .map_err(db_path_policy_error)
    }

    fn resolve_business_field_path<'a>(
        &self,
        field: &'a str,
    ) -> Result<db_boundary::DbResolvedFieldPath<'a>> {
        self.field_path_policy()
            .resolve_business_field_path(field, &self.type_name, |top| {
                self.fields.contains_key(top)
            })
            .map_err(db_path_policy_error)
    }

    fn field_path_to_mongo_name(&self, field: &FieldPath, use_case: DbFieldUse) -> Result<String> {
        let text = db_boundary::normalize_db_field_path_text(
            &field.text,
            field.segments.iter().map(String::as_str),
        );
        let resolved = self
            .field_path_policy()
            .resolve_mongo_facing_field_path(&text, &self.type_name, |top| {
                self.fields.contains_key(top)
            })
            .map_err(db_path_policy_error)?;
        self.validate_storage_field_use(&resolved, use_case, None)?;
        Ok(resolved.mongo_path().to_string())
    }

    fn query_document(&self, query: Map<String, Value>) -> Result<Document> {
        let mut filter = Document::new();
        for (field, value) in query {
            match field.as_str() {
                "$and" | "$or" => {
                    filter.insert(field, self.logical_filter_array(&value)?);
                }
                "$nor" => {
                    filter.insert(field, self.logical_filter_array(&value)?);
                }
                other if other.starts_with('$') => {
                    return Err(db_decode_error(format!(
                        "db query top-level operator {other} is not supported"
                    )));
                }
                _ => {
                    let resolved = self.resolve_business_field_path(&field)?;
                    self.validate_storage_field_use(&resolved, DbFieldUse::Predicate, None)?;
                    filter.insert(
                        resolved.mongo_path().to_string(),
                        self.field_query_value(resolved.business_path(), value)?,
                    );
                }
            }
        }
        Ok(filter)
    }

    fn logical_filter_array(&self, value: &Value) -> Result<Bson> {
        let items = value.as_array().ok_or_else(|| {
            db_decode_error("db query $and/$or value must be an array".to_string())
        })?;
        if items.is_empty() {
            return Err(db_decode_error(
                "db query $and/$or value must be a non-empty array".to_string(),
            ));
        }
        let mut filters = Vec::with_capacity(items.len());
        for item in items {
            let object = item.as_object().cloned().ok_or_else(|| {
                db_decode_error("db query $and/$or items must be objects".to_string())
            })?;
            filters.push(Bson::Document(self.query_document(object)?));
        }
        Ok(Bson::Array(filters))
    }

    fn field_query_value(&self, field: &str, value: Value) -> Result<Bson> {
        let plan = self.field_write_projection_plan_for_path(field);
        let Value::Object(object) = value else {
            return self.bson_from_business_value(&value, plan);
        };

        if !object.keys().any(|field| field.starts_with('$')) {
            return self.bson_from_business_value(&Value::Object(object), plan);
        }
        if object.keys().any(|field| !field.starts_with('$')) {
            return Err(db_decode_error(
                "db query field object cannot mix operators and exact-match fields".to_string(),
            ));
        }

        let mut operators = Document::new();
        for (operator, value) in object {
            validate_field_operator(&operator, &value)?;
            operators.insert(
                operator.clone(),
                self.bson_query_operator_value(&operator, value, plan)?,
            );
        }
        Ok(Bson::Document(operators))
    }

    fn document_from_stored_object(&self, object: Map<String, Value>) -> Result<Document> {
        let key_value = object.get(db_boundary::MONGO_ID_FIELD).ok_or_else(|| {
            db_decode_error(format!("db value missing key field {}", self.key_field))
        })?;
        let key_bson =
            self.bson_from_business_value(key_value, self.key_write_projection_plan())?;
        let encrypted_context = self.encrypted_context_from_key_bson(&key_bson)?;
        self.document_from_business_object_with_key(object, key_bson, encrypted_context.as_ref())
    }

    fn document_from_business_object_with_key(
        &self,
        object: Map<String, Value>,
        key_bson: Bson,
        encrypted_context: Option<&EncryptedRecordContext>,
    ) -> Result<Document> {
        let mut document = Document::new();
        document.insert(db_boundary::MONGO_ID_FIELD, key_bson);
        for (field, value) in object {
            if field == db_boundary::MONGO_ID_FIELD {
                continue;
            }
            validate_db_business_field_name(&field)?;
            let plaintext = self.bson_from_business_value(
                &value,
                self.field_write_projection_plan_for_path(&field),
            )?;
            document.insert(
                field.clone(),
                self.encode_field_storage(&field, encrypted_context, plaintext)?,
            );
        }
        Ok(document)
    }

    fn business_object_from_stored_document(
        &self,
        document: Document,
        encrypted_context: Option<&EncryptedRecordContext>,
    ) -> Result<Map<String, Value>> {
        let mut object = Map::new();
        for (field, value) in document {
            if db_boundary::is_reserved_db_business_metadata_name(&field) {
                continue;
            }
            validate_db_business_field_name(&field)?;
            let plaintext = self.decode_field_storage(&field, encrypted_context, value)?;
            object.insert(
                field.clone(),
                self.business_value_from_bson(
                    plaintext,
                    self.field_result_decode_plan_for_path(&field),
                )?,
            );
        }
        Ok(object)
    }

    fn runtime_business_value_from_document_inner(
        &self,
        mut document: Document,
        heap: &mut RequestHeap,
        recoverable_context: Option<&DbRecoverableRuntimeReadContext<'_>>,
    ) -> Result<RuntimeValue> {
        let key = document.remove("_id").ok_or_else(|| {
            db_decode_error(format!(
                "db document in {} missing _id",
                self.collection_name
            ))
        })?;
        let encrypted_context = self.encrypted_context_from_key_bson(&key)?;
        let mut fields = RuntimeObjectFields::new();
        fields.insert(
            self.key_field.clone(),
            self.runtime_value_from_bson(
                key,
                self.key_result_decode_plan(),
                heap,
                recoverable_context,
                Some(&self.key_field),
            )?,
        );
        for (field, value) in document {
            if db_boundary::is_reserved_db_business_metadata_name(&field) {
                continue;
            }
            validate_db_business_field_name(&field)?;
            let plaintext = self.decode_field_storage(&field, encrypted_context.as_ref(), value)?;
            fields.insert(
                field.clone(),
                self.runtime_value_from_bson(
                    plaintext,
                    self.field_result_decode_plan_for_path(&field),
                    heap,
                    recoverable_context,
                    Some(&field),
                )?,
            );
        }
        Ok(RuntimeValue::Heap(
            heap.alloc_object(RuntimeObject::unshaped(fields))?,
        ))
    }

    fn bson_query_operator_value(
        &self,
        operator: &str,
        value: Value,
        plan: Option<db_boundary::DbBoundaryValuePlanRef<'_>>,
    ) -> Result<Bson> {
        match operator {
            "$eq" | "$ne" | "$gt" | "$gte" | "$lt" | "$lte" => {
                self.bson_from_business_value(&value, plan)
            }
            "$in" | "$nin" => {
                let items = value.as_array().ok_or_else(|| {
                    db_decode_error(format!(
                        "db query operator {operator} requires an array value"
                    ))
                })?;
                items
                    .iter()
                    .map(|item| self.bson_from_business_value(item, plan))
                    .collect::<Result<Vec<_>>>()
                    .map(Bson::Array)
            }
            _ => bson::to_bson(&value).map_err(ServiceDbError::from),
        }
    }

    fn bson_from_business_value(
        &self,
        value: &Value,
        plan: Option<db_boundary::DbBoundaryValuePlanRef<'_>>,
    ) -> Result<Bson> {
        let bson = bson::to_bson(value)?;
        self.coerce_bson_value_for_type(bson, plan)
    }

    fn bson_from_runtime_business_value(
        &self,
        value: &RuntimeValue,
        heap: &RequestHeap,
        plan: Option<db_boundary::DbBoundaryValuePlanRef<'_>>,
        recoverable_context: Option<&mut DbRecoverableRuntimeWriteContext<'_>>,
        field_path: Option<&str>,
    ) -> Result<Bson> {
        let fallback_expected = RuntimeRecoverableExpectedTypePlan::unresolved("DB value");
        let expected = plan
            .map(db_boundary::DbBoundaryValuePlanRef::recoverable_expected)
            .unwrap_or(&fallback_expected);
        persistent::reject_callback_capability_graph(
            value,
            heap,
            &recoverable_db_context(),
            expected,
        )
        .map_err(|error| {
            db_decode_error(format!(
                "DB value rejects request-scoped callback capability: {error}"
            ))
        })?;
        if let Some(plan) = plan {
            if matches!(
                db_boundary::db_value_projection(plan),
                db_boundary::DbValueProjection::RecoverableEnvelope
            ) {
                return self.recoverable_envelope_bson_from_runtime_value(
                    value,
                    plan,
                    heap,
                    recoverable_context,
                    field_path,
                );
            }
        }
        let json = encode_untyped_wire_json(value, heap).map_err(|error| {
            db_decode_error(format!(
                "schema-projectable DB value encode failed: {error}"
            ))
        })?;
        self.bson_from_business_value(&json, plan)
    }

    fn coerce_bson_value_for_type(
        &self,
        value: Bson,
        plan: Option<db_boundary::DbBoundaryValuePlanRef<'_>>,
    ) -> Result<Bson> {
        validate_db_business_bson_value(&value)?;
        let Some(plan) = plan else {
            return Ok(value);
        };
        if matches!(
            db_boundary::db_value_projection(plan),
            db_boundary::DbValueProjection::RecoverableEnvelope
        ) {
            return self.recoverable_envelope_bson_from_business_value(&value, plan);
        }
        if matches!(value, Bson::Null) {
            return Ok(value);
        }
        match db_boundary::db_value_projection(plan) {
            db_boundary::DbValueProjection::RecoverableEnvelope => unreachable!(
                "recoverable envelope DB fields are encoded before schema projection dispatch"
            ),
            db_boundary::DbValueProjection::Date => self.bson_date_value(value),
            db_boundary::DbValueProjection::Record(fields) => {
                let Bson::Document(document) = value else {
                    return Ok(value);
                };
                let mut output = Document::new();
                for (field, value) in document {
                    output.insert(
                        field.clone(),
                        self.coerce_bson_value_for_type(value, fields.field(&field))?,
                    );
                }
                Ok(Bson::Document(output))
            }
            db_boundary::DbValueProjection::Array(item_plan) => {
                let Bson::Array(items) = value else {
                    return Ok(value);
                };
                items
                    .into_iter()
                    .map(|item| self.coerce_bson_value_for_type(item, Some(item_plan)))
                    .collect::<Result<Vec<_>>>()
                    .map(Bson::Array)
            }
            db_boundary::DbValueProjection::Scalar => Ok(value),
        }
    }

    fn bson_date_value(&self, value: Bson) -> Result<Bson> {
        match value {
            Bson::DateTime(_) => Ok(value),
            Bson::String(value) => {
                let ms = date_value::parse_rfc3339_millis(&value, "Mongo Date field")?;
                Ok(Bson::DateTime(DateTime::from_millis(ms)))
            }
            other => Err(db_decode_error(format!(
                "Date db field requires RFC3339 string, got {}",
                bson_kind(&other)
            ))),
        }
    }

    fn business_value_from_bson(
        &self,
        value: Bson,
        plan: Option<db_boundary::DbBoundaryValuePlanRef<'_>>,
    ) -> Result<Value> {
        validate_db_business_bson_value(&value)?;
        let Some(plan) = plan else {
            return bson::from_bson(value).map_err(ServiceDbError::from);
        };
        if matches!(
            db_boundary::db_value_projection(plan),
            db_boundary::DbValueProjection::RecoverableEnvelope
        ) {
            return self.business_value_from_recoverable_envelope_bson(value, plan);
        }
        if matches!(value, Bson::Null) {
            return Ok(Value::Null);
        }
        match db_boundary::db_value_projection(plan) {
            db_boundary::DbValueProjection::RecoverableEnvelope => unreachable!(
                "recoverable envelope DB fields are decoded before schema projection dispatch"
            ),
            db_boundary::DbValueProjection::Date => self.business_date_value(value),
            db_boundary::DbValueProjection::Record(fields) => {
                let Bson::Document(document) = value else {
                    return bson::from_bson(value).map_err(ServiceDbError::from);
                };
                let mut output = Map::new();
                for (field, value) in document {
                    output.insert(
                        field.clone(),
                        self.business_value_from_bson(value, fields.field(&field))?,
                    );
                }
                Ok(Value::Object(output))
            }
            db_boundary::DbValueProjection::Array(item_plan) => {
                let Bson::Array(items) = value else {
                    return bson::from_bson(value).map_err(ServiceDbError::from);
                };
                items
                    .into_iter()
                    .map(|item| self.business_value_from_bson(item, Some(item_plan)))
                    .collect::<Result<Vec<_>>>()
                    .map(Value::Array)
            }
            db_boundary::DbValueProjection::Scalar => {
                bson::from_bson(value).map_err(ServiceDbError::from)
            }
        }
    }

    fn runtime_value_from_bson(
        &self,
        value: Bson,
        plan: Option<db_boundary::DbBoundaryValuePlanRef<'_>>,
        heap: &mut RequestHeap,
        recoverable_context: Option<&DbRecoverableRuntimeReadContext<'_>>,
        field_path: Option<&str>,
    ) -> Result<RuntimeValue> {
        validate_db_business_bson_value(&value)?;
        if let Some(plan) = plan {
            if matches!(
                db_boundary::db_value_projection(plan),
                db_boundary::DbValueProjection::RecoverableEnvelope
            ) {
                return self.runtime_value_from_recoverable_envelope_bson(
                    value,
                    plan,
                    heap,
                    recoverable_context,
                    field_path,
                );
            }
        }
        let json = self.business_value_from_bson(value, plan)?;
        decode_untyped_wire_json(&json, heap).map_err(|error| {
            db_decode_error(format!(
                "schema-projectable DB value decode failed: {error}"
            ))
        })
    }

    fn recoverable_envelope_bson_from_business_value(
        &self,
        value: &Bson,
        plan: db_boundary::DbBoundaryValuePlanRef<'_>,
    ) -> Result<Bson> {
        let json = bson::from_bson::<Value>(value.clone())?;
        let mut heap = RequestHeap::default();
        let runtime_value = decode_untyped_wire_json(&json, &mut heap).map_err(|error| {
            db_decode_error(format!(
                "recoverable-envelope DB value encode failed: {error}"
            ))
        })?;
        let bytes = RecoverableBoundaryCodec::encode(
            &runtime_value,
            plan.recoverable_expected(),
            &recoverable_db_context(),
            &heap,
        )
        .map_err(|error| {
            db_decode_error(format!(
                "recoverable-envelope DB value encode failed: {error}"
            ))
        })?;
        Ok(Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes,
        }))
    }

    fn recoverable_envelope_bson_from_runtime_value(
        &self,
        value: &RuntimeValue,
        plan: db_boundary::DbBoundaryValuePlanRef<'_>,
        heap: &RequestHeap,
        recoverable_context: Option<&mut DbRecoverableRuntimeWriteContext<'_>>,
        field_path: Option<&str>,
    ) -> Result<Bson> {
        let fallback_context = recoverable_db_context();
        let context = recoverable_context
            .as_ref()
            .and_then(|context| context.boundary_context)
            .unwrap_or(&fallback_context);
        let expected = recoverable_context
            .as_ref()
            .and_then(|context| {
                context.recoverable_expected_override.or_else(|| {
                    recoverable_expected_override_for_field(
                        context.recoverable_expected_overrides,
                        field_path,
                    )
                })
            })
            .unwrap_or_else(|| plan.recoverable_expected());
        let bytes = if let Some(recoverable_context) = recoverable_context {
            let envelope = RecoverableBoundaryCodec::encode_envelope_with_behavior(
                value,
                expected,
                context,
                heap,
                recoverable_context.behavior_hooks,
            )
            .map_err(|error| {
                db_decode_error(format!(
                    "recoverable-envelope DB value encode failed: {error}"
                ))
            })?;
            let refs = envelope.collect_artifact_refs();
            if !refs.is_empty() {
                let artifact_store = recoverable_context.artifact_store.ok_or_else(|| {
                    db_decode_error(
                        "recoverable-envelope DB value encode requires an artifact availability store for behavior values",
                    )
                })?;
                let verified_refs = RecoverableBoundaryCodec::verify_artifact_availability(
                    &envelope,
                    artifact_store,
                    expected,
                    context,
                )
                .map_err(|error| {
                    db_decode_error(format!(
                        "recoverable-envelope DB artifact availability check failed: {error}"
                    ))
                })?;
                let retention_root_store =
                    recoverable_context
                        .retention_root_store
                        .as_deref_mut()
                        .ok_or_else(|| {
                            db_decode_error(
                                "recoverable-envelope DB value encode requires an artifact retention root store for behavior values",
                            )
                        })?;
                RecoverableBoundaryCodec::persist_artifact_retention_roots(
                    &verified_refs,
                    retention_root_store,
                    expected,
                    context,
                    recoverable_context.retention_expires_at_epoch_millis,
                )
                .map_err(|error| {
                    db_decode_error(format!(
                        "recoverable-envelope DB artifact retention root write failed: {error}"
                    ))
                })?;
            }
            RecoverableBoundaryCodec::encode_envelope_canonical(
                &envelope,
                &Default::default(),
                expected,
                context,
            )
        } else {
            RecoverableBoundaryCodec::encode(value, expected, context, heap)
        }
        .map_err(|error| {
            db_decode_error(format!(
                "recoverable-envelope DB value encode failed: {error}"
            ))
        })?;
        Ok(Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes,
        }))
    }

    fn business_value_from_recoverable_envelope_bson(
        &self,
        value: Bson,
        plan: db_boundary::DbBoundaryValuePlanRef<'_>,
    ) -> Result<Value> {
        if matches!(value, Bson::Null)
            && recoverable_expected_type_accepts_null(plan.recoverable_expected())
        {
            return Ok(Value::Null);
        }
        let Bson::Binary(binary) = value else {
            return Err(db_decode_error(format!(
                "recoverable-envelope DB field stored non-binary {}",
                bson_kind(&value)
            )));
        };
        let mut heap = RequestHeap::default();
        let runtime_value = RecoverableBoundaryCodec::decode_with_policy(
            &binary.bytes,
            plan.recoverable_expected(),
            &recoverable_db_context(),
            &mut heap,
            RecoverableDecodePolicy::durable_db(),
        )
        .map_err(|error| {
            db_decode_error(format!(
                "recoverable-envelope DB field decode failed: {error}"
            ))
        })?;
        encode_untyped_wire_json(&runtime_value, &heap).map_err(|error| {
            db_decode_error(format!(
                "recoverable-envelope DB field decode failed: {error}"
            ))
        })
    }

    fn runtime_value_from_recoverable_envelope_bson(
        &self,
        value: Bson,
        plan: db_boundary::DbBoundaryValuePlanRef<'_>,
        heap: &mut RequestHeap,
        recoverable_context: Option<&DbRecoverableRuntimeReadContext<'_>>,
        field_path: Option<&str>,
    ) -> Result<RuntimeValue> {
        let fallback_context = recoverable_db_context();
        let context = recoverable_context
            .and_then(|context| context.boundary_context)
            .unwrap_or(&fallback_context);
        let expected = recoverable_context
            .and_then(|context| {
                context.recoverable_expected_override.or_else(|| {
                    recoverable_expected_override_for_field(
                        context.recoverable_expected_overrides,
                        field_path,
                    )
                })
            })
            .unwrap_or_else(|| plan.recoverable_expected());
        if matches!(value, Bson::Null) && recoverable_expected_type_accepts_null(expected) {
            return Ok(RuntimeValue::Null);
        }
        let Bson::Binary(binary) = value else {
            return Err(db_decode_error(format!(
                "recoverable-envelope DB field stored non-binary {}",
                bson_kind(&value)
            )));
        };
        let result = if let Some(recoverable_context) = recoverable_context {
            RecoverableBoundaryCodec::decode_with_behavior_and_policy(
                &binary.bytes,
                expected,
                context,
                heap,
                recoverable_context.behavior_hooks,
                RecoverableDecodePolicy::durable_db(),
            )
        } else {
            RecoverableBoundaryCodec::decode_with_policy(
                &binary.bytes,
                expected,
                context,
                heap,
                RecoverableDecodePolicy::durable_db(),
            )
        };
        result.map_err(|error| {
            db_decode_error(format!(
                "recoverable-envelope DB field decode failed: {error}"
            ))
        })
    }

    fn business_date_value(&self, value: Bson) -> Result<Value> {
        match value {
            Bson::DateTime(value) => Ok(Value::String(date_value::format_epoch_millis(
                value.timestamp_millis(),
                "Mongo Date field",
            )?)),
            Bson::String(value) => {
                let ms = date_value::parse_rfc3339_millis(&value, "Mongo Date field")?;
                Ok(Value::String(date_value::format_epoch_millis(
                    ms,
                    "Mongo Date field",
                )?))
            }
            other => Err(db_decode_error(format!(
                "Mongo Date field stored non-date {}",
                bson_kind(&other)
            ))),
        }
    }

    fn key_write_projection_plan(&self) -> Option<db_boundary::DbBoundaryValuePlanRef<'_>> {
        self.key_ty.as_ref().map(|plan| plan.write_projection_ref())
    }

    fn key_result_decode_plan(&self) -> Option<db_boundary::DbBoundaryValuePlanRef<'_>> {
        self.key_ty.as_ref().map(|plan| plan.result_decode_ref())
    }

    fn field_write_projection_plan_for_path(
        &self,
        field: &str,
    ) -> Option<db_boundary::DbBoundaryValuePlanRef<'_>> {
        db_boundary::field_plan_for_path(field, &self.key_field, self.key_ty.as_ref(), |top| {
            self.fields.get(top)?.ty.as_ref()
        })
    }

    fn field_result_decode_plan_for_path(
        &self,
        field: &str,
    ) -> Option<db_boundary::DbBoundaryValuePlanRef<'_>> {
        db_boundary::field_result_decode_plan_for_path(
            field,
            &self.key_field,
            self.key_ty.as_ref(),
            |top| self.fields.get(top)?.ty.as_ref(),
        )
    }

    fn collection_item_plan_for_path(
        &self,
        field: &str,
    ) -> Option<db_boundary::DbBoundaryValuePlanRef<'_>> {
        db_boundary::collection_item_plan_for_path(
            field,
            &self.key_field,
            self.key_ty.as_ref(),
            |top| self.fields.get(top)?.ty.as_ref(),
        )
    }

    fn field_path_policy(&self) -> db_boundary::DbFieldPathPolicy<'_> {
        db_boundary::DbFieldPathPolicy::new(&self.key_field)
    }

    fn validate_storage_field_use(
        &self,
        resolved: &db_boundary::DbResolvedFieldPath<'_>,
        use_case: DbFieldUse,
        encrypted_context: Option<&EncryptedRecordContext>,
    ) -> Result<()> {
        let Some(field) = self.fields.get(resolved.top_level()) else {
            return Ok(());
        };
        let is_top_level = resolved.business_path() == resolved.top_level()
            || resolved.mongo_path() == resolved.top_level();
        let operation = if use_case == DbFieldUse::WholeSet && !is_top_level {
            "partial set"
        } else {
            use_case.label()
        };
        if field.storage == DbFieldStorageIr::Encrypted {
            if (use_case == DbFieldUse::Projection && is_top_level)
                || (use_case == DbFieldUse::WholeSet && is_top_level && encrypted_context.is_some())
            {
                return Ok(());
            }
            return Err(db_decode_error(format!(
                "encrypted DB field {} cannot be used for {}",
                resolved.top_level(),
                operation
            )));
        }
        let Some(plan) = field.ty.as_ref() else {
            return Ok(());
        };
        if !plan.is_recoverable_envelope_lane() {
            return Ok(());
        }
        if use_case == DbFieldUse::Projection && is_top_level {
            return Ok(());
        }
        if use_case == DbFieldUse::WholeSet && is_top_level {
            return Ok(());
        }
        Err(db_decode_error(format!(
            "recoverable-envelope DB field {} is opaque; {} on {} is not supported in P5; only full field read/write is supported",
            resolved.top_level(),
            operation,
            resolved.business_path()
        )))
    }

    fn encrypted_context_from_key_bson(
        &self,
        key: &Bson,
    ) -> Result<Option<EncryptedRecordContext>> {
        if !self.has_encrypted_fields() {
            return Ok(None);
        }
        let Bson::String(record_id) = key else {
            return Err(db_decode_error("encrypted DB object key must be a string"));
        };
        Ok(Some(EncryptedRecordContext {
            record_id: record_id.clone(),
        }))
    }

    fn encode_field_storage(
        &self,
        field_name: &str,
        context: Option<&EncryptedRecordContext>,
        plaintext: Bson,
    ) -> Result<Bson> {
        let Some(field) = self.fields.get(field_name) else {
            return Ok(plaintext);
        };
        if field.storage == DbFieldStorageIr::Identity {
            return Ok(plaintext);
        }
        let Bson::String(plaintext) = plaintext else {
            return Err(db_decode_error("encrypted DB field encode failed"));
        };
        let context = context.ok_or_else(|| db_decode_error("encrypted DB field encode failed"))?;
        let cipher = self
            .encryption_cipher
            .as_ref()
            .ok_or_else(|| db_decode_error("encrypted DB field encode failed"))?;
        cipher
            .encrypt_string(
                DbEncryptedFieldContext {
                    storage_profile: &self.storage_profile,
                    storage_service_id: &self.storage_service_id,
                    collection_name: &self.collection_name,
                    field_name,
                    record_id: &context.record_id,
                },
                &plaintext,
            )
            .map_err(|_| db_decode_error("encrypted DB field encode failed"))
    }

    fn decode_field_storage(
        &self,
        field_name: &str,
        context: Option<&EncryptedRecordContext>,
        stored: Bson,
    ) -> Result<Bson> {
        let Some(field) = self.fields.get(field_name) else {
            return Ok(stored);
        };
        if field.storage == DbFieldStorageIr::Identity {
            return Ok(stored);
        }
        let context = context.ok_or_else(|| db_decode_error("encrypted DB field decode failed"))?;
        let cipher = self
            .encryption_cipher
            .as_ref()
            .ok_or_else(|| db_decode_error("encrypted DB field decode failed"))?;
        cipher
            .decrypt_string(
                DbEncryptedFieldContext {
                    storage_profile: &self.storage_profile,
                    storage_service_id: &self.storage_service_id,
                    collection_name: &self.collection_name,
                    field_name,
                    record_id: &context.record_id,
                },
                &stored,
            )
            .map(Bson::String)
            .map_err(|_| db_decode_error("encrypted DB field decode failed"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DbFieldUse {
    Projection,
    Predicate,
    Order,
    WholeSet,
    PartialChange,
    Index,
}

impl DbFieldUse {
    fn label(self) -> &'static str {
        match self {
            Self::Projection => "nested projection",
            Self::Predicate => "predicate",
            Self::Order => "order",
            Self::WholeSet => "whole-field set",
            Self::PartialChange => "partial change",
            Self::Index => "index",
        }
    }
}

fn recoverable_db_context() -> RuntimeRecoverableBoundaryContext {
    RuntimeRecoverableBoundaryContext::new(
        RuntimeRecoverableBoundaryKind::DbValue,
        RuntimeRecoverableTrustBoundary::OwnerInternal,
        RuntimeRecoverableStorageLane::RecoverableEnvelope,
    )
}

fn recoverable_expected_override_for_field<'a>(
    overrides: Option<&'a HashMap<String, RuntimeRecoverableExpectedTypePlan>>,
    field_path: Option<&str>,
) -> Option<&'a RuntimeRecoverableExpectedTypePlan> {
    let field_path = field_path?;
    let top = field_path.split('.').next().unwrap_or(field_path);
    overrides?.get(top)
}

fn recoverable_expected_type_accepts_null(expected: &RuntimeRecoverableExpectedTypePlan) -> bool {
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::Null => true,
        RuntimeRecoverableExpectedTypeNode::Nullable { .. } => true,
        RuntimeRecoverableExpectedTypeNode::Union { items } => {
            items.iter().any(recoverable_expected_type_accepts_null)
        }
        RuntimeRecoverableExpectedTypeNode::Alias { target } => {
            recoverable_expected_type_accepts_null(target)
        }
        _ => false,
    }
}

fn runtime_object_fields<'a>(
    value: &'a RuntimeValue,
    heap: &'a RequestHeap,
    operation: &str,
) -> Result<&'a RuntimeObjectFields> {
    let RuntimeValue::Heap(handle) = value else {
        return Err(db_decode_error(format!("{operation} must be an object")));
    };
    match heap.get(*handle)? {
        HeapNode::Object(object) => Ok(object.fields()),
        _ => Err(db_decode_error(format!("{operation} must be an object"))),
    }
}

fn validate_db_business_field_name(field: &str) -> Result<()> {
    db_boundary::validate_db_business_field_name(field).map_err(db_path_policy_error)
}

fn validate_db_business_json_value(value: &Value) -> Result<()> {
    match value {
        Value::Object(object) => validate_db_business_json_object(object),
        Value::Array(items) => {
            for item in items {
                validate_db_business_json_value(item)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_db_business_json_object(object: &Map<String, Value>) -> Result<()> {
    for (field, value) in object {
        validate_db_business_field_name(field)?;
        validate_db_business_json_value(value)?;
    }
    Ok(())
}

fn validate_db_business_bson_value(value: &Bson) -> Result<()> {
    match value {
        Bson::Document(document) => validate_db_business_bson_document(document),
        Bson::Array(items) => {
            for item in items {
                validate_db_business_bson_value(item)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_db_business_bson_document(document: &Document) -> Result<()> {
    for (field, value) in document {
        validate_db_business_field_name(field)?;
        validate_db_business_bson_value(value)?;
    }
    Ok(())
}

fn bson_kind(value: &Bson) -> &'static str {
    match value {
        Bson::Double(_) => "double",
        Bson::String(_) => "string",
        Bson::Array(_) => "array",
        Bson::Document(_) => "document",
        Bson::Boolean(_) => "bool",
        Bson::Null => "null",
        Bson::RegularExpression(_) => "regex",
        Bson::JavaScriptCode(_) => "javascript",
        Bson::JavaScriptCodeWithScope(_) => "javascriptWithScope",
        Bson::Int32(_) => "int32",
        Bson::Int64(_) => "int64",
        Bson::Timestamp(_) => "timestamp",
        Bson::Binary(_) => "binary",
        Bson::ObjectId(_) => "objectId",
        Bson::DateTime(_) => "date",
        Bson::DbPointer(_) => "dbPointer",
        Bson::Symbol(_) => "symbol",
        Bson::Decimal128(_) => "decimal128",
        Bson::Undefined => "undefined",
        Bson::MaxKey => "maxKey",
        Bson::MinKey => "minKey",
    }
}

fn validate_field_operator(operator: &str, value: &Value) -> Result<()> {
    match operator {
        "$eq" | "$ne" | "$gt" | "$gte" | "$lt" | "$lte" => Ok(()),
        "$in" | "$nin" => value.as_array().map(|_| ()).ok_or_else(|| {
            db_decode_error(format!(
                "db query operator {operator} requires an array value"
            ))
        }),
        "$exists" => value.as_bool().map(|_| ()).ok_or_else(|| {
            db_decode_error("db query operator $exists requires a boolean value".to_string())
        }),
        "$regex" => value.as_str().map(|_| ()).ok_or_else(|| {
            db_decode_error("db query operator $regex requires a string value".to_string())
        }),
        "$options" => value.as_str().map(|_| ()).ok_or_else(|| {
            db_decode_error("db query operator $options requires a string value".to_string())
        }),
        other => Err(db_decode_error(format!(
            "db query field operator {other} is not supported"
        ))),
    }
}
