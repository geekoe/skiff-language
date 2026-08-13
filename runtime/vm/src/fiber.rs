mod entry_admission;
#[cfg(test)]
mod projection_tests;
#[cfg(test)]
mod tests;

use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    descriptor_for_opcode, LiteralIr, Opcode, PackageRefIr, ParamModeIr, TypeRefIr,
};
use skiff_runtime_bytecode_verifier::VerifiedResumeSite;
use skiff_runtime_deployment_image::DeploymentOwnerIdentity;
use skiff_runtime_linked_bytecode::{
    ActiveRegionIndex, CandidateTable, FrameSlotIndex, FrozenConstantNodeIndex, FunctionIndex,
    InstructionIndex, LinkedCallableSignature, LinkedCatchMatcher, LinkedExceptionRegion,
    LinkedFrozenConstantValue, LinkedFunction, LinkedInstruction, LinkedInstructionTarget,
    LinkedInterfaceTableKind, LinkedIntrinsicKind, LinkedNativeCallableSignature,
    LinkedValueTransferPlan, LinkedWritablePathSegment, ResumeSiteIndex, TypeIndex,
};
use skiff_runtime_linker::{DeploymentExecutionEntry, DeploymentExecutionImage};
use skiff_runtime_model::{
    bytecode_execution_observation::{
        BytecodeExecutionEvent, BytecodeExecutionObserver, VmFirstInstructionDispatched,
        VmFunctionFrameEntered, VmFunctionReturned, VmLocalCallDispatched, VmObservedFrameRole,
    },
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, FileAddr, LocalExecutionTypeIdentity,
        NominalTypeIdentity, PackageSchemaTypeIdentity, RequestException, TypeAddr, UnitAddr,
    },
    vm_heap::{VmHeap, VmHeapError, VmHeapPathSegment, VmRecordField},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
};

use crate::{
    admission::{is_discardable_root, validate_entry_arguments},
    control::{
        AdapterInvocation, ChildInvocation, ChildTarget, StreamItem, VmOwnedValues,
        VmResumeAuthority,
    },
    fiber::entry_admission::validate_entry_contract,
    frame::VmFrame,
    lifecycle::LifecycleExecutor,
    projection::VmProjectionHandoff,
    statement::{charge_frame_entry, charge_instruction_events},
    ResumeOutcome, VmBudget, VmControl, VmError, VmLimits, VmResumeToken, VmValueLocation,
    VmVerifiedInvariant,
};

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
        entry: DeploymentExecutionEntry,
        arguments: Box<[ValueSlot]>,
        limits: VmLimits,
        observer: BytecodeExecutionObserver,
    ) -> Result<VmFiber, VmError> {
        VmFiber::start(entry, arguments, limits, observer)
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use = "a VM fiber owns live roots until completion or explicit terminal discard"]
pub struct VmFiber {
    entry: DeploymentExecutionEntry,
    frames: Vec<VmFrame>,
    values: Vec<ValueSlot>,
    live_values: Vec<bool>,
    state: VmFiberState,
    limits: VmLimits,
    active_regions: Vec<ActiveRegionIndex>,
    region_depths: Vec<usize>,
    unwind: Option<UnwindState>,
    caught_exceptions: BTreeMap<usize, CaughtException>,
    error_correlation: Option<ErrorCorrelation>,
    pending_resume: Option<PendingResume>,
    resume_sequence: u64,
    projection_sequence: u64,
    observer: BytecodeExecutionObserver,
}

