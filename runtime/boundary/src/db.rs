use std::borrow::Cow;

use skiff_artifact_model::{LiteralIr, TypeRefIr};
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
        TypeRefIr::Builtin { name, args } => match bare_type_name(name) {
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
        TypeRefIr::AppliedNominal { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::Function { .. } => false,
    }
}

fn is_plain_db_string_key(type_ref: &TypeRefIr) -> bool {
    matches!(
        type_ref,
        TypeRefIr::Builtin { name, args } if args.is_empty() && bare_type_name(name) == "string"
    )
}

fn runtime_type_node_from_artifact_type_ref(type_ref: &TypeRefIr) -> RuntimeTypeNode {
    match type_ref {
        TypeRefIr::Builtin { name, args } => runtime_builtin_type_node_from_artifact(name, args),
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
                .map(runtime_type_plan_from_artifact_type_ref)
                .collect(),
        ),
        TypeRefIr::Nullable { inner } => {
            RuntimeTypeNode::Nullable(Box::new(runtime_type_plan_from_artifact_type_ref(inner)))
        }
        TypeRefIr::Literal {
            value: LiteralIr::String { value },
        } => RuntimeTypeNode::LiteralString(value.clone()),
        TypeRefIr::Literal { .. }
        | TypeRefIr::AppliedNominal { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::Function { .. } => RuntimeTypeNode::Unknown,
    }
}

fn runtime_builtin_type_node_from_artifact(name: &str, args: &[TypeRefIr]) -> RuntimeTypeNode {
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
        TypeRefIr::Builtin { .. } => "builtin",
        TypeRefIr::LocalType { .. } => "localType",
        TypeRefIr::PublicationType { .. } => "publicationType",
        TypeRefIr::ServiceSymbol { .. } => "serviceSymbol",
        TypeRefIr::PackageSymbol { .. } => "packageSymbol",
        TypeRefIr::PackageSchema { .. } => "packageSchema",
        TypeRefIr::AppliedNominal { .. } => "appliedNominal",
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
        TypeRefIr::Builtin { name, .. } => Some(name.clone()),
        _ => None,
    }
}

fn artifact_type_ref_identity(type_ref: &TypeRefIr) -> RuntimeTypeIdentityPlan {
    match type_ref {
        TypeRefIr::AnyInterface { interface } => RuntimeTypeIdentityPlan {
            interface: Some(interface.interface_abi_id.clone()),
            ..RuntimeTypeIdentityPlan::default()
        },
        _ => RuntimeTypeIdentityPlan::default(),
    }
}

#[cfg(test)]
mod tests;
