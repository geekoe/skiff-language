use std::collections::BTreeMap;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use serde_json::Value;
pub use skiff_artifact_model::{
    ActorAbiIdentity, ActorFieldEncodingIr, ActorImplementationIdentity, ActorMethodIdentity,
    BuiltinReceiverOp, FileIrRef, LiteralIr, MetadataValue, NativeTarget, OperationAbiRef,
    PackageRefIr, PackageSymbolRef, ReceiverCallAbi, ServiceDependencySymbolRef, ServiceSymbolRef,
    SourcePosition, SourceSpanRef, RECEIVER_BUILTIN_CAPABILITY_VERSION,
};

use super::addr::{
    ConstAddr, ExecutableAddr, ExecutableIndex, FileAddr, TypeAddr, TypeIndex, UnitAddr,
};

pub type FileIrIdentity = String;
pub type SourceAstHash = String;
pub type ConstIndex = usize;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedFileUnit {
    pub schema_version: String,
    pub file_ir_identity: FileIrIdentity,
    pub source_ast_hash: SourceAstHash,
    pub module_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir_format_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opcode_table_version: Option<String>,
    pub source_map: SourceMapDto,
    pub declarations: FileDeclarations,
    pub link_targets: FileLinkTargets,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actor_declarations: Vec<LinkedActorDeclaration>,
    #[serde(rename = "typeTable", default)]
    pub types: Vec<TypeDeclIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constants: Vec<ConstIr>,
    #[serde(default)]
    pub executables: Vec<LinkedExecutable>,
    pub external_refs: ExternalRefTable,
}

/// Canonical Actor ABI facts owned by this linked file. Actor declarations are
/// not entries in the ordinary type table: their source-level nominal identity
/// is the service symbol and their runtime owner is filled by assembly linking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedActorDeclaration {
    pub actor_type: ServiceSymbolRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation_owner: Option<LinkedActorDeclarationOwner>,
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub actor_name: String,
    pub actor_id_type: LinkedTypeRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<LinkedActorField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_methods: Vec<LinkedActorPublicMethod>,
    pub actor_runtime_abi_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedActorDeclarationOwner {
    pub unit: UnitAddr,
    pub file: FileAddr,
    pub actor_symbol: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedActorField {
    pub name: String,
    pub ty: LinkedTypeRef,
    pub encoding: ActorFieldEncodingIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedActorPublicMethod {
    pub method_identity: ActorMethodIdentity,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<LinkedFunctionTypeParamIr>,
    pub return_type: LinkedTypeRef,
    pub may_suspend: bool,
    pub implementation: LinkedActorMethodImplementation,
}

/// The private code entry owned by an Actor declaration. This address is
/// deliberately absent from [`LinkedActorMethodDispatchPlan`]: callers route
/// by logical Actor identity and the owner runtime performs this final lookup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum LinkedActorMethodImplementation {
    LocalExecutable { executable_index: u32 },
    Executable { addr: ExecutableAddr },
}

/// A linked Actor call contains only the identities needed for routed dispatch.
/// It is not an ordinary executable call and cannot bypass Actor ownership,
/// epoch fencing, or the per-instance executor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedActorMethodDispatchPlan {
    pub declaration_owner: LinkedActorDeclarationOwner,
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub method_identity: ActorMethodIdentity,
}