#[derive(Clone)]
struct UnwindState {
    envelope: Arc<RequestException>,
    cursor: UnwindCursor,
    phase: UnwindPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UnwindCursor {
    function: FunctionIndex,
    instruction: InstructionIndex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnwindPhase {
    /// Unwind was armed at a heap-free resume boundary; the frame-exit scan
    /// continues on the next heap-bearing run segment.
    Pending,
    /// Frame-exit scan toward a matching catch handler is in progress.
    Searching,
}

/// A caught envelope retained for the lifetime of its catch slot. The
/// envelope keeps the single payload authority while the catch slot holds a
/// shared snapshot of the payload for the handler body.
#[derive(Clone)]
struct CaughtException {
    envelope: Arc<RequestException>,
    plan: LinkedValueTransferPlan,
}

#[derive(Debug, Clone)]
struct PendingResume {
    image: Arc<DeploymentExecutionImage>,
    sequence: u64,
    function: FunctionIndex,
    instruction: InstructionIndex,
    resume_instruction: InstructionIndex,
    end_resume_pc: Option<InstructionIndex>,
    resume_site: ResumeSiteIndex,
    expected_stack_height: u32,
    expected_result_count: u32,
    authority: VmResumeAuthority,
}

impl VmFiber {
    fn start(
        entry: DeploymentExecutionEntry,
        arguments: Box<[ValueSlot]>,
        limits: VmLimits,
        observer: BytecodeExecutionObserver,
    ) -> Result<Self, VmError> {
        let function_index = entry.function();
        let program = entry.image();
        let function =
            verified_function(program, function_index).ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::EntryFunctionMissing,
            })?;
        validate_entry_contract(&entry, function, arguments.len())?;
        validate_entry_arguments(
            program,
            entry.signature().parameter_types(),
            entry.signature().parameter_plans(),
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

        let fiber = Self {
            entry,
            frames: vec![frame],
            values,
            live_values,
            state: VmFiberState::Runnable,
            limits,
            active_regions: Vec::new(),
            region_depths: vec![0],
            unwind: None,
            caught_exceptions: BTreeMap::new(),
            error_correlation: None,
            pending_resume: None,
            resume_sequence: 0,
            projection_sequence: 0,
            observer,
        };
        if let Ok(slot_count) = u32::try_from(slot_count) {
            if fiber.observer.claim_root_frame_entry() {
                fiber
                    .observer
                    .observe(BytecodeExecutionEvent::VmFunctionFrameEntered(
                        VmFunctionFrameEntered {
                            role: VmObservedFrameRole::Root,
                            function_index: function_index.get(),
                            frame_depth: 1,
                            slot_count,
                        },
                    ));
            }
        }
        Ok(fiber)
    }

    pub const fn state(&self) -> VmFiberState {
        self.state
    }

    pub fn owner(&self) -> &DeploymentOwnerIdentity {
        self.entry.image().owner()
    }

    pub fn active_frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn allocated_value_slot_count(&self) -> usize {
        self.values.len()
    }

    /// Supplies the request-local error correlation used when constructing
    /// throw envelopes. A throw before this is set fails closed with a
    /// [`VmError::ThrowEnvelopeUnavailable`] rather than fabricating one.
    pub fn set_error_correlation(&mut self, correlation: ErrorCorrelation) {
        self.error_correlation = Some(correlation);
    }

    /// Dormant VM-only mint seam for one exact inline projection point.
    ///
    /// All authority-bearing coordinates and dynamic shape facts come from
    /// this fiber. A caller cannot supply an image, function, PC, stack shape,
    /// or source site. Production dispatch intentionally has no consumer yet.
    #[allow(dead_code)]
    pub(crate) fn mint_projection_handoff(&mut self) -> Result<VmProjectionHandoff, VmError> {
        let result = self.mint_projection_handoff_inner();
        if result.is_err() {
            self.state = VmFiberState::Terminal;
        }
        result
    }

    fn mint_projection_handoff_inner(&mut self) -> Result<VmProjectionHandoff, VmError> {
        if self.state != VmFiberState::Runnable {
            return Err(VmError::FiberNotRunnable { state: self.state });
        }

        let frame_depth = self.frames.len();
        if self.region_depths.len() != frame_depth {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        let frame = self.current_frame()?.clone();
        let function_index = frame.function();
        let instruction_index = frame.instruction();
        let function = self.function(function_index)?;
        let instruction = function
            .instructions()
            .get(instruction_index.get() as usize)
            .ok_or(VmError::InstructionPointerOutOfBounds {
                function: function_index,
                instruction: instruction_index,
            })?;
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

        let program_point = function
            .stack_map()
            .entries()
            .get(instruction_index.get() as usize)
            .filter(|entry| entry.instruction() == instruction_index)
            .ok_or(VmError::InstructionPointerOutOfBounds {
                function: function_index,
                instruction: instruction_index,
            })?;
        let operand_height = frame.operand_height();
        let expected_operand_height = program_point.stack_before().len();
        if operand_height != expected_operand_height {
            return Err(VmError::OperandStackShapeMismatch {
                function: function_index,
                expected: expected_operand_height,
                actual: operand_height,
            });
        }

        let frame_region_base =
            *self
                .region_depths
                .last()
                .ok_or(VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        let active_frame_regions = self.active_regions.get(frame_region_base..).ok_or(
            VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            },
        )?;
        if active_frame_regions != program_point.active_regions() {
            let mismatch = active_frame_regions
                .iter()
                .copied()
                .zip(program_point.active_regions().iter().copied())
                .find(|(actual, expected)| actual != expected);
            let (actual, expected) = mismatch.unwrap_or_else(|| {
                let common = active_frame_regions
                    .len()
                    .min(program_point.active_regions().len());
                (
                    active_frame_regions
                        .get(common)
                        .copied()
                        .unwrap_or(ActiveRegionIndex::new(u32::MAX)),
                    program_point
                        .active_regions()
                        .get(common)
                        .copied()
                        .unwrap_or(ActiveRegionIndex::new(u32::MAX)),
                )
            });
            return Err(VmError::RegionLeaveMismatch {
                function: function_index,
                instruction: instruction_index,
                expected,
                actual,
            });
        }

        let projection_sequence = self.projection_sequence;
        let next_projection_sequence = projection_sequence
            .checked_add(1)
            .ok_or(VmError::ResumeTokenMismatch)?;
        let handoff = VmProjectionHandoff::new(
            Arc::clone(self.entry.image()),
            function_index,
            instruction_index,
            frame_depth,
            operand_height,
            self.active_regions.len(),
            projection_sequence,
        );
        self.projection_sequence = next_projection_sequence;
        Ok(handoff)
    }

    pub fn run_segment(&mut self, heap: &mut dyn VmHeap, budget: &mut dyn VmBudget) -> VmControl {
        if !matches!(
            self.state,
            VmFiberState::Runnable | VmFiberState::Unwinding
        ) {
            return VmControl::Complete(Err(VmError::FiberNotRunnable { state: self.state }));
        }

        match self.run_segment_inner(heap, budget) {
            Ok(SegmentResult::Continue) => VmControl::Continue,
            Ok(SegmentResult::Complete(values)) => VmControl::Complete(Ok(values)),
            Ok(SegmentResult::Throw(envelope)) => VmControl::Complete(Err(VmError::Thrown(envelope))),
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
        self.caught_exceptions.clear();
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
            ResumeOutcome::StreamEnd => self.resume_stream_end(pending),
            ResumeOutcome::Throw(envelope) => self.resume_throw(pending, envelope),
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

    fn resume_stream_end(&mut self, pending: PendingResume) -> Result<(), VmError> {
        let end_resume_pc = pending.end_resume_pc.ok_or_else(|| {
            self.state = VmFiberState::Terminal;
            VmError::StreamEndResumeUnavailable
        })?;
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
        self.current_frame_mut()?.resume_to(end_resume_pc);
        self.state = VmFiberState::Runnable;
        Ok(())
    }

    fn resume_throw(
        &mut self,
        pending: PendingResume,
        envelope: Arc<RequestException>,
    ) -> Result<(), VmError> {
        if envelope.vm_local_slot().is_none() || envelope.actual_catch_identity().is_none() {
            self.state = VmFiberState::Terminal;
            return Err(VmError::ResumeThrowEnvelopeUnavailable {
                function: pending.function,
                instruction: pending.instruction,
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
        // The resume boundary has no heap port, so the frame-exit scan is
        // armed here and continued by the next run segment, where every live
        // slot drop still routes through the Phase 2 lifecycle executor.
        self.unwind = Some(UnwindState {
            envelope,
            cursor: UnwindCursor {
                function: pending.function,
                instruction: pending.instruction,
            },
            phase: UnwindPhase::Pending,
        });
        self.state = VmFiberState::Unwinding;
        Ok(())
    }

    fn run_segment_inner(
        &mut self,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<SegmentResult, VmError> {
        budget.poll_interrupt().map_err(VmError::BudgetClosed)?;
        if self.unwind.is_some() {
            return self.resume_unwind_segment(heap);
        }
        for _ in 0..self.limits.max_segment_instructions().get() {
            self.charge_function_entry(budget)?;
            self.charge_statement_events(budget)?;
            match self.dispatch_accounted(heap, budget)? {
                DispatchOutcome::Continue => {}
                DispatchOutcome::Complete(values) => return Ok(SegmentResult::Complete(values)),
                DispatchOutcome::Throw(envelope) => return Ok(SegmentResult::Throw(envelope)),
                DispatchOutcome::Handoff(control) => return Ok(SegmentResult::Handoff(control)),
            }
        }
        Ok(SegmentResult::Continue)
    }

    /// Continues an unwind armed by `resume_throw` now that a heap port is
    /// available. The armed envelope must always be the authority here: the
    /// frame scan starts at the resume site and never re-derives an identity.
    fn resume_unwind_segment(
        &mut self,
        heap: &mut dyn VmHeap,
    ) -> Result<SegmentResult, VmError> {
        let mut lifecycle = LifecycleExecutor::new(heap);
        self.unwind_loop(&mut lifecycle)
            .map(|outcome| match outcome {
                DispatchOutcome::Continue => SegmentResult::Continue,
                DispatchOutcome::Throw(envelope) => SegmentResult::Throw(envelope),
                DispatchOutcome::Complete(values) => SegmentResult::Complete(values),
                DispatchOutcome::Handoff(control) => SegmentResult::Handoff(control),
            })
    }

    fn charge_function_entry(&mut self, budget: &mut dyn VmBudget) -> Result<(), VmError> {
        let schedule = self.entry.image().statement_schedule();
        let frame = self
            .frames
            .last_mut()
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        charge_frame_entry(schedule, frame, budget)
    }

    fn charge_statement_events(&mut self, budget: &mut dyn VmBudget) -> Result<(), VmError> {
        let schedule = self.entry.image().statement_schedule();
        let frame = self
            .frames
            .last_mut()
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        charge_instruction_events(schedule, frame, budget)
    }

    /// Sole attempted-dispatch accounting boundary. A successful budget call
    /// is immediately adjacent to exactly one private dispatch invocation.
    fn dispatch_accounted(
        &mut self,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<DispatchOutcome, VmError> {
        budget.before_dispatch().map_err(VmError::BudgetClosed)?;
        self.dispatch_one(heap)
    }

    fn dispatch_one(&mut self, heap: &mut dyn VmHeap) -> Result<DispatchOutcome, VmError> {
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

        let mut lifecycle = LifecycleExecutor::new(heap);
        let outcome = match instruction.opcode() {
            Opcode::Const => self.execute_const(function_index, instruction_index, &instruction),
            Opcode::CopySlot => self.execute_copy_slot(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::MoveSlot => self.execute_move_slot(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::StoreSlot => self.execute_store_slot(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::Drop => {
                self.execute_drop(&mut lifecycle, function_index, instruction_index, &instruction)
            }
            Opcode::Dup => self.execute_dup(&mut lifecycle, function_index, instruction_index),
            Opcode::LoadSlot => self.execute_load_slot(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::TakeSlot => self.execute_take_slot(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::Pop => self.execute_pop(&mut lifecycle, function_index, instruction_index),
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
            Opcode::CallLocal => self.execute_call_local(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::TailCallLocal => self.execute_tail_call_local(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::Return => {
                self.execute_return(&mut lifecycle, function_index, instruction_index)
            }
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
                self.execute_invoke_intrinsic(
                    lifecycle.heap(),
                    function_index,
                    instruction_index,
                    &instruction,
                )
            }
            Opcode::MakeCallback => {
                self.execute_make_callback(function_index, instruction_index, &instruction)
            }
            Opcode::InvokeCallback => {
                self.execute_invoke_callback(function_index, instruction_index, &instruction)
            }
            Opcode::Throw => self.execute_throw(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::Rethrow => self.execute_rethrow(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::EnterRegion => {
                self.execute_enter_region(function_index, instruction_index, &instruction)
            }
            Opcode::LeaveRegion => {
                self.execute_leave_region(function_index, instruction_index, &instruction)
            }
            Opcode::NewRecord => self.execute_new_record(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::GetDenseField => self.execute_get_dense_field(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::SetWritablePath => self.execute_set_writable_path(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::RepresentationWrap => self.execute_representation_wrap(
                lifecycle.heap(),
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::NewArrayBuilder => self.execute_new_array_builder(
                lifecycle.heap(),
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::ArrayBuilderPush => {
                self.execute_array_builder_push(
                    &mut lifecycle,
                    function_index,
                    instruction_index,
                )
            }
            Opcode::FreezeArray => {
                self.execute_freeze_array(lifecycle.heap(), function_index, instruction_index)
            }
            Opcode::ArrayGet => {
                self.execute_array_get(&mut lifecycle, function_index, instruction_index)
            }
            Opcode::ArrayPushOwned => self.execute_array_push_owned(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::ArrayLen => {
                self.execute_array_len(lifecycle.heap(), function_index, instruction_index)
            }
            Opcode::NewMapBuilder => {
                self.execute_new_map_builder(
                    lifecycle.heap(),
                    function_index,
                    instruction_index,
                    &instruction,
                )
            }
            Opcode::MapBuilderPut => {
                self.execute_map_builder_put(lifecycle.heap(), function_index, instruction_index)
            }
            Opcode::FreezeMap => {
                self.execute_freeze_map(lifecycle.heap(), function_index, instruction_index)
            }
            Opcode::MapGet => {
                self.execute_map_get(lifecycle.heap(), function_index, instruction_index)
            }
            Opcode::MapPutOwned => self.execute_map_put_owned(
                lifecycle.heap(),
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::MapLen => {
                self.execute_map_len(lifecycle.heap(), function_index, instruction_index)
            }
            Opcode::MapEntryAt => {
                self.execute_map_entry_at(lifecycle.heap(), function_index, instruction_index)
            }
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
            Opcode::Equal | Opcode::NotEqual => self.execute_equality(
                lifecycle.heap(),
                function_index,
                instruction_index,
                instruction.opcode(),
            ),
            _ => Err(VmError::UnsupportedOpcode {
                function: function_index,
                instruction: instruction_index,
                opcode: instruction.opcode(),
            }),
        };
        if outcome.is_ok() && self.observer.claim_vm_first_instruction_dispatch() {
            self.observer
                .observe(BytecodeExecutionEvent::VmFirstInstructionDispatched(
                    VmFirstInstructionDispatched {
                        image_owner: self.entry.image().owner().deployment().clone(),
                        root_entry_function_index: self.entry.function().get(),
                        current_function_index: function_index.get(),
                        instruction_index: instruction_index.get(),
                        opcode: instruction.opcode(),
                    },
                ));
        }
        outcome
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
        let value = self.entry.image().constant_heap().get(index).ok_or(
            VmError::ConstantIndexOutOfBounds {
                function,
                instruction,
                index: index.get(),
            },
        )?;
        self.push_operand(value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_copy_slot(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
        let plan = self.slot_plan(frame.function(), source)?;
        let shared = executor
            .share(&value, &plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::CopySlot))?;
        self.overwrite_slot(
            executor,
            &frame,
            slot_count,
            destination,
            shared,
            function,
            instruction,
            Opcode::CopySlot,
        )?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_move_slot(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
        let plan = self.slot_plan(frame.function(), source)?;
        let moved = executor
            .transfer(&value, &plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::MoveSlot))?;
        self.overwrite_slot(
            executor,
            &frame,
            slot_count,
            destination,
            moved,
            function,
            instruction,
            Opcode::MoveSlot,
        )?;
        self.clear_slot(&frame, slot_count, source)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_store_slot(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
        let plan = self.operand_plan(&frame, instruction, 0)?;
        let value = self.pop_operands(1, false)?.remove(0);
        let moved = executor
            .transfer(&value, &plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::StoreSlot))?;
        self.overwrite_slot(
            executor,
            &frame,
            slot_count,
            destination,
            moved,
            function,
            instruction,
            Opcode::StoreSlot,
        )?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_drop(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
        let plan = self.slot_plan(frame.function(), slot)?;
        executor
            .release(&value, &plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::Drop))?;
        self.clear_slot(&frame, slot_count, slot)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_load_slot(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
        let plan = self.slot_plan(frame.function(), slot)?;
        let shared = executor
            .share(&value, &plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::LoadSlot))?;
        self.push_operand(shared)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_take_slot(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
        let plan = self.slot_plan(frame.function(), slot)?;
        let moved = executor
            .transfer(&value, &plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::TakeSlot))?;
        self.clear_slot(&frame, slot_count, slot)?;
        self.push_operand(moved)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_pop(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let frame = self.current_frame()?.clone();
        let plan = self.operand_plan(&frame, instruction, 0)?;
        let position = frame.operand_base() + frame.operand_height() - 1;
        if !self.live_values[position] {
            return Err(VmError::DeadValueRead {
                location: VmValueLocation::Operand(frame.operand_height() - 1),
            });
        }
        let value = self.values[position];
        executor
            .release(&value, &plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::Pop))?;
        self.pop_operands(1, false)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_dup(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        self.ensure_operand_push(2)?;
        let frame = self.current_frame()?.clone();
        let plan = self.operand_plan(&frame, instruction, 0)?;
        let value = self.pop_operands(1, false)?.remove(0);
        let first = executor
            .share(&value, &plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::Dup))?;
        let second = executor
            .share(&value, &plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::Dup))?;
        self.push_operand(first)?;
        self.push_operand(second)?;
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
        executor: &mut LifecycleExecutor<'_>,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Type(_) = self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let frame = self.current_frame()?.clone();
        let payload_plan = self.operand_plan(&frame, instruction, 0)?;
        let payload = self.pop_operands(1, false)?.remove(0);
        let payload = executor
            .transfer(&payload, &payload_plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::Throw))?;
        // The envelope identity comes from the runtime value's own concrete
        // leaf tag, never from the throw instruction's static operand type.
        let Some(identity) = runtime_leaf_catch_identity(self.execution_image(), &payload) else {
            let _ = executor.release(&payload, &payload_plan);
            return Err(VmError::ThrowEnvelopeUnavailable {
                function,
                instruction,
                reason: "thrown value has no actual concrete leaf catch identity".to_string(),
            });
        };
        let envelope = match self.build_throw_envelope(payload, identity, function, instruction) {
            Ok(envelope) => envelope,
            Err(reason) => {
                let _ = executor.release(&payload, &payload_plan);
                return Err(VmError::ThrowEnvelopeUnavailable {
                    function,
                    instruction,
                    reason,
                });
            }
        };
        self.begin_unwind(executor, envelope, function, instruction)
    }

    fn execute_rethrow(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
        let absolute_index = Self::slot_index(&frame, slot_count, slot, frame.function())?;
        let payload = self.read_slot(&frame, slot_count, slot)?;
        let payload_plan = self.slot_plan(frame.function(), slot)?;
        let caught = self
            .caught_exceptions
            .remove(&absolute_index)
            .ok_or(VmError::RethrowEnvelopeUnavailable {
                function,
                instruction,
            })?;
        // The catch slot holds a shared snapshot of the envelope payload; the
        // envelope itself keeps the single payload authority. Rethrow releases
        // the handler's share and reuses the exact same envelope.
        executor
            .release(&payload, &payload_plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::Rethrow))?;
        self.clear_slot(&frame, slot_count, slot)?;
        self.begin_unwind(executor, caught.envelope, function, instruction)
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
        executor: &mut LifecycleExecutor<'_>,
        envelope: Arc<RequestException>,
        dispatch_function: FunctionIndex,
        dispatch_instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        self.unwind = Some(UnwindState {
            envelope,
            cursor: UnwindCursor {
                function: dispatch_function,
                instruction: dispatch_instruction,
            },
            phase: UnwindPhase::Searching,
        });
        self.unwind_loop(executor)
    }

    /// The single frame-exit scan of one unwind. Every exited frame routes
    /// its live slots through the Phase 2 lifecycle executor; catch matching
    /// compares the envelope's actual concrete leaf (the value's runtime
    /// tag) against the linked catch matchers.
    fn unwind_loop(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
    ) -> Result<DispatchOutcome, VmError> {
        let mut unwind = self
            .unwind
            .clone()
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        if let UnwindPhase::Pending = unwind.phase {
            let frame = self.current_frame()?;
            if frame.function() != unwind.cursor.function
                || frame.instruction() != unwind.cursor.instruction
            {
                return Err(VmError::FiberNotRunnable { state: self.state });
            }
            unwind.phase = UnwindPhase::Searching;
            self.unwind = Some(unwind);
        }
        loop {
            let frame = self.current_frame()?.clone();
            let function = frame.function();
            let instruction = frame.instruction();
            let envelope = self
                .unwind
                .as_ref()
                .map(|unwind| unwind.envelope.clone())
                .ok_or(VmError::FiberNotRunnable { state: self.state })?;
            let leaf = envelope_leaf_type_index(&envelope);
            let regions = self.function(function)?.exception_regions();
            if let Some(region) = find_exception_region(regions, instruction, leaf) {
                let region = region.clone();
                self.enter_handler(executor, &frame, &region, &envelope)?;
                return Ok(DispatchOutcome::Continue);
            }
            if self.frames.len() == 1 {
                self.release_frame_exit(executor, &frame, Opcode::Throw)?;
                self.frames.clear();
                self.values.clear();
                self.live_values.clear();
                self.active_regions.clear();
                self.region_depths.clear();
                self.unwind = None;
                self.state = VmFiberState::Terminal;
                return Ok(DispatchOutcome::Throw(envelope));
            }
            self.release_frame_exit(executor, &frame, Opcode::Throw)?;
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
        executor: &mut LifecycleExecutor<'_>,
        frame: &VmFrame,
        region: &LinkedExceptionRegion,
        envelope: &Arc<RequestException>,
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
            let position = index - operand_base;
            let plan = self.stack_map_operand_plan(frame.function(), frame.instruction(), position)?;
            let value = self.values[index];
            executor
                .release(&value, &plan)
                .map_err(|error| error.into_vm_error(frame.function(), frame.instruction(), Opcode::Throw))?;
            self.clear_value(index);
        }
        self.current_frame_mut()?.set_operand_height(handler_height);
        // The handler receives a shared snapshot of the envelope payload; the
        // envelope itself remains the single payload authority.
        let payload = envelope.vm_local_slot().ok_or(VmError::ThrowEnvelopeUnavailable {
            function: frame.function(),
            instruction: frame.instruction(),
            reason: "caught envelope has no opaque VM payload".to_string(),
        })?;
        let catch_plan = self.slot_plan(frame.function(), region.catch_slot())?;
        let shared = executor
            .share(&payload, &catch_plan)
            .map_err(|error| error.into_vm_error(frame.function(), frame.instruction(), Opcode::Throw))?;
        let absolute_index =
            Self::slot_index(frame, slot_count, region.catch_slot(), frame.function())?;
        self.overwrite_slot(
            executor,
            frame,
            slot_count,
            region.catch_slot(),
            shared,
            frame.function(),
            frame.instruction(),
            Opcode::Throw,
        )?;
        if let Some(previous) = self.caught_exceptions.insert(
            absolute_index,
            CaughtException {
                envelope: Arc::clone(envelope),
                plan: catch_plan,
            },
        ) {
            if let Some(slot) = previous.envelope.vm_local_slot() {
                executor
                    .release(&slot, &previous.plan)
                    .map_err(|error| error.into_vm_error(frame.function(), frame.instruction(), Opcode::Throw))?;
            }
        }
        self.current_frame_mut()?.jump_to(region.handler());
        self.unwind = None;
        self.state = VmFiberState::Runnable;
        Ok(())
    }

    /// Builds the opaque throw envelope for a VM-local throw. Fails closed
    /// (VmFailure) when the source site, frame stack or request-local
    /// correlation is unavailable; there is no static type fallback.
    fn build_throw_envelope(
        &self,
        payload: ValueSlot,
        identity: CatchIdentity,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<Arc<RequestException>, String> {
        let source = self.throw_source_site(function, instruction)?;
        let stack = self.exception_stack()?;
        let correlation = self
            .error_correlation
            .clone()
            .ok_or_else(|| "no request-local error correlation was supplied".to_string())?;
        RequestException::local_vm(payload, identity, source, stack, correlation).map(Arc::new)
    }

    /// The throw instruction's linked source site from the statement schedule.
    fn throw_source_site(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<skiff_artifact_model::InstructionSourceSite, String> {
        self.entry
            .image()
            .statement_schedule()
            .events_at(function, instruction)
            .and_then(|events| events.first())
            .map(|event| event.site().clone())
            .ok_or_else(|| "throw instruction has no linked source site".to_string())
    }

    /// The request-local frame stack, oldest frame first. Every frame reports
    /// the site of its current instruction; the root frame always contributes
    /// the throw site itself.
    fn exception_stack(&self) -> Result<Vec<ExceptionStackFrame>, String> {
        let mut stack = Vec::with_capacity(self.frames.len());
        for frame in &self.frames {
            let site = self
                .entry
                .image()
                .statement_schedule()
                .events_at(frame.function(), frame.instruction())
                .and_then(|events| events.first())
                .map(|event| event.site().clone());
            if let Some(site) = site {
                stack.push(ExceptionStackFrame::Local { site });
            }
        }
        if stack.is_empty() {
            return Err("no frame contributed a source site to the exception stack".to_string());
        }
        Ok(stack)
    }

    fn execute_call_local(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
        let argument_plans = (0..arg_count)
            .map(|ordinal| self.operand_plan(&caller, instruction, arg_count - 1 - ordinal))
            .collect::<Result<Vec<_>, VmError>>()?;
        let arguments = self.pop_operands(arg_count, false)?;
        self.values
            .resize(child_start + segment_len, ValueSlot::null());
        self.live_values.resize(child_start + segment_len, false);
        for (ordinal, destination_slot) in transfer_slots.into_iter().enumerate() {
            let value = executor
                .transfer(&arguments[ordinal], &argument_plans[ordinal])
                .map_err(|error| {
                    error.into_vm_error(function, instruction, Opcode::CallLocal)
                })?;
            self.values[child_start + destination_slot] = value;
            self.live_values[child_start + destination_slot] = true;
        }
        let caller_is_root = self.frames.len() == 1;
        self.frames.push(child);
        self.region_depths.push(self.active_regions.len());
        if caller_is_root {
            if self.observer.claim_root_local_call() {
                self.observer
                    .observe(BytecodeExecutionEvent::VmLocalCallDispatched(
                        VmLocalCallDispatched {
                            caller_function_index: caller.function().get(),
                            callee_function_index: target.get(),
                            caller_frame_depth: 1,
                            callee_frame_depth: 2,
                        },
                    ));
            }
            if let Ok(slot_count) = u32::try_from(target_slot_count) {
                if self.observer.claim_first_root_local_callee_frame_entry() {
                    self.observer
                        .observe(BytecodeExecutionEvent::VmFunctionFrameEntered(
                            VmFunctionFrameEntered {
                                role: VmObservedFrameRole::FirstRootLocalCallee,
                                function_index: target.get(),
                                frame_depth: 2,
                                slot_count,
                            },
                        ));
                }
            }
        }
        Ok(DispatchOutcome::Continue)
    }

    fn execute_tail_call_local(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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

        let argument_plans = (0..arg_count)
            .map(|ordinal| self.operand_plan(&caller, instruction, arg_count - 1 - ordinal))
            .collect::<Result<Vec<_>, VmError>>()?;
        let arguments = self.pop_operands(arg_count, true)?;
        self.release_frame_exit(executor, &caller, Opcode::TailCallLocal)?;
        let new_end = slot_base + segment_len;
        self.values.resize(new_end, ValueSlot::null());
        self.live_values.resize(new_end, false);
        for (ordinal, destination_slot) in transfer_slots.into_iter().enumerate() {
            let value = executor
                .transfer(&arguments[ordinal], &argument_plans[ordinal])
                .map_err(|error| {
                    error.into_vm_error(function, instruction, Opcode::TailCallLocal)
                })?;
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
        executor: &mut LifecycleExecutor<'_>,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let frame = self.current_frame()?.clone();
        let depth = self.frames.len();
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
        let result_plans = self
            .function(frame.function())?
            .frame()
            .result_plans()
            .to_vec();
        let mut transferred = Vec::with_capacity(result_count);
        for (ordinal, value) in results.iter().enumerate() {
            let value = executor
                .transfer(value, &result_plans[ordinal])
                .map_err(|error| error.into_vm_error(function, instruction, Opcode::Return))?;
            transferred.push(value);
        }
        if self.frames.len() == 1 {
            self.release_frame_exit(executor, &frame, Opcode::Return)?;
            let image = Arc::clone(self.entry.image());
            let values = transferred.into_boxed_slice();
            self.frames.clear();
            self.values.clear();
            self.live_values.clear();
            self.active_regions.clear();
            self.region_depths.clear();
            self.state = VmFiberState::Terminal;
            if self.observer.claim_root_return() {
                self.observer
                    .observe(BytecodeExecutionEvent::VmFunctionReturned(
                        VmFunctionReturned {
                            role: VmObservedFrameRole::Root,
                            function_index: frame.function().get(),
                            caller_function_index: None,
                            remaining_frame_depth: 0,
                        },
                    ));
            }
            return Ok(DispatchOutcome::Complete(VmOwnedValues::new(image, values)));
        }

        self.release_frame_exit(executor, &frame, Opcode::Return)?;
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
        for value in transferred {
            self.push_operand(value)?;
        }
        self.current_frame_mut()?.resume_to(resume);
        if depth == 2 {
            if let (Ok(remaining_frame_depth), Some(caller)) =
                (u32::try_from(self.frames.len()), self.frames.last())
            {
                if self.observer.claim_first_root_local_callee_return() {
                    self.observer
                        .observe(BytecodeExecutionEvent::VmFunctionReturned(
                            VmFunctionReturned {
                                role: VmObservedFrameRole::FirstRootLocalCallee,
                                function_index: child.function().get(),
                                caller_function_index: Some(caller.function().get()),
                                remaining_frame_depth,
                            },
                        ));
                }
            }
        }
        Ok(DispatchOutcome::Continue)
    }

    fn execute_new_record(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
            .execution_image()
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
        let frame = self.current_frame()?.clone();
        let values = self.pop_operands(field_count, false)?;
        let mut fields = Vec::with_capacity(field_count);
        for (ordinal, (field, value)) in shape.fields().iter().zip(values).enumerate() {
            let value = if matches!(value.kind(), Some(ValueKind::ConstRef)) {
                match self.string_slot_value(executor.heap(), &value) {
                    Ok(string) => executor.heap().alloc_string(string).map_err(VmError::Heap)?,
                    Err(_) => value,
                }
            } else {
                let plan = self.operand_plan(&frame, instruction, field_count - 1 - ordinal)?;
                executor
                    .transfer(&value, &plan)
                    .map_err(|error| error.into_vm_error(function, instruction, Opcode::NewRecord))?
            };
            fields.push(VmRecordField {
                name: field.name().to_string(),
                value,
            });
        }
        let value = executor
            .heap()
            .allocate_record(
                &fields,
                CompactTypeTag::new(shape.nominal_type().get()),
                ValueFlags::new(0),
            )
            .map_err(VmError::Heap)?;
        self.push_operand(value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_get_dense_field(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
            .execution_image()
            .shapes()
            .get(shape_index.get() as usize)
            .filter(|row| row.index() == shape_index)
            .cloned()
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Shapes,
                row: shape_index.get(),
            })?;
        if field_ordinal >= shape.fields().len() {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::GetDenseField,
            });
        }
        let frame = self.current_frame()?.clone();
        let record_plan = self.operand_plan(&frame, instruction, 0)?;
        let record = self.pop_operands(1, false)?.remove(0);
        let value = executor
            .heap()
            .get_dense_field(&record, field_ordinal)
            .map_err(VmError::Heap)?;
        let next = InstructionIndex::new(
            instruction
                .get()
                .checked_add(1)
                .ok_or(VmError::InstructionPointerOutOfBounds {
                    function,
                    instruction,
                })?,
        );
        let field_plan = self.stack_map_operand_plan(
            frame.function(),
            next,
            frame.operand_height().saturating_sub(1),
        )?;
        let shared = executor
            .share(&value, &field_plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::GetDenseField))?;
        executor
            .release(&record, &record_plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::GetDenseField))?;
        self.push_operand(shared)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_set_writable_path(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
            .execution_image()
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
        let frame = self.current_frame()?.clone();
        let slot_count = self.function(frame.function())?.frame().slot_types().len();
        let root = self.read_slot(&frame, slot_count, root_slot)?;
        let writable = self
            .function(frame.function())?
            .frame()
            .writable_local_slots()
            .binary_search(&root_slot)
            .is_ok();
        if !writable {
            return Err(VmError::LiveDestination {
                function,
                instruction,
                location: VmValueLocation::FrameSlot(root_slot),
            });
        }
        let rhs_plan = self.operand_plan(&frame, instruction, 0)?;
        let value = self.pop_operands(1, false)?.remove(0);
        let selectors = self.pop_operands(selector_count, false)?;
        let mut segments = Vec::with_capacity(path.segments().len());
        for segment in path.segments() {
            segments.push(match segment {
                LinkedWritablePathSegment::DenseField {
                    shape,
                    field_ordinal,
                } => {
                    let shape_row = self
                        .execution_image()
                        .shapes()
                        .get(shape.get() as usize)
                        .filter(|row| row.index() == *shape)
                        .ok_or(VmError::LinkedTableRowMissing {
                            table: CandidateTable::Shapes,
                            row: shape.get(),
                        })?;
                    let field = shape_row.fields().get(*field_ordinal as usize).ok_or(
                        VmError::FullValueLifecyclePlanUnavailable {
                            function,
                            instruction,
                            opcode: Opcode::SetWritablePath,
                        },
                    )?;
                    VmHeapPathSegment::DenseField {
                        field: field.name().to_string(),
                    }
                }
                LinkedWritablePathSegment::ArrayIndex { .. } => VmHeapPathSegment::ArrayIndex,
                LinkedWritablePathSegment::MapKey { .. } => VmHeapPathSegment::MapKey,
            });
        }
        let prepared = match executor
            .heap()
            .prepare_writable_path(&root, &segments, &selectors)
        {
            Ok(prepared) => prepared,
            Err(error) => {
                let _ = executor.release(&value, &rhs_plan);
                return Err(VmError::Heap(error));
            }
        };
        let value = match executor.transfer(&value, &rhs_plan) {
            Ok(value) => value,
            Err(error) => {
                let _ = executor.release(&value, &rhs_plan);
                return Err(error.into_vm_error(function, instruction, Opcode::SetWritablePath));
            }
        };
        let replacement = match executor.heap().commit_writable_path(prepared, value) {
            Ok(root) => root,
            Err(error) => {
                let _ = executor.release(&value, &rhs_plan);
                return Err(VmError::Heap(error));
            }
        };
        if replacement == root {
            // Exclusive in-place commit: the slot keeps its bits and owner.
        } else {
            let root_plan = self.slot_plan(frame.function(), root_slot)?;
            executor
                .release(&root, &root_plan)
                .map_err(|error| {
                    error.into_vm_error(function, instruction, Opcode::SetWritablePath)
                })?;
            self.install_slot_value(&frame, slot_count, root_slot, replacement)?;
        }
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_representation_wrap(
        &mut self,
        _heap: &mut dyn VmHeap,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Type(type_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        self.execution_image()
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
            opcode: Opcode::RepresentationWrap,
        })
    }

    fn execute_new_array_builder(
        &mut self,
        heap: &mut dyn VmHeap,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::Type(element_type) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        self.execution_image()
            .types()
            .get(element_type.get() as usize)
            .filter(|row| row.index() == element_type)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Types,
                row: element_type.get(),
            })?;
        let value = heap
            .allocate_array(
                &[],
                CompactTypeTag::new(element_type.get()),
                ValueFlags::new(0),
            )
            .map_err(VmError::Heap)?;
        self.push_operand(value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_array_builder_push(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let frame = self.current_frame()?.clone();
        let value_plan = self.operand_plan(&frame, instruction, 0)?;
        let values = self.pop_operands(2, false)?;
        let builder = values[0];
        let value = values[1];
        let value = match executor.transfer(&value, &value_plan) {
            Ok(value) => value,
            Err(error) => {
                let _ = executor.release(&value, &value_plan);
                return Err(error.into_vm_error(function, instruction, Opcode::ArrayBuilderPush));
            }
        };
        match executor.heap().array_push_owned(&builder, value) {
            Ok(()) => {}
            Err(error) => {
                let _ = executor.release(&value, &value_plan);
                return Err(VmError::Heap(error));
            }
        }
        self.push_operand(builder)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_freeze_array(
        &mut self,
        heap: &mut dyn VmHeap,
        _function: FunctionIndex,
        _instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let value = self.pop_operands(1, false)?.remove(0);
        heap.validate_live(&value).map_err(VmError::Heap)?;
        self.push_operand(value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_array_get(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let frame = self.current_frame()?.clone();
        let array_plan = self.operand_plan(&frame, instruction, 1)?;
        let values = self.pop_operands(2, false)?;
        let array = values[0];
        let index =
            skiff_runtime_model::vm_heap::collection_index(&values[1]).ok_or(VmError::ExpectedNumber {
            function,
            instruction,
            actual: values[1].kind(),
        })?;
        let value = executor
            .heap()
            .array_get(&array, index)
            .map_err(VmError::Heap)?;
        let next = InstructionIndex::new(
            instruction
                .get()
                .checked_add(1)
                .ok_or(VmError::InstructionPointerOutOfBounds {
                    function,
                    instruction,
                })?,
        );
        let element_plan = self.stack_map_operand_plan(
            frame.function(),
            next,
            frame.operand_height().saturating_sub(2),
        )?;
        let shared = executor
            .share(&value, &element_plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::ArrayGet))?;
        executor
            .release(&array, &array_plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::ArrayGet))?;
        self.push_operand(shared)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_array_push_owned(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
        let writable = self
            .function(frame.function())?
            .frame()
            .writable_local_slots()
            .binary_search(&slot)
            .is_ok();
        if !writable {
            return Err(VmError::LiveDestination {
                function,
                instruction,
                location: VmValueLocation::FrameSlot(slot),
            });
        }
        let array = self.read_slot(&frame, slot_count, slot)?;
        let value_plan = self.operand_plan(&frame, instruction, 0)?;
        let value = self.pop_operands(1, false)?.remove(0);
        let value = match executor.transfer(&value, &value_plan) {
            Ok(value) => value,
            Err(error) => {
                let _ = executor.release(&value, &value_plan);
                return Err(error.into_vm_error(function, instruction, Opcode::ArrayPushOwned));
            }
        };
        match executor.heap().array_push_owned(&array, value) {
            Ok(()) => {}
            Err(error) => {
                let _ = executor.release(&value, &value_plan);
                return Err(VmError::Heap(error));
            }
        }
        // In-place exclusive push keeps the slot's bits and owner.
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_array_len(
        &mut self,
        heap: &mut dyn VmHeap,
        _function: FunctionIndex,
        _instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let array = self.pop_operands(1, false)?.remove(0);
        let len = heap.array_len(&array).map_err(VmError::Heap)?;
        self.push_operand(ValueSlot::number(len as f64))?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_new_map_builder(
        &mut self,
        heap: &mut dyn VmHeap,
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
        let types = self.execution_image().types();
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
        let value = heap
            .allocate_map(
                &[],
                CompactTypeTag::new(value_type.get()),
                ValueFlags::new(0),
            )
            .map_err(VmError::Heap)?;
        self.push_operand(value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_map_builder_put(
        &mut self,
        heap: &mut dyn VmHeap,
        _function: FunctionIndex,
        _instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let values = self.pop_operands(3, false)?;
        let builder = values[0];
        let key = values[1];
        let value = values[2];
        heap.map_put_owned(&builder, key, value)
            .map_err(VmError::Heap)?;
        self.push_operand(builder)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_freeze_map(
        &mut self,
        heap: &mut dyn VmHeap,
        _function: FunctionIndex,
        _instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let value = self.pop_operands(1, false)?.remove(0);
        heap.validate_live(&value).map_err(VmError::Heap)?;
        self.push_operand(value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_map_get(
        &mut self,
        heap: &mut dyn VmHeap,
        _function: FunctionIndex,
        _instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let values = self.pop_operands(2, false)?;
        let map = values[0];
        let key = values[1];
        let value = heap.map_get(&map, &key).map_err(VmError::Heap)?;
        self.push_operand(value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_map_put_owned(
        &mut self,
        heap: &mut dyn VmHeap,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::FrameSlot(slot) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let values = self.pop_operands(2, false)?;
        let key = values[0];
        let value = values[1];
        let frame = self.current_frame()?.clone();
        let slot_count = self.function(frame.function())?.frame().slot_types().len();
        let writable = self
            .function(frame.function())?
            .frame()
            .writable_local_slots()
            .binary_search(&slot)
            .is_ok();
        if !writable {
            return Err(VmError::LiveDestination {
                function,
                instruction,
                location: VmValueLocation::FrameSlot(slot),
            });
        }
        let map = self.read_slot(&frame, slot_count, slot)?;
        heap.map_put_owned(&map, key, value)
            .map_err(VmError::Heap)?;
        // In-place exclusive put keeps the slot's bits and owner.
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_map_len(
        &mut self,
        heap: &mut dyn VmHeap,
        _function: FunctionIndex,
        _instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let map = self.pop_operands(1, false)?.remove(0);
        let len = heap.map_len(&map).map_err(VmError::Heap)?;
        self.push_operand(ValueSlot::number(len as f64))?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_map_entry_at(
        &mut self,
        heap: &mut dyn VmHeap,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let values = self.pop_operands(2, false)?;
        let map = values[0];
        let ordinal =
            skiff_runtime_model::vm_heap::collection_index(&values[1]).ok_or(VmError::ExpectedNumber {
            function,
            instruction,
            actual: values[1].kind(),
        })?;
        let entry = heap.map_entry_at(&map, ordinal).map_err(VmError::Heap)?;
        self.push_operand(entry.key)?;
        self.push_operand(entry.value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_invoke_intrinsic(
        &mut self,
        heap: &mut dyn VmHeap,
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
            .execution_image()
            .intrinsics()
            .get(intrinsic_index.get() as usize)
            .filter(|row| row.index() == intrinsic_index)
            .cloned()
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
        let values = self.pop_operands(arg_count, false)?;
        if let LinkedIntrinsicKind::Receiver(op) = intrinsic.kind() {
            if op.canonical_key == "receiver:Array.push@1" {
                if values.len() != 2 {
                    return Err(VmError::FullValueLifecyclePlanUnavailable {
                        function,
                        instruction,
                        opcode: Opcode::InvokeIntrinsic,
                    });
                }
                heap.array_push_owned(&values[0], values[1])
                    .map_err(VmError::Heap)?;
                self.advance_current_instruction()?;
                return Ok(DispatchOutcome::Continue);
            }
        }
        let result = match intrinsic.kind() {
            LinkedIntrinsicKind::Static(target) => match target.canonical_key().as_str() {
                "core.array.empty" => {
                    let result_type = intrinsic
                        .signature()
                        .result_types()
                        .first()
                        .copied()
                        .ok_or(VmError::FullValueLifecyclePlanUnavailable {
                            function,
                            instruction,
                            opcode: Opcode::InvokeIntrinsic,
                        })?;
                    heap.allocate_array(
                        &[],
                        CompactTypeTag::new(result_type.get()),
                        ValueFlags::new(0),
                    )
                    .map_err(VmError::Heap)?
                }
                "core.map.empty" => {
                    let result_type = intrinsic
                        .signature()
                        .result_types()
                        .first()
                        .copied()
                        .ok_or(VmError::FullValueLifecyclePlanUnavailable {
                            function,
                            instruction,
                            opcode: Opcode::InvokeIntrinsic,
                        })?;
                    heap.allocate_map(
                        &[],
                        CompactTypeTag::new(result_type.get()),
                        ValueFlags::new(0),
                    )
                    .map_err(VmError::Heap)?
                }
                _ => {
                    return Err(VmError::FullValueLifecyclePlanUnavailable {
                        function,
                        instruction,
                        opcode: Opcode::InvokeIntrinsic,
                    });
                }
            },
            LinkedIntrinsicKind::Receiver(op) => match op.canonical_key {
                "receiver:string.concat@1" => {
                    if values.len() != 2 {
                        return Err(VmError::FullValueLifecyclePlanUnavailable {
                            function,
                            instruction,
                            opcode: Opcode::InvokeIntrinsic,
                        });
                    }
                    let left = self.string_slot_value(heap, &values[0])?;
                    let right = self.string_slot_value(heap, &values[1])?;
                    heap.alloc_string(format!("{left}{right}"))
                        .map_err(VmError::Heap)?
                }
                "receiver:bytes.toUtf8String@1" => {
                    if values.len() != 1 {
                        return Err(VmError::FullValueLifecyclePlanUnavailable {
                            function,
                            instruction,
                            opcode: Opcode::InvokeIntrinsic,
                        });
                    }
                    let bytes = heap.bytes_value(&values[0]).map_err(VmError::Heap)?;
                    heap.alloc_string(String::from_utf8_lossy(&bytes).into_owned())
                        .map_err(VmError::Heap)?
                }
                _ => {
                    return Err(VmError::FullValueLifecyclePlanUnavailable {
                        function,
                        instruction,
                        opcode: Opcode::InvokeIntrinsic,
                    });
                }
            },
        };
        self.push_operand(result)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
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
        self.execution_image()
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
        self.execution_image()
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
            .execution_image()
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
            None,
            expected_stack_height,
            result_count as u32,
        )?;
        let invocation = ChildInvocation::new(
            target,
            VmOwnedValues::new(Arc::clone(self.entry.image()), arguments.into_boxed_slice()),
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
            .execution_image()
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
            None,
            expected_stack_height,
            result_count as u32,
        )?;
        let invocation = ChildInvocation::new(
            target,
            VmOwnedValues::new(Arc::clone(self.entry.image()), arguments.into_boxed_slice()),
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
            .execution_image()
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
            None,
            expected_stack_height,
            result_count as u32,
        )?;
        let invocation = ChildInvocation::new(
            target,
            VmOwnedValues::new(Arc::clone(self.entry.image()), arguments.into_boxed_slice()),
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
            .execution_image()
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
            None,
            expected_stack_height,
            result_count as u32,
        )?;
        let invocation = AdapterInvocation::new(
            adapter_index,
            VmOwnedValues::new(Arc::clone(self.entry.image()), arguments.into_boxed_slice()),
            token,
        )
        .map_err(|_| VmError::ResumeTokenMismatch)?;
        self.state = VmFiberState::WaitingHost;
        Ok(DispatchOutcome::Handoff(VmControl::EnterAdapter(
            invocation,
        )))
    }

    fn string_slot_value(
        &self,
        heap: &mut dyn VmHeap,
        value: &ValueSlot,
    ) -> Result<String, VmError> {
        match value.kind() {
            Some(ValueKind::ConstRef) => {
                let handle = value
                    .as_const_ref()
                    .ok_or(VmError::Heap(VmHeapError::InvalidValueMetadata))?;
                let index = FrozenConstantNodeIndex::new(
                    u32::try_from(handle.get())
                        .map_err(|_| VmError::Heap(VmHeapError::InvalidValueMetadata))?,
                );
                let node = self
                    .execution_image()
                    .frozen_constant_nodes()
                    .get(index.get() as usize)
                    .filter(|node| node.index() == index)
                    .ok_or(VmError::Heap(VmHeapError::InvalidValueMetadata))?;
                match node.value() {
                    LinkedFrozenConstantValue::Literal(LiteralIr::String { value }) => {
                        Ok(value.clone())
                    }
                    _ => Err(VmError::Heap(VmHeapError::InvalidValueMetadata)),
                }
            }
            _ => heap.string_value(value).map_err(VmError::Heap),
        }
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
        self.execution_image()
            .synthetic_callbacks()
            .get(callback_index.get() as usize)
            .filter(|row| row.index() == callback_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::SyntheticCallbacks,
                row: callback_index.get(),
            })?;
        let layout = self
            .execution_image()
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
        // StreamNext borrows the affine endpoint from its frame slot. The
        // endpoint stays live across item/end resumes so loop-backed polls can
        // read the same slot again; the child handoff carries no owned payload.
        self.read_slot(&frame, slot_count, endpoint_slot)?;
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
        let end_resume_pc = resume
            .end_resume()
            .ok_or(VmError::StreamEndResumeUnavailable)?;
        let arguments = VmOwnedValues::new(Arc::clone(self.entry.image()), Box::new([]));
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
            VmResumeAuthority::Child(ChildTarget::StreamNext),
            resume_site,
            resume.resume(),
            Some(end_resume_pc),
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
            None,
            expected_stack_height,
            0,
        )?;
        let stream_item = StreamItem::new(
            VmOwnedValues::new(Arc::clone(self.entry.image()), item.into_boxed_slice()),
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
        heap: &mut dyn VmHeap,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
    ) -> Result<DispatchOutcome, VmError> {
        let operands = self.pop_operands(2, false)?;
        let equal = self
            .comparable_equality(heap, &operands[0], &operands[1])
            .ok_or(VmError::ExpectedComparablePair {
                function,
                instruction,
                left: operands[0].kind(),
                right: operands[1].kind(),
            })?;
        let result = if opcode == Opcode::Equal {
            equal
        } else {
            !equal
        };
        self.push_operand(ValueSlot::bool(result))?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn comparable_equality(
        &self,
        heap: &mut dyn VmHeap,
        left: &ValueSlot,
        right: &ValueSlot,
    ) -> Option<bool> {
        comparable_equality_with_string_resolver(left, right, |value| {
            self.string_slot_value(heap, value).ok()
        })
    }

    fn linked_resume_site(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
        index: ResumeSiteIndex,
    ) -> Result<&VerifiedResumeSite, VmError> {
        let row = self
            .execution_image()
            .resume_sites()
            .get(index)
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
        if row
            .end_resume()
            .is_some_and(|end_resume| end_resume.get() as usize >= function_len)
        {
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
        end_resume_pc: Option<InstructionIndex>,
        expected_stack_height: u32,
        expected_result_count: u32,
    ) -> Result<VmResumeToken, VmError> {
        let image = Arc::clone(self.entry.image());
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
            end_resume_pc,
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
            end_resume_pc,
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

    fn execution_image(&self) -> &DeploymentExecutionImage {
        self.entry.image().as_ref()
    }

    fn function(&self, index: FunctionIndex) -> Result<&LinkedFunction, VmError> {
        verified_function(self.execution_image(), index).ok_or(VmError::VerifiedEntryInvariant {
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

    fn slot_plan(
        &self,
        function: FunctionIndex,
        slot: FrameSlotIndex,
    ) -> Result<LinkedValueTransferPlan, VmError> {
        self.function(function)?
            .frame()
            .slot_plans()
            .get(slot.get() as usize)
            .cloned()
            .ok_or(VmError::SlotOutOfBounds { function, slot })
    }

    fn operand_plan(
        &self,
        frame: &VmFrame,
        instruction: InstructionIndex,
        from_top: usize,
    ) -> Result<LinkedValueTransferPlan, VmError> {
        let position = frame
            .operand_height()
            .checked_sub(from_top + 1)
            .ok_or(VmError::OperandStackUnderflow {
                function: frame.function(),
                needed: from_top + 1,
                available: frame.operand_height(),
            })?;
        self.stack_map_operand_plan(frame.function(), instruction, position)
    }

    fn stack_map_operand_plan(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        position: usize,
    ) -> Result<LinkedValueTransferPlan, VmError> {
        let entry = self
            .function(function)?
            .stack_map()
            .entries()
            .get(instruction.get() as usize)
            .filter(|entry| entry.instruction() == instruction)
            .ok_or(VmError::InstructionPointerOutOfBounds {
                function,
                instruction,
            })?;
        entry
            .stack_before()
            .get(position)
            .map(|value| value.plan().clone())
            .ok_or(VmError::OperandStackShapeMismatch {
                function,
                expected: position + 1,
                actual: entry.stack_before().len(),
            })
    }

    fn overwrite_slot(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
        frame: &VmFrame,
        slot_count: usize,
        slot: FrameSlotIndex,
        value: ValueSlot,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
    ) -> Result<(), VmError> {
        let index = Self::slot_index(frame, slot_count, slot, frame.function())?;
        if self.live_values[index] {
            let writable = self
                .function(frame.function())?
                .frame()
                .writable_local_slots()
                .binary_search(&slot)
                .is_ok();
            if !writable {
                return Err(VmError::LiveDestination {
                    function,
                    instruction,
                    location: VmValueLocation::FrameSlot(slot),
                });
            }
            let old = self.values[index];
            let plan = self.slot_plan(frame.function(), slot)?;
            executor
                .release(&old, &plan)
                .map_err(|error| error.into_vm_error(function, instruction, opcode))?;
            self.clear_value(index);
        }
        self.values[index] = value;
        self.live_values[index] = true;
        Ok(())
    }

    fn install_slot_value(
        &mut self,
        frame: &VmFrame,
        slot_count: usize,
        slot: FrameSlotIndex,
        value: ValueSlot,
    ) -> Result<(), VmError> {
        let index = Self::slot_index(frame, slot_count, slot, frame.function())?;
        self.values[index] = value;
        self.live_values[index] = true;
        Ok(())
    }

    /// Releases every live slot and operand of a frame through the lifecycle
    /// executor, exactly once each, immediately before the frame exits. Slot
    /// plans come from the frame layout; operand plans come from the program
    /// point's linked stack map.
    fn release_frame_exit(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
        frame: &VmFrame,
        opcode: Opcode,
    ) -> Result<(), VmError> {
        let slot_plans = self.function(frame.function())?.frame().slot_plans().to_vec();
        let slot_count = slot_plans.len();
        for ordinal in 0..slot_count {
            let index = frame.slot_base() + ordinal;
            if self.live_values.get(index).copied() == Some(true) {
                let value = self.values[index];
                let plan = slot_plans[ordinal].clone();
                executor
                    .release(&value, &plan)
                    .map_err(|error| error.into_vm_error(frame.function(), frame.instruction(), opcode))?;
                self.clear_value(index);
            }
        }
        for position in 0..frame.operand_height() {
            let index = frame.operand_base() + position;
            if self.live_values.get(index).copied() == Some(true) {
                let plan = self.stack_map_operand_plan(
                    frame.function(),
                    frame.instruction(),
                    position,
                )?;
                let value = self.values[index];
                executor
                    .release(&value, &plan)
                    .map_err(|error| error.into_vm_error(frame.function(), frame.instruction(), opcode))?;
                self.clear_value(index);
            }
        }
        // Caught envelopes whose catch slot lives in this frame die with the
        // frame. Their retained payload authority is released exactly once;
        // a rethrow has already moved the envelope out of this map.
        let range = frame.slot_base()..frame.slot_base().saturating_add(slot_count);
        let caught: Vec<CaughtException> = self
            .caught_exceptions
            .range(range.clone())
            .map(|(_, entry)| entry.clone())
            .collect();
        self.caught_exceptions
            .retain(|index, _| !range.contains(index));
        for entry in caught {
            if let Some(slot) = entry.envelope.vm_local_slot() {
                executor.release(&slot, &entry.plan).map_err(|error| {
                    error.into_vm_error(frame.function(), frame.instruction(), opcode)
                })?;
            }
        }
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
                function: frame.function(),
                instruction: frame.instruction(),
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
            if let Some(slot) = unwind.envelope.vm_local_slot() {
                visitor.visit_root(&slot)?;
            }
        }
        Ok(())
    }
}

enum SegmentResult {
    Continue,
    Complete(VmOwnedValues),
    Throw(Arc<RequestException>),
    Handoff(VmControl),
}

enum DispatchOutcome {
    Continue,
    Complete(VmOwnedValues),
    Throw(Arc<RequestException>),
    Handoff(VmControl),
}

fn pending_matches(pending: &PendingResume, token: &VmResumeToken) -> bool {
    Arc::ptr_eq(&pending.image, token.image())
        && pending.sequence == token.sequence()
        && pending.function == token.function()
        && pending.instruction == token.instruction()
        && pending.resume_instruction == token.resume_instruction()
        && pending.end_resume_pc == token.end_resume_pc()
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

/// The actual concrete leaf identity of one runtime value, read from the
/// value's own runtime type tag plus the immutable linked type facts. This is
/// deliberately not the throw instruction's static operand type: two values
/// flowing through the same union-typed site carry different tags and yield
/// different identities.
fn runtime_leaf_catch_identity(
    image: &DeploymentExecutionImage,
    value: &ValueSlot,
) -> Option<CatchIdentity> {
    match value.kind()? {
        ValueKind::RequestHeapRef
        | ValueKind::ActorStateRef
        | ValueKind::ConstRef
        | ValueKind::ResourceRef
        | ValueKind::CallbackClosureRef => {
            linked_type_catch_identity(image, TypeIndex::new(value.compact_type_tag().get()))
        }
        _ => None,
    }
}

/// The opaque envelope's concrete leaf tag for catch matching.
fn envelope_leaf_type_index(envelope: &RequestException) -> Option<TypeIndex> {
    let slot = envelope.vm_local_slot()?;
    match slot.kind()? {
        ValueKind::RequestHeapRef
        | ValueKind::ActorStateRef
        | ValueKind::ConstRef
        | ValueKind::ResourceRef
        | ValueKind::CallbackClosureRef => {
            Some(TypeIndex::new(slot.compact_type_tag().get()))
        }
        _ => None,
    }
}

/// Derives the domain catch identity of one linked leaf type. Package schema
/// types keep their canonical schema identity; local execution types keep a
/// stable linked-execution identity keyed by the owning package slot and the
/// exact linked type index. Structural and unresolvable shapes have no
/// concrete leaf identity and fail closed.
fn linked_type_catch_identity(
    image: &DeploymentExecutionImage,
    leaf: TypeIndex,
) -> Option<CatchIdentity> {
    let entry = image.types().get(leaf.get() as usize)?;
    if entry.index() != leaf {
        return None;
    }
    match entry.type_ref() {
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            let identity = PackageSchemaTypeIdentity::new(
                package_id.clone(),
                stable_schema_key.clone(),
                package_schema_type_id.clone(),
            )
            .ok()?;
            Some(CatchIdentity::Nominal(NominalTypeIdentity::PackageSchema(
                identity,
            )))
        }
        TypeRefIr::PackageSymbol { symbol } => {
            let PackageRefIr::PackageId { package_id } = &symbol.package else {
                return None;
            };
            let package_slot = image
                .packages()
                .iter()
                .find(|package| package.package_build_id() == entry.origin().package_build_id())
                .map(|package| package.index().get() as usize)?;
            Some(CatchIdentity::Nominal(
                NominalTypeIdentity::LocalExecution(LocalExecutionTypeIdentity {
                    addr: TypeAddr {
                        unit: UnitAddr::Package(package_slot),
                        file: FileAddr::FileIrIdentity(package_id.clone()),
                        type_index: leaf.get() as usize,
                    },
                    type_arguments: Vec::new(),
                }),
            ))
        }
        _ => None,
    }
}

fn find_exception_region(
    regions: &[LinkedExceptionRegion],
    pc: InstructionIndex,
    leaf: Option<TypeIndex>,
) -> Option<&LinkedExceptionRegion> {
    regions.iter().rev().find(|region| {
        region.start().get() <= pc.get()
            && pc.get() < region.end().get()
            && region
                .catch_matchers()
                .iter()
                .any(|matcher| catch_matches(matcher, leaf))
    })
}

/// Matches one linked catch matcher against the thrown value's actual
/// concrete leaf. `leaf` is the runtime tag of the value itself (its
/// `compact_type_tag`), never the throw instruction's static payload type, so
/// an anonymous union `A | B` value whose actual leaf is `A` matches
/// `catch<A>` and not `catch<B>`.
fn catch_matches(matcher: &LinkedCatchMatcher, leaf: Option<TypeIndex>) -> bool {
    match matcher {
        LinkedCatchMatcher::CatchAll => true,
        LinkedCatchMatcher::Type(expected) => leaf == Some(*expected),
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

#[cfg(test)]
fn comparable_equality(left: &ValueSlot, right: &ValueSlot) -> Option<bool> {
    comparable_equality_with_string_resolver(left, right, |_| None)
}

fn comparable_equality_with_string_resolver(
    left: &ValueSlot,
    right: &ValueSlot,
    mut resolve_string: impl FnMut(&ValueSlot) -> Option<String>,
) -> Option<bool> {
    let kind = left.kind()?;
    let right_kind = right.kind()?;
    if matches!(kind, ValueKind::ConstRef | ValueKind::RequestHeapRef)
        || matches!(right_kind, ValueKind::ConstRef | ValueKind::RequestHeapRef)
    {
        let left = resolve_string(left)?;
        let right = resolve_string(right)?;
        return Some(left == right);
    }
    if right_kind != kind {
        match (kind, right_kind) {
            (ValueKind::Number, ValueKind::Integer) | (ValueKind::Integer, ValueKind::Number) => {
                let left = if kind == ValueKind::Number {
                    left.as_number()?
                } else {
                    left.as_integer()? as f64
                };
                let right = if right_kind == ValueKind::Number {
                    right.as_number()?
                } else {
                    right.as_integer()? as f64
                };
                return Some(left == right);
            }
            _ => return None,
        }
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
    program: &DeploymentExecutionImage,
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
