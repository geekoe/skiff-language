use std::borrow::Cow;

use skiff_artifact_model::{type_ref_abi_key, LiteralIr, TypeRefIr};
use skiff_runtime_model::{
    recoverable::{RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane},
    type_plan::{
        RuntimeRecordFieldPlan, RuntimeTypeIdentityPlan, RuntimeTypeNode, RuntimeTypePlan,
    },
};
use thiserror::Error;

use crate::{
    contract::RuntimeBoundaryContract,
    plan::{BoundaryConversionPlan, BoundaryDirection, BoundaryUse},
    type_descriptor::bare_type_name,
};

const RESERVED_DB_BUSINESS_METADATA_PREFIX: &str = "__skiff";
pub const MONGO_ID_FIELD: &str = "_id";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DbFieldPathPolicyError {
    #[error("db field path {field:?} is not supported")]
    UnsupportedFieldPath { field: String },
    #[error("db field {field} is reserved Skiff metadata")]
    ReservedBusinessMetadataName { field: String },
    #[error("db field path {field} contains reserved Skiff metadata")]
    ReservedBusinessMetadataPath { field: String },
    #[error("db field path {field} is not declared on {type_name}")]
    UndeclaredTopLevel { field: String, type_name: String },
    #[error("db change cannot modify key field {field}")]
    MutableKeyPath { field: String },
}

#[derive(Debug, Clone, Copy)]
pub struct DbFieldPathPolicy<'a> {
    key_field: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbResolvedFieldPath<'a> {
    business_path: &'a str,
    top_level: &'a str,
    mongo_path: Cow<'a, str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbFieldPlanRoot<'a> {
    Key,
    Field(&'a str),
}

#[derive(Debug, Clone)]
pub struct DbBoundaryValuePlan {
    type_ref: TypeRefIr,
    storage_lane: RuntimeRecoverableStorageLane,
    write_projection_plan: BoundaryConversionPlan,
    result_decode_plan: BoundaryConversionPlan,
    recoverable_expected_plan: RuntimeRecoverableExpectedTypePlan,
}

#[derive(Debug, Clone, Copy)]
pub struct DbBoundaryValuePlanRef<'a> {
    plan: &'a RuntimeTypePlan,
    storage_lane: RuntimeRecoverableStorageLane,
    recoverable_expected: &'a RuntimeRecoverableExpectedTypePlan,
}

#[derive(Debug, Clone, Copy)]
pub enum DbValueProjection<'a> {
    RecoverableEnvelope,
    Date,
    Record(DbRecordProjection<'a>),
    Array(DbBoundaryValuePlanRef<'a>),
    Scalar,
}

#[derive(Debug, Clone, Copy)]
pub struct DbRecordProjection<'a> {
    fields: &'a [RuntimeRecordFieldPlan],
    recoverable_expected: &'a RuntimeRecoverableExpectedTypePlan,
}

impl<'a> DbFieldPathPolicy<'a> {
    pub fn new(key_field: &'a str) -> Self {
        Self { key_field }
    }

