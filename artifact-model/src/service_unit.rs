use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
// Runtime/projection consumers still share these canonical executable-target
// leaves; this module no longer defines a service aggregate.
pub use crate::executable_target::{
    LocalReceiverExecutableRef, OperationCallableKind, OperationConstReceiverRef,
    OperationTargetRef, PackageDependencyOperationRef, PublicInstanceExport,
    PublicInstanceOperation, ReceiverCallAbi,
};

use crate::{
    file_ir::{
        DbIndexFieldIr, DbLeaseIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr, DbRetentionIr,
    },
    metadata::MetadataValue,
    publication_abi::OperationAbiRef,
    types::TypeRefIr,
};

/// Service-unit `db` metadata entry produced by the compiler runtime projection.
///
/// Mirrors `db_entry` (and the package projection in `package_db_metadata_entries`)
/// in the compiler driver runtime projection. The leaf shapes
/// (`key`, `fields`, `retention`) reuse the file-IR declaration types since the
/// emitted JSON is byte-identical.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbMetadataIr {
    pub module_path: String,
    pub source_role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_ir_identity: Option<String>,
    pub kind: DbObjectKindIr,
    #[serde(rename = "type")]
    pub ty: TypeRefIr,
    pub type_name: String,
    pub collection_name: String,
    pub key: Option<DbObjectKeyIr>,
    pub fields: Vec<DbObjectFieldIr>,
    pub retention: Option<DbRetentionIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leases: Vec<DbLeaseIr>,
    pub indexes: Vec<DbMetadataIndexIr>,
}

/// Service-unit db index entry.
///
/// Distinct from [`crate::file_ir::DbIndexIr`] only because this DTO belongs to the
/// activation-time runtime metadata projection. Both shapes carry the same closed
/// ordinary/unique index contract; partial-index predicates are not artifact data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbMetadataIndexIr {
    pub name: String,
    pub unique: bool,
    pub fields: Vec<DbIndexFieldIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnTargetIr {
    pub target_identity: String,
    pub kind: SpawnTargetKindIr,
    pub executable_target: OperationTargetRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_types: Vec<TypeRefIr>,
    pub return_type: Option<TypeRefIr>,
    pub service_protocol_identity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnTargetKindIr {
    Function,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorMetadataIr {
    pub actor_type_identity: TypeRefIr,
    pub actor_id_type_identity: TypeRefIr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<ActorMethodMetadataIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorMethodMetadataIr {
    pub method_identity: String,
    pub executable_target: OperationTargetRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_types: Vec<TypeRefIr>,
    pub return_type: Option<TypeRefIr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceTimeoutConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub methods: BTreeMap<String, u64>,
}

impl ServiceTimeoutConfig {
    pub fn is_empty(&self) -> bool {
        self.default_ms.is_none() && self.methods.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceMeta {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, MetadataValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, tag = "kind")]
pub enum ServiceOperation {
    LocalExecutable(ServiceOperationTarget),
    LocalReceiverExecutable(ServiceReceiverOperationTarget),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceOperationTarget {
    pub operation: OperationAbiRef,
    pub executable: OperationTargetRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceReceiverOperationTarget {
    pub operation: OperationAbiRef,
    pub receiver_executable: LocalReceiverExecutableRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationRouteBinding {
    pub ingress_kind: OperationIngressKind,
    pub selector: String,
    pub operation_abi_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationIngressKind {
    ServiceCall,
    HttpGateway,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum OperationMode {
    Unary,
    ServerStream,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationParam {
    pub name: String,
    pub ty: TypeRefIr,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayConfig {
    /// HTTP routes keyed by their canonical `METHOD /literal/path` identity.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub routes: BTreeMap<String, GatewayRoute>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, MetadataValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayRoute {
    pub operation: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub operation_abi_id: String,
    pub method: String,
    pub path: String,
}

impl GatewayRoute {
    /// Returns the canonical identity shared by the gateway route collection and
    /// the HTTP gateway operation-route selector.
    pub fn route_identity(&self) -> String {
        format!("{} {}", self.method.to_ascii_uppercase(), self.path)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceConfigMetadata {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, MetadataValue>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, BTreeMap<String, MetadataValue>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub package_configs: BTreeMap<String, Value>,
}
