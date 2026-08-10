//! Typed MIR/CFG over File IR (Phase 2 WP4).
//!
//! MIR is the emitter's only source-program semantic input
//! (`emit_bytecode_artifact` in `compiler/emission`). Construction is a
//! post-pass over each `FileIrUnit` plus source-owned facts (expression types,
//! callable effects, spans); after construction, an emitter must not reopen a
//! `FileIrUnit` to recover types, expressions, external references, liveness,
//! effects, or source facts (design §2.4 stop condition).
//!
//! Frozen constant graphs are deliberately not embedded in [`MirUnit`]. They
//! are produced by [`crate::ConstEvaluator`] as a unit-owned
//! [`crate::FrozenConstantBundle`] whose module owner and type/shape pools are
//! a separate, explicit emitter input. Likewise, this C0 contract does not
//! invent a `ValueTransferPlan`: a later source-owned fact must supply that
//! plan before emission. An emitter must fail closed when it is absent and
//! must never default a transfer to `SnapshotShare`.
//!
//! # Construction rules
//!
//! - One [`MirUnit`] per `FileIrUnit`; one [`MirFunction`] per File IR
//!   executable (const initializers are not MIR functions: Phase 2 keeps them
//!   as request-time bodies until the const evaluator freezes them).
//! - Blocks are derived from File IR blocks by basic-block splitting: every
//!   branch statement (`If`/`ForIn`/`While`/`Match`/`Timeout`/`Concurrent`)
//!   and every terminator (`Return`/`Throw`/`Rethrow`/`Break`/`Continue`)
//!   closes a block. Statements after a terminator in the same File IR block
//!   are unreachable and are dropped from the CFG.
//! - Branch statements keep their structured form but reference targets by
//!   block id; each block also carries its complete successor edge set, so the
//!   emitter can linearize without re-deriving control flow.
//! - `Break`/`Continue` resolve to the nearest enclosing loop's exit /
//!   header block. Expression-referenced blocks (`ValueBlock`, `DbTransaction`,
//!   `DbLeaseClaim`, `ConcurrentValue` lanes) complete into the enclosing
//!   statement's continuation.
//! - Exception regions: every `ExprIr::Catch` produces one [`MirRegion`].
//!   `catch_expr` is the function-local [`MirExpression`] index of the `Catch`
//!   node (kept exactly aligned while cloning File IR); `cleanup_depth` is the
//!   number of enclosing catch regions (nesting level, deterministic from the
//!   expression DAG).
//! - `MirFunction.statements` holds one [`MirStatementEntry`] per `MirStmt`,
//!   in the order produced by flattening `blocks` in block-id order, giving a
//!   recoverable correspondence between `MirStmt`s and the File IR statement
//!   stream (`statement_index` is the index into `ExecutableBody.statements`).
//! - Assignment targets and exactly-known mutating receiver calls carry a
//!   checked [`MirWritablePlace`]. Direct calls own a dense exact-parameter
//!   [`MirDirectCallFacts`] table; inout loans retain callee parameter
//!   ordinals, full paths, and typed function-owned index selectors.
//! - Every bracket segment owns one [`MirIndexAccessFacts`] keyed by its
//!   single-evaluation selector expression. Receiver kind, receiver/result
//!   types, read/write/loan policy, and source span come from the source
//!   model; MIR validates all-and-only coverage and never guesses them from a
//!   `TypeRefIr` or CFG shape.
//! - Timeout/concurrent statements retain their continuation/join block and a
//!   concurrent plan retains its plan-level [`InstructionSourceSite`]. File IR
//!   block labels referenced by inline `ValueBlock`/DB/concurrent expressions
//!   do not identify an exact instruction-level return point, so MIR does not
//!   expose those as an exact continuation fact. No per-region cleanup or
//!   pending fact exists upstream, so MIR does not invent one.
//! - Recursive `PatternIr::Record` trees and binding slots are validated
//!   exactly. Source nominal-pattern fields are not present in File IR
//!   (`PatternIr::Type` retains only the nominal type), so no nested nominal
//!   pattern fact is claimed here.
//! - Every File IR `statement_spans` entry and every explicit
//!   `InstructionSourceSite` (including its finite synthetic reason) is
//!   retained. File IR has no all-expression site table: ordinary expression
//!   source/synthetic origins and assert-specific origins therefore remain an
//!   upstream contract gap rather than an inferred MIR value.
//!
//! # Callable identity conventions
//!
//! - `symbol` is the File IR executable symbol
//!   (`{module_path}.{declaration}`). After requiring and stripping that exact
//!   module prefix, the bytecode function key is
//!   `"{module_path}::{declaration}"`. This is also the sole key spelling in
//!   frozen `Behavior` nodes; `"{module_path}::{symbol}"` would duplicate the
//!   module and is invalid (design §2.6).
//! - `effect_summary_ref` is the canonical typed package implementation
//!   callable identity from
//!   [`skiff_compiler_core::implementation_package_callable_id`]. The full
//!   effect summary comes from source-owned
//!   [`skiff_compiler_source::SourceCallableEffectFacts`] and
//!   [`MirFunction::may_pending`] derives conservatively from that summary;
//!   `Unknown` is always pending.
//!
//! File layout and API are owned by the WP4 worker (see
//! `doc/implementation/bytecode-vm/design/phase-2-compiler-emission.md` §2.4).

