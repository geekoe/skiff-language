use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    compile_requirements::ServiceCallRef,
    executable::{ExecutableBody, ExecutableIr},
    refs::SourceSpanRef,
    schema::{FILE_IR_FORMAT_VERSION, FILE_IR_OPCODE_TABLE_VERSION, FILE_IR_SCHEMA_VERSION},
    symbols::{PackageCallableRef, PackageSymbolRef, ServiceDependencySymbolRef, ServiceSymbolRef},
    targets::NativeTarget,
    types::{InterfaceDeclIr, TypeDeclIr, TypeRefIr},
};

mod service_calls;

pub use service_calls::{
    file_ir_service_call_sites, validate_file_ir_service_calls,
    validated_file_ir_service_call_refs, FileIrServiceCallOwner, FileIrServiceCallSite,
    FileIrServiceCallValidationError,
};

pub const FILE_IR_SOURCE_MAP_FORMAT: &str = "skiff-file-ir-source-map-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileIrUnit {
    pub schema_version: String,
    pub file_ir_identity: String,
    pub source_ast_hash: String,
    pub module_path: String,
    pub ir_format_version: String,
    pub opcode_table_version: String,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub required_receiver_builtin_capability_version: u32,
    pub source_map: SourceMapDto,
    pub declarations: FileDeclarations,
    pub link_targets: FileLinkTargets,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_table: Vec<TypeDeclIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constants: Vec<ConstIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub executables: Vec<ExecutableIr>,
    pub external_refs: ExternalRefTable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceMapDto {
    pub format: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceMapSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<SourceMapSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceMapSource {
    pub id: u64,
    pub path: String,
    pub module_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ast_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceMapSpan {
    pub id: u64,
    pub source: u64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub span: SourceSpanRef,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileDeclarations {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, TypeDeclarationIr>,
    pub interfaces: BTreeMap<String, InterfaceDeclIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub db: BTreeMap<String, DbDeclarationIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub executables: BTreeMap<String, ExecutableDeclarationIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub constants: BTreeMap<String, ConstDeclarationIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeDeclarationIr {
    pub type_index: u32,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutableDeclarationIr {
    pub executable_index: u32,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConstDeclarationIr {
    pub const_index: u32,
    pub symbol: String,
    pub ty: TypeRefIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConstIr {
    pub name: String,
    pub ty: TypeRefIr,
    pub body: ExecutableBody,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbDeclarationIr {
    pub type_ref: TypeRefIr,
    pub type_name: String,
    pub collection_name: String,
    pub kind: DbObjectKindIr,
    pub key: DbObjectKeyIr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<DbObjectFieldIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<DbRetentionIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leases: Vec<DbLeaseIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub indexes: Vec<DbIndexIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum DbObjectKindIr {
    #[default]
    Object,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbObjectKeyIr {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeRefIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbObjectFieldIr {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeRefIr,
    #[serde(default, skip_serializing_if = "DbFieldStorageIr::is_identity")]
    pub storage: DbFieldStorageIr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum DbFieldStorageIr {
    #[default]
    Identity,
    Encrypted,
}

impl DbFieldStorageIr {
    pub fn is_identity(&self) -> bool {
        *self == Self::Identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbRetentionIr {
    pub amount: u64,
    pub unit: DbRetentionUnitIr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbLeaseIr {
    pub name: String,
    pub ttl_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum DbRetentionUnitIr {
    Days,
    Hours,
    Minutes,
    Seconds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldPathIr {
    pub text: String,
    pub segments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbIndexIr {
    pub name: String,
    pub unique: bool,
    pub fields: Vec<DbIndexFieldIr>,
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub where_expr: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbIndexFieldIr {
    pub field: FieldPathIr,
    pub direction: DbIndexDirectionIr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum DbIndexDirectionIr {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileLinkTargets {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, TypeLinkTargetIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub executables: BTreeMap<String, ExecutableLinkTargetIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub constants: BTreeMap<String, ConstLinkTargetIr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeLinkTargetIr {
    pub type_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutableLinkTargetIr {
    pub executable_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConstLinkTargetIr {
    pub const_index: u32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalRefTable {
    /// Canonical service-call facts. ServiceCall instructions reference this
    /// table by ServiceCallRefIndex and never inline a second copy.
    pub service_call_refs: Vec<ServiceCallRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_symbols: Vec<ServiceSymbolRef>,
    /// Legacy runtime adapter refs. Canonical package lowering uses
    /// service_call_refs plus CallTargetIr::ServiceCall instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_dependency_symbols: Vec<ServiceDependencySymbolRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_symbols: Vec<PackageSymbolRef>,
    /// Owner-local package direct-call identities. Local ABI expectations are
    /// carried by package requirements rather than repeated here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_callables: Vec<PackageCallableRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_targets: Vec<NativeTarget>,
}

/// Owner-local index into `FileIrUnit.externalRefs.serviceCallRefs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ServiceCallRefIndex(u32);

impl ServiceCallRefIndex {
    pub fn new(index: u32) -> Self {
        Self(index)
    }

    pub fn index(self) -> u32 {
        self.0
    }
}

impl TryFrom<usize> for ServiceCallRefIndex {
    type Error = std::num::TryFromIntError;

    fn try_from(index: usize) -> Result<Self, Self::Error> {
        Ok(Self(u32::try_from(index)?))
    }
}

impl ExternalRefTable {
    pub fn service_call_ref(&self, index: ServiceCallRefIndex) -> Option<&ServiceCallRef> {
        self.service_call_refs.get(index.index() as usize)
    }
}

impl FileIrUnit {
    pub fn empty(module_path: impl Into<String>, source_ast_hash: impl Into<String>) -> Self {
        Self {
            schema_version: FILE_IR_SCHEMA_VERSION.to_string(),
            file_ir_identity: String::new(),
            source_ast_hash: source_ast_hash.into(),
            module_path: module_path.into(),
            ir_format_version: FILE_IR_FORMAT_VERSION.to_string(),
            opcode_table_version: FILE_IR_OPCODE_TABLE_VERSION.to_string(),
            required_receiver_builtin_capability_version: 0,
            source_map: SourceMapDto::empty(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            type_table: Vec::new(),
            constants: Vec::new(),
            executables: Vec::new(),
            external_refs: ExternalRefTable::default(),
        }
    }
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

impl SourceMapDto {
    pub fn empty() -> Self {
        Self {
            format: FILE_IR_SOURCE_MAP_FORMAT.to_string(),
            sources: Vec::new(),
            spans: Vec::new(),
        }
    }
}
