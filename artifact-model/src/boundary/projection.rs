use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CallableEffectSummary, CallableMayEffects, ContractOperationId};

use super::BoundaryOperationContract;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryConfigRequirement {
    pub path: String,
    pub value_type: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryStateRequirement {
    pub key: String,
    pub kind: BoundaryStateKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoundaryStateKind {
    Database,
    Actor,
    Queue,
    ExternalResource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CallableProvenanceSummary {
    Unknown {
        reason: CallableProvenanceUnknownReason,
    },
    Analyzed {
        return_origins: Vec<ValueProvenance>,
        throw_origins: Vec<ValueProvenance>,
        escape_lanes: Vec<ValueEscapeLane>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallableProvenanceUnknownReason {
    AnalysisPending,
    UnsupportedControlFlow,
    UnknownCallTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ValueProvenance {
    Fresh,
    Constant,
    CallerParameter { index: u32 },
    DependencyReturn { callable_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueEscapeLane {
    Capture,
    Callback,
    Stream,
    Spawn,
    Database,
    Native,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryImplementationRequirements {
    pub config: Vec<BoundaryConfigRequirement>,
    pub state: Vec<BoundaryStateRequirement>,
    pub native_capabilities: Vec<String>,
    pub runtime_capabilities: Vec<String>,
    pub complete_may_effects: CallableMayEffects,
    pub provenance: CallableProvenanceSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BoundaryCallableProjection {
    Available {
        operation_contract: BoundaryOperationContract,
        implementation_requirements: BoundaryImplementationRequirements,
    },
    Unavailable {
        reasons: Vec<BoundaryUnavailableReason>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BoundaryUnavailableReason {
    AnalysisPending,
    UnknownEffect,
    UnknownCallTarget,
    WritesCallerReachable,
    ReturnsCallerAlias,
    ThrowsCallerAlias,
    EscapesCallerValue { lane: ValueEscapeLane },
    RequiresSameHeapIdentity,
    CallbackAdapterUnavailable,
    NativeAdapterUnavailable,
    UnsupportedBoundaryType,
    UnsupportedStream,
}

/// All semantic facts are explicit even for a boundary-unavailable callable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableSemanticFacts {
    pub effects: CallableEffectSummary,
    pub provenance: CallableProvenanceSummary,
    pub resolved_call_targets: BTreeMap<u32, CallableTargetFact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CallableTargetFact {
    PackageDirect { package_callable_id: String },
    ContractOperation { operation_id: ContractOperationId },
    Unknown,
}
