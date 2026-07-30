use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    actor_declaration::{ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity},
    builtin_receiver_ops::BuiltinReceiverOp,
    compile_identity::PackageCallableId,
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

#[cfg(test)]
mod timeout_execution_tests;

/// Largest persisted duration that runtime execution may admit without losing
/// integer precision in JavaScript consumers.
pub const MAX_SAFE_EXECUTION_DURATION_MILLISECONDS: u64 = 9_007_199_254_740_991;

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
pub enum StmtIr {
    Let {
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallIr {
    pub target: CallTargetIr,
    pub site: InstructionSourceSite,
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
        StmtIr::Let { .. }
        | StmtIr::Assign { .. }
        | StmtIr::Timeout { .. }
        | StmtIr::Concurrent { .. }
        | StmtIr::If { .. }
        | StmtIr::ForIn { .. }
        | StmtIr::Assert { .. }
        | StmtIr::Break
        | StmtIr::Continue
        | StmtIr::Spawn { .. }
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
            for argument in call.type_args.values() {
                visit_type_ref(argument, visitor)?;
            }
            if let CallTargetIr::InterfaceMethod { interface, .. } = &call.target {
                visit_interface_type_args(interface, visitor)?;
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
mod tests {
    use serde_json::json;

    use super::*;
    use crate::refs::SourcePosition;

    fn source_site() -> InstructionSourceSite {
        InstructionSourceSite::Source {
            span: SourceSpanRef {
                source_id: 3,
                start: SourcePosition::new(8, 2),
                end: SourcePosition::new(8, 11),
            },
        }
    }

    #[test]
    fn throw_and_call_round_trip_required_source_sites() {
        let statement = StmtIr::Throw {
            value: ExprRefIr { expression: 1 },
            payload_type: TypeRefIr::builtin("string"),
            site: source_site(),
        };
        let call = CallIr {
            target: CallTargetIr::LocalExecutable {
                executable_index: 2,
            },
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
            },
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };

        for expected in [
            serde_json::to_value(&statement).unwrap(),
            serde_json::to_value(&call).unwrap(),
        ] {
            assert!(expected.get("site").is_some());
        }
        assert_eq!(
            serde_json::from_value::<StmtIr>(serde_json::to_value(&statement).unwrap()).unwrap(),
            statement
        );
        assert_eq!(
            serde_json::from_value::<CallIr>(serde_json::to_value(&call).unwrap()).unwrap(),
            call
        );
    }

    #[test]
    fn source_owned_instructions_reject_missing_or_invalid_sites() {
        let missing_throw_site = json!({
            "kind": "throw",
            "value": { "expression": 0 },
            "payloadType": { "kind": "builtin", "name": "string" }
        });
        let missing_call_site = json!({
            "target": { "kind": "localExecutable", "executableIndex": 0 }
        });
        let forged_synthetic_source = json!({
            "kind": "synthetic",
            "reason": "compilerDesugaring",
            "span": {
                "sourceId": 1,
                "start": { "line": 1, "column": 1 },
                "end": { "line": 1, "column": 2 }
            }
        });
        let unknown_reason = json!({
            "kind": "synthetic",
            "reason": "futureReason"
        });

        assert!(serde_json::from_value::<StmtIr>(missing_throw_site.clone()).is_err());
        assert!(serde_json::from_value::<ExprIr>(missing_throw_site).is_err());
        assert!(serde_json::from_value::<CallIr>(missing_call_site).is_err());
        assert!(serde_json::from_value::<InstructionSourceSite>(forged_synthetic_source).is_err());
        assert!(serde_json::from_value::<InstructionSourceSite>(unknown_reason).is_err());
    }

    #[test]
    fn catch_type_is_required_and_never_null() {
        let valid = json!({
            "kind": "catch",
            "tryExpression": { "expression": 0 },
            "catchSlot": 1,
            "catchType": { "kind": "builtin", "name": "string" },
            "body": { "expression": 2 }
        });
        assert!(serde_json::from_value::<ExprIr>(valid.clone()).is_ok());

        for replacement in [None, Some(serde_json::Value::Null)] {
            let mut invalid = valid.clone();
            match replacement {
                None => {
                    invalid.as_object_mut().unwrap().remove("catchType");
                }
                Some(value) => invalid["catchType"] = value,
            }
            assert!(serde_json::from_value::<ExprIr>(invalid).is_err());
        }
    }

    #[test]
    fn representation_wrap_has_one_required_wire_shape() {
        let expected = ExprIr::RepresentationWrap {
            value: ExprRefIr { expression: 4 },
            type_ref: TypeRefIr::AppliedNominal {
                base: crate::NominalTypeRefBaseIr::LocalType { type_index: 2 },
                arguments: vec![TypeRefIr::builtin("string")],
            },
        };
        let wire = json!({
            "kind": "representationWrap",
            "value": { "expression": 4 },
            "typeRef": {
                "kind": "appliedNominal",
                "base": { "kind": "localType", "typeIndex": 2 },
                "arguments": [{ "kind": "builtin", "name": "string" }]
            }
        });
        assert_eq!(serde_json::to_value(&expected).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<ExprIr>(wire.clone()).unwrap(),
            expected
        );

        let mut invalid = Vec::new();
        for missing in ["value", "typeRef"] {
            let mut candidate = wire.clone();
            candidate.as_object_mut().unwrap().remove(missing);
            invalid.push(candidate);
        }
        let mut null_type = wire.clone();
        null_type["typeRef"] = serde_json::Value::Null;
        invalid.push(null_type);
        for forbidden in ["display", "fields", "site", "identity"] {
            let mut candidate = wire.clone();
            candidate[forbidden] = json!("forbidden");
            invalid.push(candidate);
        }
        let mut legacy_type = wire;
        legacy_type["type"] = legacy_type["typeRef"].clone();
        legacy_type.as_object_mut().unwrap().remove("typeRef");
        invalid.push(legacy_type);

        for candidate in invalid {
            assert!(
                serde_json::from_value::<ExprIr>(candidate.clone()).is_err(),
                "strict representationWrap wire must reject {candidate}"
            );
        }
    }

    #[test]
    fn representation_wrap_type_visitor_reaches_all_nested_arguments() {
        let nested_argument = TypeRefIr::AppliedNominal {
            base: crate::NominalTypeRefBaseIr::LocalType { type_index: 1 },
            arguments: vec![TypeRefIr::builtin("string")],
        };
        let body = ExecutableBody {
            expressions: vec![ExprIr::RepresentationWrap {
                value: ExprRefIr { expression: 0 },
                type_ref: TypeRefIr::AppliedNominal {
                    base: crate::NominalTypeRefBaseIr::LocalType { type_index: 0 },
                    arguments: vec![nested_argument.clone()],
                },
            }],
            ..ExecutableBody::default()
        };
        let mut visited = Vec::new();
        visit_executable_body_type_refs(&body, &mut |ty| {
            visited.push(ty.clone());
            Ok::<(), ()>(())
        })
        .unwrap();

        assert_eq!(visited.len(), 3);
        assert!(matches!(
            &visited[0],
            TypeRefIr::AppliedNominal {
                base: crate::NominalTypeRefBaseIr::LocalType { type_index: 0 },
                ..
            }
        ));
        assert_eq!(visited[1], nested_argument);
        assert_eq!(visited[2], TypeRefIr::builtin("string"));
    }
}
