mod entry_admission;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use skiff_artifact_model::{descriptor_for_opcode, Opcode, ParamModeIr};
use skiff_runtime_bytecode_verifier::{VerifiedCodeEntry, VerifiedLinkedBytecodeImage};
use skiff_runtime_deployment_image::{DeploymentOwnerIdentity, PinnedDeploymentEntry};
use skiff_runtime_linked_bytecode::{
    ActiveRegionIndex, CandidateTable, FrameSlotIndex, FunctionIndex, InstructionIndex,
    LinkedCallableSignature, LinkedCatchMatcher, LinkedExceptionRegion, LinkedFunction,
    LinkedInstruction, LinkedInstructionTarget, LinkedInterfaceTableKind,
    LinkedNativeCallableSignature, LinkedResumeSite, ResumeSiteIndex, TypeIndex,
};
use skiff_runtime_model::{
    vm_heap::{VmHeap, VmHeapError},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{ValueKind, ValueSlot},
};

use crate::{
    admission::{is_discardable_root, validate_entry_arguments},
    control::{
        AdapterInvocation, ChildInvocation, ChildTarget, StreamItem, VmOwnedValues,
        VmResumeAuthority,
    },
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
    active_regions: Vec<ActiveRegionIndex>,
    region_depths: Vec<usize>,
    unwind: Option<UnwindState>,
    pending_resume: Option<PendingResume>,
    resume_sequence: u64,
}

#[derive(Clone)]
struct UnwindState {
    payload: ValueSlot,
}

#[derive(Debug, Clone)]
struct PendingResume {
    image: Arc<VerifiedLinkedBytecodeImage>,
    sequence: u64,
    function: FunctionIndex,
    instruction: InstructionIndex,
    resume_instruction: InstructionIndex,
    resume_site: ResumeSiteIndex,
    expected_stack_height: u32,
    expected_result_count: u32,
    authority: VmResumeAuthority,
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
            active_regions: Vec::new(),
            region_depths: vec![0],
            unwind: None,
            pending_resume: None,
            resume_sequence: 0,
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

    pub fn run_segment(&mut self, heap: &mut dyn VmHeap, budget: &mut dyn VmBudget) -> VmControl {
        if self.state != VmFiberState::Runnable {
            return VmControl::Complete(Err(VmError::FiberNotRunnable { state: self.state }));
        }

        match self.run_segment_inner(heap, budget) {
            Ok(SegmentResult::Continue) => VmControl::Continue,
            Ok(SegmentResult::Complete(values)) => VmControl::Complete(Ok(values)),
            Ok(SegmentResult::Handoff(control)) => control,
            Err(error) => {
                self.state = VmFiberState::Terminal;
                VmControl::Complete(Err(error))
            }
        }
    }

    pub fn resume(&mut self, token: VmResumeToken, outcome: ResumeOutcome) -> Result<(), VmError> {
        self.resume_inner(token, outcome)
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
        self.active_regions.clear();
        self.region_depths.clear();
        self.unwind = None;
        self.pending_resume = None;
        Ok(())
    }

    fn resume_inner(
        &mut self,
        token: VmResumeToken,
        outcome: ResumeOutcome,
    ) -> Result<(), VmError> {
        let pending = self
            .pending_resume
            .take()
            .ok_or(VmError::ResumeNotExpected)?;
        if !matches!(
            self.state,
            VmFiberState::BlockedOnChild | VmFiberState::WaitingHost
        ) || !pending_matches(&pending, &token)
        {
            self.pending_resume = Some(pending);
            return Err(VmError::ResumeTokenMismatch);
        }

        match outcome {
            ResumeOutcome::Values(values) => self.resume_values(pending, values),
            ResumeOutcome::Empty => {
                let image = Arc::clone(&pending.image);
                self.resume_values(pending, VmOwnedValues::empty(image))
            }
            ResumeOutcome::Throw(values) => self.resume_throw(pending, values),
            ResumeOutcome::Failure(error) => {
                self.state = VmFiberState::Terminal;
                Err(error)
            }
            ResumeOutcome::InternalTerminal(reason) => {
                self.state = VmFiberState::Terminal;
                Err(VmError::InternalTerminal(reason))
            }
        }
    }

    fn resume_values(
        &mut self,
        pending: PendingResume,
        values: VmOwnedValues,
    ) -> Result<(), VmError> {
        if !Arc::ptr_eq(values.image(), &pending.image) {
            self.state = VmFiberState::Terminal;
            return Err(VmError::ResumeTokenMismatch);
        }
        let expected = usize::try_from(pending.expected_result_count).map_err(|_| {
            VmError::ResumeShapeMismatch {
                expected: usize::MAX,
                actual: values.len(),
            }
        })?;
        if values.len() != expected {
            self.state = VmFiberState::Terminal;
            return Err(VmError::ResumeShapeMismatch {
                expected,
                actual: values.len(),
            });
        }
        let frame = self.current_frame()?.clone();
        if frame.function() != pending.function
            || frame.instruction() != pending.instruction
            || frame.operand_height()
                != usize::try_from(pending.expected_stack_height)
                    .map_err(|_| VmError::ResumeTokenMismatch)?
        {
            self.state = VmFiberState::Terminal;
            return Err(VmError::ResumeTokenMismatch);
        }
        for value in values.values() {
            self.push_operand(*value)?;
        }
        self.current_frame_mut()?
            .resume_to(pending.resume_instruction);
        self.state = VmFiberState::Runnable;
        Ok(())
    }

