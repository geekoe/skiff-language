use std::{collections::BTreeSet, fmt};

use crate::{
    FunctionIndex, LinkedCallableSignature, LinkedValueTransferPlan, TaskTargetIndex, TypeIndex,
};

/// Compiler-owned task scheduling plan retained by the linked image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedTaskTiming {
    Immediate,
    After { expression: u32 },
    At { expression: u32 },
}

/// One exact recoverable task payload parameter retained by the linked image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedTaskPayloadParameter {
    name: Box<str>,
    ty: TypeIndex,
    transfer: LinkedValueTransferPlan,
}

impl LinkedTaskPayloadParameter {
    pub fn new(
        name: impl Into<String>,
        ty: TypeIndex,
        transfer: LinkedValueTransferPlan,
    ) -> Result<Self, LinkedTaskTargetError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(LinkedTaskTargetError::EmptyPayloadParameterName { parameter: name });
        }
        Ok(Self {
            name: name.into_boxed_str(),
            ty,
            transfer,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn ty(&self) -> TypeIndex {
        self.ty
    }

    pub const fn transfer(&self) -> &LinkedValueTransferPlan {
        &self.transfer
    }
}

/// Compiler-owned recoverable tuple/record payload plan for one task target.
///
/// The plan keeps source parameter names and exact image-local type/transfer
/// facts. It is never reconstructed from the target signature by position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedTaskPayloadPlan {
    Tuple {
        parameters: Box<[LinkedTaskPayloadParameter]>,
    },
    Record {
        fields: Box<[LinkedTaskPayloadParameter]>,
    },
}

impl LinkedTaskPayloadPlan {
    pub fn try_tuple(
        parameters: Vec<LinkedTaskPayloadParameter>,
    ) -> Result<Self, LinkedTaskTargetError> {
        validate_unique_parameter_names(&parameters)?;
        Ok(Self::Tuple {
            parameters: parameters.into_boxed_slice(),
        })
    }

    pub fn try_record(
        fields: Vec<LinkedTaskPayloadParameter>,
    ) -> Result<Self, LinkedTaskTargetError> {
        validate_unique_parameter_names(&fields)?;
        Ok(Self::Record {
            fields: fields.into_boxed_slice(),
        })
    }

    pub fn parameters(&self) -> &[LinkedTaskPayloadParameter] {
        match self {
            Self::Tuple { parameters } | Self::Record { fields: parameters } => parameters,
        }
    }

    pub fn parameter_count(&self) -> usize {
        self.parameters().len()
    }

    pub fn parameter_names(&self) -> Box<[&str]> {
        self.parameters()
            .iter()
            .map(LinkedTaskPayloadParameter::name)
            .collect()
    }
}

fn validate_unique_parameter_names(
    parameters: &[LinkedTaskPayloadParameter],
) -> Result<(), LinkedTaskTargetError> {
    let mut seen = BTreeSet::new();
    for parameter in parameters {
        if !seen.insert(parameter.name()) {
            return Err(LinkedTaskTargetError::DuplicatePayloadParameterName {
                name: parameter.name().to_string(),
            });
        }
    }
    Ok(())
}

/// Exact linked task dispatch target inside one deployment image.
///
/// The textual identity is retained for router/host projection; execution
/// authority is the exact image-local function and signature. The linker
/// never reconstructs the function from the textual identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedTaskTarget {
    index: TaskTargetIndex,
    target_identity: Box<str>,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
    timing: LinkedTaskTiming,
    payload_plan: Option<LinkedTaskPayloadPlan>,
}

impl LinkedTaskTarget {
    pub fn new(
        index: TaskTargetIndex,
        target_identity: impl Into<String>,
        function: FunctionIndex,
        signature: LinkedCallableSignature,
        timing: LinkedTaskTiming,
    ) -> Result<Self, LinkedTaskTargetError> {
        let target_identity = target_identity.into();
        if target_identity.trim().is_empty() {
            return Err(LinkedTaskTargetError::EmptyTargetIdentity);
        }
        Ok(Self {
            index,
            target_identity: target_identity.into_boxed_str(),
            function,
            signature,
            timing,
            payload_plan: None,
        })
    }

    pub fn with_payload_plan(
        mut self,
        payload_plan: LinkedTaskPayloadPlan,
    ) -> Result<Self, LinkedTaskTargetError> {
        let expected = self.signature.parameter_types().len();
        let actual = payload_plan.parameter_count();
        if expected != actual {
            return Err(LinkedTaskTargetError::PayloadParameterCountMismatch { expected, actual });
        }
        for (ordinal, parameter) in payload_plan.parameters().iter().enumerate() {
            let expected_type = self.signature.parameter_types()[ordinal];
            if parameter.ty() != expected_type {
                return Err(LinkedTaskTargetError::PayloadParameterTypeMismatch {
                    ordinal,
                    expected: expected_type,
                    actual: parameter.ty(),
                });
            }
            if parameter.transfer() != &self.signature.parameter_plans()[ordinal] {
                return Err(LinkedTaskTargetError::PayloadParameterTransferMismatch { ordinal });
            }
        }
        self.payload_plan = Some(payload_plan);
        Ok(self)
    }

    pub const fn index(&self) -> TaskTargetIndex {
        self.index
    }

    pub fn target_identity(&self) -> &str {
        &self.target_identity
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }

    pub fn parameter_names(&self) -> Result<Box<[&str]>, LinkedTaskTargetError> {
        Ok(self.payload_plan()?.parameter_names())
    }

    pub const fn timing(&self) -> LinkedTaskTiming {
        self.timing
    }

    pub fn payload_plan(&self) -> Result<&LinkedTaskPayloadPlan, LinkedTaskTargetError> {
        self.payload_plan
            .as_ref()
            .ok_or(LinkedTaskTargetError::MissingPayloadPlan)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedTaskTargetError {
    EmptyTargetIdentity,
    MissingPayloadPlan,
    EmptyPayloadParameterName {
        parameter: String,
    },
    DuplicatePayloadParameterName {
        name: String,
    },
    PayloadParameterCountMismatch {
        expected: usize,
        actual: usize,
    },
    PayloadParameterTypeMismatch {
        ordinal: usize,
        expected: TypeIndex,
        actual: TypeIndex,
    },
    PayloadParameterTransferMismatch {
        ordinal: usize,
    },
}

impl fmt::Display for LinkedTaskTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTargetIdentity => {
                formatter.write_str("task target identity must not be empty")
            }
            Self::MissingPayloadPlan => formatter.write_str("task target payload plan is missing"),
            Self::EmptyPayloadParameterName { parameter } => write!(
                formatter,
                "task payload parameter name {parameter:?} must not be empty"
            ),
            Self::DuplicatePayloadParameterName { name } => {
                write!(
                    formatter,
                    "task payload parameter name {name:?} is duplicated"
                )
            }
            Self::PayloadParameterCountMismatch { expected, actual } => write!(
                formatter,
                "task payload plan has {actual} parameters but the target signature has {expected}"
            ),
            Self::PayloadParameterTypeMismatch {
                ordinal,
                expected,
                actual,
            } => write!(
                formatter,
                "task payload parameter {ordinal} type {} differs from signature type {}",
                actual.get(),
                expected.get()
            ),
            Self::PayloadParameterTransferMismatch { ordinal } => write!(
                formatter,
                "task payload parameter {ordinal} transfer plan differs from signature plan"
            ),
        }
    }
}

impl std::error::Error for LinkedTaskTargetError {}