/// A call-site reference to the single declaration that proved the registry
/// intrinsic's actor/id/bootstrap type arguments. Field and method tables stay
/// on `LinkedActorDeclaration`; calls never duplicate them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedActorNativeMetadata {
    pub declaration_owner: LinkedActorDeclarationOwner,
    pub actor_abi_identity: ActorAbiIdentity,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMapDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default)]
    pub sources: Vec<Value>,
    #[serde(default)]
    pub spans: Vec<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDeclarations {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, TypeDeclarationIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub interfaces: BTreeMap<String, InterfaceDeclIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub db: BTreeMap<String, DbDeclarationIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub executables: BTreeMap<String, ExecutableDeclarationIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub constants: BTreeMap<String, ConstDeclarationIr>,
    #[serde(default)]
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub symbols: BTreeMap<String, DeclarationIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeDeclarationIr {
    pub type_index: TypeIndex,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutableDeclarationIr {
    pub executable_index: ExecutableIndex,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstDeclarationIr {
    pub const_index: ConstIndex,
    pub symbol: String,
    pub ty: LinkedTypeRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbDeclarationIr {
    pub type_ref: LinkedTypeRef,
    pub type_name: String,
    pub collection_name: String,
    pub kind: DbObjectKindIr,
    pub key: DbObjectKeyIr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<DbObjectFieldIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leases: Vec<DbLeaseIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub indexes: Vec<DbIndexIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DbObjectKindIr {
    Object,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbObjectKeyIr {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: LinkedTypeRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbObjectFieldIr {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: LinkedTypeRef,
    #[serde(default, skip_serializing_if = "DbFieldStorageIr::is_identity")]
    pub storage: DbFieldStorageIr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct DbLeaseIr {
    pub name: String,
    pub ttl_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldPathIr {
    pub text: String,
    pub segments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbIndexIr {
    pub name: String,
    pub unique: bool,
    pub fields: Vec<DbIndexFieldIr>,
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub where_expr: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbIndexFieldIr {
    pub field: FieldPathIr,
    pub direction: DbIndexDirectionIr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DbIndexDirectionIr {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclarationIr {
    pub kind: String,
    pub symbol: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileLinkTargets {
    #[serde(default, deserialize_with = "deserialize_type_index_map")]
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, TypeIndex>,
    #[serde(default, deserialize_with = "deserialize_executable_index_map")]
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub executables: BTreeMap<String, ExecutableIndex>,
    #[serde(default, deserialize_with = "deserialize_const_index_map")]
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub constants: BTreeMap<String, ConstIndex>,
}

impl FileLinkTargets {
    pub fn executable_link_targets(&self) -> impl Iterator<Item = (&String, &ExecutableIndex)> {
        self.executables.iter()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeDeclIr {
    pub name: String,
    pub descriptor: LinkedTypeDescriptor,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discriminator: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implements: Vec<LinkedTypeRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum LinkedTypeDescriptor {
    Record {
        fields: BTreeMap<String, LinkedTypeRef>,
    },
    Alias {
        target: LinkedTypeRef,
    },
    Union {
        variants: Vec<LinkedTypeRef>,
    },
}

impl Default for LinkedTypeDescriptor {
    fn default() -> Self {
        Self::Record {
            fields: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalRefTable {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_symbols: Vec<ServiceSymbolRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_dependency_symbols: Vec<ServiceDependencySymbolRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_symbols: Vec<PackageSymbolRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_targets: Vec<NativeTarget>,
    #[serde(default)]
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub refs: BTreeMap<String, ExternalRefIr>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalRefIr {
    pub symbol: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutableKind {
    Function,
    ImplMethod,
    Operation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedExecutable {
    pub kind: ExecutableKind,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<String>,
    #[serde(default)]
    pub params: Vec<ParamIr>,
    #[serde(default)]
    pub return_type: Option<LinkedTypeRef>,
    #[serde(default)]
    pub self_type: Option<LinkedTypeRef>,
    #[serde(default)]
    pub slots: SlotLayoutIr,
    #[serde(default)]
    pub may_suspend: bool,
    #[serde(default)]
    pub body: LinkedExecutableBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstIr {
    pub name: String,
    pub ty: LinkedTypeRef,
    pub body: LinkedExecutableBody,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamIr {
    pub name: String,
    pub slot: usize,
    pub ty: LinkedTypeRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionTypeParamIr {
    pub name: String,
    pub ty: LinkedTypeRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceDeclIr {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<InterfaceOperationIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceOperationIr {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<FunctionTypeParamIr>,
    pub return_type: LinkedTypeRef,
    pub is_native: bool,
    pub is_provider: bool,
    pub is_static: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implicit_self: Option<LinkedTypeRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LinkedTypeRef {
    #[serde(rename = "builtin")]
    Native {
        name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<LinkedTypeRef>,
    },
    /// File-IR-local type reference. Valid **only within the owning file** before
    /// linking completes. The linker (`file_linker.rs::link_type_ref`) resolves
    /// every `LocalType` to `Address { addr: TypeAddr }` which carries the full
    /// owner context (`UnitAddr + FileAddr + type_index`).
    ///
    /// Invariant (architecture case #23): a `LocalType` must **never** escape
    /// its owning file and be used as a cross-file or cross-package type
    /// reference. Post-linking, all `LocalType` refs inside executed code have
    /// been replaced by `Address`.
    LocalType {
        type_index: TypeIndex,
    },
    PublicationType {
        module_path: String,
        type_index: TypeIndex,
    },
    ServiceSymbol {
        symbol: ServiceSymbolRef,
    },
    PackageSymbol {
        symbol: PackageSymbolRef,
    },
    Record {
        fields: BTreeMap<String, LinkedTypeRef>,
    },
    Union {
        items: Vec<LinkedTypeRef>,
    },
    Nullable {
        inner: Box<LinkedTypeRef>,
    },
    Literal {
        value: LiteralIr,
    },
    TypeParam {
        name: String,
    },
    AnyInterface {
        interface: LinkedInterfaceInstantiationRef,
    },
    Function {
        #[serde(default)]
        params: Vec<FunctionTypeParamIr>,
        return_type: Box<LinkedTypeRef>,
    },
    DbObjectSymbol {
        symbol: ServiceSymbolRef,
    },
    /// Fully-resolved type address produced by the linker. `TypeAddr` is valid
    /// only within the linked runtime program image that produced it.
    ///
    /// Invariant (architecture case #24): `TypeAddr` equality must **not** be
    /// used as ABI / artifact type equality across different runtime
    /// activations. Two activations of the same service version can assign
    /// different loaded-file indexes to the same file. Use `AbiTypeId` for
    /// cross-activation type comparison.
    Address {
        addr: TypeAddr,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedInterfaceInstantiationRef {
    pub interface_abi_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub canonical_type_args: Vec<LinkedTypeRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum LinkedBoxSourceIr {
    Local {
        concrete_type: LinkedTypeRef,
        method_table: LinkedInterfaceMethodTablePlanIr,
    },
    Remote {
        dependency_ref: String,
        public_instance_key: String,
        operations: LinkedRemoteOperationTablePlanIr,
        callee_protocol_identity: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedRemoteOperationTablePlanIr {
    pub interface: LinkedInterfaceInstantiationRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<LinkedRemoteOperationSlotPlanIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedRemoteOperationSlotPlanIr {
    pub slot: u32,
    pub method_abi_id: String,
    pub signature: LinkedInterfaceMethodSlotSignatureIr,
    pub operation_abi_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedInterfaceMethodTablePlanIr {
    pub interface: LinkedInterfaceInstantiationRef,
    pub concrete_type: LinkedTypeRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<LinkedInterfaceMethodSlotPlanIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedInterfaceMethodSlotPlanIr {
    pub slot: u32,
    pub method_name: String,
    pub method_abi_id: String,
    pub signature: LinkedInterfaceMethodSlotSignatureIr,
    pub target: LinkedInterfaceMethodSlotTargetIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedInterfaceMethodSlotSignatureIr {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<LinkedFunctionTypeParamIr>,
    pub return_type: LinkedTypeRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedFunctionTypeParamIr {
    pub name: String,
    pub ty: LinkedTypeRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedInterfaceMethodSlotTargetIr {
    pub executable_index: u32,
    pub receiver_call_abi: ReceiverCallAbi,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlotLayoutIr {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<SlotIr>,
    pub frame_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotIr {
    pub index: usize,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotBindingIr {
    pub slot: usize,
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub scope: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedExecutableBody {
    #[serde(default)]
    pub blocks: Vec<BlockIr>,
    #[serde(default)]
    pub statements: Vec<LinkedStmtIr>,
    #[serde(default)]
    pub expressions: Vec<LinkedExprIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockIr {
    pub label: String,
    #[serde(default)]
    pub statements: Vec<StmtRefIr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExprRefIr {
    pub expression: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StmtRefIr {
    pub statement: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum LinkedStmtIr {
    Let {
        slot: u32,
        value: ExprRefIr,
    },
    Assign {
        target: AssignTargetIr,
        value: ExprRefIr,
    },
    If {
        condition: ExprRefIr,
        then_block: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        else_block: Option<String>,
    },
    ForIn {
        item_slot: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_type: Option<LinkedTypeRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value_slot: Option<u32>,
        iterable: ExprRefIr,
        body: String,
    },
    Match {
        value: ExprRefIr,
        arms: Vec<MatchArmIr>,
    },
    Assert {
        condition: ExprRefIr,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<ExprRefIr>,
    },
    Break,
    Continue,
    Spawn {
        call: ExprRefIr,
    },
    Emit {
        operation: String,
        value: ExprRefIr,
    },
    Expr {
        value: ExprRefIr,
    },
    Return {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<ExprRefIr>,
    },
    Throw {
        value: ExprRefIr,
        payload_type: LinkedTypeRef,
    },
    Rethrow {
        exception_slot: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum AssignTargetIr {
    Slot {
        slot: u32,
    },
    ActorSelfField {
        field: String,
        field_type: LinkedTypeRef,
    },
    Field {
        object: ExprRefIr,
        field: String,
    },
    Index {
        object: ExprRefIr,
        index: ExprRefIr,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchArmIr {
    pub pattern: PatternIr,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum PatternIr {
    Wildcard,
    Literal { value: LiteralIr },
    Type { ty: LinkedTypeRef },
    Binding { slot: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum LinkedExprIr {
    Literal {
        value: LiteralIr,
    },
    LoadSlot {
        slot: u32,
    },
    LoadConst {
        const_index: u32,
    },
    ActorSelfField {
        field: String,
        field_type: LinkedTypeRef,
    },
    Field {
        object: ExprRefIr,
        field: String,
    },
    Construct {
        type_ref: LinkedTypeRef,
        fields: BTreeMap<String, ExprRefIr>,
    },
    InterfaceBox {
        value: ExprRefIr,
        interface: LinkedInterfaceInstantiationRef,
        source: LinkedBoxSourceIr,
    },
    MapLiteral {
        entries: BTreeMap<String, ExprRefIr>,
    },
    ArrayLiteral {
        items: Vec<ExprRefIr>,
    },
    Unary {
        op: UnaryOpIr,
        value: ExprRefIr,
    },
    Binary {
        op: BinaryOpIr,
        left: ExprRefIr,
        right: ExprRefIr,
    },
    Call {
        call: CallIr,
    },
    Throw {
        value: ExprRefIr,
        payload_type: LinkedTypeRef,
    },
    Rethrow {
        exception_slot: u32,
    },
    Catch {
        try_expression: ExprRefIr,
        catch_slot: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        catch_type: Option<LinkedTypeRef>,
        body: ExprRefIr,
    },
    ValueBlock {
        block: String,
        result: ExprRefIr,
    },
    DbOperation {
        operation: DbOperationIr,
    },
    DbQuery {
        target: DbTargetIr,
        query: DbQueryIr,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        projection: Option<DbProjectionIr>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result_type: Option<LinkedTypeRef>,
    },
    DbTransaction {
        transaction: DbTransactionIr,
    },
    DbLeaseClaim {
        claim: DbLeaseClaimIr,
    },
    DbLeaseRead {
        read: DbLeaseReadIr,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbOperationIr {
    pub op: DbOpKindIr,
    #[serde(default)]
    pub many: bool,
    pub target: DbTargetIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<DbSelectorIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<DbQueryIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<DbProjectionIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<DbBodyIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert_body: Option<DbBodyIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<DbChangeIr>,
    pub result_type: LinkedTypeRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DbOpKindIr {
    Find,
    Optional,
    Require,
    Insert,
    Update,
    Upsert,
    Replace,
    Delete,
    Count,
    Exists,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbTargetIr {
    pub type_ref: LinkedTypeRef,
    pub type_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DbSelectorIr {
    Key { value: ExprRefIr },
    Query { query: DbQueryIr },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbQueryIr {
    #[serde(default, rename = "where", skip_serializing_if = "Vec::is_empty")]
    pub where_: Vec<DbPredicateIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<DbOrderIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<ExprRefIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<ExprRefIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<ExprRefIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DbPredicateIr {
    Compare {
        field: FieldPathIr,
        op: DbPredicateCompareOpIr,
        value: ExprRefIr,
    },
    Regex {
        field: FieldPathIr,
        pattern: ExprRefIr,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        options: Option<ExprRefIr>,
    },
    And {
        predicates: Vec<DbPredicateIr>,
    },
    Or {
        predicates: Vec<DbPredicateIr>,
    },
    Not {
        predicate: Box<DbPredicateIr>,
    },
    Conditional {
        condition: ExprRefIr,
        predicate: Box<DbPredicateIr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DbPredicateCompareOpIr {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbOrderIr {
    pub field: FieldPathIr,
    pub direction: DbIndexDirectionIr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbProjectionIr {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldPathIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DbBodyIr {
    ObjectFields { fields: BTreeMap<String, ExprRefIr> },
    Values { value: ExprRefIr },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbChangeIr {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ops: Vec<DbChangeOpIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DbChangeOpIr {
    Set {
        field: FieldPathIr,
        value: ExprRefIr,
    },
    Inc {
        field: FieldPathIr,
        value: ExprRefIr,
    },
    Unset {
        field: FieldPathIr,
    },
    AddToSet {
        field: FieldPathIr,
        value: ExprRefIr,
    },
    Remove {
        field: FieldPathIr,
        value: ExprRefIr,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbTransactionIr {
    pub mode: DbTransactionModeIr,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ExprRefIr>,
    pub result_type: LinkedTypeRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbLeaseClaimIr {
    pub target: DbTargetIr,
    pub key: ExprRefIr,
    pub slot: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_slot: Option<u32>,
    pub body: String,
    pub result_type: LinkedTypeRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbLeaseReadIr {
    pub target: DbTargetIr,
    pub key: ExprRefIr,
    pub slot: String,
    pub result_type: LinkedTypeRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DbTransactionModeIr {
    Effect,
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnaryOpIr {
    Not,
    Negate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BinaryOpIr {
    Add,
    Subtract,
    Multiply,
    Divide,
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallIr {
    pub target: LinkedCallTarget,
    #[serde(default)]
    pub args: Vec<ExprRefIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub type_args: BTreeMap<String, LinkedTypeRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, MetadataValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_metadata: Option<LinkedActorNativeMetadata>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum LinkedCallTarget {
    LocalExecutable {
        executable_index: u32,
    },
    PublicationExecutable {
        module_path: String,
        executable_index: u32,
    },
    Executable {
        addr: ExecutableAddr,
    },
    /// Artifact-level Actor method facts awaiting declaration-owner resolution.
    ActorMethod {
        actor: ServiceSymbolRef,
        actor_abi_identity: ActorAbiIdentity,
        actor_implementation_identity: ActorImplementationIdentity,
        method_identity: ActorMethodIdentity,
    },
    /// Canonical routed Actor invocation. It never contains an executable
    /// address; the owner runtime resolves the method in its declaration table.
    ActorDispatch {
        plan: LinkedActorMethodDispatchPlan,
    },
    ExternalServiceSymbol {
        symbol: ServiceSymbolRef,
    },
    ServiceDependencySymbol {
        symbol: ServiceDependencySymbolRef,
    },
    PackageSymbol {
        package_ref: PackageRefIr,
        operation: OperationAbiRef,
    },
    /// Canonical package-local-ABI call resolved inside one assembly execution image.
    /// Runtime execution keeps this distinct from service-boundary dispatch.
    #[serde(skip)]
    PackageDirect {
        call: crate::LinkedPackageDirectCall,
    },
    /// Canonical service call that still requires the current activation's binding vector.
    /// It never contains a provider executable or deployment.
    #[serde(skip)]
    ActivationRelativeService {
        instruction: crate::ActivationRelativeServiceCall,
    },
    Native {
        target: NativeTarget,
    },
    Builtin {
        op: String,
    },
    ReceiverBuiltin {
        op: BuiltinReceiverOp,
    },
    InterfaceMethod {
        interface: LinkedInterfaceInstantiationRef,
        method_abi_id: String,
        slot: u32,
    },
    LocalConstReceiverExecutable {
        const_addr: ConstAddr,
        executable_addr: ExecutableAddr,
        method_abi_id: String,
        receiver_call_abi: ReceiverCallAbi,
    },
}

impl<'de> Deserialize<'de> for LinkedTypeRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let Some(kind) = value.get("kind").and_then(Value::as_str) else {
            return Err(D::Error::custom("type ref is missing kind"));
        };

        match kind {
            "builtin" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct NativeFields {
                    name: String,
                    #[serde(default)]
                    args: Vec<LinkedTypeRef>,
                }

                let fields =
                    serde_json::from_value::<NativeFields>(value).map_err(D::Error::custom)?;
                Ok(Self::Native {
                    name: fields.name,
                    args: fields.args,
                })
            }
            "localType" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct LocalTypeFields {
                    type_index: TypeIndex,
                }

                let fields =
                    serde_json::from_value::<LocalTypeFields>(value).map_err(D::Error::custom)?;
                Ok(Self::LocalType {
                    type_index: fields.type_index,
                })
            }
            "publicationType" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct PublicationTypeFields {
                    module_path: String,
                    type_index: TypeIndex,
                }

                let fields = serde_json::from_value::<PublicationTypeFields>(value)
                    .map_err(D::Error::custom)?;
                Ok(Self::PublicationType {
                    module_path: fields.module_path,
                    type_index: fields.type_index,
                })
            }
            "serviceSymbol" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct ServiceSymbolFields {
                    symbol: ServiceSymbolRef,
                }

                let fields = serde_json::from_value::<ServiceSymbolFields>(value)
                    .map_err(D::Error::custom)?;
                Ok(Self::ServiceSymbol {
                    symbol: fields.symbol,
                })
            }
            "packageSymbol" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct PackageSymbolFields {
                    symbol: PackageSymbolRef,
                }

                let fields = serde_json::from_value::<PackageSymbolFields>(value)
                    .map_err(D::Error::custom)?;
                Ok(Self::PackageSymbol {
                    symbol: fields.symbol,
                })
            }
            "record" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct RecordFields {
                    fields: BTreeMap<String, LinkedTypeRef>,
                }

                let fields =
                    serde_json::from_value::<RecordFields>(value).map_err(D::Error::custom)?;
                Ok(Self::Record {
                    fields: fields.fields,
                })
            }
            "union" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct UnionFields {
                    kind: String,
                    items: Vec<LinkedTypeRef>,
                }

                let fields =
                    serde_json::from_value::<UnionFields>(value).map_err(D::Error::custom)?;
                debug_assert_eq!(fields.kind, "union");
                Ok(Self::Union {
                    items: fields.items,
                })
            }
            "nullable" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct NullableFields {
                    inner: Box<LinkedTypeRef>,
                }

                let fields =
                    serde_json::from_value::<NullableFields>(value).map_err(D::Error::custom)?;
                Ok(Self::Nullable {
                    inner: fields.inner,
                })
            }
            "literal" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct LiteralFields {
                    value: LiteralIr,
                }

                let fields =
                    serde_json::from_value::<LiteralFields>(value).map_err(D::Error::custom)?;
                Ok(Self::Literal {
                    value: fields.value,
                })
            }
            "typeParam" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct TypeParamFields {
                    name: String,
                }

                let fields =
                    serde_json::from_value::<TypeParamFields>(value).map_err(D::Error::custom)?;
                Ok(Self::TypeParam { name: fields.name })
            }
            "anyInterface" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct AnyInterfaceFields {
                    interface: LinkedInterfaceInstantiationRef,
                }

                let fields = serde_json::from_value::<AnyInterfaceFields>(value)
                    .map_err(D::Error::custom)?;
                Ok(Self::AnyInterface {
                    interface: fields.interface,
                })
            }
            "function" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct FunctionFields {
                    params: Vec<FunctionTypeParamIr>,
                    return_type: Box<LinkedTypeRef>,
                }

                let fields =
                    serde_json::from_value::<FunctionFields>(value).map_err(D::Error::custom)?;
                Ok(Self::Function {
                    params: fields.params,
                    return_type: fields.return_type,
                })
            }
            "dbObjectSymbol" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct DbObjectSymbolFields {
                    symbol: ServiceSymbolRef,
                }

                let fields = serde_json::from_value::<DbObjectSymbolFields>(value)
                    .map_err(D::Error::custom)?;
                Ok(Self::DbObjectSymbol {
                    symbol: fields.symbol,
                })
            }
            "address" => {
                #[derive(Deserialize)]
                struct AddressFields {
                    addr: TypeAddr,
                }

                let fields =
                    serde_json::from_value::<AddressFields>(value).map_err(D::Error::custom)?;
                Ok(Self::Address { addr: fields.addr })
            }
            _ => Err(D::Error::custom(format!("unknown type ref kind {kind}"))),
        }
    }
}

fn deserialize_type_index_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, TypeIndex>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_linked_export_map(deserializer, "typeIndex")
}

fn deserialize_executable_index_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, ExecutableIndex>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_linked_export_map(deserializer, "executableIndex")
}

fn deserialize_const_index_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, ConstIndex>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_linked_export_map(deserializer, "constIndex")
}

fn deserialize_linked_export_map<'de, D>(
    deserializer: D,
    index_key: &str,
) -> Result<BTreeMap<String, usize>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = BTreeMap::<String, Value>::deserialize(deserializer)?;
    raw.into_iter()
        .map(|(symbol, value)| {
            value_to_export_index::<D::Error>(value, index_key).map(|index| (symbol, index))
        })
        .collect()
}

fn value_to_export_index<E>(value: Value, index_key: &str) -> Result<usize, E>
where
    E: serde::de::Error,
{
    if let Value::Object(object) = value {
        if let Some(index) = object.get(index_key).and_then(Value::as_u64) {
            return usize::try_from(index).map_err(E::custom);
        }
    }
    Err(E::custom(format!(
        "expected link target object with numeric {index_key}"
    )))
}

#[cfg(test)]
mod db_storage_tests {
    use super::*;

    #[test]
    fn linked_db_field_storage_round_trips_encrypted() {
        let field = DbObjectFieldIr {
            name: "apiKey".to_string(),
            ty: LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            },
            storage: DbFieldStorageIr::Encrypted,
        };
        let value = serde_json::to_value(&field).unwrap();
        assert_eq!(value["storage"], "encrypted");
        assert_eq!(
            serde_json::from_value::<DbObjectFieldIr>(value).unwrap(),
            field
        );
    }
}