    pub fn key_field(self) -> &'a str {
        self.key_field
    }

    pub fn resolve_business_field_path<'p, F>(
        self,
        field: &'p str,
        type_name: &str,
        is_declared_top_level: F,
    ) -> std::result::Result<DbResolvedFieldPath<'p>, DbFieldPathPolicyError>
    where
        F: FnOnce(&str) -> bool,
    {
        let top_level =
            self.validate_declared_business_field_path(field, type_name, is_declared_top_level)?;
        Ok(DbResolvedFieldPath {
            business_path: field,
            top_level,
            mongo_path: self.mongo_path_for_business_field_path(field),
        })
    }

    pub fn resolve_mongo_facing_field_path<'p, F>(
        self,
        field: &'p str,
        type_name: &str,
        is_declared_top_level: F,
    ) -> std::result::Result<DbResolvedFieldPath<'p>, DbFieldPathPolicyError>
    where
        F: FnOnce(&str) -> bool,
    {
        if field == MONGO_ID_FIELD {
            return Ok(DbResolvedFieldPath {
                business_path: field,
                top_level: field,
                mongo_path: Cow::Borrowed(MONGO_ID_FIELD),
            });
        }
        self.resolve_business_field_path(field, type_name, is_declared_top_level)
    }

    pub fn resolve_mutable_business_field_path<'p, F>(
        self,
        field: &'p str,
        type_name: &str,
        is_declared_top_level: F,
    ) -> std::result::Result<DbResolvedFieldPath<'p>, DbFieldPathPolicyError>
    where
        F: FnOnce(&str) -> bool,
    {
        if self.is_key_or_mongo_id_path(field) {
            return Err(DbFieldPathPolicyError::MutableKeyPath {
                field: field.to_string(),
            });
        }
        self.resolve_business_field_path(field, type_name, is_declared_top_level)
    }

    pub fn validate_declared_business_field_path<'p, F>(
        self,
        field: &'p str,
        type_name: &str,
        is_declared_top_level: F,
    ) -> std::result::Result<&'p str, DbFieldPathPolicyError>
    where
        F: FnOnce(&str) -> bool,
    {
        validate_db_business_field_path_text(field)?;
        let top_level = top_level_field(field);
        if top_level != self.key_field && !is_declared_top_level(top_level) {
            return Err(DbFieldPathPolicyError::UndeclaredTopLevel {
                field: field.to_string(),
                type_name: type_name.to_string(),
            });
        }
        Ok(top_level)
    }

    pub fn field_plan_root<'p>(
        self,
        field: &'p str,
    ) -> Option<(DbFieldPlanRoot<'p>, std::str::Split<'p, char>)> {
        let mut segments = field.split('.');
        let top_level = segments.next()?;
        let root = if top_level == MONGO_ID_FIELD || top_level == self.key_field {
            DbFieldPlanRoot::Key
        } else {
            DbFieldPlanRoot::Field(top_level)
        };
        Some((root, segments))
    }

    fn is_key_or_mongo_id_path(self, field: &str) -> bool {
        field == self.key_field
            || field
                .strip_prefix(self.key_field)
                .is_some_and(|suffix| suffix.starts_with('.'))
            || field == MONGO_ID_FIELD
            || field.starts_with("_id.")
    }

    fn mongo_path_for_business_field_path<'p>(self, field: &'p str) -> Cow<'p, str> {
        if field == self.key_field {
            Cow::Borrowed(MONGO_ID_FIELD)
        } else {
            Cow::Borrowed(field)
        }
    }
}

impl<'a> DbResolvedFieldPath<'a> {
    pub fn business_path(&self) -> &'a str {
        self.business_path
    }

    pub fn top_level(&self) -> &'a str {
        self.top_level
    }

    pub fn mongo_path(&self) -> &str {
        self.mongo_path.as_ref()
    }
}

impl DbBoundaryValuePlan {
    pub fn from_artifact_type_ref(type_ref: TypeRefIr) -> Self {
        let result_decode_plan = db_result_decode_plan_from_artifact_type_ref(&type_ref);
        let recoverable_expected_plan =
            RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                result_decode_plan.expected(),
            );
        Self {
            storage_lane: db_storage_lane_from_artifact_type_ref(&type_ref),
            write_projection_plan: db_write_projection_plan_from_artifact_type_ref(&type_ref),
            result_decode_plan,
            recoverable_expected_plan,
            type_ref,
        }
    }

    pub fn artifact_type_ref(&self) -> &TypeRefIr {
        &self.type_ref
    }

    pub fn write_projection_ref(&self) -> DbBoundaryValuePlanRef<'_> {
        DbBoundaryValuePlanRef {
            plan: self.write_projection_plan.expected(),
            storage_lane: self.storage_lane,
            recoverable_expected: &self.recoverable_expected_plan,
        }
    }

    pub fn result_decode_ref(&self) -> DbBoundaryValuePlanRef<'_> {
        DbBoundaryValuePlanRef {
            plan: self.result_decode_plan.expected(),
            storage_lane: self.storage_lane,
            recoverable_expected: &self.recoverable_expected_plan,
        }
    }

    pub fn storage_lane(&self) -> RuntimeRecoverableStorageLane {
        self.storage_lane
    }

    pub fn is_recoverable_envelope_lane(&self) -> bool {
        self.storage_lane == RuntimeRecoverableStorageLane::RecoverableEnvelope
    }

    pub fn recoverable_expected_ref(&self) -> &RuntimeRecoverableExpectedTypePlan {
        &self.recoverable_expected_plan
    }
}

impl<'a> DbBoundaryValuePlanRef<'a> {
    pub fn storage_lane(self) -> RuntimeRecoverableStorageLane {
        self.storage_lane
    }

