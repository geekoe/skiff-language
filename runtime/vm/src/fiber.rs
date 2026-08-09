use std::sync::Arc;

use skiff_artifact_model::{descriptor_for_opcode, Opcode, ParamModeIr, ValueTransferPlanKind};
use skiff_runtime_bytecode_verifier::{VerifiedCodeEntry, VerifiedLinkedBytecodeImage};
use skiff_runtime_deployment_image::{DeploymentOwnerIdentity, PinnedDeploymentEntry};
use skiff_runtime_linked_bytecode::{
    FrameSlotIndex, FunctionIndex, InstructionIndex, LinkedCallableSignature, LinkedFrameLayout,
    LinkedFunction,
};
use skiff_runtime_model::{
    vm_heap::{VmHeap, VmHeapError},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::ValueSlot,
};

use crate::{
    admission::validate_entry_arguments, frame::VmFrame, ResumeOutcome, VmBudget, VmControl,
    VmError, VmLimits, VmOwnedValues, VmResumeToken, VmSemanticCharge, VmSemanticChargeKind,
    VmValueLocation, VmVerifiedInvariant,
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

    /// Starts execution from one exact, verified and deployment-pinned entry.
    ///
    /// There is deliberately no overload for a candidate, a raw verified
    /// image plus function index, or a generic deployment entry.
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

/// Synchronous bytecode fiber. Frame and value-stack representation is private
/// so callers cannot forge a pc, inject a function index, or bypass entry pin
/// admission.
#[must_use = "a VM fiber owns live roots until completion or explicit terminal discard"]
pub struct VmFiber {
    entry: VerifiedVmEntry,
    frames: Vec<VmFrame>,
    values: Vec<ValueSlot>,
    live_values: Vec<bool>,
    value_plans: Vec<Option<ValueTransferPlanKind>>,
    state: VmFiberState,
    limits: VmLimits,
    raw_fuel_remaining: u32,
    entry_heap_validation_pending: bool,
}

impl VmFiber {
    fn start(
        entry: VerifiedVmEntry,
        arguments: Box<[ValueSlot]>,
        limits: VmLimits,
    ) -> Result<Self, VmError> {
        validate_entry_arguments(&arguments)?;

        let function_index = entry.entry().function();
        let program = entry.image().program();
        let function =
            verified_function(program, function_index).ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::EntryFunctionMissing,
            })?;
        validate_entry_contract(entry.entry(), function, arguments.len())?;

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

        let operand_base = slot_count;
        let frame = VmFrame::root(
            function_index,
            0,
            slot_count,
            operand_base,
            operand_capacity,
        );
        debug_assert_eq!(frame.segment_end(), Some(segment_len));
        let mut values = vec![ValueSlot::null(); segment_len];
        let mut live_values = vec![false; segment_len];
        let mut value_plans = vec![None; segment_len];
        for ((argument, parameter_slot), plan) in arguments
            .into_vec()
            .into_iter()
            .zip(function.frame().parameter_slots().iter().copied())
            .zip(entry.entry().signature().parameter_plans().iter().copied())
        {
            let index = usize::try_from(parameter_slot.get()).map_err(|_| {
                VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                }
            })?;
            values[index] = argument;
            live_values[index] = true;
            value_plans[index] = Some(require_concrete_transfer_plan(plan)?);
        }

        Ok(Self {
            entry,
            frames: vec![frame],
            values,
            live_values,
            value_plans,
            state: VmFiberState::Runnable,
            limits,
            raw_fuel_remaining: 0,
            entry_heap_validation_pending: true,
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

    /// Runs a bounded synchronous segment. External work is never performed
    /// through a callback/trait port; implemented effect opcodes will return a
    /// typed [`VmControl`] handoff to the scheduler.
    pub fn run_segment(&mut self, heap: &mut dyn VmHeap, budget: &mut dyn VmBudget) -> VmControl {
        if self.state != VmFiberState::Runnable {
            return VmControl::Complete(Err(VmError::FiberNotRunnable { state: self.state }));
        }

        match self.run_segment_inner(heap, budget) {
            Ok(Some(values)) => VmControl::Complete(Ok(values)),
            Ok(None) => VmControl::Continue,
            Err(error) => {
                self.state = VmFiberState::Terminal;
                VmControl::Complete(Err(error))
            }
        }
    }

    /// Resumes a continuation previously moved out through typed control.
    ///
    /// The current opcode slice cannot emit such a token. Until an effect
    /// opcode is implemented, every attempted resume fails closed.
    pub fn resume(&mut self, token: VmResumeToken, outcome: ResumeOutcome) -> Result<(), VmError> {
        let _ = (token, outcome);
        Err(VmError::ResumeNotExpected)
    }

    /// Explicitly releases every value still owned by an error-terminal fiber.
    ///
    /// `run_segment` never performs this cleanup implicitly: the fiber remains
    /// a root source after failure until its owner successfully calls this
    /// method (and may retry after a heap error).
    pub fn discard_terminal_roots(&mut self, heap: &mut dyn VmHeap) -> Result<(), VmError> {
        if self.state != VmFiberState::Terminal {
            return Err(VmError::DiscardRequiresTerminal { state: self.state });
        }

        for index in 0..self.values.len() {
            if self.live_values[index] {
                let _ = self.require_live_plan(index)?;
            }
        }
        for index in 0..self.values.len() {
            if self.live_values[index] {
                let plan = self.require_live_plan(index)?;
                heap.drop_value(self.values[index], plan)?;
                self.clear_value(index);
            }
        }
        self.frames.clear();
        self.values.clear();
        self.live_values.clear();
        self.value_plans.clear();
        Ok(())
    }

    fn run_segment_inner(
        &mut self,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<Option<VmOwnedValues>, VmError> {
        self.validate_entry_heap_values(heap)?;

        for _ in 0..self.limits.max_segment_instructions().get() {
            self.charge_function_entry(budget)?;
            self.consume_raw_fuel(budget)?;
            self.charge_statement_entries(budget)?;
            if let Some(values) = self.dispatch_one(heap, budget)? {
                return Ok(Some(values));
            }
        }
        Ok(None)
    }

    fn validate_entry_heap_values(&mut self, heap: &dyn VmHeap) -> Result<(), VmError> {
        if !self.entry_heap_validation_pending {
            return Ok(());
        }
        for (value, live) in self.values.iter().zip(&self.live_values) {
            if *live {
                heap.validate_live(value)?;
            }
        }
        self.entry_heap_validation_pending = false;
        Ok(())
    }

    fn charge_function_entry(&mut self, budget: &mut dyn VmBudget) -> Result<(), VmError> {
        let frame = self.current_frame()?;
        if !frame.function_entry_pending() {
            return Ok(());
        }
        let function = frame.function();
        let instruction = frame.instruction();
        budget.charge_semantic(VmSemanticCharge::new(
            function,
            instruction,
            VmSemanticChargeKind::FunctionEntry,
        ))?;
        self.current_frame_mut()?.mark_function_entry_charged();
        Ok(())
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

    fn charge_statement_entries(&self, budget: &mut dyn VmBudget) -> Result<(), VmError> {
        let frame = self.current_frame()?;
        let function = self.function(frame.function())?;
        for statement in function
            .statement_entries()
            .iter()
            .filter(|entry| entry.instruction() == frame.instruction())
        {
            budget.charge_semantic(VmSemanticCharge::new(
                frame.function(),
                frame.instruction(),
                VmSemanticChargeKind::Statement {
                    statement_id: statement.statement_id(),
                },
            ))?;
        }
        Ok(())
    }

    fn dispatch_one(
        &mut self,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<Option<VmOwnedValues>, VmError> {
        let (function_index, instruction_index, instruction) = {
            let frame = self.current_frame()?;
            let function = self.function(frame.function())?;
            let instruction = function
                .instructions()
                .get(index_to_usize(frame.instruction()))
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
            Opcode::CopySlot => self.execute_copy_slot(heap, instruction.operands())?,
            Opcode::MoveSlot => self.execute_move_slot(heap, instruction.operands())?,
            Opcode::StoreSlot => self.execute_store_slot(instruction.operands())?,
            Opcode::Dup => self.execute_dup(heap)?,
            Opcode::LoadSlot => self.execute_load_slot(heap, instruction.operands())?,
            Opcode::TakeSlot => self.execute_take_slot(heap, instruction.operands())?,
            Opcode::Pop => self.execute_pop(heap)?,
            Opcode::Drop => self.execute_drop(heap, instruction.operands())?,
            Opcode::BudgetCheckpoint => {
                budget.poll_interrupt()?;
                budget.charge_semantic(VmSemanticCharge::new(
                    function_index,
                    instruction_index,
                    VmSemanticChargeKind::BudgetCheckpoint,
                ))?;
                self.advance_current_instruction()?;
            }
            Opcode::Return => return self.finish_root_return(heap).map(Some),
            opcode => {
                // In particular, Const remains here until the verified
                // constant heap exposes its concrete linked transfer plan;
                // the VM never infers that plan from ValueKind.
                return Err(VmError::UnsupportedOpcode {
                    function: function_index,
                    instruction: instruction_index,
                    opcode,
                });
            }
        }
        Ok(None)
    }

    fn execute_copy_slot(
        &mut self,
        heap: &mut dyn VmHeap,
        operands: &[u32],
    ) -> Result<(), VmError> {
        let source = FrameSlotIndex::new(operands[0]);
        let destination = FrameSlotIndex::new(operands[1]);
        let source_index = self.absolute_slot(source)?;
        let destination_index = self.absolute_slot(destination)?;
        self.require_dead_destination(destination_index, VmValueLocation::FrameSlot(destination))?;
        let value = self.require_live_value(source_index, VmValueLocation::FrameSlot(source))?;
        let source_plan = self.slot_plan(source)?;
        self.require_expected_plan(
            source_index,
            VmValueLocation::FrameSlot(source),
            source_plan,
        )?;
        let destination_plan = self.slot_plan(destination)?;
        self.require_matching_plans(
            VmValueLocation::FrameSlot(destination),
            destination_plan,
            source_plan,
        )?;
        let snapshot = heap.snapshot(&value, source_plan)?;
        self.set_live_value(destination_index, snapshot, destination_plan);
        self.advance_current_instruction()
    }

    fn execute_move_slot(
        &mut self,
        heap: &mut dyn VmHeap,
        operands: &[u32],
    ) -> Result<(), VmError> {
        let source = FrameSlotIndex::new(operands[0]);
        let destination = FrameSlotIndex::new(operands[1]);
        let source_index = self.absolute_slot(source)?;
        let destination_index = self.absolute_slot(destination)?;
        self.require_dead_destination(destination_index, VmValueLocation::FrameSlot(destination))?;
        let value = self.require_live_value(source_index, VmValueLocation::FrameSlot(source))?;
        let source_plan = self.slot_plan(source)?;
        self.require_expected_plan(
            source_index,
            VmValueLocation::FrameSlot(source),
            source_plan,
        )?;
        let destination_plan = self.slot_plan(destination)?;
        self.require_matching_plans(
            VmValueLocation::FrameSlot(destination),
            destination_plan,
            source_plan,
        )?;
        let transferred = heap.transfer(value, source_plan)?;
        self.clear_value(source_index);
        self.set_live_value(destination_index, transferred, destination_plan);
        self.advance_current_instruction()
    }

    fn execute_store_slot(&mut self, operands: &[u32]) -> Result<(), VmError> {
        let destination = FrameSlotIndex::new(operands[0]);
        let destination_index = self.absolute_slot(destination)?;
        self.require_dead_destination(destination_index, VmValueLocation::FrameSlot(destination))?;
        let destination_plan = self.slot_plan(destination)?;
        let (_, operand_plan) = self.peek_operand_with_plan()?;
        self.require_matching_plans(
            VmValueLocation::FrameSlot(destination),
            destination_plan,
            operand_plan,
        )?;
        let (value, plan) = self.pop_operand()?;
        self.set_live_value(destination_index, value, plan);
        self.advance_current_instruction()
    }

    fn execute_dup(&mut self, heap: &mut dyn VmHeap) -> Result<(), VmError> {
        let (value, plan) = self.peek_operand_with_plan()?;
        let (destination, depth) = self.prepare_operand_push()?;
        let snapshot = heap.snapshot(&value, plan)?;
        self.commit_operand_push(destination, depth, snapshot, plan)?;
        self.advance_current_instruction()
    }

    fn execute_load_slot(
        &mut self,
        heap: &mut dyn VmHeap,
        operands: &[u32],
    ) -> Result<(), VmError> {
        let source = FrameSlotIndex::new(operands[0]);
        let source_index = self.absolute_slot(source)?;
        let value = self.require_live_value(source_index, VmValueLocation::FrameSlot(source))?;
        let plan = self.slot_plan(source)?;
        self.require_expected_plan(source_index, VmValueLocation::FrameSlot(source), plan)?;
        let (destination, depth) = self.prepare_operand_push()?;
        let snapshot = heap.snapshot(&value, plan)?;
        self.commit_operand_push(destination, depth, snapshot, plan)?;
        self.advance_current_instruction()
    }

    fn execute_take_slot(
        &mut self,
        heap: &mut dyn VmHeap,
        operands: &[u32],
    ) -> Result<(), VmError> {
        let source = FrameSlotIndex::new(operands[0]);
        let source_index = self.absolute_slot(source)?;
        let value = self.require_live_value(source_index, VmValueLocation::FrameSlot(source))?;
        let plan = self.slot_plan(source)?;
        self.require_expected_plan(source_index, VmValueLocation::FrameSlot(source), plan)?;
        let (destination, depth) = self.prepare_operand_push()?;
        let transferred = heap.transfer(value, plan)?;
        self.clear_value(source_index);
        self.commit_operand_push(destination, depth, transferred, plan)?;
        self.advance_current_instruction()
    }

    fn execute_pop(&mut self, heap: &mut dyn VmHeap) -> Result<(), VmError> {
        let (value, plan) = self.peek_operand_with_plan()?;
        heap.drop_value(value, plan)?;
        let _ = self.pop_operand()?;
        self.advance_current_instruction()
    }

    fn execute_drop(&mut self, heap: &mut dyn VmHeap, operands: &[u32]) -> Result<(), VmError> {
        let slot = FrameSlotIndex::new(operands[0]);
        let index = self.absolute_slot(slot)?;
        let value = self.require_live_value(index, VmValueLocation::FrameSlot(slot))?;
        let plan = self.slot_plan(slot)?;
        self.require_expected_plan(index, VmValueLocation::FrameSlot(slot), plan)?;
        heap.drop_value(value, plan)?;
        self.clear_value(index);
        self.advance_current_instruction()
    }

    fn finish_root_return(&mut self, heap: &mut dyn VmHeap) -> Result<VmOwnedValues, VmError> {
        let frame = self.current_frame()?;
        let function_index = frame.function();
        let function = self.function(function_index)?;
        let expected = function.frame().result_types().len();
        let actual = frame.operand_depth();
        if actual != expected {
            return Err(VmError::OperandStackShapeMismatch {
                function: function_index,
                expected,
                actual,
            });
        }

        let slot_base = frame.slot_base();
        let slot_count = frame.slot_count();
        let operand_base = frame.operand_base();
        let slot_plans = function.frame().slot_plans().to_vec();
        let result_plans = function.frame().result_plans().to_vec();
        let image = Arc::clone(self.entry.image().program());

        for (relative, plan) in slot_plans.iter().copied().enumerate().take(slot_count) {
            let index = slot_base + relative;
            if self.live_values[index] {
                let slot = u32::try_from(relative)
                    .map(FrameSlotIndex::new)
                    .map_err(|_| VmError::VerifiedEntryInvariant {
                        invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                    })?;
                self.require_expected_plan(index, VmValueLocation::FrameSlot(slot), plan)?;
            }
        }

        let mut results = Vec::with_capacity(expected);
        for (offset, plan) in result_plans.iter().copied().enumerate() {
            let index = operand_base + offset;
            let value = self.require_live_value(index, VmValueLocation::Operand(offset))?;
            self.require_expected_plan(index, VmValueLocation::Operand(offset), plan)?;
            results.push(value);
        }

        for (relative, plan) in slot_plans.into_iter().enumerate().take(slot_count) {
            let index = slot_base + relative;
            if self.live_values[index] {
                heap.drop_value(self.values[index], plan)?;
                self.clear_value(index);
            }
        }
        for offset in 0..expected {
            let index = operand_base + offset;
            self.clear_value(index);
        }

        self.frames.clear();
        self.values.clear();
        self.live_values.clear();
        self.value_plans.clear();
        self.state = VmFiberState::Terminal;
        VmOwnedValues::new(
            image,
            results.into_boxed_slice(),
            result_plans.into_boxed_slice(),
        )
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

    fn slot_plan(&self, slot: FrameSlotIndex) -> Result<ValueTransferPlanKind, VmError> {
        let frame = self.current_frame()?;
        let plan = self
            .function(frame.function())?
            .frame()
            .slot_plans()
            .get(index_to_usize(slot))
            .copied()
            .ok_or(VmError::SlotOutOfBounds {
                function: frame.function(),
                slot,
            })?;
        require_concrete_transfer_plan(plan)
    }

    fn absolute_slot(&self, slot: FrameSlotIndex) -> Result<usize, VmError> {
        let frame = self.current_frame()?;
        let relative = index_to_usize(slot);
        if relative >= frame.slot_count() {
            return Err(VmError::SlotOutOfBounds {
                function: frame.function(),
                slot,
            });
        }
        Ok(frame.slot_base() + relative)
    }

    fn require_live_value(
        &self,
        index: usize,
        location: VmValueLocation,
    ) -> Result<ValueSlot, VmError> {
        if !self.live_values.get(index).copied().unwrap_or(false) {
            return Err(VmError::DeadValueRead { location });
        }
        Ok(self.values[index])
    }

    fn require_dead_destination(
        &self,
        index: usize,
        location: VmValueLocation,
    ) -> Result<(), VmError> {
        if self.live_values.get(index).copied().unwrap_or(false)
            || self.value_plans.get(index).copied().flatten().is_some()
        {
            return Err(VmError::LiveDestination { location });
        }
        Ok(())
    }

    fn require_live_plan(&self, index: usize) -> Result<ValueTransferPlanKind, VmError> {
        self.value_plans
            .get(index)
            .copied()
            .flatten()
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::MissingLiveValueTransferPlan,
            })
    }

    fn require_expected_plan(
        &self,
        index: usize,
        location: VmValueLocation,
        expected: ValueTransferPlanKind,
    ) -> Result<(), VmError> {
        self.require_matching_plans(location, expected, self.require_live_plan(index)?)
    }

    fn require_matching_plans(
        &self,
        location: VmValueLocation,
        expected: ValueTransferPlanKind,
        actual: ValueTransferPlanKind,
    ) -> Result<(), VmError> {
        if expected != actual {
            return Err(VmError::ValueTransferPlanMismatch {
                location,
                expected,
                actual,
            });
        }
        Ok(())
    }

    fn set_live_value(&mut self, index: usize, value: ValueSlot, plan: ValueTransferPlanKind) {
        self.values[index] = value;
        self.live_values[index] = true;
        self.value_plans[index] = Some(plan);
    }

    fn clear_value(&mut self, index: usize) {
        self.values[index] = ValueSlot::null();
        self.live_values[index] = false;
        self.value_plans[index] = None;
    }

    fn prepare_operand_push(&self) -> Result<(usize, usize), VmError> {
        let frame = self.current_frame()?;
        let depth = frame.operand_depth();
        if depth >= frame.operand_capacity() {
            return Err(VmError::OperandStackOverflow {
                function: frame.function(),
                capacity: frame.operand_capacity(),
            });
        }
        let index = frame.operand_base() + depth;
        self.require_dead_destination(index, VmValueLocation::Operand(depth))?;
        Ok((index, depth))
    }

    fn commit_operand_push(
        &mut self,
        destination: usize,
        depth: usize,
        value: ValueSlot,
        plan: ValueTransferPlanKind,
    ) -> Result<(), VmError> {
        self.set_live_value(destination, value, plan);
        self.current_frame_mut()?.set_operand_depth(depth + 1);
        Ok(())
    }

    fn peek_operand_with_plan(&self) -> Result<(ValueSlot, ValueTransferPlanKind), VmError> {
        let frame = self.current_frame()?;
        let depth = frame.operand_depth();
        if depth == 0 {
            return Err(VmError::OperandStackUnderflow {
                function: frame.function(),
                needed: 1,
                available: 0,
            });
        }
        let index = frame.operand_base() + depth - 1;
        Ok((
            self.require_live_value(index, VmValueLocation::Operand(depth - 1))?,
            self.require_live_plan(index)?,
        ))
    }

    fn pop_operand(&mut self) -> Result<(ValueSlot, ValueTransferPlanKind), VmError> {
        let frame = self.current_frame()?;
        let depth = frame.operand_depth();
        if depth == 0 {
            return Err(VmError::OperandStackUnderflow {
                function: frame.function(),
                needed: 1,
                available: 0,
            });
        }
        let index = frame.operand_base() + depth - 1;
        let value = self.require_live_value(index, VmValueLocation::Operand(depth - 1))?;
        let plan = self.require_live_plan(index)?;
        self.clear_value(index);
        self.current_frame_mut()?.set_operand_depth(depth - 1);
        Ok((value, plan))
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

fn validate_entry_contract(
    entry: &VerifiedCodeEntry,
    function: &LinkedFunction,
    argument_count: usize,
) -> Result<(), VmError> {
    if function.index() != entry.function() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::FunctionIndexMismatch,
        });
    }

    let signature = entry.signature();
    validate_signature_shape(signature, argument_count)?;
    validate_frame_shape(function.frame(), signature)
}

fn validate_signature_shape(
    signature: &LinkedCallableSignature,
    argument_count: usize,
) -> Result<(), VmError> {
    if signature.parameter_types().len() != signature.parameter_modes().len()
        || signature.parameter_types().len() != signature.parameter_plans().len()
    {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::EntryParameterCount,
        });
    }
    if signature.result_types().len() != signature.result_plans().len() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ResultTransferPlan,
        });
    }
    if signature.parameter_types().len() != argument_count {
        return Err(VmError::EntryArgumentCountMismatch {
            expected: signature.parameter_types().len(),
            actual: argument_count,
        });
    }
    if signature
        .parameter_modes()
        .iter()
        .any(|mode| *mode == ParamModeIr::InOut)
    {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ExternalInOutParameter,
        });
    }
    Ok(())
}

