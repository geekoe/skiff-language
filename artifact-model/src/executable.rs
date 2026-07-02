use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    builtin_receiver_ops::BuiltinReceiverOp,
    file_ir::{DbIndexDirectionIr, FieldPathIr},
    metadata::MetadataValue,
    publication_abi::{InterfaceInstantiationRef, OperationAbiRef},
    refs::SourceSpanRef,
    symbols::{PackageRefIr, ServiceDependencySymbolRef, ServiceSymbolRef},
    targets::NativeTarget,
    types::{FunctionTypeParamIr, LiteralIr, TypeRefIr},
    ReceiverCallAbi,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutableSignatureIr {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamIr>,
    pub return_type: TypeRefIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_type: Option<TypeRefIr>,
    pub may_suspend: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParamIr {
    pub name: String,
    pub slot: u32,
    pub ty: TypeRefIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutableIr {
    pub kind: ExecutableKind,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamIr>,
    pub return_type: TypeRefIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_type: Option<TypeRefIr>,
    pub slots: SlotLayout,
    pub may_suspend: bool,
    pub body: ExecutableBody,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ExecutableKind {
    Function,
    ImplMethod,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlotLayout {
    pub slots: Vec<SlotIr>,
    pub frame_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlotIr {
    pub index: u32,
    pub name: String,
    pub kind: SlotKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum SlotKind {
    Param,
    SelfValue,
    Local,
    Temp,
    Pattern,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutableBody {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<BlockIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statements: Vec<StmtIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expressions: Vec<ExprIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlockIr {
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statements: Vec<StmtRefIr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExprRefIr {
    pub expression: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StmtRefIr {
    pub statement: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum StmtIr {
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
        item_type: Option<TypeRefIr>,
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
        payload_type: TypeRefIr,
    },
    Rethrow {
        exception_slot: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum AssignTargetIr {
    Slot { slot: u32 },
    Field { object: ExprRefIr, field: String },
    Index { object: ExprRefIr, index: ExprRefIr },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MatchArmIr {
    pub pattern: PatternIr,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum PatternIr {
    Wildcard,
    Literal { value: LiteralIr },
    Type { ty: TypeRefIr },
    Binding { slot: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum ExprIr {
    Literal {
        value: LiteralIr,
    },
    LoadSlot {
        slot: u32,
    },
    LoadConst {
        const_index: u32,
    },
    Field {
        object: ExprRefIr,
        field: String,
    },
    Construct {
        type_ref: TypeRefIr,
        fields: BTreeMap<String, ExprRefIr>,
    },
    InterfaceBox {
        value: ExprRefIr,
        interface: InterfaceInstantiationRef,
        source: BoxSourceIr,
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
        payload_type: TypeRefIr,
    },
    Rethrow {
        exception_slot: u32,
    },
    Catch {
        try_expression: ExprRefIr,
        catch_slot: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        catch_type: Option<TypeRefIr>,
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
        query: DbQueryValueIr,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbOperationIr {
    pub op: DbOpKindIr,
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
    pub result_type: TypeRefIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbQueryValueIr {
    pub target: DbTargetIr,
    pub query: DbQueryIr,
    pub result_type: TypeRefIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbTargetIr {
    pub type_ref: TypeRefIr,
    pub type_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum DbSelectorIr {
    Key { value: ExprRefIr },
    Query { query: DbQueryIr },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbQueryIr {
    #[serde(default, rename = "where", skip_serializing_if = "Vec::is_empty")]
    pub where_clauses: Vec<DbPredicateIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<DbOrderEntryIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<ExprRefIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<ExprRefIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<ExprRefIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum DbPredicateCompareOpIr {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbOrderEntryIr {
    pub field: FieldPathIr,
    pub direction: DbIndexDirectionIr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbProjectionIr {
    pub fields: Vec<FieldPathIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum DbBodyIr {
    ObjectFields { fields: BTreeMap<String, ExprRefIr> },
    Values { value: ExprRefIr },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbChangeIr {
    pub ops: Vec<DbChangeOpIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum DbChangeOpIr {
    Set { path: FieldPathIr, value: ExprRefIr },
    Inc { path: FieldPathIr, value: ExprRefIr },
    Unset { path: FieldPathIr },
    AddToSet { path: FieldPathIr, value: ExprRefIr },
    Remove { path: FieldPathIr, value: ExprRefIr },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbTransactionIr {
    pub mode: DbBlockModeIr,
    pub body: String,
    pub result: ExprRefIr,
    pub result_type: TypeRefIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbLeaseClaimIr {
    pub target: DbTargetIr,
    pub key: ExprRefIr,
    pub slot: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_slot: Option<u32>,
    pub body: String,
    pub result_type: TypeRefIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbLeaseReadIr {
    pub target: DbTargetIr,
    pub key: ExprRefIr,
    pub slot: String,
    pub result_type: TypeRefIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum BoxSourceIr {
    Local {
        concrete_type: TypeRefIr,
        method_table: InterfaceMethodTablePlanIr,
    },
    Remote {
        dependency_ref: String,
        public_instance_key: String,
        operations: RemoteOperationTablePlanIr,
        callee_protocol_identity: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteOperationTablePlanIr {
    pub interface: InterfaceInstantiationRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<RemoteOperationSlotPlanIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteOperationSlotPlanIr {
    pub slot: u32,
    pub method_abi_id: String,
    pub signature: InterfaceMethodSlotSignatureIr,
    pub operation_abi_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceMethodTablePlanIr {
    pub interface: InterfaceInstantiationRef,
    pub concrete_type: TypeRefIr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<InterfaceMethodSlotPlanIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceMethodSlotPlanIr {
    pub slot: u32,
    pub method_name: String,
    pub method_abi_id: String,
    pub signature: InterfaceMethodSlotSignatureIr,
    pub target: InterfaceMethodSlotTargetIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceMethodSlotSignatureIr {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<FunctionTypeParamIr>,
    pub return_type: TypeRefIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceMethodSlotTargetIr {
    pub executable_index: u32,
    pub receiver_call_abi: ReceiverCallAbi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum DbBlockModeIr {
    Effect,
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum UnaryOpIr {
    Not,
    Negate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallIr {
    pub target: CallTargetIr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<ExprRefIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub type_args: BTreeMap<String, TypeRefIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, MetadataValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum CallTargetIr {
    LocalExecutable {
        executable_index: u32,
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
        interface: InterfaceInstantiationRef,
        method_abi_id: String,
        slot: u32,
    },
}
