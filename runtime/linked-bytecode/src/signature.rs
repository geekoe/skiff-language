use std::fmt;

use skiff_artifact_model::{CallableEffectSummary, ValueTransferPlanKind};

use crate::TypeIndex;

/// Concrete, declarative signature facts carried by a non-local call target.
/// The independent verifier must compare these untrusted facts with the call
/// instruction, target contract and reachable effects before sealing an image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedCallableSignature {
    parameter_types: Box<[TypeIndex]>,
    parameter_plans: Box<[ValueTransferPlanKind]>,
    result_types: Box<[TypeIndex]>,
    result_plans: Box<[ValueTransferPlanKind]>,
    effect_summary: CallableEffectSummary,
}

impl LinkedCallableSignature {
    pub fn new(
        parameter_types: Box<[TypeIndex]>,
        parameter_plans: Box<[ValueTransferPlanKind]>,
        result_types: Box<[TypeIndex]>,
        result_plans: Box<[ValueTransferPlanKind]>,
        effect_summary: CallableEffectSummary,
    ) -> Result<Self, LinkedCallableSignatureError> {
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
            parameter_plans,
            result_types,
            result_plans,
            effect_summary,
        })
    }

    pub fn parameter_types(&self) -> &[TypeIndex] {
        &self.parameter_types
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