fn validate_frame_shape(
    frame: &LinkedFrameLayout,
    signature: &LinkedCallableSignature,
) -> Result<(), VmError> {
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
    for plan in frame
        .slot_plans()
        .iter()
        .chain(frame.result_plans())
        .chain(signature.parameter_plans())
        .chain(signature.result_plans())
        .copied()
    {
        require_concrete_transfer_plan(plan)?;
    }
    if frame.parameter_slots().len() != signature.parameter_types().len() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ParameterSlotCount,
        });
    }
    let mut seen_parameter_slots = vec![false; frame.slot_types().len()];
    for (ordinal, slot) in frame.parameter_slots().iter().copied().enumerate() {
        let slot = index_to_usize(slot);
        let Some(seen) = seen_parameter_slots.get_mut(slot) else {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::ParameterType,
            });
        };
        if *seen {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::DuplicateParameterSlot,
            });
        }
        *seen = true;
        if frame.slot_types().get(slot) != signature.parameter_types().get(ordinal) {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::ParameterType,
            });
        }
        if frame.slot_plans().get(slot) != signature.parameter_plans().get(ordinal) {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::ParameterTransferPlan,
            });
        }
    }
    if frame.result_types() != signature.result_types() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ResultType,
        });
    }
    if frame.result_plans() != signature.result_plans() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ResultTransferPlan,
        });
    }
    Ok(())
}

