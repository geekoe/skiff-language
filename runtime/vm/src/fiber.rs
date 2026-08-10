mod entry_admission;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use skiff_artifact_model::{descriptor_for_opcode, Opcode, ParamModeIr};
use skiff_runtime_bytecode_verifier::{VerifiedCodeEntry, VerifiedLinkedBytecodeImage};
use skiff_runtime_deployment_image::{DeploymentOwnerIdentity, PinnedDeploymentEntry};
use skiff_runtime_linked_bytecode::{
    FrameSlotIndex, FunctionIndex, InstructionIndex, LinkedFunction, LinkedInstruction,
    LinkedInstructionTarget,
};
use skiff_runtime_model::{
    vm_heap::{VmHeap, VmHeapError},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{ValueKind, ValueSlot},
};

use crate::{
    admission::{is_discardable_root, validate_entry_arguments},
    control::VmOwnedValues,
    fiber::entry_admission::validate_entry_contract,
    frame::VmFrame,
    statement::{charge_frame_entry, charge_instruction_events},
    ResumeOutcome, VmBudget, VmControl, VmError, VmLimits, VmResumeToken, VmValueLocation,
    VmVerifiedInvariant,
};

pub type VerifiedVmEntry = PinnedDeploymentEntry<VerifiedLinkedBytecodeImage, VerifiedCodeEntry>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmFiberState {
    Runnable,
    BlockedOnChild,
    WaitingHost,
    Unwinding,
    Terminal,
}

/// Stateless constructor namespace for production VM fibers.
pub struct Vm;

impl Vm {
    pub const fn new() -> Self {
        Self
    }

    pub fn start(
        entry: VerifiedVmEntry,
        arguments: Box<[ValueSlot]>,
        limits: VmLimits,
    ) -> Result<VmFiber, VmError> {
        VmFiber::start(entry, arguments, limits)
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use = "a VM fiber owns live roots until completion or explicit terminal discard"]
pub struct VmFiber {
    entry: VerifiedVmEntry,
    frames: Vec<VmFrame>,
    values: Vec<ValueSlot>,
    live_values: Vec<bool>,
    state: VmFiberState,
    limits: VmLimits,
    raw_fuel_remaining: u32,
}

impl VmFiber {
    fn start(
        entry: VerifiedVmEntry,
        arguments: Box<[ValueSlot]>,
        limits: VmLimits,
    ) -> Result<Self, VmError> {
        let function_index = entry.entry().function();
        let program = entry.image().program();
        let function =
            verified_function(program, function_index).ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::EntryFunctionMissing,
            })?;
        validate_entry_contract(entry.entry(), function, arguments.len())?;
        validate_entry_arguments(
            program,
            entry.entry().signature().parameter_types(),
            &arguments,
        )?;