    pub fn recoverable_expected(self) -> &'a RuntimeRecoverableExpectedTypePlan {
        self.recoverable_expected
    }

    pub fn projection(self) -> DbValueProjection<'a> {
        if self.storage_lane == RuntimeRecoverableStorageLane::RecoverableEnvelope {
            return DbValueProjection::RecoverableEnvelope;
        }
        match self.non_nullable_plan().node() {
            RuntimeTypeNode::Date => DbValueProjection::Date,
            RuntimeTypeNode::Record { fields, .. } => {
                DbValueProjection::Record(DbRecordProjection {
                    fields,
                    recoverable_expected: self.recoverable_expected,
                })
            }
            RuntimeTypeNode::Array(item) => DbValueProjection::Array(DbBoundaryValuePlanRef {
                plan: item,
                storage_lane: self.storage_lane,
                recoverable_expected: self.recoverable_expected,
            }),
            _ => DbValueProjection::Scalar,
        }
    }

    pub fn descend_path<'b>(mut self, segments: impl Iterator<Item = &'b str>) -> Option<Self> {
        for segment in segments {
            if self.storage_lane == RuntimeRecoverableStorageLane::RecoverableEnvelope {
                return None;
            }
            let RuntimeTypeNode::Record { fields, .. } = self.non_nullable_plan().node() else {
                return None;
            };
            self = fields
                .iter()
                .find(|field| field.name == segment)
                .map(|field| DbBoundaryValuePlanRef {
                    plan: &field.ty,
                    storage_lane: self.storage_lane,
                    recoverable_expected: self.recoverable_expected,
                })?;
        }
        Some(self)
    }

    fn non_nullable_plan(self) -> &'a RuntimeTypePlan {
        match self.plan.node() {
            RuntimeTypeNode::Nullable(inner) => inner,
            _ => self.plan,
        }
    }
}

impl<'a> DbRecordProjection<'a> {
    pub fn field(self, name: &str) -> Option<DbBoundaryValuePlanRef<'a>> {
        self.fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| DbBoundaryValuePlanRef {
                plan: &field.ty,
                storage_lane: RuntimeRecoverableStorageLane::SchemaProjectable,
                recoverable_expected: self.recoverable_expected,
            })
    }
}

pub fn db_value_projection(plan: DbBoundaryValuePlanRef<'_>) -> DbValueProjection<'_> {
    plan.projection()
}

pub fn field_plan_for_path<'a, F>(
    field: &str,
    key_field: &str,
    key_plan: Option<&'a DbBoundaryValuePlan>,
    field_plan: F,
) -> Option<DbBoundaryValuePlanRef<'a>>
where
    F: FnOnce(&str) -> Option<&'a DbBoundaryValuePlan>,
{
    field_plan_for_path_with(
        field,
        key_field,
        key_plan,
        field_plan,
        DbBoundaryValuePlan::write_projection_ref,
    )
}

pub fn field_result_decode_plan_for_path<'a, F>(
    field: &str,
    key_field: &str,
    key_plan: Option<&'a DbBoundaryValuePlan>,
    field_plan: F,
) -> Option<DbBoundaryValuePlanRef<'a>>
where
    F: FnOnce(&str) -> Option<&'a DbBoundaryValuePlan>,
{
    field_plan_for_path_with(
        field,
        key_field,
        key_plan,
        field_plan,
        DbBoundaryValuePlan::result_decode_ref,
    )
}

fn field_plan_for_path_with<'a, F>(
    field: &str,
    key_field: &str,
    key_plan: Option<&'a DbBoundaryValuePlan>,
    field_plan: F,
    value_ref: fn(&'a DbBoundaryValuePlan) -> DbBoundaryValuePlanRef<'a>,
) -> Option<DbBoundaryValuePlanRef<'a>>
where
    F: FnOnce(&str) -> Option<&'a DbBoundaryValuePlan>,
{
    let policy = DbFieldPathPolicy::new(key_field);
    let (root, segments) = policy.field_plan_root(field)?;
    let root = match root {
        DbFieldPlanRoot::Key => value_ref(key_plan?),
        DbFieldPlanRoot::Field(top) => value_ref(field_plan(top)?),
    };
    root.descend_path(segments)
}

pub fn collection_item_plan_for_path<'a, F>(
    field: &str,
    key_field: &str,
    key_plan: Option<&'a DbBoundaryValuePlan>,
    field_plan: F,
) -> Option<DbBoundaryValuePlanRef<'a>>
where
    F: FnOnce(&str) -> Option<&'a DbBoundaryValuePlan>,
{
    let plan = field_plan_for_path(field, key_field, key_plan, field_plan)?;
    match db_value_projection(plan) {
        DbValueProjection::Array(item_plan) => Some(item_plan),
        _ => Some(plan),
    }
}

pub fn is_reserved_db_business_metadata_name(name: &str) -> bool {
    name.starts_with(RESERVED_DB_BUSINESS_METADATA_PREFIX)
}