fn verified_function(
    program: &VerifiedLinkedBytecodeImage,
    index: FunctionIndex,
) -> Option<&LinkedFunction> {
    let function = program.functions().get(index_to_usize(index))?;
    (function.index() == index).then_some(function)
}

fn require_concrete_transfer_plan(
    plan: ValueTransferPlanKind,
) -> Result<ValueTransferPlanKind, VmError> {
    #[allow(unreachable_patterns)]
    match plan {
        ValueTransferPlanKind::SnapshotShare
        | ValueTransferPlanKind::MoveOnly
        | ValueTransferPlanKind::AffineResource
        | ValueTransferPlanKind::ExplicitCloneLease => Ok(plan),
        _ => Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::NonConcreteValueTransferPlan,
        }),
    }
}

fn index_to_usize<I>(index: I) -> usize
where
    I: ImageIndex,
{
    index.into_u32() as usize
}

trait ImageIndex {
    fn into_u32(self) -> u32;
}

impl ImageIndex for FunctionIndex {
    fn into_u32(self) -> u32 {
        FunctionIndex::get(self)
    }
}

impl ImageIndex for InstructionIndex {
    fn into_u32(self) -> u32 {
        InstructionIndex::get(self)
    }
}

impl ImageIndex for FrameSlotIndex {
    fn into_u32(self) -> u32 {
        FrameSlotIndex::get(self)
    }
}

#[cfg(test)]
mod tests {
    use skiff_runtime_model::vm_value::ValueSlot;

    use super::{VerifiedVmEntry, Vm, VmFiber};
    use crate::{VmError, VmLimits};

    #[test]
    fn production_start_signature_requires_the_concrete_pinned_entry() {
        let entry: fn(VerifiedVmEntry, Box<[ValueSlot]>, VmLimits) -> Result<VmFiber, VmError> =
            Vm::start;

        let _ = entry;
    }

    #[test]
    fn fiber_keeps_frame_and_values_out_of_the_managed_heap() {
        fn assert_root_source<T: skiff_runtime_model::vm_root::VmRootSource>() {}
        assert_root_source::<VmFiber>();
    }
}
