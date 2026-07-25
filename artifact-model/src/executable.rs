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
    ExternalServiceSymbol {
        symbol: ServiceSymbolRef,
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
}