        let slot_count = function.frame().slot_types().len();
        let operand_capacity = usize::try_from(function.max_operand_depth()).map_err(|_| {
            VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            }
        })?;
        let segment_len =
            slot_count
                .checked_add(operand_capacity)
                .ok_or(VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        let value_limit = limits.max_value_slots().get();
        if segment_len > value_limit {
            return Err(VmError::ValueStackLimitExceeded {
                limit: value_limit,
                requested: segment_len,
            });
        }

        let frame = VmFrame::root(function_index, slot_count, operand_capacity);
        if frame.segment_end() != Some(segment_len) {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        let mut values = vec![ValueSlot::null(); segment_len];
        let mut live_values = vec![false; segment_len];
        for (argument, parameter) in arguments
            .into_vec()
            .into_iter()
            .zip(function.frame().parameters())
        {
            let index = usize::try_from(parameter.slot().get()).map_err(|_| {
                VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                }
            })?;
            values[index] = argument;
            live_values[index] = true;
        }

        Ok(Self {
            entry,
            frames: vec![frame],
            values,
            live_values,
            state: VmFiberState::Runnable,
            limits,
            raw_fuel_remaining: 0,
        })
    }

    pub const fn state(&self) -> VmFiberState {
        self.state
    }

    pub fn owner(&self) -> &DeploymentOwnerIdentity {
        self.entry.owner()
    }

    pub fn active_frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn allocated_value_slot_count(&self) -> usize {
        self.values.len()
    }

    pub fn run_segment(&mut self, _heap: &mut dyn VmHeap, budget: &mut dyn VmBudget) -> VmControl {
        if self.state != VmFiberState::Runnable {
            return VmControl::Complete(Err(VmError::FiberNotRunnable { state: self.state }));
        }

        match self.run_segment_inner(budget) {
            Ok(None) => VmControl::Continue,
            Ok(Some(values)) => VmControl::Complete(Ok(values)),
            Err(error) => {
                self.state = VmFiberState::Terminal;
                VmControl::Complete(Err(error))
            }
        }
    }

    pub fn resume(&mut self, token: VmResumeToken, outcome: ResumeOutcome) -> Result<(), VmError> {
        let _ = (token, outcome);
        Err(VmError::ResumeNotExpected)
    }

    pub fn discard_terminal_roots(&mut self, _heap: &mut dyn VmHeap) -> Result<(), VmError> {
        if self.state != VmFiberState::Terminal {
            return Err(VmError::DiscardRequiresTerminal { state: self.state });
        }

        for (index, (value, live)) in self.values.iter().zip(&self.live_values).enumerate() {
            if *live && !is_discardable_root(value) {
                return Err(VmError::TerminalRootLifecycleUnavailable {
                    index,
                    kind: value.kind(),
                });
            }
        }
        self.values.fill(ValueSlot::null());
        self.live_values.fill(false);
        self.frames.clear();
        self.values.clear();
        self.live_values.clear();
        Ok(())
    }

    fn run_segment_inner(
        &mut self,
        budget: &mut dyn VmBudget,
    ) -> Result<Option<VmOwnedValues>, VmError> {
        for _ in 0..self.limits.max_segment_instructions().get() {
            self.charge_function_entry(budget)?;
            self.consume_raw_fuel(budget)?;
            self.charge_statement_events(budget)?;
            match self.dispatch_one()? {
                DispatchOutcome::Continue => {}
                DispatchOutcome::Complete(values) => return Ok(Some(values)),
            }
        }
        Ok(None)
    }

    fn charge_function_entry(&mut self, budget: &mut dyn VmBudget) -> Result<(), VmError> {
        let schedule = self.entry.image().program().statement_schedule();
        let frame = self
            .frames
            .last_mut()
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        charge_frame_entry(schedule, frame, budget)
    }

    fn consume_raw_fuel(&mut self, budget: &mut dyn VmBudget) -> Result<(), VmError> {
        if self.raw_fuel_remaining == 0 {
            let maximum = self.limits.raw_fuel_quantum();
            let granted = budget.replenish_raw_fuel(maximum)?;
            if granted > maximum {
                return Err(VmError::InvalidFuelGrant {
                    requested_maximum: maximum,
                    granted,
                });
            }
            self.raw_fuel_remaining = granted.get();
        }
        self.raw_fuel_remaining -= 1;
        Ok(())
    }

    fn charge_statement_events(&mut self, budget: &mut dyn VmBudget) -> Result<(), VmError> {
        let schedule = self.entry.image().program().statement_schedule();
        let frame = self
            .frames
            .last_mut()
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        charge_instruction_events(schedule, frame, budget)
    }

    fn dispatch_one(&mut self) -> Result<DispatchOutcome, VmError> {
        let (function_index, instruction_index, instruction) = {
            let frame = self.current_frame()?;
            let function = self.function(frame.function())?;
            let instruction = function
                .instructions()
                .get(frame.instruction().get() as usize)
                .cloned()
                .ok_or(VmError::InstructionPointerOutOfBounds {
                    function: frame.function(),
                    instruction: frame.instruction(),
                })?;
            (frame.function(), frame.instruction(), instruction)
        };

        let descriptor = descriptor_for_opcode(instruction.opcode());
        if instruction.operands().len() != descriptor.operand_layout.len() {
            return Err(VmError::MalformedInstruction {
                function: function_index,
                instruction: instruction_index,
                opcode: instruction.opcode(),
                expected_operands: descriptor.operand_layout.len(),
                actual_operands: instruction.operands().len(),
            });
        }

        match instruction.opcode() {
            Opcode::Const => self.execute_const(function_index, instruction_index, &instruction),
            Opcode::CopySlot => {
                self.execute_copy_slot(function_index, instruction_index, &instruction)
            }
            Opcode::MoveSlot => {
                self.execute_move_slot(function_index, instruction_index, &instruction)
            }
            Opcode::StoreSlot => {
                self.execute_store_slot(function_index, instruction_index, &instruction)
            }
            Opcode::Drop => self.execute_drop(function_index, instruction_index, &instruction),
            Opcode::Dup => self.execute_dup(function_index, instruction_index),
            Opcode::LoadSlot => {
                self.execute_load_slot(function_index, instruction_index, &instruction)
            }
            Opcode::TakeSlot => {
                self.execute_take_slot(function_index, instruction_index, &instruction)
            }
            Opcode::Pop => self.execute_pop(function_index, instruction_index),
            Opcode::Jump => self.execute_jump(function_index, instruction_index, &instruction),
            Opcode::JumpIfTrue => {
                self.execute_jump_if(function_index, instruction_index, &instruction, true)
            }
            Opcode::JumpIfFalse => {
                self.execute_jump_if(function_index, instruction_index, &instruction, false)
            }
            Opcode::BudgetCheckpoint => self.execute_budget_checkpoint(function_index),
            Opcode::CallLocal => {
                self.execute_call_local(function_index, instruction_index, &instruction)
            }
            Opcode::TailCallLocal => {
                self.execute_tail_call_local(function_index, instruction_index, &instruction)
            }
            Opcode::Return => self.execute_return(function_index, instruction_index),
            Opcode::Not => self.execute_not(function_index, instruction_index),
            Opcode::Negate => self.execute_negate(function_index, instruction_index),
            Opcode::Add | Opcode::Subtract | Opcode::Multiply | Opcode::Divide => {
                self.execute_binary_number(function_index, instruction_index, instruction.opcode())
            }
            Opcode::LessThan
            | Opcode::LessOrEqual
            | Opcode::GreaterThan
            | Opcode::GreaterOrEqual => self.execute_number_comparison(
                function_index,
                instruction_index,
                instruction.opcode(),
            ),
            Opcode::Equal | Opcode::NotEqual => {
                self.execute_equality(function_index, instruction_index, instruction.opcode())
            }
            _ => Err(VmError::UnsupportedOpcode {
                function: function_index,
                instruction: instruction_index,
                opcode: instruction.opcode(),
            }),
        }
    }

    fn execute_const(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Constant(index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let value = self
            .entry
            .image()
            .program()
            .constant_heap()
            .get(index)
            .ok_or(VmError::ConstantIndexOutOfBounds {
                function,
                instruction,
                index: index.get(),
            })?;
        self.push_operand(value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_copy_slot(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::FrameSlot(source) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let LinkedInstructionTarget::FrameSlot(destination) =
            self.resolved_target(function, instruction, decoded, 1)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let frame = self.current_frame()?.clone();
        let slot_count = self.function(frame.function())?.frame().slot_types().len();
        let value = self.read_slot(&frame, slot_count, source)?;
        self.write_slot(&frame, slot_count, destination, value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_move_slot(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::FrameSlot(source) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let LinkedInstructionTarget::FrameSlot(destination) =
            self.resolved_target(function, instruction, decoded, 1)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let frame = self.current_frame()?.clone();
        let slot_count = self.function(frame.function())?.frame().slot_types().len();
        let value = self.read_slot(&frame, slot_count, source)?;
        self.write_slot(&frame, slot_count, destination, value)?;
        self.clear_slot(&frame, slot_count, source)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_store_slot(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::FrameSlot(destination) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let frame = self.current_frame()?.clone();
        let slot_count = self.function(frame.function())?.frame().slot_types().len();
        self.ensure_slot_dead(&frame, slot_count, destination)?;
        let value = self.pop_operands(1, false)?.remove(0);
        self.write_slot(&frame, slot_count, destination, value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_drop(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::FrameSlot(slot) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let frame = self.current_frame()?.clone();
        let slot_count = self.function(frame.function())?.frame().slot_types().len();
        let _ = self.read_slot(&frame, slot_count, slot)?;
        self.clear_slot(&frame, slot_count, slot)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_load_slot(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::FrameSlot(slot) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let frame = self.current_frame()?.clone();
        let slot_count = self.function(frame.function())?.frame().slot_types().len();
        let value = self.read_slot(&frame, slot_count, slot)?;
        self.push_operand(value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_take_slot(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::FrameSlot(slot) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let frame = self.current_frame()?.clone();
        let slot_count = self.function(frame.function())?.frame().slot_types().len();
        self.ensure_operand_push(1)?;
        let value = self.read_slot(&frame, slot_count, slot)?;
        self.clear_slot(&frame, slot_count, slot)?;
        self.push_operand(value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_pop(
        &mut self,
        _function: FunctionIndex,
        _instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let _ = self.pop_operands(1, false)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_dup(
        &mut self,
        _function: FunctionIndex,
        _instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        self.ensure_operand_push(1)?;
        let value = self.pop_operands(1, false)?.remove(0);
        self.push_operand(value)?;
        self.push_operand(value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_jump(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Branch(target) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        self.validate_branch_target(target)?;
        self.current_frame_mut()?.jump_to(target);
        Ok(DispatchOutcome::Continue)
    }

    fn execute_jump_if(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
        jump_when: bool,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Branch(target) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let value = self.pop_operands(1, false)?.remove(0);
        let condition = value.as_bool().ok_or(VmError::ExpectedBoolean {
            function,
            instruction,
            actual: value.kind(),
        })?;
        if condition == jump_when {
            self.validate_branch_target(target)?;
            self.current_frame_mut()?.jump_to(target);
        } else {
            self.advance_current_instruction()?;
        }
        Ok(DispatchOutcome::Continue)
    }

    fn execute_budget_checkpoint(
        &mut self,
        _function: FunctionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_call_local(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Function(target) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let arg_count = self.operand_usize(decoded, 1, function, instruction)?;
        let result_count = self.operand_usize(decoded, 2, function, instruction)?;
        let caller = self.current_frame()?.clone();
        let resume_instruction = {
            let caller_function = self.function(caller.function())?;
            let next = caller.instruction().get().checked_add(1).ok_or(
                VmError::InstructionPointerOutOfBounds {
                    function: caller.function(),
                    instruction: caller.instruction(),
                },
            )?;
            if next as usize >= caller_function.instructions().len() {
                return Err(VmError::InstructionPointerOutOfBounds {
                    function: caller.function(),
                    instruction: caller.instruction(),
                });
            }
            InstructionIndex::new(next)
        };
        let (target_slot_count, target_operand_capacity, target_arg_count, target_result_count) = {
            let target_function = self.function(target)?;
            self.validate_local_frame_layout(target_function)?;
            (
                target_function.frame().slot_types().len(),
                usize::try_from(target_function.max_operand_depth()).map_err(|_| {
                    VmError::VerifiedEntryInvariant {
                        invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                    }
                })?,
                target_function.frame().parameters().len(),
                target_function.frame().result_types().len(),
            )
        };
        if target_arg_count != arg_count || target_result_count != result_count {
            return Err(VmError::LocalCallTargetMismatch {
                function,
                instruction,
                target,
                expected_arguments: target_arg_count,
                actual_arguments: arg_count,
                expected_results: target_result_count,
                actual_results: result_count,
            });
        }
        let transfer_slots = self.parameter_transfer_slots(target, target_slot_count)?;
        let segment_len = target_slot_count
            .checked_add(target_operand_capacity)
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        let requested =
            self.values
                .len()
                .checked_add(segment_len)
                .ok_or(VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        if self.frames.len() >= self.limits.max_frames().get() {
            return Err(VmError::FrameLimitExceeded {
                limit: self.limits.max_frames().get(),
            });
        }
        if requested > self.limits.max_value_slots().get() {
            return Err(VmError::ValueStackLimitExceeded {
                limit: self.limits.max_value_slots().get(),
                requested,
            });
        }

        let child_start = self.values.len();
        let child = VmFrame::child(
            target,
            InstructionIndex::new(0),
            child_start,
            target_slot_count,
            target_operand_capacity,
            resume_instruction,
        );
        if child.segment_end() != Some(child_start + segment_len) {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        let arguments = self.pop_operands(arg_count, false)?;
        self.values
            .resize(child_start + segment_len, ValueSlot::null());
        self.live_values.resize(child_start + segment_len, false);
        for (ordinal, destination_slot) in transfer_slots.into_iter().enumerate() {
            let value = arguments[ordinal];
            self.values[child_start + destination_slot] = value;
            self.live_values[child_start + destination_slot] = true;
        }
        self.frames.push(child);
        Ok(DispatchOutcome::Continue)
    }

    fn execute_tail_call_local(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Function(target) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let arg_count = self.operand_usize(decoded, 1, function, instruction)?;
        let caller = self.current_frame()?.clone();
        let caller_end = caller
            .segment_end()
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        if self.values.len() != caller_end {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        let caller_result_count = self
            .function(caller.function())?
            .frame()
            .result_types()
            .len();
        let (target_slot_count, target_operand_capacity, target_arg_count, target_result_count) = {
            let target_function = self.function(target)?;
            self.validate_local_frame_layout(target_function)?;
            (
                target_function.frame().slot_types().len(),
                usize::try_from(target_function.max_operand_depth()).map_err(|_| {
                    VmError::VerifiedEntryInvariant {
                        invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                    }
                })?,
                target_function.frame().parameters().len(),
                target_function.frame().result_types().len(),
            )
        };
        if target_arg_count != arg_count || target_result_count != caller_result_count {
            return Err(VmError::TailCallTargetMismatch {
                function,
                instruction,
                target,
                expected_arguments: target_arg_count,
                actual_arguments: arg_count,
                expected_results: target_result_count,
                actual_results: caller_result_count,
            });
        }
        let transfer_slots = self.parameter_transfer_slots(target, target_slot_count)?;
        let segment_len = target_slot_count
            .checked_add(target_operand_capacity)
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        let slot_base = caller.slot_base();
        let requested =
            slot_base
                .checked_add(segment_len)
                .ok_or(VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        if requested > self.limits.max_value_slots().get() {
            return Err(VmError::ValueStackLimitExceeded {
                limit: self.limits.max_value_slots().get(),
                requested,
            });
        }

        let arguments = self.pop_operands(arg_count, true)?;
        for index in slot_base..caller_end {
            self.clear_value(index);
        }
        let new_end = slot_base + segment_len;
        self.values.resize(new_end, ValueSlot::null());
        self.live_values.resize(new_end, false);
        for (ordinal, destination_slot) in transfer_slots.into_iter().enumerate() {
            let value = arguments[ordinal];
            self.values[slot_base + destination_slot] = value;
            self.live_values[slot_base + destination_slot] = true;
        }
        let replacement = VmFrame::replacement(
            target,
            slot_base,
            target_slot_count,
            target_operand_capacity,
            caller.resume_instruction(),
        );
        let current = self
            .frames
            .last_mut()
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        *current = replacement;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_return(
        &mut self,
        _function: FunctionIndex,
        _instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let frame = self.current_frame()?.clone();
        let result_count = self
            .function(frame.function())?
            .frame()
            .result_types()
            .len();
        if self.frames.len() > 1 {
            let caller = &self.frames[self.frames.len() - 2];
            let Some(height) = caller.operand_height().checked_add(result_count) else {
                return Err(VmError::OperandStackOverflow {
                    function: caller.function(),
                    capacity: caller.operand_capacity(),
                });
            };
            if height > caller.operand_capacity() {
                return Err(VmError::OperandStackOverflow {
                    function: caller.function(),
                    capacity: caller.operand_capacity(),
                });
            }
        }
        let results = self.pop_operands(result_count, true)?;
        if self.frames.len() == 1 {
            let image = Arc::clone(self.entry.image().program());
            let values = results.into_boxed_slice();
            self.frames.clear();
            self.values.clear();
            self.live_values.clear();
            self.state = VmFiberState::Terminal;
            return Ok(DispatchOutcome::Complete(VmOwnedValues::new(image, values)));
        }

        let child = self
            .frames
            .pop()
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        let resume = child
            .resume_instruction()
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::ChildFrameResumeMissing,
            })?;
        let caller_end =
            self.current_frame()?
                .segment_end()
                .ok_or(VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        self.values.truncate(caller_end);
        self.live_values.truncate(caller_end);
        for value in results {
            self.push_operand(value)?;
        }
        self.current_frame_mut()?.resume_to(resume);
        Ok(DispatchOutcome::Continue)
    }

    fn execute_not(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let value = self.pop_operands(1, false)?.remove(0);
        let boolean = value.as_bool().ok_or(VmError::ExpectedBoolean {
            function,
            instruction,
            actual: value.kind(),
        })?;
        self.push_operand(ValueSlot::bool(!boolean))?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_negate(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let value = self.pop_operands(1, false)?.remove(0);
        let number = value.as_number().ok_or(VmError::ExpectedNumber {
            function,
            instruction,
            actual: value.kind(),
        })?;
        let result = -number;
        if !result.is_finite() {
            return Err(VmError::ScalarNonFinite {
                function,
                instruction,
            });
        }
        self.push_operand(ValueSlot::number(result))?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_binary_number(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
    ) -> Result<DispatchOutcome, VmError> {
        let operands = self.pop_operands(2, false)?;
        let left = operands[0].as_number().ok_or(VmError::ExpectedNumber {
            function,
            instruction,
            actual: operands[0].kind(),
        })?;
        let right = operands[1].as_number().ok_or(VmError::ExpectedNumber {
            function,
            instruction,
            actual: operands[1].kind(),
        })?;
        let result = match opcode {
            Opcode::Add => left + right,
            Opcode::Subtract => left - right,
            Opcode::Multiply => left * right,
            Opcode::Divide => {
                if right == 0.0 {
                    return Err(VmError::DivideByZero {
                        function,
                        instruction,
                    });
                }
                left / right
            }
            _ => unreachable!("binary number opcode was matched above"),
        };
        if !result.is_finite() {
            return Err(VmError::ScalarNonFinite {
                function,
                instruction,
            });
        }
        self.push_operand(ValueSlot::number(result))?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_number_comparison(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
    ) -> Result<DispatchOutcome, VmError> {
        let operands = self.pop_operands(2, false)?;
        let left = operands[0].as_number().ok_or(VmError::ExpectedNumber {
            function,
            instruction,
            actual: operands[0].kind(),
        })?;
        let right = operands[1].as_number().ok_or(VmError::ExpectedNumber {
            function,
            instruction,
            actual: operands[1].kind(),
        })?;
        let result = match opcode {
            Opcode::LessThan => left < right,
            Opcode::LessOrEqual => left <= right,
            Opcode::GreaterThan => left > right,
            Opcode::GreaterOrEqual => left >= right,
            _ => unreachable!("number comparison opcode was matched above"),
        };
        self.push_operand(ValueSlot::bool(result))?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_equality(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
    ) -> Result<DispatchOutcome, VmError> {
        let operands = self.pop_operands(2, false)?;
        let equal = comparable_equality(&operands[0], &operands[1]).ok_or(
            VmError::ExpectedComparablePair {
                function,
                instruction,
                left: operands[0].kind(),
                right: operands[1].kind(),
            },
        )?;
        let result = if opcode == Opcode::Equal {
            equal
        } else {
            !equal
        };
        self.push_operand(ValueSlot::bool(result))?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn current_frame(&self) -> Result<&VmFrame, VmError> {
        self.frames
            .last()
            .ok_or(VmError::FiberNotRunnable { state: self.state })
    }

    fn current_frame_mut(&mut self) -> Result<&mut VmFrame, VmError> {
        self.frames
            .last_mut()
            .ok_or(VmError::FiberNotRunnable { state: self.state })
    }

    fn program(&self) -> &VerifiedLinkedBytecodeImage {
        self.entry.image().program().as_ref()
    }

    fn function(&self, index: FunctionIndex) -> Result<&LinkedFunction, VmError> {
        verified_function(self.program(), index).ok_or(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::EntryFunctionMissing,
        })
    }

    fn advance_current_instruction(&mut self) -> Result<(), VmError> {
        let frame = self.current_frame_mut()?;
        let function = frame.function();
        let instruction = frame.instruction();
        if !frame.advance_instruction() {
            return Err(VmError::InstructionPointerOutOfBounds {
                function,
                instruction,
            });
        }
        Ok(())
    }

    fn resolved_target(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
        operand_ordinal: u32,
    ) -> Result<LinkedInstructionTarget, VmError> {
        decoded
            .resolved_operands()
            .iter()
            .find(|operand| operand.operand_ordinal() == operand_ordinal)
            .map(|operand| operand.target())
            .ok_or_else(|| self.malformed_instruction(function, instruction, decoded))
    }

    fn malformed_instruction(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> VmError {
        let descriptor = descriptor_for_opcode(decoded.opcode());
        VmError::MalformedInstruction {
            function,
            instruction,
            opcode: decoded.opcode(),
            expected_operands: descriptor.operand_layout.len(),
            actual_operands: decoded.operands().len(),
        }
    }

    fn operand_usize(
        &self,
        decoded: &LinkedInstruction,
        position: usize,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<usize, VmError> {
        let word = decoded
            .operands()
            .get(position)
            .copied()
            .ok_or_else(|| self.malformed_instruction(function, instruction, decoded))?;
        usize::try_from(word).map_err(|_| VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::FrameLayoutOverflow,
        })
    }

    fn validate_branch_target(&self, target: InstructionIndex) -> Result<(), VmError> {
        let frame = self.current_frame()?;
        let function = self.function(frame.function())?;
        if target.get() as usize >= function.instructions().len() {
            return Err(VmError::BranchTargetOutOfBounds {
                function: frame.function(),
                target,
            });
        }
        Ok(())
    }

    fn slot_index(
        frame: &VmFrame,
        slot_count: usize,
        slot: FrameSlotIndex,
        function: FunctionIndex,
    ) -> Result<usize, VmError> {
        let slot_index =
            usize::try_from(slot.get()).map_err(|_| VmError::SlotOutOfBounds { function, slot })?;
        if slot_index >= slot_count {
            return Err(VmError::SlotOutOfBounds { function, slot });
        }
        frame
            .slot_base()
            .checked_add(slot_index)
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })
    }

    fn read_slot(
        &self,
        frame: &VmFrame,
        slot_count: usize,
        slot: FrameSlotIndex,
    ) -> Result<ValueSlot, VmError> {
        let index = Self::slot_index(frame, slot_count, slot, frame.function())?;
        if !self.live_values[index] {
            return Err(VmError::DeadValueRead {
                location: VmValueLocation::FrameSlot(slot),
            });
        }
        Ok(self.values[index])
    }

    fn ensure_slot_dead(
        &self,
        frame: &VmFrame,
        slot_count: usize,
        slot: FrameSlotIndex,
    ) -> Result<(), VmError> {
        let index = Self::slot_index(frame, slot_count, slot, frame.function())?;
        if self.live_values[index] {
            return Err(VmError::LiveDestination {
                location: VmValueLocation::FrameSlot(slot),
            });
        }
        Ok(())
    }

    fn write_slot(
        &mut self,
        frame: &VmFrame,
        slot_count: usize,
        slot: FrameSlotIndex,
        value: ValueSlot,
    ) -> Result<(), VmError> {
        let index = Self::slot_index(frame, slot_count, slot, frame.function())?;
        if self.live_values[index] {
            return Err(VmError::LiveDestination {
                location: VmValueLocation::FrameSlot(slot),
            });
        }
        self.values[index] = value;
        self.live_values[index] = true;
        Ok(())
    }

    fn clear_slot(
        &mut self,
        frame: &VmFrame,
        slot_count: usize,
        slot: FrameSlotIndex,
    ) -> Result<(), VmError> {
        let index = Self::slot_index(frame, slot_count, slot, frame.function())?;
        if !self.live_values[index] {
            return Err(VmError::DeadValueRead {
                location: VmValueLocation::FrameSlot(slot),
            });
        }
        self.clear_value(index);
        Ok(())
    }

    fn clear_value(&mut self, index: usize) {
        self.values[index] = ValueSlot::null();
        self.live_values[index] = false;
    }

    fn ensure_operand_push(&self, extra: usize) -> Result<(), VmError> {
        let frame = self.current_frame()?;
        let Some(height) = frame.operand_height().checked_add(extra) else {
            return Err(VmError::OperandStackOverflow {
                function: frame.function(),
                capacity: frame.operand_capacity(),
            });
        };
        if height > frame.operand_capacity() {
            return Err(VmError::OperandStackOverflow {
                function: frame.function(),
                capacity: frame.operand_capacity(),
            });
        }
        Ok(())
    }

    fn pop_operands(&mut self, count: usize, exact: bool) -> Result<Vec<ValueSlot>, VmError> {
        let frame = self.current_frame()?.clone();
        let function = frame.function();
        if frame.operand_height() < count {
            return Err(VmError::OperandStackUnderflow {
                function,
                needed: count,
                available: frame.operand_height(),
            });
        }
        if exact && frame.operand_height() != count {
            return Err(VmError::OperandStackShapeMismatch {
                function,
                expected: count,
                actual: frame.operand_height(),
            });
        }
        let start = frame.operand_base() + (frame.operand_height() - count);
        let end = frame.operand_base() + frame.operand_height();
        let mut values = Vec::with_capacity(count);
        for index in start..end {
            if !self.live_values[index] {
                return Err(VmError::DeadValueRead {
                    location: VmValueLocation::Operand(index - frame.operand_base()),
                });
            }
            values.push(self.values[index]);
        }
        for index in start..end {
            self.clear_value(index);
        }
        self.set_current_frame_operand_height(&frame, frame.operand_height() - count)
            .map(|()| values)
    }

    fn push_operand(&mut self, value: ValueSlot) -> Result<(), VmError> {
        let frame = self.current_frame()?.clone();
        let function = frame.function();
        if frame.operand_height() >= frame.operand_capacity() {
            return Err(VmError::OperandStackOverflow {
                function,
                capacity: frame.operand_capacity(),
            });
        }
        let index = frame.operand_base() + frame.operand_height();
        if self.live_values.get(index).copied() == Some(true) {
            return Err(VmError::LiveDestination {
                location: VmValueLocation::Operand(frame.operand_height()),
            });
        }
        if index >= self.values.len() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        self.values[index] = value;
        self.live_values[index] = true;
        self.set_current_frame_operand_height(&frame, frame.operand_height() + 1)
    }

    fn set_current_frame_operand_height(
        &mut self,
        frame: &VmFrame,
        height: usize,
    ) -> Result<(), VmError> {
        let current = self.current_frame_mut()?;
        if current.function() != frame.function() || current.operand_base() != frame.operand_base()
        {
            return Err(VmError::FiberNotRunnable { state: self.state });
        }
        current.set_operand_height(height);
        Ok(())
    }

    fn validate_local_frame_layout(&self, function: &LinkedFunction) -> Result<(), VmError> {
        let frame = function.frame();
        if frame.slot_types().len() != frame.slot_plans().len() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameSlotPlanCount,
            });
        }
        if frame.result_types().len() != frame.result_plans().len() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::ResultTransferPlan,
            });
        }
        if frame
            .parameters()
            .iter()
            .any(|parameter| parameter.mode() != ParamModeIr::Value)
        {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::ParameterMode,
            });
        }
        Ok(())
    }

    fn parameter_transfer_slots(
        &self,
        target: FunctionIndex,
        slot_count: usize,
    ) -> Result<Vec<usize>, VmError> {
        let target_function = self.function(target)?;
        let mut seen = vec![false; slot_count];
        let mut slots = Vec::with_capacity(target_function.frame().parameters().len());
        for parameter in target_function.frame().parameters() {
            let slot = usize::try_from(parameter.slot().get()).map_err(|_| {
                VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::ParameterSlotCount,
                }
            })?;
            if slot >= slot_count {
                return Err(VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::ParameterSlotCount,
                });
            }
            if seen[slot] {
                return Err(VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::DuplicateParameterSlot,
                });
            }
            seen[slot] = true;
            slots.push(slot);
        }
        Ok(slots)
    }
}

impl VmRootSource for VmFiber {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        for (value, live) in self.values.iter().zip(&self.live_values) {
            if *live {
                visitor.visit_root(value)?;
            }
        }
        Ok(())
    }
}

enum DispatchOutcome {
    Continue,
    Complete(VmOwnedValues),
}

fn comparable_equality(left: &ValueSlot, right: &ValueSlot) -> Option<bool> {
    let kind = left.kind()?;
    if right.kind() != Some(kind) {
        return None;
    }
    match kind {
        ValueKind::Null => Some(left.is_null() && right.is_null()),
        ValueKind::Bool => Some(left.as_bool()? == right.as_bool()?),
        ValueKind::Number => Some(left.as_number()? == right.as_number()?),
        ValueKind::Integer => Some(left.as_integer()? == right.as_integer()?),
        ValueKind::Date => Some(left.as_date()? == right.as_date()?),
        _ => None,
    }
}

fn verified_function(
    program: &VerifiedLinkedBytecodeImage,
    index: FunctionIndex,
) -> Option<&LinkedFunction> {
    let function = program.functions().get(index.get() as usize)?;
    (function.index() == index).then_some(function)
}

#[cfg(test)]
fn opcode_supported(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Const
            | Opcode::CopySlot
            | Opcode::MoveSlot
            | Opcode::StoreSlot
            | Opcode::Drop
            | Opcode::Dup
            | Opcode::LoadSlot
            | Opcode::TakeSlot
            | Opcode::Pop
            | Opcode::Jump
            | Opcode::JumpIfTrue
            | Opcode::JumpIfFalse
            | Opcode::BudgetCheckpoint
            | Opcode::CallLocal
            | Opcode::TailCallLocal
            | Opcode::Return
            | Opcode::Not
            | Opcode::Negate
            | Opcode::Add
            | Opcode::Subtract
            | Opcode::Multiply
            | Opcode::Divide
            | Opcode::Equal
            | Opcode::NotEqual
            | Opcode::LessThan
            | Opcode::LessOrEqual
            | Opcode::GreaterThan
            | Opcode::GreaterOrEqual
    )
}
