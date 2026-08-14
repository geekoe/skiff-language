use skiff_artifact_model::{contract_for_opcode, PendingContract, PendingMode, ResumeErrorMode};
use skiff_runtime_linked_bytecode::{
    FrameSlotIndex, FunctionIndex, InstructionIndex, LinkedBytecodeCandidate,
    LinkedInstructionTarget, LinkedResumeResultMaterialization, LinkedValueTransferPlan,
    ResumeSiteIndex, ShapeIndex, TypeIndex,
};

use super::ExecutionImageConstructionError;

/// Dense image-local resume descriptors consumed by checked VM dispatch.
#[derive(Debug)]
pub struct ExecutionResumeSites {
    rows: Box<[ExecutionResumeSite]>,
}

impl ExecutionResumeSites {
    pub fn rows(&self) -> &[ExecutionResumeSite] {
        &self.rows
    }

    pub fn get(&self, index: ResumeSiteIndex) -> Option<&ExecutionResumeSite> {
        self.rows
            .get(index.get() as usize)
            .filter(|row| row.index == index)
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionResumeSite {
    index: ResumeSiteIndex,
    function: FunctionIndex,
    site: InstructionIndex,
    resume: InstructionIndex,
    end_resume: Option<InstructionIndex>,
    expected_stack_height_before_result: u32,
    result_types: Box<[TypeIndex]>,
    result_plans: Box<[LinkedValueTransferPlan]>,
    result_materializations: Box<[Option<LinkedResumeResultMaterialization>]>,
    emit_stream_item_shape: Option<ShapeIndex>,
    error_mode: ResumeErrorMode,
    kind: ExecutionResumeKind,
}

impl ExecutionResumeSite {
    pub const fn index(&self) -> ResumeSiteIndex {
        self.index
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn site(&self) -> InstructionIndex {
        self.site
    }

    pub const fn resume(&self) -> InstructionIndex {
        self.resume
    }

    pub const fn end_resume(&self) -> Option<InstructionIndex> {
        self.end_resume
    }

    pub const fn expected_stack_height_before_result(&self) -> u32 {
        self.expected_stack_height_before_result
    }

    pub fn result_types(&self) -> &[TypeIndex] {
        &self.result_types
    }

    pub fn result_plans(&self) -> &[LinkedValueTransferPlan] {
        &self.result_plans
    }

    pub fn result_materializations(&self) -> &[Option<LinkedResumeResultMaterialization>] {
        &self.result_materializations
    }

    pub const fn emit_stream_item_shape(&self) -> Option<ShapeIndex> {
        self.emit_stream_item_shape
    }

    pub const fn error_mode(&self) -> ResumeErrorMode {
        self.error_mode
    }

    pub const fn kind(&self) -> &ExecutionResumeKind {
        &self.kind
    }
}

/// Mechanical runtime dispatch class taken from the pinned opcode contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionResumeKind {
    StreamRead {
        endpoint_slot: FrameSlotIndex,
        item_type: TypeIndex,
        end_resume: InstructionIndex,
    },
    StreamBackpressure,
    ServiceBoundary,
    ActorBoundary,
    InterfaceBoundary,
    CallbackBoundary,
    HostEffect,
}

pub(in crate::bytecode) fn build_resume_sites(
    linked: &LinkedBytecodeCandidate,
) -> Result<ExecutionResumeSites, ExecutionImageConstructionError> {
    let mut rows = Vec::with_capacity(linked.resume_sites().len());
    for row in linked.resume_sites() {
        let function = linked
            .functions()
            .get(row.function().get() as usize)
            .filter(|function| function.index() == row.function())
            .ok_or(ExecutionImageConstructionError::ResumeFunctionMissing {
                resume_site: row.index(),
                function: row.function(),
            })?;
        let instruction = function
            .instructions()
            .get(row.site().get() as usize)
            .ok_or(ExecutionImageConstructionError::ResumeInstructionMissing {
                resume_site: row.index(),
                function: row.function(),
                instruction: row.site(),
            })?;
        let PendingContract::ActualWithResume { mode, .. } =
            contract_for_opcode(instruction.opcode()).pending
        else {
            return Err(ExecutionImageConstructionError::ResumeOpcodeNotPending {
                resume_site: row.index(),
                function: row.function(),
                instruction: row.site(),
                opcode: instruction.opcode(),
            });
        };
        let matching_targets = instruction
            .resolved_operands()
            .iter()
            .filter(|operand| {
                matches!(
                    operand.target(),
                    LinkedInstructionTarget::ResumeSite(index) if index == row.index()
                )
            })
            .count();
        if matching_targets != 1 {
            return Err(ExecutionImageConstructionError::ResumeTargetMismatch {
                resume_site: row.index(),
                function: row.function(),
                instruction: row.site(),
                matching_targets,
            });
        }
        let kind = classify_resume(mode, row, instruction)?;
        rows.push(ExecutionResumeSite {
            index: row.index(),
            function: row.function(),
            site: row.site(),
            resume: row.resume(),
            end_resume: row.end_resume(),
            expected_stack_height_before_result: row.expected_stack_height_before_result(),
            result_types: row.result_types().into(),
            result_plans: row.result_plans().into(),
            result_materializations: row.result_materializations().into(),
            emit_stream_item_shape: row.emit_stream_item_shape(),
            error_mode: row.error_mode(),
            kind,
        });
    }
    require_pending_instruction_coverage(linked)?;
    Ok(ExecutionResumeSites {
        rows: rows.into_boxed_slice(),
    })
}

fn classify_resume(
    mode: PendingMode,
    row: &skiff_runtime_linked_bytecode::LinkedResumeSite,
    instruction: &skiff_runtime_linked_bytecode::LinkedInstruction,
) -> Result<ExecutionResumeKind, ExecutionImageConstructionError> {
    Ok(match mode {
        PendingMode::StreamRead => {
            let endpoint_slot = instruction
                .resolved_operands()
                .iter()
                .find_map(|operand| match operand.target() {
                    LinkedInstructionTarget::FrameSlot(slot) => Some(slot),
                    _ => None,
                })
                .ok_or(ExecutionImageConstructionError::StreamResumeShape {
                    resume_site: row.index(),
                })?;
            let [item_type] = row.result_types() else {
                return Err(ExecutionImageConstructionError::StreamResumeShape {
                    resume_site: row.index(),
                });
            };
            let end_resume =
                row.end_resume()
                    .ok_or(ExecutionImageConstructionError::StreamResumeShape {
                        resume_site: row.index(),
                    })?;
            ExecutionResumeKind::StreamRead {
                endpoint_slot,
                item_type: *item_type,
                end_resume,
            }
        }
        PendingMode::StreamBackpressure => ExecutionResumeKind::StreamBackpressure,
        PendingMode::ServiceBoundary => ExecutionResumeKind::ServiceBoundary,
        PendingMode::ActorBoundary => ExecutionResumeKind::ActorBoundary,
        PendingMode::InterfaceBoundary => ExecutionResumeKind::InterfaceBoundary,
        PendingMode::CallbackBoundary => ExecutionResumeKind::CallbackBoundary,
        PendingMode::HostEffect => ExecutionResumeKind::HostEffect,
    })
}

fn require_pending_instruction_coverage(
    linked: &LinkedBytecodeCandidate,
) -> Result<(), ExecutionImageConstructionError> {
    for function in linked.functions() {
        for (ordinal, instruction) in function.instructions().iter().enumerate() {
            if !matches!(
                contract_for_opcode(instruction.opcode()).pending,
                PendingContract::ActualWithResume { .. }
            ) {
                continue;
            }
            let instruction_index =
                u32::try_from(ordinal)
                    .map(InstructionIndex::new)
                    .map_err(
                        |_| ExecutionImageConstructionError::ResumeInstructionOverflow {
                            function: function.index(),
                        },
                    )?;
            let targets = instruction
                .resolved_operands()
                .iter()
                .filter_map(|operand| match operand.target() {
                    LinkedInstructionTarget::ResumeSite(index) => Some(index),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let [target] = targets.as_slice() else {
                return Err(
                    ExecutionImageConstructionError::PendingInstructionResumeCardinality {
                        function: function.index(),
                        instruction: instruction_index,
                        actual: targets.len(),
                    },
                );
            };
            if linked
                .resume_sites()
                .get(target.get() as usize)
                .filter(|row| {
                    row.index() == *target
                        && row.function() == function.index()
                        && row.site() == instruction_index
                })
                .is_none()
            {
                return Err(
                    ExecutionImageConstructionError::PendingInstructionResumeMismatch {
                        function: function.index(),
                        instruction: instruction_index,
                        resume_site: *target,
                    },
                );
            }
        }
    }
    Ok(())
}
