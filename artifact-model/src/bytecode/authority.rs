//! Canonical bytecode execution authorities derived from the immutable
//! artifact during structural validation.

use serde::{Deserialize, Serialize};

use crate::types::TypeRefIr;

use super::dto::{BytecodeIntrinsicRef, ResumeErrorMode, ValueTransferPlan};

/// Exact authority for a function that produces `Stream<T>`.
///
/// The item type and lifecycle plan are explicit in the validated projection.
/// Natural stream end is a declared contract, never inferred by the consumer.
/// The authority comes only from `FrameLayout::stream_result_type_ref`; an
/// ordinary result frame is never inferred to be a producer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FunctionStreamItemAuthority {
    pub function_key: String,
    /// Pool index of the exact `Stream<T>` function result type.
    pub stream_result_type_ref: u32,
    /// Exact item type `T`, retained inline so linker/verifier do not have to
    /// derive it from the stream wrapper.
    pub item_type: TypeRefIr,
    /// Item lifecycle plan in result-transfer order.
    pub item_plan: ValueTransferPlan,
    /// Natural stream end plan.
    pub end: FunctionStreamEndContract,
}

/// How a producer function ends its stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FunctionStreamEndContract {
    /// Normal function exit is an explicit stream end with no payload.
    NormalExit,
    /// No canonical end authority is available; consumers must fail closed.
    Unavailable,
}

/// Resume authority for an intrinsic adapter call.
///
/// `Never` is a verified no-pending claim. `Pending` names the artifact resume
/// descriptor that supplies stack height, result type refs/plans and error
/// mode. `Unavailable` is the fail-closed state when an artifact claims
/// pending effects without the exact continuation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum IntrinsicResumeContract {
    Never,
    Pending { resume_descriptor_index: u32 },
    Unavailable,
}

/// Exact result and continuation plan for one validated `IntrinsicRef`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntrinsicAdapterResultPlan {
    pub result_types: Vec<TypeRefIr>,
    pub result_plans: Vec<ValueTransferPlan>,
    pub resume: IntrinsicResumeContract,
}

/// Validated intrinsic target contract bound to its relocation row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidatedIntrinsicContract {
    pub function_key: String,
    pub relocation_index: u32,
    pub target: BytecodeIntrinsicRef,
    pub plan: IntrinsicAdapterResultPlan,
}

/// Validated stream producer contract bound to its function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidatedFunctionStreamItem {
    pub function_key: String,
    pub authority: FunctionStreamItemAuthority,
}

/// Exact pending-site result authority for a `VmResumeToken` mint.
///
/// The artifact already stores the ordinary result plan on
/// [`ResumeDescriptor`](super::dto::ResumeDescriptor); this projection binds
/// it to the opcode-specific stream authority when present.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidatedResumeResultAuthority {
    pub descriptor_index: u32,
    pub end_resume_pc: Option<u32>,
    pub expected_stack_height_before_result: u32,
    pub result_type_refs: Vec<u32>,
    pub result_plans: Vec<ValueTransferPlan>,
    pub error_mode: ResumeErrorMode,
    pub stream_item: Option<FunctionStreamItemAuthority>,
}
