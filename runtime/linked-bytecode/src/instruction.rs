use std::fmt;

use skiff_artifact_model::{contract_for_opcode, LinkedOperandKind, Opcode};

use crate::{
    ActiveRegionIndex, ActorMethodIndex, CallLoanLayoutIndex, CallbackCaptureLayoutIndex,
    ConstantIndex, FrameSlotIndex, FunctionIndex, HostEffectAdapterIndex, InstructionIndex,
    InterfaceTableIndex, IntrinsicIndex, ResumeSiteIndex, ServiceOperationIndex, ShapeIndex,
    SwitchTableIndex, SyntheticCallbackIndex, TypeIndex, WritablePathIndex,
};

/// Typed resolution of one non-immediate linked operand. Immediate words stay
/// in `LinkedInstruction::operands`; every address-like word is retained here
/// so the verifier never has to guess a target-table kind from an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedInstructionTarget {
    FrameSlot(FrameSlotIndex),
    Branch(InstructionIndex),
    SwitchTable(SwitchTableIndex),
    ActiveRegion(ActiveRegionIndex),
    CallLoanLayout(CallLoanLayoutIndex),
    Function(FunctionIndex),
    ServiceOperation(ServiceOperationIndex),
    ActorMethod(ActorMethodIndex),
    InterfaceTable(InterfaceTableIndex),
    SyntheticCallback(SyntheticCallbackIndex),
    HostEffectAdapter(HostEffectAdapterIndex),
    Intrinsic(IntrinsicIndex),
    Constant(ConstantIndex),
    Type(TypeIndex),
    Shape(ShapeIndex),
    WritablePath(WritablePathIndex),
    CallbackCaptureLayout(CallbackCaptureLayoutIndex),
    ResumeSite(ResumeSiteIndex),
}

impl LinkedInstructionTarget {
    /// Canonical image-local target kind declared by the artifact-model
    /// [`skiff_artifact_model::OpcodeContract`].
    pub const fn kind(self) -> LinkedOperandKind {
        match self {
            Self::FrameSlot(_) => LinkedOperandKind::FrameSlot,
            Self::Branch(_) => LinkedOperandKind::Instruction,
            Self::SwitchTable(_) => LinkedOperandKind::SwitchTable,
            Self::ActiveRegion(_) => LinkedOperandKind::ActiveRegion,
            Self::CallLoanLayout(_) => LinkedOperandKind::CallLoanLayout,
            Self::Function(_) => LinkedOperandKind::Function,
            Self::ServiceOperation(_) => LinkedOperandKind::ServiceOperation,
            Self::ActorMethod(_) => LinkedOperandKind::ActorMethod,
            Self::InterfaceTable(_) => LinkedOperandKind::InterfaceTable,
            Self::SyntheticCallback(_) => LinkedOperandKind::SyntheticCallback,
            Self::HostEffectAdapter(_) => LinkedOperandKind::HostEffectAdapter,
            Self::Intrinsic(_) => LinkedOperandKind::Intrinsic,
            Self::Constant(_) => LinkedOperandKind::Constant,
            Self::Type(_) => LinkedOperandKind::Type,
            Self::Shape(_) => LinkedOperandKind::Shape,
            Self::WritablePath(_) => LinkedOperandKind::WritablePath,
            Self::CallbackCaptureLayout(_) => LinkedOperandKind::CallbackCaptureLayout,
            Self::ResumeSite(_) => LinkedOperandKind::ResumeSite,
        }
    }
}

/// Typed resolution attached to one raw operand ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkedResolvedOperand {
    operand_ordinal: u32,
    target: LinkedInstructionTarget,
}

impl LinkedResolvedOperand {
    pub const fn new(operand_ordinal: u32, target: LinkedInstructionTarget) -> Self {
        Self {
            operand_ordinal,
            target,
        }
    }

    pub const fn operand_ordinal(&self) -> u32 {
        self.operand_ordinal
    }

    pub const fn target(&self) -> LinkedInstructionTarget {
        self.target
    }
}

/// One decoded semantic instruction. Raw operands preserve exact decoded
/// words; address-like operands additionally carry their typed linked target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedInstruction {
    opcode: Opcode,
    operands: Box<[u32]>,
    resolved_operands: Box<[LinkedResolvedOperand]>,
    artifact_pc: u32,
}