pub fn field_path_has_reserved_db_business_metadata(field: &str) -> bool {
    field.split('.').any(is_reserved_db_business_metadata_name)
}

pub fn validate_db_business_field_name(
    field: &str,
) -> std::result::Result<(), DbFieldPathPolicyError> {
    if is_reserved_db_business_metadata_name(field) {
        return Err(DbFieldPathPolicyError::ReservedBusinessMetadataName {
            field: field.to_string(),
        });
    }
    Ok(())
}

pub fn validate_db_business_field_path_text(
    field: &str,
) -> std::result::Result<(), DbFieldPathPolicyError> {
    if field.trim().is_empty()
        || field.contains('\0')
        || field
            .split('.')
            .any(|segment| segment.is_empty() || segment.starts_with('$'))
    {
        return Err(DbFieldPathPolicyError::UnsupportedFieldPath {
            field: field.to_string(),
        });
    }
    if field_path_has_reserved_db_business_metadata(field) {
        return Err(DbFieldPathPolicyError::ReservedBusinessMetadataPath {
            field: field.to_string(),
        });
    }
    Ok(())
}

pub fn normalize_db_field_path_text<I, S>(text: &str, segments: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    if !text.trim().is_empty() {
        return text.to_string();
    }
    segments
        .into_iter()
        .map(|segment| segment.as_ref().to_string())
        .collect::<Vec<_>>()
        .join(".")
}

pub fn top_level_field(field: &str) -> &str {
    field.split('.').next().unwrap_or(field)
}

pub fn db_write_projection_plan_from_artifact_type_ref(
    type_ref: &TypeRefIr,
) -> BoundaryConversionPlan {
    RuntimeBoundaryContract::default().conversion_plan(
        runtime_type_plan_from_artifact_type_ref(type_ref),
        BoundaryUse::DbWriteProjection,
        BoundaryDirection::Project,
    )
}

pub fn db_result_decode_plan_from_artifact_type_ref(
    type_ref: &TypeRefIr,
) -> BoundaryConversionPlan {
    RuntimeBoundaryContract::default().conversion_plan(
        runtime_type_plan_from_artifact_type_ref(type_ref),
        BoundaryUse::DbResultDecode,
        BoundaryDirection::Decode,
    )
}

pub fn db_storage_lane_from_artifact_type_ref(
    type_ref: &TypeRefIr,
) -> RuntimeRecoverableStorageLane {
    if schema_projectable_db_type_ref(type_ref) {
        RuntimeRecoverableStorageLane::SchemaProjectable
    } else {
        RuntimeRecoverableStorageLane::RecoverableEnvelope
    }
}

pub fn runtime_type_plan_from_artifact_type_ref(type_ref: &TypeRefIr) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: artifact_type_ref_label(type_ref).to_string(),
        named_type_name: artifact_type_ref_named_type_name(type_ref),
        identity: artifact_type_ref_identity(type_ref),
        node: runtime_type_node_from_artifact_type_ref(type_ref),
    }
}

fn schema_projectable_db_type_ref(type_ref: &TypeRefIr) -> bool {
    match type_ref {
        TypeRefIr::Native { name, args } => match bare_type_name(name) {
            "string" | "integer" | "number" | "bool" | "boolean" | "null" | "void" | "Date"
            | "bytes" | "Bytes" | "Json" | "JsonObject" => args.is_empty(),
            "Array" => matches!(
                args.as_slice(),
                [item] if schema_projectable_db_type_ref(item)
            ),
            "Map" => matches!(
                args.as_slice(),
                [key, value] if is_plain_db_string_key(key) && schema_projectable_db_type_ref(value)
            ),
            _ => false,
        },
        TypeRefIr::Record { fields } => fields.values().all(schema_projectable_db_type_ref),
        TypeRefIr::Union { items } => items.iter().all(schema_projectable_db_type_ref),
        TypeRefIr::Nullable { inner } => schema_projectable_db_type_ref(inner),
        TypeRefIr::Literal { .. } => true,
        TypeRefIr::AnyInterface { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::Function { .. } => false,
    }
}

fn is_plain_db_string_key(type_ref: &TypeRefIr) -> bool {
    matches!(
        type_ref,
        TypeRefIr::Native { name, args } if args.is_empty() && bare_type_name(name) == "string"
    )
}

fn runtime_type_node_from_artifact_type_ref(type_ref: &TypeRefIr) -> RuntimeTypeNode {
    match type_ref {
        TypeRefIr::Native { name, args } => runtime_native_type_node_from_artifact(name, args),
        TypeRefIr::Record { fields } => RuntimeTypeNode::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    RuntimeRecordFieldPlan::new(
                        name.clone(),
                        runtime_type_plan_from_artifact_type_ref(ty),
                        !matches!(ty, TypeRefIr::Nullable { .. }),
                    )
                })
                .collect(),
            boundary_record_kind: None,
        },
        TypeRefIr::Union { items } => RuntimeTypeNode::Union(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let mut plan = runtime_type_plan_from_artifact_type_ref(item);
                    if plan.identity.union_branch.is_none() {
                        plan.identity.union_branch =
                            Some(format!("{}#branch:{index}", type_ref_abi_key(type_ref)));
                    }
                    plan
                })
                .collect(),
        ),
        TypeRefIr::Nullable { inner } => {
            RuntimeTypeNode::Nullable(Box::new(runtime_type_plan_from_artifact_type_ref(inner)))
        }
        TypeRefIr::Literal {
            value: LiteralIr::String { value },
        } => RuntimeTypeNode::LiteralString(value.clone()),
        TypeRefIr::Literal { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::Function { .. } => RuntimeTypeNode::Unknown,
    }
}

