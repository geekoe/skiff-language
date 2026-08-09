use std::fmt;

use skiff_artifact_model::{CallableEffectSummary, ParamModeIr, ValueTransferPlanKind};

use crate::TypeIndex;

/// Concrete, declarative signature facts carried by a non-local call target.
/// The independent verifier must compare these untrusted facts with the call
/// instruction, target contract and reachable effects before sealing an image.
/// Parameter modes are retained exactly so boundary targets can reject
/// unsupported `inout` parameters instead of treating them as values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedCallableSignature {
    parameter_types: Box<[TypeIndex]>,
    parameter_modes: Box<[ParamModeIr]>,
    parameter_plans: Box<[ValueTransferPlanKind]>,
    result_types: Box<[TypeIndex]>,
    result_plans: Box<[ValueTransferPlanKind]>,
    effect_summary: CallableEffectSummary,
}

impl LinkedCallableSignature {
    pub fn new(
        parameter_types: Box<[TypeIndex]>,
        parameter_modes: Box<[ParamModeIr]>,
        parameter_plans: Box<[ValueTransferPlanKind]>,
        result_types: Box<[TypeIndex]>,
        result_plans: Box<[ValueTransferPlanKind]>,
        effect_summary: CallableEffectSummary,
    ) -> Result<Self, LinkedCallableSignatureError> {
        if parameter_types.len() != parameter_modes.len() {
            return Err(LinkedCallableSignatureError::ParameterModeCountMismatch {
                parameter_type_count: parameter_types.len(),
                parameter_mode_count: parameter_modes.len(),
            });
        }
        if parameter_types.len() != parameter_plans.len() {
            return Err(LinkedCallableSignatureError::ParameterPlanCountMismatch {
                parameter_type_count: parameter_types.len(),
                parameter_plan_count: parameter_plans.len(),
            });
        }
        if result_types.len() != result_plans.len() {
            return Err(LinkedCallableSignatureError::ResultPlanCountMismatch {
                result_type_count: result_types.len(),
                result_plan_count: result_plans.len(),
            });
        }
        Ok(Self {
            parameter_types,
            parameter_modes,
            parameter_plans,
            result_types,
            result_plans,
            effect_summary,
        })
    }

    pub fn parameter_types(&self) -> &[TypeIndex] {
        &self.parameter_types
    }

    pub fn parameter_modes(&self) -> &[ParamModeIr] {
        &self.parameter_modes
    }

    pub fn parameter_plans(&self) -> &[ValueTransferPlanKind] {
        &self.parameter_plans
    }

    pub fn result_types(&self) -> &[TypeIndex] {
        &self.result_types
    }

    pub fn result_plans(&self) -> &[ValueTransferPlanKind] {
        &self.result_plans
    }

    pub const fn effect_summary(&self) -> &CallableEffectSummary {
        &self.effect_summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedCallableSignatureError {
    ParameterModeCountMismatch {
        parameter_type_count: usize,
        parameter_mode_count: usize,
    },
    ParameterPlanCountMismatch {
        parameter_type_count: usize,
        parameter_plan_count: usize,
    },
    ResultPlanCountMismatch {
        result_type_count: usize,
        result_plan_count: usize,
    },
}

impl fmt::Display for LinkedCallableSignatureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParameterModeCountMismatch {
                parameter_type_count,
                parameter_mode_count,
            } => write!(
                formatter,
                "callable has {parameter_type_count} parameter types but {parameter_mode_count} parameter modes"
            ),
            Self::ParameterPlanCountMismatch {
                parameter_type_count,
                parameter_plan_count,
            } => write!(
                formatter,
                "callable has {parameter_type_count} parameter types but {parameter_plan_count} parameter plans"
            ),
            Self::ResultPlanCountMismatch {
                result_type_count,
                result_plan_count,
            } => write!(
                formatter,
                "callable has {result_type_count} result types but {result_plan_count} result plans"
            ),
        }
    }
}

impl std::error::Error for LinkedCallableSignatureError {}
