use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    abi_identity::AbiIdentityFacts,
    file_ir::{
        DbIndexFieldIr, DbLeaseIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr, DbRetentionIr,
    },
    metadata::MetadataValue,
    package_unit::{PackageAbiExpectation, PackageDependencyConstraint},
    publication_abi::{OperationAbiRef, PublicationAbiUnit},
    recoverable::RecoverableArtifactMetadata,
    refs::FileIrRef,
    schema::SERVICE_UNIT_SCHEMA_VERSION,
    types::TypeRefIr,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceUnit {
    pub schema_version: String,
    pub service: ServiceMeta,
    pub version: String,
    pub protocol_identity: String,
    #[serde(default, skip_serializing_if = "AbiIdentityFacts::is_empty")]
    pub abi_identity_projection: AbiIdentityFacts,
    pub publication_abi: PublicationAbiUnit,
    pub files: Vec<FileIrRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_dependencies: Vec<PackageDependencyConstraint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_dependencies: Vec<ServiceDependencyConstraint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_abi_expectations: Vec<PackageAbiExpectation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<ServiceOperation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operation_route_bindings: Vec<OperationRouteBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_instances: Vec<PublicInstanceExport>,
    #[serde(default, skip_serializing_if = "RecoverableArtifactMetadata::is_empty")]
    pub recoverable_metadata: RecoverableArtifactMetadata,
    #[serde(default)]
    pub db: Vec<DbMetadataIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spawn_targets: Vec<SpawnTargetIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<ActorMetadataIr>,
    pub gateway: GatewayConfig,
    #[serde(default, skip_serializing_if = "ServiceTimeoutConfig::is_empty")]
    pub timeout: ServiceTimeoutConfig,
    pub config: ServiceConfigMetadata,
}

impl ServiceUnit {
    pub fn empty(
        service_id: impl Into<String>,
        version: impl Into<String>,
        protocol_identity: impl Into<String>,
    ) -> Self {
        let service_id = service_id.into();
        let version = version.into();
        Self {
            schema_version: SERVICE_UNIT_SCHEMA_VERSION.to_string(),
            service: ServiceMeta {
                id: service_id.clone(),
                display_name: None,
                metadata: BTreeMap::new(),
            },
            version: version.clone(),
            protocol_identity: protocol_identity.into(),
            abi_identity_projection: AbiIdentityFacts::default(),
            publication_abi: PublicationAbiUnit::empty(service_id, version, ""),
            files: Vec::new(),
            package_dependencies: Vec::new(),
            service_dependencies: Vec::new(),
            package_abi_expectations: Vec::new(),
            operations: Vec::new(),
            operation_route_bindings: Vec::new(),
            public_instances: Vec::new(),
            recoverable_metadata: RecoverableArtifactMetadata::default(),
            db: Vec::new(),
            spawn_targets: Vec::new(),
            actors: Vec::new(),
            gateway: GatewayConfig::default(),
            timeout: ServiceTimeoutConfig::default(),
            config: ServiceConfigMetadata::default(),
        }
    }
}

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
/// Distinct from [`crate::file_ir::DbIndexIr`]: the runtime projection always emits
/// the `where` key (null when absent) rather than omitting it. `where_expr` carries a
/// serialized `skiff` expression AST; strong-typing it is tracked separately because
/// the expression AST is a large recursive structure with externally-tagged serde.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbMetadataIndexIr {
    pub name: String,
    pub unique: bool,
    pub fields: Vec<DbIndexFieldIr>,
    #[serde(rename = "where")]
    pub where_expr: Option<Value>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDependencyConstraint {
    pub id: String,
    pub version: String,
    pub alias: String,
    pub build_id: String,
    pub service_protocol_identity: String,
    pub publication_abi: PublicationAbiUnit,
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
    WebSocketGateway,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicInstanceExport {
    pub name: String,
    pub module_path: String,
    pub declared_receiver_type: TypeRefIr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implemented_interfaces: Vec<TypeRefIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<PublicInstanceOperation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicInstanceOperation {
    pub operation: OperationAbiRef,
    pub receiver_executable: LocalReceiverExecutableRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageDependencyOperationRef {
    pub package_ref: String,
    pub operation: OperationAbiRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationTargetRef {
    pub file_ref: FileIrRef,
    pub executable_index: u32,
    pub callable_abi_id: String,
    pub callable_kind: OperationCallableKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationConstReceiverRef {
    pub file_ref: FileIrRef,
    pub const_index: u32,
    pub const_abi_id: String,
    pub const_type_abi_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationCallableKind {
    PublicFunction,
    ReceiverMethod,
    ImplMethod,
    InternalFunction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReceiverCallAbi {
    ExplicitSelfFirst,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalReceiverExecutableRef {
    pub receiver: OperationConstReceiverRef,
    pub executable_target: OperationTargetRef,
    pub method_abi_id: String,
    pub receiver_call_abi: ReceiverCallAbi,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDependencyOperationRef {
    pub dependency_ref: String,
    pub operation: OperationAbiRef,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub routes: BTreeMap<String, GatewayRoute>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub web_sockets: BTreeMap<String, GatewayWebSocket>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayWebSocket {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub operation: String,
    pub operation_abi_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_operation_abi_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<GatewayWebSocketRoute>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayWebSocketRoute {
    pub path: String,
    pub operation: String,
    pub operation_abi_id: String,
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