mod abi;
mod contract;
mod events;
mod facts;
mod index;

pub mod builder;
pub mod liveness;

pub use abi::{MirCallArgument, MirDirectCallFacts, MirReceiverFacts};
pub use contract::{MirBuildError, MirContractError};
pub(crate) use events::{
    finalize_mir_source_event_plan, ExpressionEventKind, MirSourceEventCollector,
};
pub use events::{
    MirControlFlowEdge, MirEmissionAnchor, MirSourceEvent, MirSourceEventPlan,
    MirSourceEventPlanError, MirSourceEventUnavailableReason, MirStatementPlacement,
};
pub use facts::{
    MirCallWritableFacts, MirForInBinding, MirForInFacts, MirForInItemKind, MirInOutLoan,
    MirInOutPathSegment, MirWritablePathSegment, MirWritablePlace, MirWritableRoot,
};
pub use index::{MirIndexAccessFacts, MirIndexPolicy, MirIndexReceiverKind, MirSourceFacts};

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    ActorDeclarationIr, AssignTargetIr, CallableEffectSummary, ExprIr, ExprRefIr, ExternalRefTable,
    FileLinkTargets, InstructionSourceSite, PackageCallableId, PackageExecutableCoordinate,
    PatternIr, SourceMapDto, SourceSpanRef, TypeDeclIr, TypeRefIr,
};

