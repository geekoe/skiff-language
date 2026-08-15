use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    actor_declaration::{ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity},
    builtin_receiver_ops::BuiltinReceiverOp,
    compile_identity::PackageCallableId,
    effects::CallableEffectSummary,
    file_ir::{DbIndexDirectionIr, FieldPathIr, ServiceCallRefIndex},
    metadata::MetadataValue,
    publication_abi::InterfaceInstantiationRef,
    refs::SourceSpanRef,
    symbols::{PackageRefIr, ServiceDependencySymbolRef, ServiceSymbolRef},
    targets::NativeTarget,
    types::{visit_type_ref, FunctionTypeParamIr, LiteralIr, TypeRefIr},
    ReceiverCallAbi,
};

mod concurrent_plan;

pub use concurrent_plan::{ConcurrentLaneIr, ConcurrentPlanIr};

/// Largest persisted duration that runtime execution may admit without losing
/// integer precision in JavaScript consumers.
pub const MAX_SAFE_EXECUTION_DURATION_MILLISECONDS: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutableSignatureIr {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamIr>,
    pub return_type: TypeRefIr,
    /// Required-nullable receiver fact. `Some` covers both implicit Self and
    /// an explicit leading `self`; the receiver is always Value parameter
    /// ordinal zero with this exact type.
    #[serde(deserialize_with = "deserialize_required_option")]
    pub self_type: Option<TypeRefIr>,
    pub may_suspend: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParamIr {
    pub name: String,
    pub slot: u32,
    pub ty: TypeRefIr,
    /// `inout` parameter mode. Skipped when `Value` so legacy File IR stays
    /// byte-identical; the MIR builder reads the mode from here.
    #[serde(default, skip_serializing_if = "ParamModeIr::is_value")]
    pub mode: ParamModeIr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum ParamModeIr {
    #[default]
    Value,
    InOut,
}

impl ParamModeIr {
    pub fn is_value(&self) -> bool {
        matches!(self, Self::Value)
    }
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
    /// Required-nullable receiver fact. `Some` covers both implicit Self and
    /// an explicit leading `self`; the receiver is always Value parameter
    /// ordinal zero with this exact type.
    #[serde(deserialize_with = "deserialize_required_option")]
    pub self_type: Option<TypeRefIr>,
    pub slots: SlotLayout,
    pub may_suspend: bool,
    pub body: ExecutableBody,
    /// Static type of every `body.expressions` entry, in index order. Written
    /// by lowering from source-owned expression type facts (Phase 2 design
    /// §2.2); the emitter and MIR never recover types from File IR.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expression_types: Vec<TypeRefIr>,
    /// Source span of every `body.statements` entry, in index order. `None`
    /// for compiler-generated statements without a source fact.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statement_spans: Vec<Option<SourceSpanRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ExecutableKind {
    Function,
    ImplMethod,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlotLayout {
    pub slots: Vec<SlotIr>,
    pub frame_size: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, try_from = "SlotIrWire")]
pub struct SlotIr {
    pub index: u32,
    pub name: String,
    pub kind: SlotKind,
    /// Producer-owned mutability fact. True only for a source `var` local;
    /// parameters, Self, temps, patterns, and immutable locals are false.
    pub writable_local: bool,
    /// Static type of the slot written by lowering. Skipped when unknown
    /// (synthetic temps, pattern bindings) so legacy File IR stays
    /// byte-identical where no type fact exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<TypeRefIr>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SlotIrWire {
    index: u32,
    name: String,
    kind: SlotKind,
    writable_local: bool,
    #[serde(default)]
    ty: Option<TypeRefIr>,
}

impl TryFrom<SlotIrWire> for SlotIr {
    type Error = String;

    fn try_from(wire: SlotIrWire) -> Result<Self, Self::Error> {
        if wire.writable_local && wire.kind != SlotKind::Local {
            return Err("writableLocal may only be true for a source local slot".to_string());
        }
        Ok(Self {
            index: wire.index,
            name: wire.name,
            kind: wire.kind,
            writable_local: wire.writable_local,
            ty: wire.ty,
        })
    }
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

/// Required origin for instructions that create an exception boundary.
///
/// Source-authored instructions carry their real source span. Generated
/// instructions must state a stable, finite reason and cannot masquerade as a
/// source location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum InstructionSourceSite {
    Source {
        span: SourceSpanRef,
    },
    Synthetic {
        reason: SyntheticInstructionSiteReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyntheticInstructionSiteReason {
    CompilerDesugaring,
    CompilerGeneratedWrapper,
    CompilerGeneratedTestHarness,
    RuntimeBoundaryDispatch,
    RuntimeControlFlow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
// This is a cold-path typed artifact DTO; boxing a variant would change its
// public construction API solely to optimize a non-hot representation.
#[allow(clippy::large_enum_variant)]
pub enum StmtIr {
    InitSlot {
        slot: u32,
        value: ExprRefIr,
    },
    Assign {
        target: AssignTargetIr,
        value: ExprRefIr,
    },
    Timeout {
        duration_ms: u64,
        body: String,
        site: InstructionSourceSite,
    },
    Concurrent {
        plan: ConcurrentPlanIr,
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
    While {
        condition: ExprRefIr,
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
    Dispatch {
        call: ExprRefIr,
    },
    Emit {
        operation: String,
        value: ExprRefIr,
    },
    /// Installs one compiler-checked test effect outcome in the current test
    /// execution context. This statement is emitted only in compiler-owned
    /// hidden test setup executables. Package targets retain an immutable
    /// package callable key; service targets retain the canonical file-owned
    /// service-call ref. Assembly linking replaces either form with the same
    /// exact target used by normal runtime dispatch.
    TestEffectRegister {
        target: TestEffectRegisterTargetIr,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expect: Option<TestEffectExpectedIr>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step_expect: Option<TestEffectExpectedIr>,
        outcome: TestEffectOutcomeIr,
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
        site: InstructionSourceSite,
    },
    Rethrow {
        exception_slot: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum TestEffectRegisterTargetIr {
    PackageCallable {
        package_ref: PackageRefIr,
        callable_id: PackageCallableId,
    },
    ContractOperation {
        service_call_ref_index: ServiceCallRefIndex,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestEffectExpectedIr {
    pub value: ExprRefIr,
    pub request_type: TypeRefIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum TestEffectOutcomeIr {
    Respond {
        value: ExprRefIr,
        value_type: TypeRefIr,
    },
    Throw {
        value: ExprRefIr,
        payload_type: TypeRefIr,
    },
    Stream {
        values: Vec<ExprRefIr>,
        item_type: TypeRefIr,
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
    Slot {
        slot: u32,
    },
    /// An Actor's durable field frame. This target is only valid in an
    /// executable owned by an Actor method implementation.
    ActorSelfField {
        field: String,
        field_type: TypeRefIr,
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
    Record { fields: Vec<RecordPatternFieldIr> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordPatternFieldIr {
    pub name: String,
    pub pattern: PatternIr,
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
    /// Exact constant owned by a direct package dependency. The dependency
    /// requirement carries the selected Local ABI and, for top-level test
    /// access, the exact implementation build.
    LoadPackageConst {
        symbol: crate::symbols::PackageSymbolRef,
    },
    /// Reads the current Actor instance's durable field frame. The linker
    /// validates that only the owning Actor method can carry this expression.
    ActorSelfField {
        field: String,
        field_type: TypeRefIr,
    },
    Field {
        object: ExprRefIr,
        field: String,
    },
    Index {
        object: ExprRefIr,
        index: ExprRefIr,
    },
    Construct {
        type_ref: TypeRefIr,
        fields: BTreeMap<String, ExprRefIr>,
    },
    RepresentationWrap {
        value: ExprRefIr,
        type_ref: TypeRefIr,
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
        site: InstructionSourceSite,
    },
    Rethrow {
        exception_slot: u32,
    },
    Catch {
        try_expression: ExprRefIr,
        catch_slot: u32,
        catch_type: TypeRefIr,
        body: ExprRefIr,
    },
    Timeout {
        duration_ms: u64,
        value: ExprRefIr,
        site: InstructionSourceSite,
    },
    ValueBlock {
        block: String,
        result: ExprRefIr,
    },
    ConcurrentValue {
        plan: ConcurrentPlanIr,
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
#[serde(rename_all = "camelCase", deny_unknown_fields, try_from = "CallIrWire")]
pub struct CallIr {
    pub target: CallTargetIr,
    /// Exact instantiated Self type for receiver-bound direct targets. The
    /// field is required and serializes as `null` for all other calls.
    pub concrete_receiver: Option<TypeRefIr>,
    pub site: InstructionSourceSite,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<ExprRefIr>,
    /// Writable places passed `inout` at this call site. Positions align with
    /// the callee's `ParamIr.mode`; the runtime legacy does not execute inout
    /// calls, the representation exists for the emitter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inout_args: Vec<InOutArgIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub type_args: BTreeMap<String, TypeRefIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, MetadataValue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CallIrWire {
    target: CallTargetIr,
    #[serde(deserialize_with = "deserialize_required_option")]
    concrete_receiver: Option<TypeRefIr>,
    site: InstructionSourceSite,
    #[serde(default)]
    args: Vec<ExprRefIr>,
    #[serde(default)]
    inout_args: Vec<InOutArgIr>,
    #[serde(default)]
    type_args: BTreeMap<String, TypeRefIr>,
    #[serde(default)]
    metadata: BTreeMap<String, MetadataValue>,
}

impl TryFrom<CallIrWire> for CallIr {
    type Error = String;

    fn try_from(wire: CallIrWire) -> Result<Self, Self::Error> {
        if wire.concrete_receiver.is_some()
            && !matches!(
                &wire.target,
                CallTargetIr::LocalExecutable { .. }
                    | CallTargetIr::PublicationExecutable { .. }
                    | CallTargetIr::PackageCallable { .. }
            )
        {
            return Err(
                "concreteReceiver is only valid for a direct executable/package call".to_string(),
            );
        }
        Ok(Self {
            target: wire.target,
            concrete_receiver: wire.concrete_receiver,
            site: wire.site,
            args: wire.args,
            inout_args: wire.inout_args,
            type_args: wire.type_args,
            metadata: wire.metadata,
        })
    }
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

/// One `inout <place>` argument: the caller writable root slot and the exact
/// selector path into it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InOutArgIr {
    /// Callee parameter ordinal. `CallIr.args` remains the compact value
    /// stream, so this coordinate cannot be inferred from vector position.
    pub parameter_ordinal: u32,
    pub root_slot: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<InOutPathSegmentIr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum InOutPathSegmentIr {
    Field { name: String },
    Index { selector: ExprRefIr },
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
    PublicationExecutable {
        module_path: String,
        executable_index: u32,
    },
    ServiceDependencySymbol {
        symbol: ServiceDependencySymbolRef,
    },
    /// Canonical service boundary call. The full fact has one owner in the
    /// containing FileIrUnit external-ref table.
    ServiceCall {
        service_call_ref_index: ServiceCallRefIndex,
    },
    /// Canonical direct call into a package dependency. Local ABI expectations
    /// remain owned by the matching package requirement.
    PackageCallable {
        package_ref: PackageRefIr,
        package_callable_id: PackageCallableId,
    },
    /// Canonical Actor boundary dispatch. The declaration and method tables
    /// remain owned by the Actor declaration; a call only pins their identities.
    ActorMethod {
        actor: ServiceSymbolRef,
        actor_abi_identity: ActorAbiIdentity,
        actor_implementation_identity: ActorImplementationIdentity,
        method_identity: ActorMethodIdentity,
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
    /// Exact same-Runtime callback invocation. The requirement rows are
    /// carried here so the emitter does not need to reconstruct a local
    /// concrete implementation table for a callback carrier.
    CallbackMethod {
        interface: InterfaceInstantiationRef,
        method_abi_id: String,
        slot: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        methods: Vec<CallbackInterfaceMethodIr>,
    },
}

/// One exact callback interface requirement row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallbackInterfaceMethodIr {
    pub slot: u32,
    pub method_abi_id: String,
    pub signature: InterfaceMethodSlotSignatureIr,
    pub effects: CallableEffectSummary,
}

pub(crate) fn visit_executable_type_refs<E>(
    executable: &ExecutableIr,
    visitor: &mut impl FnMut(&TypeRefIr) -> Result<(), E>,
) -> Result<(), E> {
    for parameter in &executable.params {
        visit_type_ref(&parameter.ty, visitor)?;
    }
    visit_type_ref(&executable.return_type, visitor)?;
    if let Some(self_type) = &executable.self_type {
        visit_type_ref(self_type, visitor)?;
    }
    visit_executable_body_type_refs(&executable.body, visitor)
}

pub(crate) fn visit_executable_body_type_refs<E>(
    body: &ExecutableBody,
    visitor: &mut impl FnMut(&TypeRefIr) -> Result<(), E>,
) -> Result<(), E> {
    for statement in &body.statements {
        visit_statement_type_refs(statement, visitor)?;
    }
    for expression in &body.expressions {
        visit_expression_type_refs(expression, visitor)?;
    }
    Ok(())
}

fn visit_statement_type_refs<E>(
    statement: &StmtIr,
    visitor: &mut impl FnMut(&TypeRefIr) -> Result<(), E>,
) -> Result<(), E> {
    match statement {
        StmtIr::Assign {
            target: AssignTargetIr::ActorSelfField { field_type, .. },
            ..
        } => visit_type_ref(field_type, visitor)?,
        StmtIr::ForIn {
            item_type: Some(item_type),
            ..
        } => visit_type_ref(item_type, visitor)?,
        StmtIr::Match { arms, .. } => {
            for arm in arms {
                if let PatternIr::Type { ty } = &arm.pattern {
                    visit_type_ref(ty, visitor)?;
                }
            }
        }
        StmtIr::TestEffectRegister {
            expect,
            step_expect,
            outcome,
            ..
        } => {
            for expected in expect.iter().chain(step_expect.iter()) {
                visit_type_ref(&expected.request_type, visitor)?;
            }
            match outcome {
                TestEffectOutcomeIr::Respond { value_type, .. } => {
                    visit_type_ref(value_type, visitor)?;
                }
                TestEffectOutcomeIr::Throw { payload_type, .. } => {
                    visit_type_ref(payload_type, visitor)?;
                }
                TestEffectOutcomeIr::Stream { item_type, .. } => {
                    visit_type_ref(item_type, visitor)?;
                }
            }
        }
        StmtIr::Throw { payload_type, .. } => visit_type_ref(payload_type, visitor)?,
        StmtIr::InitSlot { .. }
        | StmtIr::Assign { .. }
        | StmtIr::Timeout { .. }
        | StmtIr::Concurrent { .. }
        | StmtIr::If { .. }
        | StmtIr::ForIn { .. }
        | StmtIr::While { .. }
        | StmtIr::Assert { .. }
        | StmtIr::Break
        | StmtIr::Continue
        | StmtIr::Dispatch { .. }
        | StmtIr::Emit { .. }
        | StmtIr::Expr { .. }
        | StmtIr::Return { .. }
        | StmtIr::Rethrow { .. } => {}
    }
    Ok(())
}

fn visit_expression_type_refs<E>(
    expression: &ExprIr,
    visitor: &mut impl FnMut(&TypeRefIr) -> Result<(), E>,
) -> Result<(), E> {
    match expression {
        ExprIr::ActorSelfField { field_type, .. } => visit_type_ref(field_type, visitor)?,
        ExprIr::Construct { type_ref, .. } => visit_type_ref(type_ref, visitor)?,
        ExprIr::RepresentationWrap { type_ref, .. } => visit_type_ref(type_ref, visitor)?,
        ExprIr::InterfaceBox {
            interface, source, ..
        } => {
            visit_interface_type_args(interface, visitor)?;
            visit_box_source_type_refs(source, visitor)?;
        }
        ExprIr::Call { call } => {
            if let Some(receiver) = &call.concrete_receiver {
                visit_type_ref(receiver, visitor)?;
            }
            for argument in call.type_args.values() {
                visit_type_ref(argument, visitor)?;
            }
            match &call.target {
                CallTargetIr::InterfaceMethod { interface, .. } => {
                    visit_interface_type_args(interface, visitor)?;
                }
                CallTargetIr::CallbackMethod {
                    interface, methods, ..
                } => {
                    visit_interface_type_args(interface, visitor)?;
                    for method in methods {
                        for parameter in &method.signature.params {
                            visit_type_ref(&parameter.ty, visitor)?;
                        }
                        visit_type_ref(&method.signature.return_type, visitor)?;
                    }
                }
                _ => {}
            }
        }
        ExprIr::Throw { payload_type, .. } => visit_type_ref(payload_type, visitor)?,
        ExprIr::Catch { catch_type, .. } => visit_type_ref(catch_type, visitor)?,
        ExprIr::DbOperation { operation } => {
            visit_db_target_type_refs(&operation.target, visitor)?;
            visit_type_ref(&operation.result_type, visitor)?;
        }
        ExprIr::DbQuery { query } => {
            visit_db_target_type_refs(&query.target, visitor)?;
            visit_type_ref(&query.result_type, visitor)?;
        }
        ExprIr::DbTransaction { transaction } => {
            visit_type_ref(&transaction.result_type, visitor)?;
        }
        ExprIr::DbLeaseClaim { claim } => {
            visit_db_target_type_refs(&claim.target, visitor)?;
            visit_type_ref(&claim.result_type, visitor)?;
        }
        ExprIr::DbLeaseRead { read } => {
            visit_db_target_type_refs(&read.target, visitor)?;
            visit_type_ref(&read.result_type, visitor)?;
        }
        ExprIr::Literal { .. }
        | ExprIr::LoadSlot { .. }
        | ExprIr::LoadConst { .. }
        | ExprIr::LoadPackageConst { .. }
        | ExprIr::Field { .. }
        | ExprIr::Index { .. }
        | ExprIr::MapLiteral { .. }
        | ExprIr::ArrayLiteral { .. }
        | ExprIr::Unary { .. }
        | ExprIr::Binary { .. }
        | ExprIr::Rethrow { .. }
        | ExprIr::Timeout { .. }
        | ExprIr::ValueBlock { .. }
        | ExprIr::ConcurrentValue { .. } => {}
    }
    Ok(())
}

fn visit_db_target_type_refs<E>(
    target: &DbTargetIr,
    visitor: &mut impl FnMut(&TypeRefIr) -> Result<(), E>,
) -> Result<(), E> {
    visit_type_ref(&target.type_ref, visitor)
}

fn visit_box_source_type_refs<E>(
    source: &BoxSourceIr,
    visitor: &mut impl FnMut(&TypeRefIr) -> Result<(), E>,
) -> Result<(), E> {
    match source {
        BoxSourceIr::Local {
            concrete_type,
            method_table,
        } => {
            visit_type_ref(concrete_type, visitor)?;
            visit_interface_type_args(&method_table.interface, visitor)?;
            visit_type_ref(&method_table.concrete_type, visitor)?;
            for slot in &method_table.slots {
                visit_interface_method_signature_type_refs(&slot.signature, visitor)?;
            }
        }
        BoxSourceIr::Remote { operations, .. } => {
            visit_interface_type_args(&operations.interface, visitor)?;
            for slot in &operations.slots {
                visit_interface_method_signature_type_refs(&slot.signature, visitor)?;
            }
        }
    }
    Ok(())
}

fn visit_interface_method_signature_type_refs<E>(
    signature: &InterfaceMethodSlotSignatureIr,
    visitor: &mut impl FnMut(&TypeRefIr) -> Result<(), E>,
) -> Result<(), E> {
    for parameter in &signature.params {
        visit_type_ref(&parameter.ty, visitor)?;
    }
    visit_type_ref(&signature.return_type, visitor)
}

fn visit_interface_type_args<E>(
    interface: &InterfaceInstantiationRef,
    visitor: &mut impl FnMut(&TypeRefIr) -> Result<(), E>,
) -> Result<(), E> {
    for argument in &interface.canonical_type_args {
        visit_type_ref(argument, visitor)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