    fn resume_throw(
        &mut self,
        pending: PendingResume,
        values: VmOwnedValues,
    ) -> Result<(), VmError> {
        if !Arc::ptr_eq(values.image(), &pending.image) {
            self.state = VmFiberState::Terminal;
            return Err(VmError::ResumeTokenMismatch);
        }
        if values.len() != 1 {
            self.state = VmFiberState::Terminal;
            return Err(VmError::ResumeShapeMismatch {
                expected: 1,
                actual: values.len(),
            });
        }
        let payload = values.values()[0];
        let payload_type = self
            .program()
            .candidate()
            .resume_sites()
            .get(pending.resume_site.get() as usize)
            .filter(|row| row.index() == pending.resume_site)
            .and_then(|row| row.result_types().first().copied());
        self.begin_unwind(payload, payload_type)?;
        self.state = VmFiberState::Runnable;
        Ok(())
    }

    fn run_segment_inner(
        &mut self,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<SegmentResult, VmError> {
        for _ in 0..self.limits.max_segment_instructions().get() {
            self.charge_function_entry(budget)?;
            self.consume_raw_fuel(budget)?;
            self.charge_statement_events(budget)?;
            match self.dispatch_one(heap)? {
                DispatchOutcome::Continue => {}
                DispatchOutcome::Complete(values) => return Ok(SegmentResult::Complete(values)),
                DispatchOutcome::Handoff(control) => return Ok(SegmentResult::Handoff(control)),
            }
        }
        Ok(SegmentResult::Continue)
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

    fn dispatch_one(&mut self, _heap: &mut dyn VmHeap) -> Result<DispatchOutcome, VmError> {
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
            Opcode::SwitchTag => {
                self.execute_switch_tag(function_index, instruction_index, &instruction)
            }
            Opcode::Trap => self.execute_trap(function_index, instruction_index, &instruction),
            Opcode::BudgetCheckpoint => self.execute_budget_checkpoint(function_index),
            Opcode::CallLocal => {
                self.execute_call_local(function_index, instruction_index, &instruction)
            }
            Opcode::TailCallLocal => {
                self.execute_tail_call_local(function_index, instruction_index, &instruction)
            }
            Opcode::Return => self.execute_return(function_index, instruction_index),
            Opcode::CallService => {
                self.execute_call_service(function_index, instruction_index, &instruction)
            }
            Opcode::CallActor => {
                self.execute_call_actor(function_index, instruction_index, &instruction)
            }
            Opcode::CallInterface => {
                self.execute_call_interface(function_index, instruction_index, &instruction)
            }
            Opcode::InvokeHost => {
                self.execute_invoke_host(function_index, instruction_index, &instruction)
            }
            Opcode::InvokeIntrinsic => {
                self.execute_invoke_intrinsic(function_index, instruction_index, &instruction)
            }
            Opcode::MakeCallback => {
                self.execute_make_callback(function_index, instruction_index, &instruction)
            }
            Opcode::InvokeCallback => {
                self.execute_invoke_callback(function_index, instruction_index, &instruction)
            }
            Opcode::Throw => self.execute_throw(function_index, instruction_index, &instruction),
            Opcode::Rethrow => {
                self.execute_rethrow(function_index, instruction_index, &instruction)
            }
            Opcode::EnterRegion => {
                self.execute_enter_region(function_index, instruction_index, &instruction)
            }
            Opcode::LeaveRegion => {
                self.execute_leave_region(function_index, instruction_index, &instruction)
            }
            Opcode::NewRecord => {
                self.execute_new_record(function_index, instruction_index, &instruction)
            }
            Opcode::GetDenseField => {
                self.execute_get_dense_field(function_index, instruction_index, &instruction)
            }
            Opcode::SetWritablePath => {
                self.execute_set_writable_path(function_index, instruction_index, &instruction)
            }
            Opcode::RepresentationWrap => {
                self.execute_representation_wrap(function_index, instruction_index, &instruction)
            }
            Opcode::NewArrayBuilder => {
                self.execute_new_array_builder(function_index, instruction_index, &instruction)
            }
            Opcode::ArrayBuilderPush => {
                self.execute_array_builder_push(function_index, instruction_index)
            }
            Opcode::FreezeArray => self.execute_freeze_array(function_index, instruction_index),
            Opcode::ArrayGet => self.execute_array_get(function_index, instruction_index),
            Opcode::ArrayPushOwned => {
                self.execute_array_push_owned(function_index, instruction_index, &instruction)
            }
            Opcode::ArrayLen => self.execute_array_len(function_index, instruction_index),
            Opcode::NewMapBuilder => {
                self.execute_new_map_builder(function_index, instruction_index, &instruction)
            }
            Opcode::MapBuilderPut => {
                self.execute_map_builder_put(function_index, instruction_index)
            }
            Opcode::FreezeMap => self.execute_freeze_map(function_index, instruction_index),
            Opcode::MapGet => self.execute_map_get(function_index, instruction_index),
            Opcode::MapPutOwned => {
                self.execute_map_put_owned(function_index, instruction_index, &instruction)
            }
            Opcode::MapLen => self.execute_map_len(function_index, instruction_index),
            Opcode::MapEntryAt => self.execute_map_entry_at(function_index, instruction_index),
            Opcode::InterfaceBoxLocal => {
                self.execute_interface_box_local(function_index, instruction_index, &instruction)
            }
            Opcode::InterfaceBoxRemote => {
                self.execute_interface_box_remote(function_index, instruction_index, &instruction)
            }
            Opcode::StreamNext => {
                self.execute_stream_next(function_index, instruction_index, &instruction)
            }
            Opcode::EmitStream => {
                self.execute_emit_stream(function_index, instruction_index, &instruction)
            }
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

    fn execute_switch_tag(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::SwitchTable(table_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let value = self.pop_operands(1, false)?.remove(0);
        let tag = nominal_tag_index(&value);
        let table = self
            .function(function)?
            .switch_tables()
            .get(table_index.get() as usize)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Functions,
                row: table_index.get(),
            })?;
        let target = table
            .cases()
            .iter()
            .find(|case| case.tag_type().get() == tag)
            .map(|case| case.target())
            .unwrap_or_else(|| table.default_target());
        self.validate_branch_target(target)?;
        self.current_frame_mut()?.jump_to(target);
        Ok(DispatchOutcome::Continue)
    }

    fn execute_trap(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        _decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let value = self.pop_operands(1, false)?.remove(0);
        let condition = value.as_bool().ok_or(VmError::ExpectedBoolean {
            function,
            instruction,
            actual: value.kind(),
        })?;
        if condition {
            self.advance_current_instruction()?;
            Ok(DispatchOutcome::Continue)
        } else {
            Err(VmError::AssertionFailed {
                function,
                instruction,
            })
        }
    }

    fn execute_throw(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Type(type_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let payload = self.pop_operands(1, false)?.remove(0);
        self.begin_unwind(payload, Some(type_index))
    }

    fn execute_rethrow(
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
        let payload = self.read_slot(&frame, slot_count, slot)?;
        let payload_type = self
            .function(frame.function())?
            .frame()
            .slot_types()
            .get(slot.get() as usize)
            .copied();
        self.begin_unwind(payload, payload_type)
    }

    fn execute_enter_region(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::ActiveRegion(region) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        self.function(function)?
            .active_regions()
            .get(region.get() as usize)
            .filter(|row| row.index() == region)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Functions,
                row: region.get(),
            })?;
        self.active_regions.push(region);
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_leave_region(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::ActiveRegion(region) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let actual = self
            .active_regions
            .last()
            .copied()
            .ok_or(VmError::RegionLeaveMismatch {
                function,
                instruction,
                expected: region,
                actual: ActiveRegionIndex::new(u32::MAX),
            })?;
        if actual != region {
            return Err(VmError::RegionLeaveMismatch {
                function,
                instruction,
                expected: region,
                actual,
            });
        }
        self.active_regions.pop();
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn begin_unwind(
        &mut self,
        payload: ValueSlot,
        payload_type: Option<TypeIndex>,
    ) -> Result<DispatchOutcome, VmError> {
        self.unwind = Some(UnwindState { payload });
        loop {
            let frame = self.current_frame()?.clone();
            let function = frame.function();
            let instruction = frame.instruction();
            let regions = self.function(function)?.exception_regions();
            if let Some(region) = find_exception_region(regions, instruction, payload_type) {
                let region = region.clone();
                self.enter_handler(&frame, &region, payload)?;
                return Ok(DispatchOutcome::Continue);
            }
            if self.frames.len() == 1 {
                self.frames.clear();
                self.values.clear();
                self.live_values.clear();
                self.active_regions.clear();
                self.region_depths.clear();
                self.unwind = None;
                self.state = VmFiberState::Terminal;
                return Err(VmError::UnhandledThrow {
                    function,
                    instruction,
                    payload_type,
                });
            }
            self.frames.pop();
            self.region_depths.pop();
            let caller_end =
                self.current_frame()?
                    .segment_end()
                    .ok_or(VmError::VerifiedEntryInvariant {
                        invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                    })?;
            self.values.truncate(caller_end);
            self.live_values.truncate(caller_end);
            let caller_depth = *self
                .region_depths
                .last()
                .ok_or(VmError::FiberNotRunnable { state: self.state })?;
            self.active_regions.truncate(caller_depth);
        }
    }

    fn enter_handler(
        &mut self,
        frame: &VmFrame,
        region: &LinkedExceptionRegion,
        payload: ValueSlot,
    ) -> Result<(), VmError> {
        let slot_count = self.function(frame.function())?.frame().slot_types().len();
        let handler_height = usize::try_from(region.handler_stack_height()).map_err(|_| {
            VmError::OperandStackOverflow {
                function: frame.function(),
                capacity: frame.operand_capacity(),
            }
        })?;
        if handler_height > frame.operand_capacity() {
            return Err(VmError::OperandStackOverflow {
                function: frame.function(),
                capacity: frame.operand_capacity(),
            });
        }
        let operand_base = frame.operand_base();
        let operand_end = operand_base + frame.operand_height();
        let handler_base = operand_base + handler_height;
        for index in handler_base..operand_end {
            self.clear_value(index);
        }
        self.current_frame_mut()?.set_operand_height(handler_height);
        self.write_slot(frame, slot_count, region.catch_slot(), payload)?;
        self.current_frame_mut()?.jump_to(region.handler());
        self.unwind = None;
        Ok(())
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
        self.region_depths.push(self.active_regions.len());
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
        let entry_depth = *self
            .region_depths
            .last()
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        self.active_regions.truncate(entry_depth);
        let current = self
            .frames
            .last_mut()
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        *current = replacement;
        *self
            .region_depths
            .last_mut()
            .ok_or(VmError::FiberNotRunnable { state: self.state })? = entry_depth;
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
            self.active_regions.clear();
            self.region_depths.clear();
            self.state = VmFiberState::Terminal;
            return Ok(DispatchOutcome::Complete(VmOwnedValues::new(image, values)));
        }

        let child = self
            .frames
            .pop()
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        self.region_depths.pop();
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
        let caller_depth = *self
            .region_depths
            .last()
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        self.active_regions.truncate(caller_depth);
        for value in results {
            self.push_operand(value)?;
        }
        self.current_frame_mut()?.resume_to(resume);
        Ok(DispatchOutcome::Continue)
    }

    fn execute_new_record(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Shape(shape_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let field_count = self.operand_usize(decoded, 1, function, instruction)?;
        let shape = self
            .program()
            .candidate()
            .shapes()
            .get(shape_index.get() as usize)
            .filter(|row| row.index() == shape_index)
            .cloned()
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Shapes,
                row: shape_index.get(),
            })?;
        if shape.fields().len() != field_count {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::NewRecord,
            });
        }
        let _ = self.pop_operands(field_count, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::NewRecord,
        })
    }

    fn execute_get_dense_field(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Shape(shape_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let field_ordinal = self.operand_usize(decoded, 1, function, instruction)?;
        let shape = self
            .program()
            .candidate()
            .shapes()
            .get(shape_index.get() as usize)
            .filter(|row| row.index() == shape_index)
            .cloned()
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Shapes,
                row: shape_index.get(),
            })?;
        let _ = self.pop_operands(1, false)?;
        if field_ordinal >= shape.fields().len() {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::GetDenseField,
            });
        }
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::GetDenseField,
        })
    }

    fn execute_set_writable_path(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::FrameSlot(root_slot) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let LinkedInstructionTarget::WritablePath(path_index) =
            self.resolved_target(function, instruction, decoded, 1)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let selector_count = self.operand_usize(decoded, 2, function, instruction)?;
        let path = self
            .program()
            .candidate()
            .writable_paths()
            .get(path_index.get() as usize)
            .filter(|row| row.index() == path_index)
            .cloned()
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::WritablePaths,
                row: path_index.get(),
            })?;
        if usize::try_from(path.selector_count()).unwrap_or(usize::MAX) != selector_count {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::SetWritablePath,
            });
        }
        let values = self.pop_operands(selector_count + 1, false)?;
        if path.segments().is_empty() {
            let frame = self.current_frame()?.clone();
            let slot_count = self.function(frame.function())?.frame().slot_types().len();
            let value = *values.last().ok_or(VmError::OperandStackUnderflow {
                function,
                needed: selector_count + 1,
                available: selector_count,
            })?;
            if self.live_values[Self::slot_index(&frame, slot_count, root_slot, function)?] {
                self.clear_slot(&frame, slot_count, root_slot)?;
            }
            self.write_slot(&frame, slot_count, root_slot, value)?;
            self.advance_current_instruction()?;
            return Ok(DispatchOutcome::Continue);
        }
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::SetWritablePath,
        })
    }

    fn execute_representation_wrap(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Type(type_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        self.program()
            .candidate()
            .types()
            .get(type_index.get() as usize)
            .filter(|row| row.index() == type_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Types,
                row: type_index.get(),
            })?;
        let _ = self.pop_operands(1, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::RepresentationWrap,
        })
    }

    fn execute_new_array_builder(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Type(type_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        self.program()
            .candidate()
            .types()
            .get(type_index.get() as usize)
            .filter(|row| row.index() == type_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Types,
                row: type_index.get(),
            })?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::NewArrayBuilder,
        })
    }

    fn execute_array_builder_push(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let _ = self.pop_operands(2, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::ArrayBuilderPush,
        })
    }

    fn execute_freeze_array(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let _ = self.pop_operands(1, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::FreezeArray,
        })
    }

    fn execute_array_get(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let _ = self.pop_operands(2, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::ArrayGet,
        })
    }

    fn execute_array_push_owned(
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
        let _ = self.pop_operands(1, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::ArrayPushOwned,
        })
    }

    fn execute_array_len(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let _ = self.pop_operands(1, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::ArrayLen,
        })
    }

    fn execute_new_map_builder(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Type(key_type) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let LinkedInstructionTarget::Type(value_type) =
            self.resolved_target(function, instruction, decoded, 1)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let types = self.program().candidate().types();
        types
            .get(key_type.get() as usize)
            .filter(|row| row.index() == key_type)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Types,
                row: key_type.get(),
            })?;
        types
            .get(value_type.get() as usize)
            .filter(|row| row.index() == value_type)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Types,
                row: value_type.get(),
            })?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::NewMapBuilder,
        })
    }

    fn execute_map_builder_put(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let _ = self.pop_operands(3, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::MapBuilderPut,
        })
    }

    fn execute_freeze_map(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let _ = self.pop_operands(1, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::FreezeMap,
        })
    }

    fn execute_map_get(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let _ = self.pop_operands(2, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::MapGet,
        })
    }

    fn execute_map_put_owned(
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
        let _ = self.pop_operands(2, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::MapPutOwned,
        })
    }

    fn execute_map_len(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let _ = self.pop_operands(1, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::MapLen,
        })
    }

    fn execute_map_entry_at(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let _ = self.pop_operands(2, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::MapEntryAt,
        })
    }

    fn execute_interface_box_local(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::InterfaceTable(table_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        self.program()
            .candidate()
            .interface_tables()
            .get(table_index.get() as usize)
            .filter(|row| row.index() == table_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::InterfaceTables,
                row: table_index.get(),
            })?;
        let _ = self.pop_operands(1, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::InterfaceBoxLocal,
        })
    }

    fn execute_interface_box_remote(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::InterfaceTable(table_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        self.program()
            .candidate()
            .interface_tables()
            .get(table_index.get() as usize)
            .filter(|row| row.index() == table_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::InterfaceTables,
                row: table_index.get(),
            })?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::InterfaceBoxRemote,
        })
    }

    fn execute_call_service(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::ServiceOperation(target_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let arg_count = self.operand_usize(decoded, 1, function, instruction)?;
        let result_count = self.operand_usize(decoded, 2, function, instruction)?;
        let LinkedInstructionTarget::ResumeSite(resume_site) =
            self.resolved_target(function, instruction, decoded, 3)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let target = self
            .program()
            .candidate()
            .service_operations()
            .get(target_index.get() as usize)
            .filter(|row| row.index() == target_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::ServiceOperations,
                row: target_index.get(),
            })?;
        validate_signature_counts(
            target.signature(),
            arg_count,
            result_count,
            function,
            instruction,
            Opcode::CallService,
        )?;
        let resume = self
            .linked_resume_site(function, instruction, Opcode::CallService, resume_site)?
            .clone();
        if resume.result_types().len() != result_count {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::CallService,
            });
        }
        let arguments = self.pop_operands(arg_count, false)?;
        let expected_stack_height =
            self.current_frame()?
                .operand_height()
                .try_into()
                .map_err(|_| VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        if expected_stack_height != resume.expected_stack_height_before_result() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        let target = ChildTarget::Service(target_index);
        let token = self.mint_resume(
            function,
            instruction,
            VmResumeAuthority::Child(target),
            resume_site,
            resume.resume(),
            expected_stack_height,
            result_count as u32,
        )?;
        let invocation = ChildInvocation::new(
            target,
            VmOwnedValues::new(
                Arc::clone(self.entry.image().program()),
                arguments.into_boxed_slice(),
            ),
            token,
        )
        .map_err(|_| VmError::ResumeTokenMismatch)?;
        self.state = VmFiberState::BlockedOnChild;
        Ok(DispatchOutcome::Handoff(VmControl::EnterChild(invocation)))
    }

    fn execute_call_actor(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::ActorMethod(target_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let arg_count = self.operand_usize(decoded, 1, function, instruction)?;
        let result_count = self.operand_usize(decoded, 2, function, instruction)?;
        let LinkedInstructionTarget::ResumeSite(resume_site) =
            self.resolved_target(function, instruction, decoded, 3)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let target = self
            .program()
            .candidate()
            .actor_methods()
            .get(target_index.get() as usize)
            .filter(|row| row.index() == target_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::ActorMethods,
                row: target_index.get(),
            })?;
        validate_signature_counts(
            target.signature(),
            arg_count,
            result_count,
            function,
            instruction,
            Opcode::CallActor,
        )?;
        let resume = self
            .linked_resume_site(function, instruction, Opcode::CallActor, resume_site)?
            .clone();
        if resume.result_types().len() != result_count {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::CallActor,
            });
        }
        let arguments = self.pop_operands(arg_count, false)?;
        let expected_stack_height =
            self.current_frame()?
                .operand_height()
                .try_into()
                .map_err(|_| VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        if expected_stack_height != resume.expected_stack_height_before_result() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        let target = ChildTarget::Actor(target_index);
        let token = self.mint_resume(
            function,
            instruction,
            VmResumeAuthority::Child(target),
            resume_site,
            resume.resume(),
            expected_stack_height,
            result_count as u32,
        )?;
        let invocation = ChildInvocation::new(
            target,
            VmOwnedValues::new(
                Arc::clone(self.entry.image().program()),
                arguments.into_boxed_slice(),
            ),
            token,
        )
        .map_err(|_| VmError::ResumeTokenMismatch)?;
        self.state = VmFiberState::BlockedOnChild;
        Ok(DispatchOutcome::Handoff(VmControl::EnterChild(invocation)))
    }

    fn execute_call_interface(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        self.execute_interface_boundary(function, instruction, decoded, Opcode::CallInterface)
    }

    fn execute_invoke_callback(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        self.execute_interface_boundary(function, instruction, decoded, Opcode::InvokeCallback)
    }

    fn execute_interface_boundary(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
        opcode: Opcode,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::InterfaceTable(table_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let method_ordinal = self.operand_usize(decoded, 1, function, instruction)?;
        let arg_count = self.operand_usize(decoded, 2, function, instruction)?;
        let result_count = self.operand_usize(decoded, 3, function, instruction)?;
        let LinkedInstructionTarget::ResumeSite(resume_site) =
            self.resolved_target(function, instruction, decoded, 4)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let table = self
            .program()
            .candidate()
            .interface_tables()
            .get(table_index.get() as usize)
            .filter(|row| row.index() == table_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::InterfaceTables,
                row: table_index.get(),
            })?;
        let signature = match table.kind() {
            LinkedInterfaceTableKind::Requirement(requirement)
            | LinkedInterfaceTableKind::Callback(requirement) => requirement
                .methods()
                .get(method_ordinal)
                .map(|method| method.signature()),
            LinkedInterfaceTableKind::Local(local) => local
                .methods()
                .get(method_ordinal)
                .map(|method| method.signature()),
            LinkedInterfaceTableKind::Remote(remote) => remote
                .methods()
                .get(method_ordinal)
                .map(|method| method.signature()),
        }
        .ok_or(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode,
        })?;
        if signature.result_types().len() != result_count {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode,
            });
        }
        let resume = self
            .linked_resume_site(function, instruction, opcode, resume_site)?
            .clone();
        if resume.result_types().len() != result_count {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode,
            });
        }
        let input_count = arg_count
            .checked_add(1)
            .ok_or(VmError::OperandStackOverflow {
                function,
                capacity: self.current_frame()?.operand_capacity(),
            })?;
        let arguments = self.pop_operands(input_count, false)?;
        let expected_stack_height =
            self.current_frame()?
                .operand_height()
                .try_into()
                .map_err(|_| VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        if expected_stack_height != resume.expected_stack_height_before_result() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        let target = ChildTarget::Interface {
            table: table_index,
            method_ordinal: method_ordinal as u32,
        };
        let token = self.mint_resume(
            function,
            instruction,
            VmResumeAuthority::Child(target),
            resume_site,
            resume.resume(),
            expected_stack_height,
            result_count as u32,
        )?;
        let invocation = ChildInvocation::new(
            target,
            VmOwnedValues::new(
                Arc::clone(self.entry.image().program()),
                arguments.into_boxed_slice(),
            ),
            token,
        )
        .map_err(|_| VmError::ResumeTokenMismatch)?;
        self.state = VmFiberState::BlockedOnChild;
        Ok(DispatchOutcome::Handoff(VmControl::EnterChild(invocation)))
    }

    fn execute_invoke_host(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::HostEffectAdapter(adapter_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let arg_count = self.operand_usize(decoded, 1, function, instruction)?;
        let result_count = self.operand_usize(decoded, 2, function, instruction)?;
        let LinkedInstructionTarget::ResumeSite(resume_site) =
            self.resolved_target(function, instruction, decoded, 3)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let adapter = self
            .program()
            .candidate()
            .host_effect_adapters()
            .get(adapter_index.get() as usize)
            .filter(|row| row.index() == adapter_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::HostEffectAdapters,
                row: adapter_index.get(),
            })?;
        validate_native_signature_counts(
            adapter.signature(),
            arg_count,
            result_count,
            function,
            instruction,
            Opcode::InvokeHost,
        )?;
        let resume = self
            .linked_resume_site(function, instruction, Opcode::InvokeHost, resume_site)?
            .clone();
        if resume.result_types().len() != result_count {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeHost,
            });
        }
        let arguments = self.pop_operands(arg_count, false)?;
        let expected_stack_height =
            self.current_frame()?
                .operand_height()
                .try_into()
                .map_err(|_| VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        if expected_stack_height != resume.expected_stack_height_before_result() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        let token = self.mint_resume(
            function,
            instruction,
            VmResumeAuthority::Adapter(adapter_index),
            resume_site,
            resume.resume(),
            expected_stack_height,
            result_count as u32,
        )?;
        let invocation = AdapterInvocation::new(
            adapter_index,
            VmOwnedValues::new(
                Arc::clone(self.entry.image().program()),
                arguments.into_boxed_slice(),
            ),
            token,
        )
        .map_err(|_| VmError::ResumeTokenMismatch)?;
        self.state = VmFiberState::WaitingHost;
        Ok(DispatchOutcome::Handoff(VmControl::EnterAdapter(
            invocation,
        )))
    }

    fn execute_invoke_intrinsic(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Intrinsic(intrinsic_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let arg_count = self.operand_usize(decoded, 1, function, instruction)?;
        let result_count = self.operand_usize(decoded, 2, function, instruction)?;
        let intrinsic = self
            .program()
            .candidate()
            .intrinsics()
            .get(intrinsic_index.get() as usize)
            .filter(|row| row.index() == intrinsic_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Intrinsics,
                row: intrinsic_index.get(),
            })?;
        validate_native_signature_counts(
            intrinsic.signature(),
            arg_count,
            result_count,
            function,
            instruction,
            Opcode::InvokeIntrinsic,
        )?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::InvokeIntrinsic,
        })
    }

    fn execute_make_callback(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::SyntheticCallback(callback_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let LinkedInstructionTarget::CallbackCaptureLayout(layout_index) =
            self.resolved_target(function, instruction, decoded, 1)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let capture_count = self.operand_usize(decoded, 2, function, instruction)?;
        self.program()
            .candidate()
            .synthetic_callbacks()
            .get(callback_index.get() as usize)
            .filter(|row| row.index() == callback_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::SyntheticCallbacks,
                row: callback_index.get(),
            })?;
        let layout = self
            .program()
            .candidate()
            .callback_capture_layouts()
            .get(layout_index.get() as usize)
            .filter(|row| row.index() == layout_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::CallbackCaptureLayouts,
                row: layout_index.get(),
            })?;
        if layout.captures().len() != capture_count {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::MakeCallback,
            });
        }
        let _ = self.pop_operands(capture_count, false)?;
        Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode: Opcode::MakeCallback,
        })
    }

    fn execute_stream_next(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::FrameSlot(endpoint_slot) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let LinkedInstructionTarget::ResumeSite(resume_site) =
            self.resolved_target(function, instruction, decoded, 1)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let frame = self.current_frame()?.clone();
        let slot_count = self.function(frame.function())?.frame().slot_types().len();
        let endpoint = self.read_slot(&frame, slot_count, endpoint_slot)?;
        self.clear_slot(&frame, slot_count, endpoint_slot)?;
        let resume = self
            .linked_resume_site(function, instruction, Opcode::StreamNext, resume_site)?
            .clone();
        if resume.result_types().len() != 1 {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::StreamNext,
            });
        }
        let arguments = VmOwnedValues::new(
            Arc::clone(self.entry.image().program()),
            Box::new([endpoint]),
        );
        let expected_stack_height =
            self.current_frame()?
                .operand_height()
                .try_into()
                .map_err(|_| VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        if expected_stack_height != resume.expected_stack_height_before_result() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        let token = self.mint_resume(
            function,
            instruction,
            VmResumeAuthority::StreamChild(ChildTarget::StreamNext),
            resume_site,
            resume.resume(),
            expected_stack_height,
            1,
        )?;
        let invocation = ChildInvocation::new(ChildTarget::StreamNext, arguments, token)
            .map_err(|_| VmError::ResumeTokenMismatch)?;
        self.state = VmFiberState::BlockedOnChild;
        Ok(DispatchOutcome::Handoff(VmControl::EnterChild(invocation)))
    }

    fn execute_emit_stream(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::ResumeSite(resume_site) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let resume = self
            .linked_resume_site(function, instruction, Opcode::EmitStream, resume_site)?
            .clone();
        if !resume.result_types().is_empty() {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::EmitStream,
            });
        }
        let item = self.pop_operands(1, false)?;
        let expected_stack_height =
            self.current_frame()?
                .operand_height()
                .try_into()
                .map_err(|_| VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        if expected_stack_height != resume.expected_stack_height_before_result() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        let token = self.mint_resume(
            function,
            instruction,
            VmResumeAuthority::StreamItem,
            resume_site,
            resume.resume(),
            expected_stack_height,
            0,
        )?;
        let stream_item = StreamItem::new(
            VmOwnedValues::new(
                Arc::clone(self.entry.image().program()),
                item.into_boxed_slice(),
            ),
            token,
        )
        .map_err(|_| VmError::ResumeTokenMismatch)?;
        self.state = VmFiberState::WaitingHost;
        Ok(DispatchOutcome::Handoff(VmControl::EmitStream(stream_item)))
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

    fn linked_resume_site(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
        index: ResumeSiteIndex,
    ) -> Result<&LinkedResumeSite, VmError> {
        let row = self
            .program()
            .candidate()
            .resume_sites()
            .get(index.get() as usize)
            .filter(|row| {
                row.index() == index && row.function() == function && row.site() == instruction
            })
            .ok_or(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode,
            })?;
        let function_len = self.function(function)?.instructions().len();
        if row.resume().get() as usize >= function_len {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode,
            });
        }
        Ok(row)
    }

    #[allow(clippy::too_many_arguments)]
    fn mint_resume(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        authority: VmResumeAuthority,
        resume_site: ResumeSiteIndex,
        resume_instruction: InstructionIndex,
        expected_stack_height: u32,
        expected_result_count: u32,
    ) -> Result<VmResumeToken, VmError> {
        let image = Arc::clone(self.entry.image().program());
        let sequence = self.resume_sequence;
        self.resume_sequence = self
            .resume_sequence
            .checked_add(1)
            .ok_or(VmError::ResumeTokenMismatch)?;
        let token = VmResumeToken::new(
            image.clone(),
            sequence,
            function,
            instruction,
            resume_instruction,
            resume_site,
            expected_stack_height,
            expected_result_count,
            authority,
        );
        self.pending_resume = Some(PendingResume {
            image,
            sequence,
            function,
            instruction,
            resume_instruction,
            resume_site,
            expected_stack_height,
            expected_result_count,
            authority,
        });
        Ok(token)
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
        if let Some(unwind) = &self.unwind {
            visitor.visit_root(&unwind.payload)?;
        }
        Ok(())
    }
}