/// One `FileIrUnit`'s self-contained typed CFG. Pure in-memory; never
/// serialized.
///
/// Expression-owned indices such as `ServiceCallRefIndex` resolve only
/// against this unit's cloned [`ExternalRefTable`]. Local type indices and
/// source spans likewise resolve against this unit's cloned type/source facts.
/// Const graphs and their pool-index owners are a separate explicit
/// [`crate::FrozenConstantBundle`]; an emitter must exact-match its module and
/// must not reopen the source `FileIrUnit` for any of these facts.
#[derive(Debug, Clone, PartialEq)]
pub struct MirUnit {
    pub file_ir_identity: String,
    pub module_path: String,
    /// Complete File IR actor authority retained for checked joins with the
    /// PackageArtifact manifest. MIR does not synthesize actor rows.
    pub actor_declarations: Vec<ActorDeclarationIr>,
    pub external_refs: ExternalRefTable,
    pub source_map: SourceMapDto,
    pub type_table: Vec<TypeDeclIr>,
    pub link_targets: FileLinkTargets,
    /// Dense local-constant metadata. Frozen graphs remain a separate
    /// ConstEvaluator input keyed by `MirConst::symbol`; initializer bodies
    /// are intentionally not copied into MIR.
    pub constants: Vec<MirConst>,
    pub functions: Vec<MirFunction>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MirFunction {
    /// Exact index in the owning File IR executable table. Local/publication
    /// call targets retain this index and resolve it through `MirUnit`.
    pub executable_index: u32,
    pub origin: PackageExecutableCoordinate,
    pub symbol: String,
    pub kind: MirExecutableKind,
    pub type_params: Vec<String>,
    pub params: Vec<MirParam>,
    pub return_type: TypeRefIr,
    pub self_type: Option<TypeRefIr>,
    pub receiver: Option<MirReceiverFacts>,
    pub slots: Vec<MirSlot>,
    /// Exact source bracket facts keyed by the function-owned selector
    /// expression index. Every bracket access in expressions/places/loans is
    /// covered exactly once.
    pub index_accesses: BTreeMap<u32, MirIndexAccessFacts>,
    /// Function-owned expression DAG. Every entry's `index` is exactly its
    /// position and its `ty` is the source-owned type at that index.
    pub expressions: Vec<MirExpression>,
    pub blocks: Vec<MirBlock>,
    pub regions: Vec<MirRegion>,
    /// One entry per `MirStmt` across `blocks` in block-id order
    /// (`statement_index` = index into the File IR statement stream).
    pub statements: Vec<MirStatementEntry>,
    /// Checked final-index source-event placements, or a structured reason
    /// why this executable cannot yet be emitted. Unavailable is never an
    /// alias for an available zero-event plan.
    pub source_event_plan: MirSourceEventPlan,
    pub liveness: MirLiveness,
    pub effect_summary_ref: PackageCallableId,
    pub effect_summary: CallableEffectSummary,
    pub source_span: Option<SourceSpanRef>,
}

/// One exact entry in a function-owned expression DAG.
#[derive(Debug, Clone, PartialEq)]
pub struct MirExpression {
    pub index: u32,
    pub expression: ExprIr,
    pub ty: TypeRefIr,
    /// Checked root/path and loan facts for mutating/inout calls. This remains
    /// `None` for calls without either write channel and for non-call nodes.
    pub writable: Option<MirCallWritableFacts>,
    /// Dense exact ABI facts for Local/Publication/Package direct calls.
    pub direct_call: Option<MirDirectCallFacts>,
}

/// Emitter-facing metadata for one compile-time-evaluated local constant.
/// The frozen graph is supplied separately by [`crate::ConstEvaluator`] under
/// `symbol`; no request-time initializer body is retained here.
#[derive(Debug, Clone, PartialEq)]
pub struct MirConst {
    pub index: u32,
    pub symbol: String,
    pub ty: TypeRefIr,
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirExecutableKind {
    Function,
    ImplMethod,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MirParam {
    pub name: String,
    pub slot: u32,
    pub ty: TypeRefIr,
    pub mode: MirParamMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirParamMode {
    Value,
    InOut,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MirSlot {
    pub slot: u32,
    pub name: String,
    pub kind: MirSlotKind,
    pub writable_local: bool,
    pub ty: Option<TypeRefIr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirSlotKind {
    Param,
    SelfValue,
    Local,
    Temp,
    Pattern,
}

/// Explicit CFG node. `successors` is the complete edge set (branch targets,
/// fall-through continuation, loop-back / loop-exit edges).
#[derive(Debug, Clone, PartialEq)]
pub struct MirBlock {
    pub id: u32,
    pub label: String,
    pub statements: Vec<MirStmt>,
    pub successors: Vec<u32>,
}

/// One CFG-ized File IR statement. `statement_index` restores the
/// correspondence with `ExecutableBody.statements`.
#[derive(Debug, Clone, PartialEq)]
pub struct MirStmt {
    pub statement_index: u32,
    pub span: Option<SourceSpanRef>,
    pub kind: MirStmtKind,
}

/// CFG-ized `StmtIr`: branch statements reference targets by block id;
/// `Jump` is the unconditional fall-through / loop edge.
#[derive(Debug, Clone, PartialEq)]
pub enum MirStmtKind {
    Let {
        slot: u32,
        value: ExprRefIr,
    },
    Assign {
        target: AssignTargetIr,
        /// Exact root/path projection of `target`, checked against this
        /// function's owned expressions and slots during MIR construction.
        place: MirWritablePlace,
        value: ExprRefIr,
    },
    Assert {
        condition: ExprRefIr,
        message: Option<ExprRefIr>,
    },
    Dispatch {
        call: ExprRefIr,
    },
    Emit {
        operation: String,
        value: ExprRefIr,
    },
    TestEffectRegister {
        target: skiff_artifact_model::TestEffectRegisterTargetIr,
        expect: Option<skiff_artifact_model::TestEffectExpectedIr>,
        step_expect: Option<skiff_artifact_model::TestEffectExpectedIr>,
        outcome: skiff_artifact_model::TestEffectOutcomeIr,
    },
    Expr {
        value: ExprRefIr,
    },
    Return {
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
    /// `else_block` is always resolved when the File IR `If` lacks one: the
    /// builder substitutes the statement continuation so the CFG stays
    /// explicit.
    If {
        condition: ExprRefIr,
        then_block: u32,
        else_block: Option<u32>,
    },
    ForIn {
        iterable: ExprRefIr,
        facts: MirForInFacts,
        body: u32,
        continuation: u32,
    },
    While {
        condition: ExprRefIr,
        body: u32,
    },
    Match {
        value: ExprRefIr,
        arms: Vec<MirMatchArmIr>,
    },
    Timeout {
        duration_ms: u64,
        body: u32,
        continuation: u32,
        site: InstructionSourceSite,
    },
    Concurrent {
        plan: MirConcurrentPlanIr,
    },
    Break,
    Continue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MirMatchArmIr {
    pub pattern: PatternIr,
    pub body: u32,
}

/// `ConcurrentPlanIr` with lane bodies resolved to block ids.
#[derive(Debug, Clone, PartialEq)]
pub struct MirConcurrentPlanIr {
    pub lanes: Vec<MirConcurrentLaneIr>,
    /// Exact plan-level source or finite synthetic origin. File IR already
    /// owns this fact; MIR must not drop it while resolving lane block ids.
    pub site: InstructionSourceSite,
    /// Exact continuation reached when statement lanes join.
    pub join_block: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MirConcurrentLaneIr {
    Statement {
        source_order: u32,
        dependencies: Vec<u32>,
        body: u32,
        site: InstructionSourceSite,
    },
    Serial {
        source_order: u32,
        dependencies: Vec<u32>,
        body: u32,
        site: InstructionSourceSite,
    },
    Tail {
        source_order: u32,
        dependencies: Vec<u32>,
        tail: ExprRefIr,
        site: InstructionSourceSite,
    },
}

/// One exception region: the function-local [`MirExpression`] index of the
/// `Catch` node, the slot receiving the caught exception, its static type and
/// the nesting depth (number of enclosing catch regions).
#[derive(Debug, Clone, PartialEq)]
pub struct MirRegion {
    pub id: u32,
    pub catch_expr: u32,
    pub catch_slot: u32,
    pub catch_type: TypeRefIr,
    pub cleanup_depth: u32,
}

/// Source span side-channel for one `MirStmt`, in the same order as the
/// flattened `MirStmt` stream.
#[derive(Debug, Clone, PartialEq)]
pub struct MirStatementEntry {
    pub statement_index: u32,
    pub span: Option<SourceSpanRef>,
}

/// Standard may-liveness (slot granularity) for one `MirFunction`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MirLiveness {
    pub blocks: BTreeMap<u32, MirBlockLiveness>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MirBlockLiveness {
    pub live_in: Vec<u32>,
    pub live_out: Vec<u32>,
}