fn runtime_native_type_node_from_artifact(name: &str, args: &[TypeRefIr]) -> RuntimeTypeNode {
    if bare_type_name(name) == "Array" && args.len() == 1 {
        return RuntimeTypeNode::Array(Box::new(runtime_type_plan_from_artifact_type_ref(
            &args[0],
        )));
    }
    if bare_type_name(name) == "Map" && args.len() == 2 {
        return RuntimeTypeNode::Map {
            key: Box::new(runtime_type_plan_from_artifact_type_ref(&args[0])),
            value: Box::new(runtime_type_plan_from_artifact_type_ref(&args[1])),
        };
    }
    if bare_type_name(name) == "Stream" && args.len() == 1 {
        return RuntimeTypeNode::Stream(Box::new(runtime_type_plan_from_artifact_type_ref(
            &args[0],
        )));
    }
    match bare_type_name(name) {
        "Json" => RuntimeTypeNode::Json,
        "JsonObject" => RuntimeTypeNode::JsonObject,
        "bytes" => RuntimeTypeNode::Bytes,
        "Date" => RuntimeTypeNode::Date,
        "string" => RuntimeTypeNode::String,
        "bool" | "boolean" => RuntimeTypeNode::Bool,
        "integer" => RuntimeTypeNode::Integer,
        "number" => RuntimeTypeNode::Number,
        "null" | "void" => RuntimeTypeNode::Null,
        _ => RuntimeTypeNode::Unknown,
    }
}

fn artifact_type_ref_label(type_ref: &TypeRefIr) -> &'static str {
    match type_ref {
        TypeRefIr::Native { .. } => "builtin",
        TypeRefIr::LocalType { .. } => "localType",
        TypeRefIr::ServiceSymbol { .. } => "serviceSymbol",
        TypeRefIr::PackageSymbol { .. } => "packageSymbol",
        TypeRefIr::DbObjectSymbol { .. } => "dbObjectSymbol",
        TypeRefIr::Record { .. } => "record",
        TypeRefIr::Union { .. } => "union",
        TypeRefIr::Nullable { .. } => "nullable",
        TypeRefIr::Literal { .. } => "literal",
        TypeRefIr::TypeParam { .. } => "typeParam",
        TypeRefIr::AnyInterface { .. } => "anyInterface",
        TypeRefIr::Function { .. } => "function",
    }
}

fn artifact_type_ref_named_type_name(type_ref: &TypeRefIr) -> Option<String> {
    match type_ref {
        TypeRefIr::Native { name, .. } => Some(name.clone()),
        _ => None,
    }
}