enum SegmentResult {
    Continue,
    Complete(VmOwnedValues),
    Handoff(VmControl),
}

enum DispatchOutcome {
    Continue,
    Complete(VmOwnedValues),
    Handoff(VmControl),
}

fn pending_matches(pending: &PendingResume, token: &VmResumeToken) -> bool {
    Arc::ptr_eq(&pending.image, token.image())
        && pending.sequence == token.sequence()
        && pending.function == token.function()
        && pending.instruction == token.instruction()
        && pending.resume_instruction == token.resume_instruction()
        && pending.resume_site == token.resume_site()
        && pending.expected_stack_height == token.expected_stack_height()
        && pending.expected_result_count == token.expected_result_count()
        && pending.authority == token.authority()
}

fn nominal_tag_index(value: &ValueSlot) -> u32 {
    match value.kind() {
        Some(
            ValueKind::RequestHeapRef
            | ValueKind::ActorStateRef
            | ValueKind::ConstRef
            | ValueKind::ResourceRef
            | ValueKind::CallbackClosureRef,
        ) => value.compact_type_tag().get(),
        _ => 0,
    }
}

fn find_exception_region(
    regions: &[LinkedExceptionRegion],
    pc: InstructionIndex,
    payload_type: Option<TypeIndex>,
) -> Option<&LinkedExceptionRegion> {
    regions.iter().rev().find(|region| {
        region.start().get() <= pc.get()
            && pc.get() < region.end().get()
            && region
                .catch_matchers()
                .iter()
                .any(|matcher| catch_matches(matcher, payload_type))
    })
}