impl LinkedInstruction {
    pub fn new(
        opcode: Opcode,
        operands: Box<[u32]>,
        resolved_operands: Box<[LinkedResolvedOperand]>,
        artifact_pc: u32,
    ) -> Result<Self, LinkedInstructionError> {
        let contract = contract_for_opcode(opcode);
        if operands.len() != contract.operands.len() {
            return Err(LinkedInstructionError::OperandCountMismatch {
                opcode,
                expected: contract.operands.len(),
                actual: operands.len(),
            });
        }

        let mut previous = None;
        for resolved in &resolved_operands {
            if resolved.operand_ordinal() as usize >= operands.len() {
                return Err(LinkedInstructionError::OperandOrdinalOutOfBounds {
                    operand_ordinal: resolved.operand_ordinal(),
                    operand_count: operands.len(),
                });
            }
            if let Some(previous) = previous {
                if previous >= resolved.operand_ordinal() {
                    return Err(LinkedInstructionError::NonCanonicalResolvedOperandOrder {
                        previous,
                        current: resolved.operand_ordinal(),
                    });
                }
            }
            previous = Some(resolved.operand_ordinal());
        }

        let mut resolved_position = 0;
        for (operand_ordinal, specification) in contract.operands.iter().enumerate() {
            let operand_ordinal = operand_ordinal as u32;
            let resolved = resolved_operands
                .get(resolved_position)
                .filter(|resolved| resolved.operand_ordinal() == operand_ordinal);
            if specification.linked_kind == LinkedOperandKind::Immediate {
                if let Some(resolved) = resolved {
                    return Err(LinkedInstructionError::UnexpectedResolvedOperand {
                        operand_ordinal,
                        actual: resolved.target().kind(),
                    });
                }
                continue;
            }

            let Some(resolved) = resolved else {
                return Err(LinkedInstructionError::MissingResolvedOperand {
                    operand_ordinal,
                    expected: specification.linked_kind,
                });
            };
            let actual = resolved.target().kind();
            if actual != specification.linked_kind {
                return Err(LinkedInstructionError::ResolvedOperandKindMismatch {
                    operand_ordinal,
                    expected: specification.linked_kind,
                    actual,
                });
            }
            resolved_position += 1;
        }

        Ok(Self {
            opcode,
            operands,
            resolved_operands,
            artifact_pc,
        })
    }

    pub const fn opcode(&self) -> Opcode {
        self.opcode
    }

    pub fn operands(&self) -> &[u32] {
        &self.operands
    }

    pub fn resolved_operands(&self) -> &[LinkedResolvedOperand] {
        &self.resolved_operands
    }

    pub const fn artifact_pc(&self) -> u32 {
        self.artifact_pc
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedInstructionError {
    OperandCountMismatch {
        opcode: Opcode,
        expected: usize,
        actual: usize,
    },
    OperandOrdinalOutOfBounds {
        operand_ordinal: u32,
        operand_count: usize,
    },
    NonCanonicalResolvedOperandOrder {
        previous: u32,
        current: u32,
    },
    MissingResolvedOperand {
        operand_ordinal: u32,
        expected: LinkedOperandKind,
    },
    UnexpectedResolvedOperand {
        operand_ordinal: u32,
        actual: LinkedOperandKind,
    },
    ResolvedOperandKindMismatch {
        operand_ordinal: u32,
        expected: LinkedOperandKind,
        actual: LinkedOperandKind,
    },
}

impl fmt::Display for LinkedInstructionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OperandCountMismatch {
                opcode,
                expected,
                actual,
            } => write!(
                formatter,
                "opcode {} requires {expected} operands but linked instruction has {actual}",
                opcode.name()
            ),
            Self::OperandOrdinalOutOfBounds {
                operand_ordinal,
                operand_count,
            } => write!(
                formatter,
                "resolved operand ordinal {operand_ordinal} is outside {operand_count} raw operands"
            ),
            Self::NonCanonicalResolvedOperandOrder { previous, current } => write!(
                formatter,
                "resolved operand ordinal {current} must sort after {previous}"
            ),
            Self::MissingResolvedOperand {
                operand_ordinal,
                expected,
            } => write!(
                formatter,
                "operand {operand_ordinal} requires a resolved {} target",
                expected.name()
            ),
            Self::UnexpectedResolvedOperand {
                operand_ordinal,
                actual,
            } => write!(
                formatter,
                "immediate operand {operand_ordinal} must not carry a resolved {} target",
                actual.name()
            ),
            Self::ResolvedOperandKindMismatch {
                operand_ordinal,
                expected,
                actual,
            } => write!(
                formatter,
                "operand {operand_ordinal} requires a resolved {} target, not {}",
                expected.name(),
                actual.name()
            ),
        }
    }
}

impl std::error::Error for LinkedInstructionError {}