fn artifact_type_ref_identity(type_ref: &TypeRefIr) -> RuntimeTypeIdentityPlan {
    match type_ref {
        TypeRefIr::AnyInterface { interface } => RuntimeTypeIdentityPlan {
            interface: Some(interface.interface_abi_id.clone()),
            ..RuntimeTypeIdentityPlan::default()
        },
        TypeRefIr::Union { .. } => RuntimeTypeIdentityPlan {
            union: Some(type_ref_abi_key(type_ref)),
            ..RuntimeTypeIdentityPlan::default()
        },
        TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. } => RuntimeTypeIdentityPlan {
            nominal: Some(type_ref_abi_key(type_ref)),
            ..RuntimeTypeIdentityPlan::default()
        },
        _ => RuntimeTypeIdentityPlan::default(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{InterfaceInstantiationRef, TypeRefIr};
    use skiff_runtime_model::{
        recoverable::{
            RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
            RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
            RuntimeRecoverableTrustBoundary,
        },
        request_heap::RequestHeap,
        runtime_value::{InterfaceCarrier, InterfaceValue, RemoteOperationTable, RuntimeValue},
        type_plan::RuntimeTypeNode,
    };

    use crate::{
        db::{
            collection_item_plan_for_path, db_result_decode_plan_from_artifact_type_ref,
            db_storage_lane_from_artifact_type_ref, db_value_projection,
            db_write_projection_plan_from_artifact_type_ref,
            field_path_has_reserved_db_business_metadata, field_plan_for_path,
            is_reserved_db_business_metadata_name, normalize_db_field_path_text,
            DbBoundaryValuePlan, DbFieldPathPolicy, DbFieldPathPolicyError, DbValueProjection,
            MONGO_ID_FIELD,
        },
        error::RecoverableBoundaryErrorCode,
        plan::{BoundaryDirection, BoundaryUse},
        recoverable::{FailClosedRecoverableBehaviorHooks, RecoverableBoundaryCodec},
    };

    #[test]
    fn db_type_path_descends_nullable_records() {
        let plan = DbBoundaryValuePlan::from_artifact_type_ref(nullable(record([
            ("createdAt", native("Date")),
            (
                "payload",
                nullable(record([
                    ("recoverAt", native("Date")),
                    (
                        "attempts",
                        TypeRefIr::Native {
                            name: "Array".to_string(),
                            args: vec![record([("at", native("Date"))])],
                        },
                    ),
                ])),
            ),
        ])));

        let payload_recover = plan
            .write_projection_ref()
            .descend_path(["payload", "recoverAt"].into_iter())
            .expect("nested type should resolve");
        assert!(matches!(
            db_value_projection(payload_recover),
            DbValueProjection::Date
        ));

        let attempts = plan
            .write_projection_ref()
            .descend_path(["payload", "attempts"].into_iter())
            .expect("array field should resolve");
        let item = match db_value_projection(attempts) {
            DbValueProjection::Array(item) => item,
            other => panic!("expected array projection, got {other:?}"),
        };
        assert!(matches!(
            item.descend_path(["at"].into_iter())
                .map(db_value_projection),
            Some(DbValueProjection::Date)
        ));
    }

    #[test]
    fn db_type_path_does_not_descend_into_recoverable_envelope_lane() {
        let plan = DbBoundaryValuePlan::from_artifact_type_ref(record([
            ("provider", any_interface()),
            ("label", native("string")),
        ]));

        assert_eq!(
            plan.storage_lane(),
            RuntimeRecoverableStorageLane::RecoverableEnvelope
        );
        assert!(matches!(
            db_value_projection(plan.write_projection_ref()),
            DbValueProjection::RecoverableEnvelope
        ));
        assert!(plan
            .write_projection_ref()
            .descend_path(["label"].into_iter())
            .is_none());
    }

    #[test]
    fn db_field_type_helpers_route_key_and_declared_fields() {
        let key_plan = DbBoundaryValuePlan::from_artifact_type_ref(native("string"));
        let payload_plan =
            DbBoundaryValuePlan::from_artifact_type_ref(record([("recoverAt", native("Date"))]));

        let plan = field_plan_for_path("_id", "id", Some(&key_plan), |_| None)
            .expect("_id should use the key type");
        assert!(matches!(
            db_value_projection(plan),
            DbValueProjection::Scalar
        ));

        let plan = field_plan_for_path("payload.recoverAt", "id", Some(&key_plan), |top| {
            (top == "payload").then_some(&payload_plan)
        })
        .expect("declared field path should resolve");
        assert!(matches!(db_value_projection(plan), DbValueProjection::Date));

        let item =
            collection_item_plan_for_path("payload.recoverAt", "id", Some(&key_plan), |top| {
                (top == "payload").then_some(&payload_plan)
            })
            .expect("non-array field should return its own type");
        assert!(matches!(db_value_projection(item), DbValueProjection::Date));
    }

    #[test]
    fn db_reserved_business_metadata_predicate_is_prefix_based() {
        assert!(is_reserved_db_business_metadata_name("__skiffLease"));
        assert!(field_path_has_reserved_db_business_metadata(
            "payload.__skiffType"
        ));
        assert!(!is_reserved_db_business_metadata_name("skiffType"));
        assert!(!field_path_has_reserved_db_business_metadata(
            "payload.public"
        ));
    }

    #[test]
    fn db_field_path_policy_maps_key_and_accepts_internal_mongo_id() {
        let policy = DbFieldPathPolicy::new("id");

        let key = policy
            .resolve_business_field_path("id", "Thread", |_| false)
            .expect("business key should resolve even when it is not a value field");
        assert_eq!(key.business_path(), "id");
        assert_eq!(key.top_level(), "id");
        assert_eq!(key.mongo_path(), MONGO_ID_FIELD);

        let title = policy
            .resolve_business_field_path("title", "Thread", |top| top == "title")
            .expect("declared business field should resolve");
        assert_eq!(title.mongo_path(), "title");

        let mongo_id = policy
            .resolve_mongo_facing_field_path(MONGO_ID_FIELD, "Thread", |_| false)
            .expect("_id should stay accepted for mongo-facing paths");
        assert_eq!(mongo_id.mongo_path(), MONGO_ID_FIELD);
    }

    #[test]
    fn db_field_path_policy_rejects_unsupported_reserved_and_undeclared_paths() {
        let policy = DbFieldPathPolicy::new("id");

        let error = policy
            .resolve_business_field_path("title.", "Thread", |top| top == "title")
            .expect_err("empty path segments should be rejected");
        assert!(matches!(
            error,
            DbFieldPathPolicyError::UnsupportedFieldPath { .. }
        ));

        let error = policy
            .resolve_business_field_path("title.__skiffType", "Thread", |top| top == "title")
            .expect_err("reserved metadata segments should be rejected");
        assert!(matches!(
            error,
            DbFieldPathPolicyError::ReservedBusinessMetadataPath { .. }
        ));

        let error = policy
            .resolve_business_field_path("missing.nested", "Thread", |top| top == "title")
            .expect_err("undeclared top-level fields should be rejected");
        assert!(matches!(
            error,
            DbFieldPathPolicyError::UndeclaredTopLevel { .. }
        ));
    }

    #[test]
    fn db_field_path_policy_keeps_mutable_key_rejection() {
        let policy = DbFieldPathPolicy::new("id");

        for field in ["id", "id.part", "_id", "_id.part"] {
            let error = policy
                .resolve_mutable_business_field_path(field, "Thread", |_| true)
                .expect_err("key and mongo id paths should not be mutable");
            assert!(matches!(
                error,
                DbFieldPathPolicyError::MutableKeyPath { .. }
            ));
        }
    }

    #[test]
    fn db_field_path_text_normalization_prefers_text_then_segments() {
        assert_eq!(
            normalize_db_field_path_text("title", ["ignored"]),
            "title".to_string()
        );
        assert_eq!(
            normalize_db_field_path_text("", ["payload", "createdAt"]),
            "payload.createdAt".to_string()
        );
        assert_eq!(
            normalize_db_field_path_text("   ", ["payload", "createdAt"]),
            "payload.createdAt".to_string()
        );
    }

    #[test]
    fn db_artifact_type_ref_projection_builds_boundary_plans_without_program_context() {
        let ty = record([
            ("id", native("string")),
            ("createdAt", nullable(native("Date"))),
            (
                "tags",
                TypeRefIr::Native {
                    name: "Array".to_string(),
                    args: vec![native("string")],
                },
            ),
        ]);

        let write_plan = db_write_projection_plan_from_artifact_type_ref(&ty);
        assert_eq!(write_plan.use_case(), BoundaryUse::DbWriteProjection);
        assert_eq!(write_plan.direction(), BoundaryDirection::Project);
        assert!(matches!(
            write_plan.expected().node(),
            RuntimeTypeNode::Record { fields, .. } if fields.len() == 3
        ));

        let decode_plan = db_result_decode_plan_from_artifact_type_ref(&ty);
        assert_eq!(decode_plan.use_case(), BoundaryUse::DbResultDecode);
        assert_eq!(decode_plan.direction(), BoundaryDirection::Decode);

        let value_plan = DbBoundaryValuePlan::from_artifact_type_ref(ty);
        assert_eq!(
            value_plan.storage_lane(),
            RuntimeRecoverableStorageLane::SchemaProjectable
        );
        assert!(matches!(
            value_plan.write_projection_ref().projection(),
            DbValueProjection::Record(fields) if fields.field("createdAt").is_some()
        ));
        assert!(matches!(
            value_plan.result_decode_ref().projection(),
            DbValueProjection::Record(fields) if fields.field("createdAt").is_some()
        ));
    }

    #[test]
    fn any_interface_artifact_type_ref_is_unknown_for_db_boundary_projection() {
        let ty = any_interface();

        let plan = db_result_decode_plan_from_artifact_type_ref(&ty);

        assert!(matches!(plan.expected().node(), RuntimeTypeNode::Unknown));
        assert_eq!(plan.expected().interface_identity(), Some("reader"));
        assert_eq!(
            db_storage_lane_from_artifact_type_ref(&ty),
            RuntimeRecoverableStorageLane::RecoverableEnvelope
        );
    }

    #[test]
    fn db_lane_selects_schema_projectable_for_plain_nested_data() {
        let ty = record([
            ("label", native("string")),
            (
                "tags",
                TypeRefIr::Native {
                    name: "Array".to_string(),
                    args: vec![native("string")],
                },
            ),
        ]);

        assert_eq!(
            db_storage_lane_from_artifact_type_ref(&ty),
            RuntimeRecoverableStorageLane::SchemaProjectable
        );
    }

    #[test]
    fn db_lane_selects_recoverable_envelope_for_behavior_or_nominal_nodes() {
        let nested_behavior = record([("provider", any_interface()), ("label", native("string"))]);

        for ty in [
            any_interface(),
            TypeRefIr::LocalType { type_index: 0 },
            nested_behavior,
        ] {
            assert_eq!(
                db_storage_lane_from_artifact_type_ref(&ty),
                RuntimeRecoverableStorageLane::RecoverableEnvelope
            );
        }
    }

    #[test]
    fn recoverable_envelope_db_context_roundtrips_plain_value() {
        let heap = RequestHeap::default();
        let value = RuntimeValue::String("plain".to_string());
        let expected = RuntimeRecoverableExpectedTypePlan::unresolved("db envelope field");
        let context = recoverable_db_context();

        let bytes = RecoverableBoundaryCodec::encode(&value, &expected, &context, &heap)
            .expect("plain value should encode for DB envelope lane");
        let decoded = RecoverableBoundaryCodec::decode(
            &bytes,
            &expected,
            &context,
            &mut RequestHeap::default(),
        )
        .expect("plain value should decode for DB envelope lane");

        assert_eq!(decoded, value);
    }

    #[test]
    fn recoverable_envelope_db_context_rejects_remote_interface_carrier() {
        let mut heap = RequestHeap::default();
        let remote = RuntimeValue::Heap(
            heap.alloc_interface(InterfaceValue::new(
                "reader".to_string(),
                InterfaceCarrier::Remote {
                    dependency_ref: "dep:llm".to_string(),
                    public_instance_key: "provider".to_string(),
                    operations: RemoteOperationTable::new(
                        "remote:reader".to_string(),
                        "reader".to_string(),
                        Vec::new(),
                    ),
                },
            ))
            .expect("remote interface should allocate"),
        );
        let expected = RuntimeRecoverableExpectedTypePlan::unresolved("db envelope field");
        let context = recoverable_db_context();
        let hooks = FailClosedRecoverableBehaviorHooks;

        let error = RecoverableBoundaryCodec::encode_with_behavior(
            &remote, &expected, &context, &heap, &hooks,
        )
        .expect_err("remote carrier must not be persistable through DB");

        let crate::RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::RemoteCarrierNotPersistable
        );
    }

    #[test]
    fn db_artifact_type_ref_union_identity_metadata_does_not_change_shape() {
        let ty = TypeRefIr::Union {
            items: vec![native("string"), native("number")],
        };

        let plan = db_result_decode_plan_from_artifact_type_ref(&ty);

        assert!(plan.expected().union_identity().is_some());
        let RuntimeTypeNode::Union(items) = plan.expected().node() else {
            panic!("expected union plan");
        };
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0].node(), RuntimeTypeNode::String));
        assert!(items[0].union_branch_identity().is_some());
    }

    fn native(name: &str) -> TypeRefIr {
        TypeRefIr::Native {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    fn nullable(inner: TypeRefIr) -> TypeRefIr {
        TypeRefIr::Nullable {
            inner: Box::new(inner),
        }
    }

    fn record<const N: usize>(fields: [(&str, TypeRefIr); N]) -> TypeRefIr {
        TypeRefIr::Record {
            fields: BTreeMap::from(fields.map(|(name, ty)| (name.to_string(), ty))),
        }
    }

    fn any_interface() -> TypeRefIr {
        TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: "reader".to_string(),
                canonical_type_args: vec![native("string")],
            },
        }
    }

    fn recoverable_db_context() -> RuntimeRecoverableBoundaryContext {
        RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::DbValue,
            RuntimeRecoverableTrustBoundary::OwnerInternal,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        )
    }
}
