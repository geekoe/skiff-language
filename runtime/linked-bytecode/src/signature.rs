use std::fmt;

use skiff_artifact_model::{CallableEffectSummary, CallableMayEffects, ParamModeIr};

use crate::{LinkedValueTransferPlan, TypeIndex};

/// Concrete, declarative signature facts carried by a bytecode callable.
/// These facts remain untrusted until independently matched and recomputed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedCallableSignature {
    parameter_types: Box<[TypeIndex]>,
    parameter_modes: Box<[ParamModeIr]>,
    parameter_plans: Box<[LinkedValueTransferPlan]>,
    result_types: Box<[TypeIndex]>,
    result_plans: Box<[LinkedValueTransferPlan]>,
    effect_summary: CallableEffectSummary,
}

impl LinkedCallableSignature {
    pub fn new(
        parameter_types: Box<[TypeIndex]>,
        parameter_modes: Box<[ParamModeIr]>,
        parameter_plans: Box<[LinkedValueTransferPlan]>,
        result_types: Box<[TypeIndex]>,
        result_plans: Box<[LinkedValueTransferPlan]>,
        effect_summary: CallableEffectSummary,
    ) -> Result<Self, LinkedCallableSignatureError> {
        validate_shape(
            parameter_types.len(),
            parameter_modes.len(),
            parameter_plans.len(),
            result_types.len(),
            result_plans.len(),
        )?;
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

    pub fn parameter_plans(&self) -> &[LinkedValueTransferPlan] {
        &self.parameter_plans
    }

    pub fn result_types(&self) -> &[TypeIndex] {
        &self.result_types
    }

    pub fn result_plans(&self) -> &[LinkedValueTransferPlan] {
        &self.result_plans
    }

    pub const fn effect_summary(&self) -> &CallableEffectSummary {
        &self.effect_summary
    }
}

/// Exact instantiated signature declared by a host or intrinsic registry
/// target. Unlike a bytecode callable summary, this cannot be `Unknown`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedNativeCallableSignature {
    parameter_types: Box<[TypeIndex]>,
    parameter_modes: Box<[ParamModeIr]>,
    parameter_plans: Box<[LinkedValueTransferPlan]>,
    result_types: Box<[TypeIndex]>,
    result_plans: Box<[LinkedValueTransferPlan]>,
    effects: CallableMayEffects,
}

impl LinkedNativeCallableSignature {
    pub fn new(
        parameter_types: Box<[TypeIndex]>,
        parameter_modes: Box<[ParamModeIr]>,
        parameter_plans: Box<[LinkedValueTransferPlan]>,
        result_types: Box<[TypeIndex]>,
        result_plans: Box<[LinkedValueTransferPlan]>,
        effects: CallableMayEffects,
    ) -> Result<Self, LinkedCallableSignatureError> {
        validate_shape(
            parameter_types.len(),
            parameter_modes.len(),
            parameter_plans.len(),
            result_types.len(),
            result_plans.len(),
        )?;
        Ok(Self {
            parameter_types,
            parameter_modes,
            parameter_plans,
            result_types,
            result_plans,
            effects,
        })
    }

    pub fn parameter_types(&self) -> &[TypeIndex] {
        &self.parameter_types
    }

    pub fn parameter_modes(&self) -> &[ParamModeIr] {
        &self.parameter_modes
    }

    pub fn parameter_plans(&self) -> &[LinkedValueTransferPlan] {
        &self.parameter_plans
    }

    pub fn result_types(&self) -> &[TypeIndex] {
        &self.result_types
    }

    pub fn result_plans(&self) -> &[LinkedValueTransferPlan] {
        &self.result_plans
    }

    pub const fn effects(&self) -> &CallableMayEffects {
        &self.effects
    }
}

fn validate_shape(
    parameter_type_count: usize,
    parameter_mode_count: usize,
    parameter_plan_count: usize,
    result_type_count: usize,
    result_plan_count: usize,
) -> Result<(), LinkedCallableSignatureError> {
    if parameter_type_count != parameter_mode_count {
        return Err(LinkedCallableSignatureError::ParameterModeCountMismatch {
            parameter_type_count,
            parameter_mode_count,
        });
    }
    if parameter_type_count != parameter_plan_count {
        return Err(LinkedCallableSignatureError::ParameterPlanCountMismatch {
            parameter_type_count,
            parameter_plan_count,
        });
    }
    if result_type_count != result_plan_count {
        return Err(LinkedCallableSignatureError::ResultPlanCountMismatch {
            result_type_count,
            result_plan_count,
        });
    }
    Ok(())
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