fn catch_matches(matcher: &LinkedCatchMatcher, payload_type: Option<TypeIndex>) -> bool {
    match matcher {
        LinkedCatchMatcher::CatchAll => true,
        LinkedCatchMatcher::Type(expected) => payload_type == Some(*expected),
    }
}

fn validate_signature_counts(
    signature: &LinkedCallableSignature,
    arg_count: usize,
    result_count: usize,
    function: FunctionIndex,
    instruction: InstructionIndex,
    opcode: Opcode,
) -> Result<(), VmError> {
    if signature.parameter_types().len() != arg_count
        || signature.result_types().len() != result_count
        || signature.parameter_modes().contains(&ParamModeIr::InOut)
    {
        return Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode,
        });
    }
    Ok(())
}

fn validate_native_signature_counts(
    signature: &LinkedNativeCallableSignature,
    arg_count: usize,
    result_count: usize,
    function: FunctionIndex,
    instruction: InstructionIndex,
    opcode: Opcode,
) -> Result<(), VmError> {
    if signature.parameter_types().len() != arg_count
        || signature.result_types().len() != result_count
        || signature.parameter_modes().contains(&ParamModeIr::InOut)
    {
        return Err(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode,
        });
    }
    Ok(())
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
            | Opcode::SwitchTag
            | Opcode::Trap
            | Opcode::BudgetCheckpoint
            | Opcode::CallLocal
            | Opcode::TailCallLocal
            | Opcode::Return
            | Opcode::CallService
            | Opcode::CallActor
            | Opcode::CallInterface
            | Opcode::MakeCallback
            | Opcode::InvokeCallback
            | Opcode::NewRecord
            | Opcode::GetDenseField
            | Opcode::SetWritablePath
            | Opcode::RepresentationWrap
            | Opcode::NewArrayBuilder
            | Opcode::ArrayBuilderPush
            | Opcode::FreezeArray
            | Opcode::ArrayGet
            | Opcode::ArrayPushOwned
            | Opcode::ArrayLen
            | Opcode::NewMapBuilder
            | Opcode::MapBuilderPut
            | Opcode::FreezeMap
            | Opcode::MapGet
            | Opcode::MapPutOwned
            | Opcode::MapLen
            | Opcode::MapEntryAt
            | Opcode::StreamNext
            | Opcode::EmitStream
            | Opcode::Throw
            | Opcode::Rethrow
            | Opcode::EnterRegion
            | Opcode::LeaveRegion
            | Opcode::InvokeHost
            | Opcode::InvokeIntrinsic
            | Opcode::InterfaceBoxLocal
            | Opcode::InterfaceBoxRemote
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
