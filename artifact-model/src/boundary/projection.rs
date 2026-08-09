use std::collections::BTreeMap;
use std::fmt;

use serde::{de, Deserialize, Deserializer, Serialize};

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
    Redis,
    Actor,
    Queue,
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
        direct_return_origins: Vec<ValueProvenance>,
        throw_origins: Vec<ValueProvenance>,
        escape_lanes: Vec<ValueEscapeLane>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallableProvenanceUnknownReason {
    AnalysisPending,
    UnsupportedControlFlow,
    UnsupportedHeapStore,
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
    CallerParameter {
        index: u32,
    },
    CallerParameterProjection {
        index: u32,
        path: ValueProjectionPath,
    },
    DependencyReturn {
        callable_id: String,
    },
}

pub const MAX_VALUE_PROJECTION_PATH_STEPS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ValueProjectionStep {
    Field { name: String },
    ContainerElement {},
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueProjectionPath {
    steps: Vec<ValueProjectionStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueProjectionPathError {
    message: String,
}

impl ValueProjectionPath {
    pub fn new(steps: Vec<ValueProjectionStep>) -> Result<Self, ValueProjectionPathError> {
        validate_projection_steps(&steps)?;
        Ok(Self { steps })
    }

    pub fn field(name: impl Into<String>) -> Result<Self, ValueProjectionPathError> {
        Self::new(vec![ValueProjectionStep::Field { name: name.into() }])
    }

    pub fn container_element() -> Self {
        Self {
            steps: vec![ValueProjectionStep::ContainerElement {}],
        }
    }

    pub fn steps(&self) -> &[ValueProjectionStep] {
        &self.steps
    }

    pub fn appended(&self, suffix: &ValueProjectionPath) -> Result<Self, ValueProjectionPathError> {
        let mut steps = Vec::with_capacity(self.steps.len().saturating_add(suffix.steps.len()));
        steps.extend(self.steps.iter().cloned());
        steps.extend(suffix.steps.iter().cloned());
        Self::new(steps)
    }
}

impl fmt::Display for ValueProjectionPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ValueProjectionPathError {}

impl<'de> Deserialize<'de> for ValueProjectionPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            steps: Vec<ValueProjectionStep>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.steps).map_err(de::Error::custom)
    }
}

fn validate_projection_steps(
    steps: &[ValueProjectionStep],
) -> Result<(), ValueProjectionPathError> {
    if steps.is_empty() {
        return Err(ValueProjectionPathError {
            message: "value projection path must contain at least one step".to_string(),
        });
    }
    if steps.len() > MAX_VALUE_PROJECTION_PATH_STEPS {
        return Err(ValueProjectionPathError {
            message: format!(
                "value projection path exceeds {MAX_VALUE_PROJECTION_PATH_STEPS} steps"
            ),
        });
    }
    for step in steps {
        if let ValueProjectionStep::Field { name } = step {
            if name.is_empty() || name.trim() != name {
                return Err(ValueProjectionPathError {
                    message: "value projection field name must be non-empty and unpadded"
                        .to_string(),
                });
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueEscapeLane {
    Capture,
    Callback,
    Stream,
    Dispatch,
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
// This is a cold-path typed artifact DTO; boxing a variant would change its
// public construction API solely to optimize a non-hot representation.
#[allow(clippy::large_enum_variant)]
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
    EscapesCallerValue {
        lane: ValueEscapeLane,
    },
    RequiresSameHeapIdentity,
    CallbackAdapterUnavailable,
    NativeAdapterUnavailable,
    UnsupportedBoundaryType,
    UnsupportedStream,
    /// A callable carries an `inout` parameter. Inout loans cannot cross the
    /// service boundary (value materialization always applies there).
    InOutNotAllowedAtServiceBoundary,
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
