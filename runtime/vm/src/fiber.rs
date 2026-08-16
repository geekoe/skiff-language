mod entry_admission;
mod ownership_transactions;
#[cfg(test)]
mod projection_tests;
#[cfg(test)]
mod tests;

use std::{
    any::Any,
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use skiff_artifact_model::{
    descriptor_for_opcode, LiteralIr, NativeResourceDropPlan, NativeValueDropPlan,
    NativeValueEmbedding, NativeValueLifecycleConcrete, Opcode, PackageRefIr, ParamModeIr,
    PrivilegedAffineCompositeIdentity, PrivilegedAffineFieldAccess, TypeRefIr,
    NATIVE_VALUE_LIFECYCLE_REGISTRY,
};
use skiff_runtime_deployment_image::DeploymentOwnerIdentity;
use skiff_runtime_linked_bytecode::{
    ActiveRegionIndex, CandidateTable, FrameSlotIndex, FrozenConstantNodeIndex, FunctionIndex,
    InstructionIndex, LinkedCallableSignature, LinkedCatchMatcher, LinkedExceptionRegion,
    LinkedFrozenConstantValue, LinkedFunction, LinkedInstruction, LinkedInstructionTarget,
    LinkedInterfaceTable, LinkedInterfaceTableKind, LinkedIntrinsicKind,
    LinkedNativeCallableSignature, LinkedResourceDropPlan, LinkedTaskTiming, LinkedValueDropPlan,
    LinkedValueTransferPlan, LinkedWritablePathSegment, ResumeSiteIndex, TypeIndex,
};
use skiff_runtime_linker::ExecutionResumeSite;
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
    vm_heap::{
        VmHeap, VmHeapError, VmHeapPathSegment, VmLocalInterfaceTable, VmRecordField,
        VmRemoteInterfaceTable,
    },
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
};

use crate::{
    admission::{is_discardable_root, validate_entry_arguments},
    control::{
        AdapterInvocation, ChildInvocation, ChildTarget, InterfaceCallPlan,
        RemoteInterfaceCallPlan, StreamItem, TaskDispatchIndex, TaskDispatchTiming,
        TaskIntrinsicResumePlan, VmCompletion, VmLifecycleSite, VmOwnedException, VmOwnedValues,
        VmResumeAuthority, VmResumeBinding, VmTerminalCause, VmTerminalEscrow, VmTerminalOwner,
    },
    fiber::entry_admission::validate_entry_contract,
    frame::VmFrame,
    lifecycle::LifecycleExecutor,
    projection::VmProjectionHandoff,
    statement::{charge_frame_entry, charge_instruction_events},
    ResumeOutcome, VmBudget, VmControl, VmError, VmLimits, VmResumeFailure, VmResumeToken,
    VmValueLocation, VmVerifiedInvariant,
};

use ownership_transactions::*;

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
        VmFiber::start_with_retained(entry, arguments, limits, observer, &[])
    }

    /// Starts a fiber whose first parameter root is retained by the caller.
    ///
    /// Actor method frames use this for the self record: the host Actor arena
    /// owns that root, so frame exit must not release it.
    pub fn start_with_retained_parameter(
        entry: DeploymentExecutionEntry,
        arguments: Box<[ValueSlot]>,
        limits: VmLimits,
        observer: BytecodeExecutionObserver,
    ) -> Result<VmFiber, VmError> {
        VmFiber::start_with_retained(
            entry,
            arguments,
            limits,
            observer,
            &[FrameSlotIndex::new(0)],
        )
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
    terminal_escrow: Vec<EscrowedOwner>,
    terminal_handoff: Option<VmTerminalEscrow>,
    caught_exceptions: BTreeMap<usize, CaughtException>,
    caught_by_payload: HashMap<u64, usize>,
    error_correlation: Option<ErrorCorrelation>,
    pending_resume: Option<PendingResume>,
    resume_sequence: u64,
    projection_sequence: u64,
    observer: BytecodeExecutionObserver,
    retained_slots: Vec<FrameSlotIndex>,
}

struct UnwindState {
    envelope: Arc<RequestException>,
    /// Exact plan for a VM-local payload whose ownership originated in this
    /// fiber. A child-provided envelope has no locally reconstructable plan
    /// until it enters a linked catch slot, so that lane retains the envelope
    /// safely instead of guessing a cleanup operation.
    payload_plan: Option<LinkedValueTransferPlan>,
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
    payload_handle: u64,
    site: VmLifecycleSite,
}

#[derive(Debug, Clone)]
struct PendingResume {
    binding: Arc<VmResumeBinding>,
}

/// A push destination whose frame, bounds, liveness, and next height have all
/// been checked. Nothing that allocates or otherwise creates a new owner may
/// run before this reservation exists; committing it is deliberately
/// infallible so a newly created owner can never fall between the VM stack and
/// cleanup.
#[derive(Clone, Copy)]
struct OperandPushReservation {
    frame_ordinal: usize,
    value_index: usize,
    next_height: usize,
}

/// A top-of-stack operand window that can be atomically replaced by one
/// newly allocated owner. The consumed operands remain live until allocation
/// succeeds; failures therefore leave every current or already-materialized
/// owner reachable from the fiber.
#[derive(Clone, Copy)]
struct OperandWindowReplacementReservation {
    frame_ordinal: usize,
    start: usize,
    end: usize,
    next_height: usize,
}

impl VmFiber {
    fn start_with_retained(
        entry: DeploymentExecutionEntry,
        arguments: Box<[ValueSlot]>,
        limits: VmLimits,
        observer: BytecodeExecutionObserver,
        retained_slots: &[FrameSlotIndex],
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
            terminal_escrow: Vec::new(),
            terminal_handoff: None,
            caught_exceptions: BTreeMap::new(),
            caught_by_payload: HashMap::new(),
            error_correlation: None,
            pending_resume: None,
            resume_sequence: 0,
            projection_sequence: 0,
            observer,
            retained_slots: retained_slots.to_vec(),
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

    /// Reads a live slot from the current frame while the fiber is runnable.
    ///
    /// Actor create execution captures the updated self root after
    /// `SetWritablePath` before the frame is retired.
    pub fn frame_slot_value(&self, slot: FrameSlotIndex) -> Result<ValueSlot, VmError> {
        let frame = self.current_frame()?;
        let slot_count = self.function(frame.function())?.frame().slot_types().len();
        let index = Self::slot_index(&frame, slot_count, slot, frame.function())?;
        if !self.live_values[index] {
            return Err(VmError::DeadValueRead {
                location: VmValueLocation::FrameSlot(slot),
            });
        }
        Ok(self.values[index])
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
        if !matches!(self.state, VmFiberState::Runnable | VmFiberState::Unwinding) {
            let error = VmError::FiberNotRunnable { state: self.state };
            let image = Arc::clone(self.entry.image());
            let residual = self.take_completion_residual();
            return VmControl::Complete(VmCompletion::failed(image, error, residual));
        }

        match self.run_segment_inner(heap, budget) {
            Ok(SegmentResult::Continue) => VmControl::Continue,
            Ok(SegmentResult::Complete(values)) => {
                let residual = self.take_completion_residual();
                VmControl::Complete(VmCompletion::returned(values, residual))
            }
            Ok(SegmentResult::Throw(envelope)) => {
                let residual = self.take_completion_residual();
                VmControl::Complete(VmCompletion::thrown(envelope, residual))
            }
            Ok(SegmentResult::Handoff(control)) => control,
            Err(error) => {
                self.state = VmFiberState::Terminal;
                let image = Arc::clone(self.entry.image());
                let residual = self.take_completion_residual();
                VmControl::Complete(VmCompletion::failed(image, error, residual))
            }
        }
    }

    pub fn resume(
        &mut self,
        token: VmResumeToken,
        outcome: ResumeOutcome,
    ) -> Result<(), VmResumeFailure> {
        self.resume_inner(token, outcome)
    }

    pub fn discard_terminal_roots(&mut self, heap: &mut dyn VmHeap) -> Result<(), VmError> {
        if self.state != VmFiberState::Terminal {
            return Err(VmError::DiscardRequiresTerminal { state: self.state });
        }
        self.ensure_terminal_handoff();
        let terminal = self
            .terminal_handoff
            .as_mut()
            .expect("terminal handoff is installed before cleanup");
        terminal.release_all(heap)?;
        self.terminal_handoff = None;
        Ok(())
    }

    /// Moves every root still owned by this fiber into a non-cloneable
    /// scheduler/request carrier. This operation is heap-free and infallible:
    /// damaged linked state becomes an explicit retained owner rather than a
    /// guessed release. Calling it abandons further execution and leaves this
    /// fiber terminal and rootless.
    pub fn take_terminal_escrow(&mut self) -> VmTerminalEscrow {
        self.ensure_terminal_handoff();
        self.terminal_handoff
            .take()
            .expect("terminal handoff is installed before transfer")
    }

    /// Consumes a rejected resume input while this fiber supplies the exact
    /// receiving image pin. The rejection remains the primary error and the
    /// outcome becomes a non-cloneable terminal owner carrier.
    pub fn escrow_rejected_resume(
        &self,
        rejected: VmResumeFailure,
    ) -> (VmTerminalCause, VmTerminalEscrow) {
        rejected.into_terminal_escrow(Arc::clone(self.entry.image()))
    }

    fn ensure_terminal_handoff(&mut self) {
        if self.terminal_handoff.is_some() {
            return;
        }
        self.state = VmFiberState::Terminal;
        self.terminal_handoff = Some(self.collect_terminal_escrow());
    }

    fn take_completion_residual(&mut self) -> VmTerminalEscrow {
        self.state = VmFiberState::Terminal;
        let mut residual = self
            .terminal_handoff
            .take()
            .unwrap_or_else(|| VmTerminalEscrow::empty(Arc::clone(self.entry.image())));
        residual.merge(self.collect_terminal_escrow());
        residual
    }

    fn collect_terminal_escrow(&mut self) -> VmTerminalEscrow {
        let image = Arc::clone(self.entry.image());
        let mut owners = Vec::new();
        let mut claimed_storage = vec![false; self.values.len()];

        for frame in self.frames.clone() {
            let opcode = self
                .function(frame.function())
                .ok()
                .and_then(|function| {
                    function
                        .instructions()
                        .get(frame.instruction().get() as usize)
                })
                .map(LinkedInstruction::opcode)
                .unwrap_or(Opcode::Return);
            let site = VmLifecycleSite {
                function: frame.function(),
                instruction: frame.instruction(),
                opcode,
            };
            let slot_count = frame.operand_base().saturating_sub(frame.slot_base());
            for ordinal in 0..slot_count {
                let Some(index) = frame.slot_base().checked_add(ordinal) else {
                    continue;
                };
                self.capture_terminal_storage_owner(
                    index,
                    self.function(frame.function())
                        .ok()
                        .and_then(|function| function.frame().slot_plans().get(ordinal))
                        .cloned(),
                    site,
                    &mut claimed_storage,
                    &mut owners,
                );
            }
            for position in 0..frame.operand_height() {
                let Some(index) = frame.operand_base().checked_add(position) else {
                    continue;
                };
                self.capture_terminal_storage_owner(
                    index,
                    self.stack_map_operand_plan(frame.function(), frame.instruction(), position)
                        .ok(),
                    site,
                    &mut claimed_storage,
                    &mut owners,
                );
            }
        }

        let fallback_site = self
            .frames
            .last()
            .map(|frame| VmLifecycleSite {
                function: frame.function(),
                instruction: frame.instruction(),
                opcode: Opcode::Return,
            })
            .unwrap_or(VmLifecycleSite {
                function: FunctionIndex::new(0),
                instruction: InstructionIndex::new(0),
                opcode: Opcode::Return,
            });
        for index in 0..self.values.len() {
            if self.live_values.get(index).copied().unwrap_or(false) && !claimed_storage[index] {
                self.capture_terminal_storage_owner(
                    index,
                    None,
                    fallback_site,
                    &mut claimed_storage,
                    &mut owners,
                );
            }
        }

        for owner in std::mem::take(&mut self.terminal_escrow) {
            if !is_discardable_root(&owner.value) {
                let diagnostic_index = owners.len();
                owners.push(VmTerminalOwner::exact(
                    owner.value,
                    owner.plan,
                    owner.site,
                    diagnostic_index,
                ));
            }
        }
        for (_, caught) in std::mem::take(&mut self.caught_exceptions) {
            if let Some(value) = caught.envelope.vm_local_slot() {
                if !is_discardable_root(&value) {
                    let diagnostic_index = owners.len();
                    owners.push(VmTerminalOwner::exact(
                        value,
                        caught.plan,
                        caught.site,
                        diagnostic_index,
                    ));
                }
            }
        }
        if let Some(unwind) = self.unwind.take() {
            if let Some(value) = unwind.envelope.vm_local_slot() {
                if !is_discardable_root(&value) {
                    let diagnostic_index = owners.len();
                    let site = VmLifecycleSite {
                        function: unwind.cursor.function,
                        instruction: unwind.cursor.instruction,
                        opcode: Opcode::Throw,
                    };
                    owners.push(match unwind.payload_plan {
                        Some(plan) => VmTerminalOwner::exact(value, plan, site, diagnostic_index),
                        None => VmTerminalOwner::damaged_retained(value, site, diagnostic_index),
                    });
                }
            }
        }

        self.frames.clear();
        self.values.clear();
        self.live_values.clear();
        self.active_regions.clear();
        self.region_depths.clear();
        self.caught_by_payload.clear();
        self.pending_resume = None;
        VmTerminalEscrow::new(image, owners)
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_terminal_storage_owner(
        &self,
        index: usize,
        plan: Option<LinkedValueTransferPlan>,
        site: VmLifecycleSite,
        claimed_storage: &mut [bool],
        owners: &mut Vec<VmTerminalOwner>,
    ) {
        let Some(claimed) = claimed_storage.get_mut(index) else {
            return;
        };
        if *claimed || !self.live_values.get(index).copied().unwrap_or(false) {
            return;
        }
        *claimed = true;
        let Some(value) = self.values.get(index).copied() else {
            return;
        };
        if is_discardable_root(&value) {
            return;
        }
        owners.push(match plan {
            Some(plan) => VmTerminalOwner::exact(value, plan, site, index),
            None => VmTerminalOwner::damaged_retained(value, site, index),
        });
    }

    fn resume_inner(
        &mut self,
        token: VmResumeToken,
        outcome: ResumeOutcome,
    ) -> Result<(), VmResumeFailure> {
        let Some(pending) = self.pending_resume.take() else {
            self.state = VmFiberState::Terminal;
            return Err(VmResumeFailure::rejected(
                VmError::ResumeNotExpected,
                token,
                outcome,
            ));
        };
        if !matches!(
            self.state,
            VmFiberState::BlockedOnChild | VmFiberState::WaitingHost
        ) || !pending_matches(&pending, &token)
        {
            self.state = VmFiberState::Terminal;
            return Err(VmResumeFailure::rejected(
                VmError::ResumeTokenMismatch,
                token,
                outcome,
            ));
        }

        match outcome {
            ResumeOutcome::Values(values) => self
                .resume_values(&pending, &values, true)
                .map_err(|error| self.reject_resume(error, token, ResumeOutcome::Values(values))),
            ResumeOutcome::Empty => {
                let image = Arc::clone(pending.binding.image());
                let values = VmOwnedValues::empty(image);
                self.resume_values(&pending, &values, false)
                    .map_err(|error| self.reject_resume(error, token, ResumeOutcome::Empty))
            }
            ResumeOutcome::StreamEnd => self
                .resume_stream_end(&pending)
                .map_err(|error| self.reject_resume(error, token, ResumeOutcome::StreamEnd)),
            ResumeOutcome::Throw(exception) => match self.resume_throw(&pending, exception) {
                Ok(()) => Ok(()),
                Err((error, exception)) => {
                    Err(self.reject_resume(error, token, ResumeOutcome::Throw(exception)))
                }
            },
            ResumeOutcome::Failure(error) => {
                if matches!(&error, VmError::Thrown(_)) {
                    Err(self.reject_resume(
                        VmError::ResumeTokenMismatch,
                        token,
                        ResumeOutcome::Failure(error),
                    ))
                } else {
                    self.state = VmFiberState::Terminal;
                    Err(VmResumeFailure::terminal(error))
                }
            }
            ResumeOutcome::InternalTerminal(reason) => {
                self.state = VmFiberState::Terminal;
                Err(VmResumeFailure::terminal(VmError::InternalTerminal(reason)))
            }
        }
    }

    fn reject_resume(
        &mut self,
        error: VmError,
        resume: VmResumeToken,
        outcome: ResumeOutcome,
    ) -> VmResumeFailure {
        self.state = VmFiberState::Terminal;
        VmResumeFailure::rejected(error, resume, outcome)
    }

    fn resume_values(
        &mut self,
        pending: &PendingResume,
        values: &VmOwnedValues,
        binding_required: bool,
    ) -> Result<(), VmError> {
        if !Arc::ptr_eq(values.image(), pending.binding.image())
            || (binding_required && !values.is_bound_to(&pending.binding))
        {
            self.state = VmFiberState::Terminal;
            return Err(VmError::ResumeTokenMismatch);
        }
        let expected = usize::try_from(pending.binding.expected_result_count()).map_err(|_| {
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
        if frame.function() != pending.binding.function()
            || frame.instruction() != pending.binding.instruction()
            || frame.operand_height()
                != usize::try_from(pending.binding.expected_stack_height())
                    .map_err(|_| VmError::ResumeTokenMismatch)?
        {
            self.state = VmFiberState::Terminal;
            return Err(VmError::ResumeTokenMismatch);
        }
        let frame_ordinal = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        let reservation = self.reserve_operand_push_window(frame_ordinal, expected)?;
        self.commit_operand_push_window(reservation, values.values());
        self.frames[frame_ordinal].resume_to(pending.binding.resume_instruction());
        self.state = VmFiberState::Runnable;
        Ok(())
    }

    fn resume_stream_end(&mut self, pending: &PendingResume) -> Result<(), VmError> {
        let end_resume_pc = pending.binding.end_resume_pc().ok_or_else(|| {
            self.state = VmFiberState::Terminal;
            VmError::StreamEndResumeUnavailable
        })?;
        let frame = self.current_frame()?.clone();
        if frame.function() != pending.binding.function()
            || frame.instruction() != pending.binding.instruction()
            || frame.operand_height()
                != usize::try_from(pending.binding.expected_stack_height())
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
        pending: &PendingResume,
        exception: VmOwnedException,
    ) -> Result<(), (VmError, VmOwnedException)> {
        if !Arc::ptr_eq(exception.origin_image(), pending.binding.image())
            || !exception.is_bound_to(&pending.binding)
        {
            self.state = VmFiberState::Terminal;
            return Err((VmError::ResumeTokenMismatch, exception));
        }
        if exception.unresolved_count() != 0 {
            self.state = VmFiberState::Terminal;
            return Err((
                VmError::ResumeThrowEnvelopeUnavailable {
                    function: pending.binding.function(),
                    instruction: pending.binding.instruction(),
                },
                exception,
            ));
        }
        let Some(payload) = exception.exception().vm_local_slot() else {
            self.state = VmFiberState::Terminal;
            return Err((
                VmError::ResumeThrowEnvelopeUnavailable {
                    function: pending.binding.function(),
                    instruction: pending.binding.instruction(),
                },
                exception,
            ));
        };
        let Some(actual_identity) = exception.exception().actual_catch_identity() else {
            self.state = VmFiberState::Terminal;
            return Err((
                VmError::ResumeThrowEnvelopeUnavailable {
                    function: pending.binding.function(),
                    instruction: pending.binding.instruction(),
                },
                exception,
            ));
        };
        // Phase 5 has only same-image StreamNext/controlled resume
        // reachability. The origin Arc check above is the provenance guard;
        // this identity check additionally rejects a forged envelope whose
        // payload metadata disagrees with that exact image.
        if runtime_leaf_catch_identity(pending.binding.image(), &payload).as_ref()
            != Some(actual_identity)
        {
            self.state = VmFiberState::Terminal;
            return Err((
                VmError::ResumeThrowEnvelopeUnavailable {
                    function: pending.binding.function(),
                    instruction: pending.binding.instruction(),
                },
                exception,
            ));
        }
        let frame = match self.current_frame() {
            Ok(frame) => frame.clone(),
            Err(error) => return Err((error, exception)),
        };
        let expected_height = match usize::try_from(pending.binding.expected_stack_height()) {
            Ok(height) => height,
            Err(_) => return Err((VmError::ResumeTokenMismatch, exception)),
        };
        if frame.function() != pending.binding.function()
            || frame.instruction() != pending.binding.instruction()
            || frame.operand_height() != expected_height
        {
            self.state = VmFiberState::Terminal;
            return Err((VmError::ResumeTokenMismatch, exception));
        }
        // Validation completed without consuming the caller's outcome. Only
        // now does the fiber take custody of the unchanged opaque envelope.
        let (envelope, payload_plan) = exception.into_unwind_parts();
        self.unwind = Some(UnwindState {
            envelope,
            payload_plan,
            cursor: UnwindCursor {
                function: pending.binding.function(),
                instruction: pending.binding.instruction(),
            },
            phase: UnwindPhase::Pending,
        });
        // The resume boundary has no heap port, so the already-armed frame
        // exit scan continues in the next heap-bearing run segment.
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
    fn resume_unwind_segment(&mut self, heap: &mut dyn VmHeap) -> Result<SegmentResult, VmError> {
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
            Opcode::Drop => self.execute_drop(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
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
            Opcode::CallInterface => self.execute_call_interface(
                lifecycle.heap(),
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::InvokeHost => {
                self.execute_invoke_host(function_index, instruction_index, &instruction)
            }
            Opcode::InvokeIntrinsic => self.execute_invoke_intrinsic(
                &mut lifecycle,
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::MakeCallback => {
                self.execute_make_callback(function_index, instruction_index, &instruction)
            }
            Opcode::InvokeCallback => self.execute_invoke_callback(
                lifecycle.heap(),
                function_index,
                instruction_index,
                &instruction,
            ),
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
            Opcode::TakeDenseField => self.execute_take_dense_field(
                lifecycle.heap(),
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
                self.execute_array_builder_push(&mut lifecycle, function_index, instruction_index)
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
            Opcode::NewMapBuilder => self.execute_new_map_builder(
                lifecycle.heap(),
                function_index,
                instruction_index,
                &instruction,
            ),
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
            Opcode::InterfaceBoxLocal => self.execute_interface_box_local(
                lifecycle.heap(),
                function_index,
                instruction_index,
                &instruction,
            ),
            Opcode::InterfaceBoxRemote => self.execute_interface_box_remote(
                lifecycle.heap(),
                function_index,
                instruction_index,
                &instruction,
            ),
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
                &mut lifecycle,
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
        let reservation =
            self.reserve_copy_slot(&frame, slot_count, destination, function, instruction)?;
        let shared = executor
            .share(&value, &plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::CopySlot))?;
        self.terminal_escrow.push(EscrowedOwner::new(
            shared,
            plan.clone(),
            function,
            instruction,
            Opcode::CopySlot,
        ));
        if let Err(error) = self.release_reserved_destination(
            executor,
            &reservation.destination,
            function,
            instruction,
            Opcode::CopySlot,
        ) {
            return Err(error);
        }
        self.commit_copy_slot(reservation, shared);
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
        let reservation = self.reserve_move_slot(
            &frame,
            slot_count,
            source,
            destination,
            function,
            instruction,
        )?;
        let value = self.values[reservation.source_index];
        let plan = self.slot_plan(frame.function(), source)?;
        let moved = executor
            .transfer(&value, &plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::MoveSlot))?;
        // `transfer_owner` may return different slot bits. Re-anchor them in
        // the still-live source cell before the destination release can fail.
        self.values[reservation.source_index] = moved;
        self.release_reserved_destination(
            executor,
            &reservation.destination,
            function,
            instruction,
            Opcode::MoveSlot,
        )?;
        self.commit_move_slot(reservation);
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
        let (operand_type, plan) = self.operand_type_and_plan(&frame, instruction, 0)?;
        let destination_type = self.slot_type(frame.function(), destination)?;
        let destination_plan = self.slot_plan(frame.function(), destination)?;
        if !LifecycleExecutor::supports_transfer(&plan) {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::StoreSlot,
            });
        }
        let reservation =
            self.reserve_store_slot(&frame, slot_count, destination, function, instruction)?;
        let mut value = self.values[reservation.operand_index];
        if matches!(value.kind(), Some(ValueKind::ConstRef)) {
            let owned = self.materialize_store_string_constant(
                executor.heap(),
                &value,
                operand_type,
                &plan,
                destination_type,
                &destination_plan,
                function,
                instruction,
            )?;
            // The image constant is a borrow. Once materialized, the new
            // request owner is installed in its operand cell immediately so a
            // transfer failure remains rooted and a retry does not allocate a
            // second owner.
            self.values[reservation.operand_index] = owned;
            value = owned;
        }
        let moved = executor
            .transfer(&value, &plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::StoreSlot))?;
        self.values[reservation.operand_index] = moved;
        self.release_reserved_destination(
            executor,
            &reservation.destination,
            function,
            instruction,
            Opcode::StoreSlot,
        )?;
        self.commit_store_slot(reservation);
        Ok(DispatchOutcome::Continue)
    }

    #[allow(clippy::too_many_arguments)]
    fn materialize_store_string_constant(
        &self,
        heap: &mut dyn VmHeap,
        value: &ValueSlot,
        operand_type: TypeIndex,
        operand_plan: &LinkedValueTransferPlan,
        destination_type: TypeIndex,
        destination_plan: &LinkedValueTransferPlan,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<ValueSlot, VmError> {
        let constant_type = value
            .compact_type_tag()
            .map(CompactTypeTag::type_index)
            .map(TypeIndex::new)
            .ok_or(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::StoreSlot,
            })?;
        let constant_type_ref = self
            .execution_image()
            .types()
            .get(constant_type.get() as usize)
            .filter(|row| row.index() == constant_type)
            .map(|row| row.type_ref())
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Types,
                row: constant_type.get(),
            })?;
        let operand_type_ref = self
            .execution_image()
            .types()
            .get(operand_type.get() as usize)
            .filter(|row| row.index() == operand_type)
            .map(|row| row.type_ref())
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Types,
                row: operand_type.get(),
            })?;
        let destination_type_ref = self
            .execution_image()
            .types()
            .get(destination_type.get() as usize)
            .filter(|row| row.index() == destination_type)
            .map(|row| row.type_ref())
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Types,
                row: destination_type.get(),
            })?;
        let string = self.string_slot_value(heap, value)?;
        if value.flags() != ValueFlags::new(0) {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::StoreSlot,
            });
        }
        let Some(materialized) = allocate_store_string_constant(
            heap,
            string,
            compact_type_tag(function, instruction, operand_type)?,
            constant_type_ref,
            operand_type_ref,
            operand_plan,
            destination_type_ref,
            destination_plan,
        )
        .map_err(VmError::Heap)?
        else {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::StoreSlot,
            });
        };
        Ok(materialized)
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
        let tag = nominal_type_index(&value);
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
            .find(|case| Some(case.tag_type()) == tag)
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
        let LinkedInstructionTarget::Type(_) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let frame = self.current_frame()?.clone();
        let payload_plan = self.operand_plan(&frame, instruction, 0)?;
        let payload_position =
            frame
                .operand_height()
                .checked_sub(1)
                .ok_or(VmError::OperandStackUnderflow {
                    function,
                    needed: 1,
                    available: frame.operand_height(),
                })?;
        let payload_index = frame.operand_base().checked_add(payload_position).ok_or(
            VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            },
        )?;
        if !self
            .live_values
            .get(payload_index)
            .copied()
            .unwrap_or(false)
        {
            return Err(VmError::DeadValueRead {
                location: VmValueLocation::Operand(payload_position),
            });
        }
        let payload = self.values[payload_index];
        let payload = executor
            .transfer(&payload, &payload_plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::Throw))?;
        self.values[payload_index] = payload;
        // The envelope identity comes from the runtime value's own concrete
        // leaf tag, never from the throw instruction's static operand type.
        let Some(identity) = runtime_leaf_catch_identity(self.execution_image(), &payload) else {
            return Err(VmError::ThrowEnvelopeUnavailable {
                function,
                instruction,
                reason: "thrown value has no actual concrete leaf catch identity".to_string(),
            });
        };
        let envelope = match self.build_throw_envelope(payload, identity, function, instruction) {
            Ok(envelope) => envelope,
            Err(reason) => {
                return Err(VmError::ThrowEnvelopeUnavailable {
                    function,
                    instruction,
                    reason,
                });
            }
        };
        self.unwind = Some(UnwindState {
            envelope,
            payload_plan: Some(payload_plan),
            cursor: UnwindCursor {
                function,
                instruction,
            },
            phase: UnwindPhase::Searching,
        });
        self.clear_value(payload_index);
        self.frames
            .last_mut()
            .expect("validated throw retains its current frame")
            .set_operand_height(payload_position);
        self.unwind_loop(executor)
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
        let exception_record = self.read_slot(&frame, slot_count, slot)?;
        let exception_plan = self.slot_plan(frame.function(), slot)?;
        // The rethrow source slot holds the canonical `Exception<E>` record
        // that wraps the caught payload. Unwrap the payload to find the exact
        // envelope authority by its runtime handle; the original envelope is
        // then reused unchanged.
        let payload = executor
            .heap()
            .record_field(&exception_record, "error")
            .map_err(VmError::Heap)?;
        let payload_handle = payload.as_handle().map(|handle| handle.get()).ok_or(
            VmError::RethrowEnvelopeUnavailable {
                function,
                instruction,
            },
        )?;
        let absolute_index = self.caught_by_payload.get(&payload_handle).copied().ok_or(
            VmError::RethrowEnvelopeUnavailable {
                function,
                instruction,
            },
        )?;
        let caught = self.caught_exceptions.get(&absolute_index).cloned().ok_or(
            VmError::RethrowEnvelopeUnavailable {
                function,
                instruction,
            },
        )?;
        let source_index = Self::slot_index(&frame, slot_count, slot, frame.function())?;
        if caught.payload_handle != payload_handle
            || caught
                .envelope
                .vm_local_slot()
                .and_then(|value| value.as_handle())
                .map(|handle| handle.get())
                != Some(payload_handle)
        {
            return Err(VmError::RethrowEnvelopeUnavailable {
                function,
                instruction,
            });
        }
        // The rethrow source slot releases its `Exception<E>` record share;
        // the envelope keeps its payload authority and reuses the exact same
        // envelope, so the actual catch identity stays unchanged.
        executor
            .release(&exception_record, &exception_plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::Rethrow))?;
        self.clear_value(source_index);
        self.caught_by_payload.remove(&payload_handle);
        self.caught_exceptions.remove(&absolute_index);
        self.begin_unwind(
            executor,
            caught.envelope,
            Some(caught.plan),
            function,
            instruction,
        )
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
        payload_plan: Option<LinkedValueTransferPlan>,
        dispatch_function: FunctionIndex,
        dispatch_instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        self.unwind = Some(UnwindState {
            envelope,
            payload_plan,
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
        let pending_cursor = self
            .unwind
            .as_ref()
            .filter(|unwind| unwind.phase == UnwindPhase::Pending)
            .map(|unwind| unwind.cursor);
        if let Some(cursor) = pending_cursor {
            let frame = self.current_frame()?;
            if frame.function() != cursor.function || frame.instruction() != cursor.instruction {
                return Err(VmError::FiberNotRunnable { state: self.state });
            }
            self.unwind
                .as_mut()
                .expect("pending cursor came from the installed unwind")
                .phase = UnwindPhase::Searching;
        } else if self.unwind.is_none() {
            return Err(VmError::FiberNotRunnable { state: self.state });
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
                let unwind = self
                    .unwind
                    .take()
                    .ok_or(VmError::FiberNotRunnable { state: self.state })?;
                let site = VmLifecycleSite {
                    function: unwind.cursor.function,
                    instruction: unwind.cursor.instruction,
                    opcode: Opcode::Throw,
                };
                let exception = VmOwnedException::from_origin_authority(
                    Arc::clone(self.entry.image()),
                    unwind.envelope,
                    unwind.payload_plan,
                    site,
                );
                self.frames.clear();
                self.values.clear();
                self.live_values.clear();
                self.active_regions.clear();
                self.region_depths.clear();
                self.caught_exceptions.clear();
                self.caught_by_payload.clear();
                self.state = VmFiberState::Terminal;
                return Ok(DispatchOutcome::Throw(exception));
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
            let plan =
                self.stack_map_operand_plan(frame.function(), frame.instruction(), position)?;
            let value = self.values[index];
            executor.release(&value, &plan).map_err(|error| {
                error.into_vm_error(frame.function(), frame.instruction(), Opcode::Throw)
            })?;
            self.clear_value(index);
        }
        self.current_frame_mut()?.set_operand_height(handler_height);
        // The handler receives a shared snapshot of the envelope payload; the
        // envelope itself remains the single payload authority.
        let payload = envelope
            .vm_local_slot()
            .ok_or(VmError::ThrowEnvelopeUnavailable {
                function: frame.function(),
                instruction: frame.instruction(),
                reason: "caught envelope has no opaque VM payload".to_string(),
            })?;
        let catch_plan = self.slot_plan(frame.function(), region.catch_slot())?;
        let shared = executor.share(&payload, &catch_plan).map_err(|error| {
            error.into_vm_error(frame.function(), frame.instruction(), Opcode::Throw)
        })?;
        self.terminal_escrow.push(EscrowedOwner::new(
            shared,
            catch_plan.clone(),
            frame.function(),
            frame.instruction(),
            Opcode::Throw,
        ));
        let absolute_index =
            Self::slot_index(frame, slot_count, region.catch_slot(), frame.function())?;
        if let Err(error) = self.overwrite_slot(
            executor,
            frame,
            slot_count,
            region.catch_slot(),
            shared,
            frame.function(),
            frame.instruction(),
            Opcode::Throw,
        ) {
            return Err(error);
        }
        let adopted = self
            .terminal_escrow
            .pop()
            .expect("catch-slot shared owner is escrowed until slot adoption");
        debug_assert!(adopted.value == shared);
        let payload_handle = payload.as_handle().map(|handle| handle.get()).ok_or(
            VmError::ThrowEnvelopeUnavailable {
                function: frame.function(),
                instruction: frame.instruction(),
                reason: "caught envelope payload has no heap handle".to_string(),
            },
        )?;
        let entry = CaughtException {
            envelope: Arc::clone(envelope),
            plan: catch_plan,
            payload_handle,
            site: VmLifecycleSite {
                function: frame.function(),
                instruction: frame.instruction(),
                opcode: Opcode::Throw,
            },
        };
        if let Some(previous) = self.caught_exceptions.get(&absolute_index).cloned() {
            if let Some(slot) = previous.envelope.vm_local_slot() {
                executor.release(&slot, &previous.plan).map_err(|error| {
                    error.into_vm_error(frame.function(), frame.instruction(), Opcode::Throw)
                })?;
            }
            self.caught_by_payload.remove(&previous.payload_handle);
            self.caught_exceptions.remove(&absolute_index);
        }
        self.caught_exceptions.insert(absolute_index, entry);
        self.caught_by_payload
            .insert(payload_handle, absolute_index);
        self.frames
            .last_mut()
            .expect("validated catch handler retains its frame")
            .jump_to(region.handler());
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
        if !argument_plans
            .iter()
            .all(LifecycleExecutor::supports_transfer)
        {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::CallLocal,
            });
        }
        let (argument_frame, argument_start, arguments) = self.borrow_operands(arg_count)?;
        debug_assert_eq!(argument_frame.function(), caller.function());
        debug_assert_eq!(argument_frame.instruction(), caller.instruction());
        let remaining_height = caller.operand_height() - arg_count;
        let caller_ordinal = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        for ordinal in 0..arg_count {
            let moved = executor
                .transfer(&arguments[ordinal], &argument_plans[ordinal])
                .map_err(|error| error.into_vm_error(function, instruction, Opcode::CallLocal))?;
            // A transfer may change slot bits. Keep the new owner in its
            // original live operand cell until every argument transfer has
            // succeeded and the child segment can be committed atomically.
            self.values[argument_start + ordinal] = moved;
        }
        self.values
            .resize(child_start + segment_len, ValueSlot::null());
        self.live_values.resize(child_start + segment_len, false);
        for (ordinal, destination_slot) in transfer_slots.into_iter().enumerate() {
            let source_index = argument_start + ordinal;
            let value = self.values[source_index];
            self.values[child_start + destination_slot] = value;
            self.live_values[child_start + destination_slot] = true;
            self.clear_value(source_index);
        }
        self.frames[caller_ordinal].set_operand_height(remaining_height);
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
        if !argument_plans
            .iter()
            .all(LifecycleExecutor::supports_transfer)
        {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::TailCallLocal,
            });
        }
        if !self.terminal_escrow.is_empty() {
            return Err(VmError::FiberNotRunnable { state: self.state });
        }
        if caller.operand_height() != arg_count {
            return Err(VmError::OperandStackShapeMismatch {
                function,
                expected: arg_count,
                actual: caller.operand_height(),
            });
        }
        let frame_ordinal = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        let region_ordinal = self
            .region_depths
            .len()
            .checked_sub(1)
            .filter(|ordinal| *ordinal == frame_ordinal)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        let entry_depth = *self
            .region_depths
            .get(region_ordinal)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        let replacement = VmFrame::replacement(
            target,
            slot_base,
            target_slot_count,
            target_operand_capacity,
            caller.resume_instruction(),
        );
        let transfer_destinations = transfer_slots
            .into_iter()
            .map(|destination_slot| {
                slot_base
                    .checked_add(destination_slot)
                    .filter(|index| *index < requested)
                    .ok_or(VmError::VerifiedEntryInvariant {
                        invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (_, argument_start, arguments) = self.borrow_operands(arg_count)?;
        for ordinal in 0..arg_count {
            let moved = executor
                .transfer(&arguments[ordinal], &argument_plans[ordinal])
                .map_err(|error| {
                    error.into_vm_error(function, instruction, Opcode::TailCallLocal)
                })?;
            self.values[argument_start + ordinal] = moved;
        }
        self.terminal_escrow = argument_plans
            .into_iter()
            .enumerate()
            .map(|(ordinal, plan)| {
                EscrowedOwner::new(
                    self.values[argument_start + ordinal],
                    plan,
                    function,
                    instruction,
                    Opcode::TailCallLocal,
                )
            })
            .collect();
        for ordinal in 0..arg_count {
            self.clear_value(argument_start + ordinal);
        }
        self.frames[frame_ordinal].set_operand_height(0);
        self.release_frame_exit(executor, &caller, Opcode::TailCallLocal)?;
        let new_end = requested;
        self.values.resize(new_end, ValueSlot::null());
        self.live_values.resize(new_end, false);
        for (owner, destination_index) in std::mem::take(&mut self.terminal_escrow)
            .into_iter()
            .zip(transfer_destinations)
        {
            let value = owner.value;
            self.values[destination_index] = value;
            self.live_values[destination_index] = true;
        }
        self.active_regions.truncate(entry_depth);
        self.frames[frame_ordinal] = replacement;
        self.region_depths[region_ordinal] = entry_depth;
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
        if !self.terminal_escrow.is_empty() {
            return Err(VmError::FiberNotRunnable { state: self.state });
        }
        let (result_count, result_plans) = {
            let layout = self.function(frame.function())?.frame();
            (layout.result_types().len(), layout.result_plans().to_vec())
        };
        if result_count != result_plans.len()
            || !result_plans
                .iter()
                .all(LifecycleExecutor::supports_transfer)
        {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::Return,
            });
        }
        let child_return = if depth > 1 {
            Some(self.reserve_child_return(&frame, result_count)?)
        } else {
            None
        };
        let results = self.pop_operands(result_count, true)?;
        self.terminal_escrow = results
            .into_iter()
            .zip(result_plans)
            .map(|(value, plan)| {
                EscrowedOwner::new(value, plan, function, instruction, Opcode::Return)
            })
            .collect();
        for ordinal in 0..self.terminal_escrow.len() {
            let value = self.terminal_escrow[ordinal].value;
            let plan = self.terminal_escrow[ordinal].plan.clone();
            let moved = executor
                .transfer(&value, &plan)
                .map_err(|error| error.into_vm_error(function, instruction, Opcode::Return))?;
            self.terminal_escrow[ordinal].value = moved;
        }
        if depth == 1 {
            self.release_frame_exit(executor, &frame, Opcode::Return)?;
            let image = Arc::clone(self.entry.image());
            let root_results = std::mem::take(&mut self.terminal_escrow);
            let mut values = Vec::with_capacity(root_results.len());
            let mut plans = Vec::with_capacity(root_results.len());
            for owner in root_results {
                values.push(owner.value);
                plans.push(owner.plan);
            }
            self.frames.clear();
            self.values.clear();
            self.live_values.clear();
            self.active_regions.clear();
            self.region_depths.clear();
            self.caught_exceptions.clear();
            self.caught_by_payload.clear();
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
            return Ok(DispatchOutcome::Complete(VmOwnedValues::new_exact(
                image,
                values.into_boxed_slice(),
                plans.into_boxed_slice(),
            )));
        }

        self.release_frame_exit(executor, &frame, Opcode::Return)?;
        let child_return = child_return.expect("non-root return is fully reserved");
        debug_assert_eq!(child_return.child_frame_ordinal + 1, self.frames.len());
        let child = self
            .frames
            .pop()
            .expect("child return reservation keeps the active frame");
        self.region_depths
            .pop()
            .expect("child return reservation keeps the active region depth");
        self.values.truncate(child_return.caller_end);
        self.live_values.truncate(child_return.caller_end);
        self.active_regions
            .truncate(child_return.caller_region_depth);
        let values = std::mem::take(&mut self.terminal_escrow)
            .into_iter()
            .map(|owner| owner.value)
            .collect::<Vec<_>>();
        self.commit_operand_push_window(child_return.caller_destination, &values);
        self.frames[child_return.caller_frame_ordinal].resume_to(child_return.resume_instruction);
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
        let (record_tag, field_tags) = compact_record_type_tags(
            function,
            instruction,
            shape.nominal_type(),
            shape.fields().iter().map(|field| field.ty()),
        )?;
        let frame = self.current_frame()?.clone();
        let field_plans = (0..field_count)
            .map(|ordinal| self.operand_plan(&frame, instruction, field_count - 1 - ordinal))
            .collect::<Result<Vec<_>, _>>()?;
        if !field_plans.iter().all(LifecycleExecutor::supports_transfer) {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::NewRecord,
            });
        }

        // Resolve every image-borrowed payload and every output index before
        // the first heap mutation. The operand window stays live throughout
        // materialization and transfer; each successful owner is written back
        // into its original root slot before another fallible step begins.
        let (reservation, values) = self.reserve_operand_window_replacement(field_count)?;
        let constant_strings = values
            .iter()
            .map(|source| {
                if matches!(source.kind(), Some(ValueKind::ConstRef)) {
                    self.string_slot_value(executor.heap(), source).map(Some)
                } else {
                    Ok(None)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut fields = Vec::with_capacity(field_count);
        for (ordinal, field) in shape.fields().iter().enumerate() {
            let source_index = reservation.start + ordinal;
            let source = self.values[source_index];
            let value = if let Some(string) = constant_strings[ordinal].clone() {
                materialize_new_record_const_string(executor, string, field_tags[ordinal])?
            } else {
                executor
                    .transfer(&source, &field_plans[ordinal])
                    .map_err(|error| {
                        error.into_vm_error(function, instruction, Opcode::NewRecord)
                    })?
            };
            self.values[source_index] = value;
            fields.push(VmRecordField {
                name: field.name().to_string(),
                value,
            });
        }
        let value = executor
            .heap()
            .allocate_record(&fields, record_tag, ValueFlags::new(0))
            .map_err(VmError::Heap)?;
        self.commit_operand_window_replacement(reservation, value);
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
        let (reservation, operands) =
            self.reserve_operand_consume(function, instruction, Opcode::GetDenseField, 1, 1)?;
        let record = operands[0];
        let field_plan = self.reserved_result_plan(&reservation, 0).clone();
        let value = executor
            .heap()
            .get_dense_field(&record, field_ordinal)
            .map_err(VmError::Heap)?;
        let shared = executor
            .share(&value, &field_plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::GetDenseField))?;
        self.terminal_escrow.push(EscrowedOwner::new(
            shared,
            field_plan,
            function,
            instruction,
            Opcode::GetDenseField,
        ));
        self.release_reserved_sources_reverse(executor, &reservation, 0, 1)?;
        let shared = self
            .terminal_escrow
            .pop()
            .expect("dense field result was escrowed before source release")
            .value;
        self.commit_operand_result(reservation, shared);
        Ok(DispatchOutcome::Continue)
    }

    fn execute_take_dense_field(
        &mut self,
        heap: &mut dyn VmHeap,
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
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Shapes,
                row: shape_index.get(),
            })?;
        let Some(field) = shape.fields().get(field_ordinal) else {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::TakeDenseField,
            });
        };
        let nominal_type = self
            .execution_image()
            .types()
            .get(shape.nominal_type().get() as usize)
            .filter(|row| row.index() == shape.nominal_type())
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::Types,
                row: shape.nominal_type().get(),
            })?;
        let privileged_schema = match nominal_type.type_ref() {
            TypeRefIr::PackageSymbol { symbol } => NATIVE_VALUE_LIFECYCLE_REGISTRY
                .privileged_affine_composite_for_symbol(symbol)
                .filter(|schema| {
                    schema.identity == PrivilegedAffineCompositeIdentity::HttpClientStreamHandle
                }),
            _ => None,
        };
        let exact_affine_field = privileged_schema
            .filter(|schema| {
                schema.embedding == NativeValueEmbedding::Privileged
                    && matches!(
                        &schema.lifecycle,
                        NativeValueLifecycleConcrete::MoveOnly {
                            drop: NativeValueDropPlan::PrivilegedRecursiveShape
                        }
                    )
                    && schema.fields.len() == shape.fields().len()
                    && schema.fields.iter().zip(shape.fields()).all(
                        |(schema_field, linked_field)| {
                            schema_field.name == linked_field.name()
                                && linked_plan_matches_native(
                                    linked_field.plan(),
                                    &schema_field.lifecycle,
                                )
                        },
                    )
            })
            .and_then(|schema| schema.fields.get(field_ordinal))
            .is_some_and(|schema_field| {
                schema_field.name == field.name()
                    && schema_field.access == PrivilegedAffineFieldAccess::AffineTake
                    && matches!(
                        &schema_field.lifecycle,
                        NativeValueLifecycleConcrete::AffineResource {
                            drop: NativeResourceDropPlan::ResourceTableRelease
                        }
                    )
            });
        let frame = self.current_frame()?.clone();
        let record_plan = self.operand_plan(&frame, instruction, 0)?;
        let exact_record_plan = matches!(
            record_plan,
            LinkedValueTransferPlan::MoveOnly {
                drop: LinkedValueDropPlan::RecursiveShape { shape }
            } if shape == shape_index
        );
        let exact_field_plan = matches!(
            field.plan(),
            LinkedValueTransferPlan::AffineResource {
                drop: LinkedResourceDropPlan::ResourceTableRelease
            }
        );
        if !exact_affine_field || !exact_record_plan || !exact_field_plan {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::TakeDenseField,
            });
        }

        // This opcode is 1 -> 1. Keep the aggregate rooted in its operand
        // cell until the physical take succeeds, then replace that same cell
        // with the returned affine owner. No error path can lose either owner.
        let operand_position =
            frame
                .operand_height()
                .checked_sub(1)
                .ok_or(VmError::OperandStackUnderflow {
                    function,
                    needed: 1,
                    available: frame.operand_height(),
                })?;
        let value_index = frame.operand_base().checked_add(operand_position).ok_or(
            VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            },
        )?;
        if !self.live_values.get(value_index).copied().unwrap_or(false) {
            return Err(VmError::DeadValueRead {
                location: VmValueLocation::Operand(operand_position),
            });
        }
        let record = self.values[value_index];
        let taken = heap
            .take_dense_field(&record, field_ordinal)
            .map_err(VmError::Heap)?;
        self.values[value_index] = taken;
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
            .is_ok()
            || self
                .function(frame.function())?
                .frame()
                .parameters()
                .iter()
                .any(|parameter| {
                    parameter.slot() == root_slot && root.kind() == Some(ValueKind::RequestHeapRef)
                });
        if !writable {
            return Err(VmError::LiveDestination {
                function,
                instruction,
                location: VmValueLocation::FrameSlot(root_slot),
            });
        }
        let root_index = Self::slot_index(&frame, slot_count, root_slot, frame.function())?;
        let root_plan = self.slot_plan(frame.function(), root_slot)?;
        if !LifecycleExecutor::supports_release(&root_plan) {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::SetWritablePath,
            });
        }
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
        let source_count = selector_count
            .checked_add(1)
            .ok_or(VmError::OperandStackOverflow {
                function,
                capacity: frame.operand_capacity(),
            })?;
        let (reservation, operands) = self.reserve_operand_consume(
            function,
            instruction,
            Opcode::SetWritablePath,
            source_count,
            0,
        )?;
        let rhs_ordinal = selector_count;
        let rhs_plan = self.reserved_source_plan(&reservation, rhs_ordinal).clone();
        if !LifecycleExecutor::supports_transfer(&rhs_plan) {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::SetWritablePath,
            });
        }
        let prepared = executor
            .heap()
            .prepare_writable_path(&root, &segments, &operands[..selector_count])
            .map_err(VmError::Heap)?;
        let value = executor
            .transfer(&operands[rhs_ordinal], &rhs_plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::SetWritablePath))?;
        self.reanchor_reserved_source(&reservation, rhs_ordinal, value);
        let replacement = executor
            .heap()
            .commit_writable_path(prepared, value)
            .map_err(VmError::Heap)?;
        self.adopt_reserved_source(&reservation, rhs_ordinal);

        let replaced_root = replacement != root;
        if replaced_root {
            // A copy-on-write root becomes a VM owner at the successful heap
            // commit. Root it before any selector or old-root release can
            // fail; terminal collection then retains both sides exactly once.
            self.terminal_escrow.push(EscrowedOwner::new(
                replacement,
                root_plan.clone(),
                function,
                instruction,
                Opcode::SetWritablePath,
            ));
        }
        self.release_reserved_sources_reverse(executor, &reservation, 0, selector_count)?;
        if replaced_root {
            if let Err(error) = executor.release(&root, &root_plan) {
                self.state = VmFiberState::Terminal;
                return Err(error.into_vm_error(function, instruction, Opcode::SetWritablePath));
            }
            self.clear_value(root_index);
            let replacement = self
                .terminal_escrow
                .pop()
                .expect("writable-path replacement was escrowed before root release")
                .value;
            self.values[root_index] = replacement;
            self.live_values[root_index] = true;
        }
        self.commit_consumed_operands(reservation);
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
                compact_type_tag(function, instruction, element_type)?,
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
        let (reservation, values) =
            self.reserve_operand_consume(function, instruction, Opcode::ArrayBuilderPush, 2, 1)?;
        let builder = values[0];
        if self.reserved_source_plan(&reservation, 0) != self.reserved_result_plan(&reservation, 0)
        {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::ArrayBuilderPush,
            });
        }
        let value_plan = self.reserved_source_plan(&reservation, 1).clone();
        if !LifecycleExecutor::supports_transfer(&value_plan) {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::ArrayBuilderPush,
            });
        }
        let value = executor
            .transfer(&values[1], &value_plan)
            .map_err(|error| {
                error.into_vm_error(function, instruction, Opcode::ArrayBuilderPush)
            })?;
        self.reanchor_reserved_source(&reservation, 1, value);
        executor
            .heap()
            .array_push_owned(&builder, value)
            .map_err(VmError::Heap)?;
        self.adopt_reserved_source(&reservation, 1);
        self.commit_operand_result(reservation, builder);
        Ok(DispatchOutcome::Continue)
    }

    fn execute_freeze_array(
        &mut self,
        heap: &mut dyn VmHeap,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let (reservation, operands) =
            self.reserve_operand_consume(function, instruction, Opcode::FreezeArray, 1, 1)?;
        let value = operands[0];
        if self.reserved_source_plan(&reservation, 0) != self.reserved_result_plan(&reservation, 0)
        {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::FreezeArray,
            });
        }
        heap.validate_live(&value).map_err(VmError::Heap)?;
        self.commit_operand_result(reservation, value);
        Ok(DispatchOutcome::Continue)
    }

    fn execute_array_get(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<DispatchOutcome, VmError> {
        let (reservation, values) =
            self.reserve_operand_consume(function, instruction, Opcode::ArrayGet, 2, 1)?;
        let array = values[0];
        let index = skiff_runtime_model::vm_heap::collection_index(&values[1]).ok_or(
            VmError::ExpectedNumber {
                function,
                instruction,
                actual: values[1].kind(),
            },
        )?;
        let value = executor
            .heap()
            .array_get(&array, index)
            .map_err(VmError::Heap)?;
        let element_plan = self.reserved_result_plan(&reservation, 0).clone();
        let shared = executor
            .share(&value, &element_plan)
            .map_err(|error| error.into_vm_error(function, instruction, Opcode::ArrayGet))?;
        self.terminal_escrow.push(EscrowedOwner::new(
            shared,
            element_plan,
            function,
            instruction,
            Opcode::ArrayGet,
        ));
        self.release_reserved_sources_reverse(executor, &reservation, 0, 2)?;
        let shared = self
            .terminal_escrow
            .pop()
            .expect("array element result was escrowed before source release")
            .value;
        self.commit_operand_result(reservation, shared);
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
                compact_type_tag(function, instruction, value_type)?,
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
        let ordinal = skiff_runtime_model::vm_heap::collection_index(&values[1]).ok_or(
            VmError::ExpectedNumber {
                function,
                instruction,
                actual: values[1].kind(),
            },
        )?;
        let entry = heap.map_entry_at(&map, ordinal).map_err(VmError::Heap)?;
        self.push_operand(entry.key)?;
        self.push_operand(entry.value)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_invoke_intrinsic(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
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
        self.execute_resolved_intrinsic(
            executor,
            function,
            instruction,
            &intrinsic,
            arg_count,
            result_count,
        )
    }

    fn execute_resolved_intrinsic(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
        function: FunctionIndex,
        instruction: InstructionIndex,
        intrinsic: &skiff_runtime_linked_bytecode::LinkedIntrinsicTarget,
        arg_count: usize,
        result_count: usize,
    ) -> Result<DispatchOutcome, VmError> {
        if self.state != VmFiberState::Runnable {
            return Err(VmError::FiberNotRunnable { state: self.state });
        }
        validate_native_signature_counts(
            intrinsic.signature(),
            arg_count,
            result_count,
            function,
            instruction,
            Opcode::InvokeIntrinsic,
        )?;
        if !intrinsic
            .signature()
            .parameter_plans()
            .iter()
            .chain(intrinsic.signature().result_plans())
            .all(LifecycleExecutor::supports_release)
        {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeIntrinsic,
            });
        }
        if let LinkedIntrinsicKind::Receiver(op) = intrinsic.kind() {
            if op.canonical_key == "receiver:Array.push@1" {
                // Production emission uses the dedicated ArrayPushOwned
                // opcode, whose ownership contract explicitly adopts only
                // the item. The legacy intrinsic form fails before moving
                // either still-rooted operand.
                return Err(VmError::FullValueLifecyclePlanUnavailable {
                    function,
                    instruction,
                    opcode: Opcode::InvokeIntrinsic,
                });
            }
        }
        if intrinsic.db_operation().is_some() {
            return self.execute_db_intrinsic_child(
                function,
                instruction,
                intrinsic,
                arg_count,
                result_count,
            );
        }
        if intrinsic.task_target().is_some() {
            return self.execute_task_intrinsic_child(
                function,
                instruction,
                intrinsic,
                arg_count,
                result_count,
            );
        }
        if result_count != 1 {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeIntrinsic,
            });
        }
        let (reservation, values) =
            self.reserve_intrinsic_result(function, instruction, arg_count)?;
        let result_type = intrinsic.signature().result_types().first().copied();
        let result_plan = intrinsic.signature().result_plans().first();
        let (result_type, _result_plan) =
            result_type
                .zip(result_plan)
                .ok_or(VmError::FullValueLifecyclePlanUnavailable {
                    function,
                    instruction,
                    opcode: Opcode::InvokeIntrinsic,
                })?;
        let result_type_tag = compact_type_tag(function, instruction, result_type)?;
        let payload = self.read_borrowing_intrinsic_result(
            executor.heap(),
            intrinsic.kind(),
            &values,
            function,
            instruction,
        )?;
        self.release_intrinsic_argument_window(
            executor,
            &reservation,
            &values,
            intrinsic.signature().parameter_plans(),
            function,
            instruction,
        )?;
        let result = match materialize_intrinsic_result(executor.heap(), payload, result_type_tag) {
            Ok(result) => result,
            Err(error) => {
                // Argument release has committed monotonically. The original
                // instruction cannot be re-read after that point, so an
                // allocation failure is terminal even when dispatch is called
                // directly outside `run_segment`.
                self.state = VmFiberState::Terminal;
                return Err(error);
            }
        };
        self.commit_intrinsic_result(reservation, result);
        Ok(DispatchOutcome::Continue)
    }

    fn execute_db_intrinsic_child(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        intrinsic: &skiff_runtime_linked_bytecode::LinkedIntrinsicTarget,
        arg_count: usize,
        result_count: usize,
    ) -> Result<DispatchOutcome, VmError> {
        let operation = intrinsic.db_operation().cloned().ok_or_else(|| {
            VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeIntrinsic,
            }
        })?;
        if arg_count != 1 || result_count != 1 {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeIntrinsic,
            });
        }
        let signature = intrinsic.signature();
        if signature.parameter_plans().len() != 1
            || signature.result_types().len() != 1
            || signature.result_plans().len() != 1
            || signature.parameter_plans()[0] != *operation.parameter_plan()
            || signature.result_types()[0] != operation.result_type()
            || signature.result_plans()[0] != *operation.result_plan()
        {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeIntrinsic,
            });
        }
        let arguments = self.pop_operands(arg_count, false)?;
        let argument_plans = signature.parameter_plans().to_vec();
        let expected_stack_height =
            self.current_frame()?
                .operand_height()
                .try_into()
                .map_err(|_| VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        let frame = self.current_frame()?.clone();
        let advance = self.reserve_instruction_advance(&frame, function, instruction)?;
        let target = ChildTarget::Db(intrinsic.index());
        let token = self.mint_resume(
            function,
            instruction,
            VmResumeAuthority::Child(target),
            ResumeSiteIndex::new(0),
            advance.next_instruction(),
            None,
            expected_stack_height,
            result_count as u32,
            None,
            None,
        )?;
        let invocation = ChildInvocation::new(
            target,
            VmOwnedValues::new_exact(
                Arc::clone(self.entry.image()),
                arguments.into_boxed_slice(),
                argument_plans.into_boxed_slice(),
            ),
            token,
        )
        .map_err(|_| VmError::ResumeTokenMismatch)?;
        self.state = VmFiberState::BlockedOnChild;
        Ok(DispatchOutcome::Handoff(VmControl::EnterChild(invocation)))
    }

    fn execute_task_intrinsic_child(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        intrinsic: &skiff_runtime_linked_bytecode::LinkedIntrinsicTarget,
        arg_count: usize,
        result_count: usize,
    ) -> Result<DispatchOutcome, VmError> {
        let task_target = intrinsic.task_target().cloned().ok_or_else(|| {
            VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeIntrinsic,
            }
        })?;
        let intrinsic_signature = intrinsic.signature();
        let target_signature = task_target.signature();
        let payload_count = target_signature.parameter_types().len();
        let timing_operand_count = match task_target.timing() {
            LinkedTaskTiming::Immediate => 0,
            LinkedTaskTiming::After { .. } | LinkedTaskTiming::At { .. } => 1,
        };
        let expected_arg_count = payload_count + timing_operand_count;
        if arg_count != expected_arg_count
            || arg_count != intrinsic_signature.parameter_types().len()
            || result_count != 1
            || intrinsic_signature.parameter_plans().len() != arg_count
            || intrinsic_signature.result_types().len() != 1
            || intrinsic_signature.result_plans().len() != 1
        {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeIntrinsic,
            });
        }
        if target_signature.parameter_types()
            != &intrinsic_signature.parameter_types()[..payload_count]
            || target_signature.parameter_modes()
                != &intrinsic_signature.parameter_modes()[..payload_count]
            || target_signature.parameter_plans()
                != &intrinsic_signature.parameter_plans()[..payload_count]
        {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeIntrinsic,
            });
        }
        let timing_operand = if timing_operand_count == 1 {
            Some(self.borrow_operands(1)?.2.remove(0))
        } else {
            None
        };
        let timing = TaskDispatchTiming::resolve_from_slot(task_target.timing(), timing_operand)
            .map_err(|error| VmError::TaskDispatchTimingUnavailable {
                reason: error.to_string(),
            })?;
        if timing_operand.is_some() {
            self.pop_operands(1, false)?;
        }
        let result_type = intrinsic.signature().result_types()[0];
        let result_plan = intrinsic.signature().result_plans()[0].clone();
        if self.execution_image().type_plan(result_type) != Some(&result_plan) {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeIntrinsic,
            });
        }
        let dispatch =
            TaskDispatchIndex::from_task_target_index(task_target.index()).ok_or_else(|| {
                VmError::FullValueLifecyclePlanUnavailable {
                    function,
                    instruction,
                    opcode: Opcode::InvokeIntrinsic,
                }
            })?;
        let arguments = self.pop_operands(payload_count, false)?;
        let argument_plans = target_signature.parameter_plans().to_vec();
        let expected_stack_height =
            self.current_frame()?
                .operand_height()
                .try_into()
                .map_err(|_| VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        let frame = self.current_frame()?.clone();
        let advance = self.reserve_instruction_advance(&frame, function, instruction)?;
        let target = ChildTarget::Task(dispatch);
        let task_plan = TaskIntrinsicResumePlan::new(task_target, result_type, result_plan, timing);
        let token = self.mint_resume(
            function,
            instruction,
            VmResumeAuthority::Child(target),
            ResumeSiteIndex::new(0),
            advance.next_instruction(),
            None,
            expected_stack_height,
            result_count as u32,
            None,
            Some(task_plan),
        )?;
        let invocation = ChildInvocation::new(
            target,
            VmOwnedValues::new_exact(
                Arc::clone(self.entry.image()),
                arguments.into_boxed_slice(),
                argument_plans.into_boxed_slice(),
            ),
            token,
        )
        .map_err(|_| VmError::ResumeTokenMismatch)?;
        self.state = VmFiberState::BlockedOnChild;
        Ok(DispatchOutcome::Handoff(VmControl::EnterChild(invocation)))
    }

    fn read_borrowing_intrinsic_result(
        &self,
        heap: &mut dyn VmHeap,
        kind: &LinkedIntrinsicKind,
        values: &[ValueSlot],
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<IntrinsicResultPayload, VmError> {
        match kind {
            LinkedIntrinsicKind::Static(target) => match target.canonical_key().as_str() {
                "core.array.empty" => Ok(IntrinsicResultPayload::EmptyArray),
                "core.map.empty" => Ok(IntrinsicResultPayload::EmptyMap),
                "std.string.length" => {
                    if values.len() != 1 {
                        return Err(VmError::FullValueLifecyclePlanUnavailable {
                            function,
                            instruction,
                            opcode: Opcode::InvokeIntrinsic,
                        });
                    }
                    let value = self.string_slot_value(heap, &values[0])?;
                    Ok(IntrinsicResultPayload::Number(value.chars().count() as f64))
                }
                "core.bytes.fromUtf8" => {
                    if values.len() != 1 {
                        return Err(VmError::FullValueLifecyclePlanUnavailable {
                            function,
                            instruction,
                            opcode: Opcode::InvokeIntrinsic,
                        });
                    }
                    let value = self.string_slot_value(heap, &values[0])?;
                    Ok(IntrinsicResultPayload::Bytes(value.into_bytes()))
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
                "receiver:string.length@1" => {
                    if values.len() != 1 {
                        return Err(VmError::FullValueLifecyclePlanUnavailable {
                            function,
                            instruction,
                            opcode: Opcode::InvokeIntrinsic,
                        });
                    }
                    let value = self.string_slot_value(heap, &values[0])?;
                    Ok(IntrinsicResultPayload::Number(value.chars().count() as f64))
                }
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
                    Ok(IntrinsicResultPayload::String(format!("{left}{right}")))
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
                    Ok(IntrinsicResultPayload::String(
                        String::from_utf8_lossy(&bytes).into_owned(),
                    ))
                }
                _ => {
                    return Err(VmError::FullValueLifecyclePlanUnavailable {
                        function,
                        instruction,
                        opcode: Opcode::InvokeIntrinsic,
                    });
                }
            },
        }
    }

    fn execute_interface_box_local(
        &mut self,
        heap: &mut dyn VmHeap,
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
        let row = self
            .execution_image()
            .interface_tables()
            .get(table_index.get() as usize)
            .filter(|row| row.index() == table_index)
            .expect("interface table row was just checked");
        let LinkedInterfaceTableKind::Local(local) = row.kind() else {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InterfaceBoxLocal,
            });
        };
        let local = local.clone();
        // The linked local table carries the exact concrete payload type. The
        // stack-map carrier type is normalized into an owner-form PackageSymbol
        // while the relocation keeps its publication form; fall back only to
        // the exact table fact when that normalized carrier row is unavailable.
        let carrier_type =
            interface_carrier_type(self.execution_image(), row).unwrap_or(local.concrete_type());
        let exact: Arc<dyn Any + Send + Sync> = Arc::new(local.clone());
        let table = VmLocalInterfaceTable::new(
            table_index.get(),
            local.concrete_type().get(),
            local.methods().len(),
            exact,
        );
        let payload = self.pop_operands(1, false)?.remove(0);
        let carrier = heap
            .allocate_local_interface(
                &payload,
                table,
                compact_type_tag(function, instruction, carrier_type)?,
                ValueFlags::new(0),
            )
            .map_err(VmError::Heap)?;
        self.push_operand(carrier)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
    }

    fn execute_interface_box_remote(
        &mut self,
        heap: &mut dyn VmHeap,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        let LinkedInstructionTarget::InterfaceTable(table_index) =
            self.resolved_target(function, instruction, decoded, 0)?
        else {
            return Err(self.malformed_instruction(function, instruction, decoded));
        };
        let row = self
            .execution_image()
            .interface_tables()
            .get(table_index.get() as usize)
            .filter(|row| row.index() == table_index)
            .ok_or(VmError::LinkedTableRowMissing {
                table: CandidateTable::InterfaceTables,
                row: table_index.get(),
            })?;
        let LinkedInterfaceTableKind::Remote(remote) = row.kind() else {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InterfaceBoxRemote,
            });
        };
        let carrier_type =
            interface_carrier_type(self.execution_image(), row).ok_or_else(|| {
                VmError::FullValueLifecyclePlanUnavailable {
                    function,
                    instruction,
                    opcode: Opcode::InterfaceBoxRemote,
                }
            })?;
        let exact: Arc<dyn Any + Send + Sync> = Arc::new(remote.clone());
        let table = VmRemoteInterfaceTable::new(table_index.get(), remote.methods().len(), exact);
        let carrier = heap
            .allocate_remote_interface(
                table,
                compact_type_tag(function, instruction, carrier_type)?,
                ValueFlags::new(0),
            )
            .map_err(VmError::Heap)?;
        self.push_operand(carrier)?;
        self.advance_current_instruction()?;
        Ok(DispatchOutcome::Continue)
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
        let argument_plans = target.signature().parameter_plans().to_vec();
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
            None,
            None,
        )?;
        let invocation = ChildInvocation::new(
            target,
            VmOwnedValues::new_exact(
                Arc::clone(self.entry.image()),
                arguments.into_boxed_slice(),
                argument_plans.into_boxed_slice(),
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
        let argument_plans = target.signature().parameter_plans().to_vec();
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
            None,
            None,
        )?;
        let invocation = ChildInvocation::new(
            target,
            VmOwnedValues::new_exact(
                Arc::clone(self.entry.image()),
                arguments.into_boxed_slice(),
                argument_plans.into_boxed_slice(),
            ),
            token,
        )
        .map_err(|_| VmError::ResumeTokenMismatch)?;
        self.state = VmFiberState::BlockedOnChild;
        Ok(DispatchOutcome::Handoff(VmControl::EnterChild(invocation)))
    }

    fn execute_call_interface(
        &mut self,
        heap: &mut dyn VmHeap,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        self.execute_interface_boundary(heap, function, instruction, decoded, Opcode::CallInterface)
    }

    fn execute_invoke_callback(
        &mut self,
        heap: &mut dyn VmHeap,
        function: FunctionIndex,
        instruction: InstructionIndex,
        decoded: &LinkedInstruction,
    ) -> Result<DispatchOutcome, VmError> {
        self.execute_interface_boundary(
            heap,
            function,
            instruction,
            decoded,
            Opcode::InvokeCallback,
        )
    }

    fn execute_interface_boundary(
        &mut self,
        heap: &mut dyn VmHeap,
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
        let (signature, remote_plan) = match table.kind() {
            LinkedInterfaceTableKind::Requirement(requirement)
            | LinkedInterfaceTableKind::Callback(requirement) => requirement
                .methods()
                .get(method_ordinal)
                .map(|method| (Some(method.signature()), None)),
            LinkedInterfaceTableKind::Local(local) => local
                .methods()
                .get(method_ordinal)
                .map(|method| (Some(method.signature()), None)),
            LinkedInterfaceTableKind::Remote(remote) => {
                Some(match remote.methods().get(method_ordinal) {
                    Some(method) => (
                        Some(method.signature()),
                        Some(RemoteInterfaceCallPlan::new(remote.clone(), method.clone())),
                    ),
                    None => (None, None),
                })
            }
        }
        .ok_or(VmError::FullValueLifecyclePlanUnavailable {
            function,
            instruction,
            opcode,
        })?;
        let Some(signature) = signature else {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode,
            });
        };
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
        let argument_plans = signature.parameter_plans().to_vec();
        match table.kind() {
            LinkedInterfaceTableKind::Local(local) => {
                let (_, _, operands) = self.borrow_operands(input_count)?;
                let carrier = operands
                    .first()
                    .ok_or_else(|| VmError::OperandStackUnderflow {
                        function,
                        needed: 1,
                        available: 0,
                    })?;
                let carrier_table = heap.local_interface_table(carrier).map_err(VmError::Heap)?;
                if carrier_table.table_index() != table_index.get()
                    || carrier_table.concrete_type() != local.concrete_type().get()
                    || carrier_table.method_count() != local.methods().len()
                    || method_ordinal >= local.methods().len()
                {
                    return Err(VmError::FullValueLifecyclePlanUnavailable {
                        function,
                        instruction,
                        opcode,
                    });
                }
            }
            LinkedInterfaceTableKind::Remote(remote) => {
                let (_, _, operands) = self.borrow_operands(input_count)?;
                let carrier = operands
                    .first()
                    .ok_or_else(|| VmError::OperandStackUnderflow {
                        function,
                        needed: 1,
                        available: 0,
                    })?;
                let carrier_table = heap
                    .remote_interface_table(carrier)
                    .map_err(VmError::Heap)?;
                if carrier_table.table_index() != table_index.get()
                    || carrier_table.method_count() != remote.methods().len()
                    || method_ordinal >= remote.methods().len()
                {
                    return Err(VmError::FullValueLifecyclePlanUnavailable {
                        function,
                        instruction,
                        opcode,
                    });
                }
            }
            LinkedInterfaceTableKind::Requirement(_) | LinkedInterfaceTableKind::Callback(_) => {}
        }
        let carrier_plan = signature
            .parameter_plans()
            .first()
            .cloned()
            .ok_or_else(|| VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode,
            })?;
        let interface_plan = InterfaceCallPlan::new(signature.clone(), carrier_plan, remote_plan);
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
            Some(interface_plan),
            None,
        )?;
        let invocation = ChildInvocation::new(
            target,
            VmOwnedValues::new_exact(
                Arc::clone(self.entry.image()),
                arguments.into_boxed_slice(),
                argument_plans.into_boxed_slice(),
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
            .execution_image()
            .host_effect_target(adapter_index)
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
        let argument_plans = adapter
            .signature()
            .parameter_plans()
            .to_vec()
            .into_boxed_slice();
        if argument_plans.len() != arg_count {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeHost,
            });
        }
        let frame = self.current_frame()?.clone();
        if frame.operand_height() < arg_count {
            return Err(VmError::OperandStackUnderflow {
                function,
                needed: arg_count,
                available: frame.operand_height(),
            });
        }
        let stack_argument_plans = (0..arg_count)
            .map(|ordinal| self.operand_plan(&frame, instruction, arg_count - 1 - ordinal))
            .collect::<Result<Vec<_>, VmError>>()?;
        if stack_argument_plans.as_slice() != argument_plans.as_ref() {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeHost,
            });
        }
        let remaining_height = frame.operand_height() - arg_count;
        let start = frame.operand_base().checked_add(remaining_height).ok_or(
            VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            },
        )?;
        let end = start
            .checked_add(arg_count)
            .filter(|end| *end <= self.values.len() && *end <= self.live_values.len())
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        let mut arguments = Vec::with_capacity(arg_count);
        for index in start..end {
            if !self.live_values[index] {
                return Err(VmError::DeadValueRead {
                    location: VmValueLocation::Operand(index - frame.operand_base()),
                });
            }
            arguments.push(self.values[index]);
        }
        let expected_stack_height =
            remaining_height
                .try_into()
                .map_err(|_| VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        if expected_stack_height != resume.expected_stack_height_before_result() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        let expected_result_count =
            u32::try_from(result_count).map_err(|_| VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        let token = self.mint_resume(
            function,
            instruction,
            VmResumeAuthority::Adapter(adapter_index),
            resume_site,
            resume.resume(),
            None,
            expected_stack_height,
            expected_result_count,
            None,
            None,
        )?;
        let invocation = AdapterInvocation::new(
            adapter_index,
            crate::VmHostEffectArguments::new(
                VmOwnedValues::new_exact(
                    Arc::clone(self.entry.image()),
                    arguments.into_boxed_slice(),
                    argument_plans.clone(),
                ),
                argument_plans,
                function,
                instruction,
            ),
            token,
        );
        // Every fallible validation and continuation mint completed before
        // this physical ownership transfer. No `?`, callback or allocation is
        // permitted between clearing the source slots and returning the
        // sealed invocation owner.
        for index in start..end {
            self.clear_value(index);
        }
        self.frames
            .last_mut()
            .expect("validated runnable host call retains its current frame")
            .set_operand_height(remaining_height);
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
        let endpoint = self
            .read_slot(&frame, slot_count, endpoint_slot)?
            .as_resource_ref()
            .ok_or(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::StreamNext,
            })?;
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
        let arguments = VmOwnedValues::empty(Arc::clone(self.entry.image()));
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
            None,
            None,
        )?;
        let invocation = ChildInvocation::new_stream_next(
            crate::control::StreamEndpointRef::new(endpoint),
            arguments,
            token,
        )
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
        let frame = self.current_frame()?.clone();
        if frame.operand_height() < 1 {
            return Err(VmError::OperandStackUnderflow {
                function,
                needed: 1,
                available: frame.operand_height(),
            });
        }
        let (item_type, item_plan) = self.operand_type_and_plan(&frame, instruction, 0)?;
        let item_shape =
            resume
                .emit_stream_item_shape()
                .ok_or(VmError::FullValueLifecyclePlanUnavailable {
                    function,
                    instruction,
                    opcode: Opcode::EmitStream,
                })?;
        if !LifecycleExecutor::supports_release(&item_plan) {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::EmitStream,
            });
        }
        let remaining_height = frame.operand_height() - 1;
        let item_index = frame
            .operand_base()
            .checked_add(remaining_height)
            .filter(|index| *index < self.values.len() && *index < self.live_values.len())
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        if !self.live_values[item_index] {
            return Err(VmError::DeadValueRead {
                location: VmValueLocation::Operand(remaining_height),
            });
        }
        let item = self.values[item_index];
        let expected_stack_height =
            remaining_height
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
            None,
            None,
        )?;
        let stream_item = StreamItem::new(
            VmOwnedValues::new_exact(
                Arc::clone(self.entry.image()),
                Box::new([item]),
                Box::new([item_plan.clone()]),
            ),
            item_type,
            item_shape,
            item_plan,
            function,
            instruction,
            token,
        );
        // Every fallible validation and continuation mint completed before
        // this physical ownership transfer. No `?`, callback or allocation is
        // permitted between clearing the source slot and returning the sealed
        // stream-item owner.
        self.clear_value(item_index);
        self.frames
            .last_mut()
            .expect("validated runnable EmitStream retains its current frame")
            .set_operand_height(remaining_height);
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
        executor: &mut LifecycleExecutor<'_>,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
    ) -> Result<DispatchOutcome, VmError> {
        let (reservation, operands) =
            self.reserve_operand_consume(function, instruction, opcode, 2, 1)?;
        let equal = self
            .comparable_equality(executor.heap(), &operands[0], &operands[1])
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
        self.release_reserved_sources_reverse(executor, &reservation, 0, 2)?;
        self.commit_operand_result(reservation, ValueSlot::bool(result));
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
    ) -> Result<&ExecutionResumeSite, VmError> {
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
        interface_plan: Option<crate::control::InterfaceCallPlan>,
        task_plan: Option<crate::control::TaskIntrinsicResumePlan>,
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
            interface_plan,
            task_plan,
        );
        self.pending_resume = Some(PendingResume {
            binding: Arc::clone(token.binding()),
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

    fn slot_type(
        &self,
        function: FunctionIndex,
        slot: FrameSlotIndex,
    ) -> Result<TypeIndex, VmError> {
        self.function(function)?
            .frame()
            .slot_types()
            .get(slot.get() as usize)
            .copied()
            .ok_or(VmError::SlotOutOfBounds { function, slot })
    }

    fn operand_plan(
        &self,
        frame: &VmFrame,
        instruction: InstructionIndex,
        from_top: usize,
    ) -> Result<LinkedValueTransferPlan, VmError> {
        let position = frame.operand_height().checked_sub(from_top + 1).ok_or(
            VmError::OperandStackUnderflow {
                function: frame.function(),
                needed: from_top + 1,
                available: frame.operand_height(),
            },
        )?;
        self.stack_map_operand_plan(frame.function(), instruction, position)
    }

    fn operand_type_and_plan(
        &self,
        frame: &VmFrame,
        instruction: InstructionIndex,
        from_top: usize,
    ) -> Result<(TypeIndex, LinkedValueTransferPlan), VmError> {
        let position = frame.operand_height().checked_sub(from_top + 1).ok_or(
            VmError::OperandStackUnderflow {
                function: frame.function(),
                needed: from_top + 1,
                available: frame.operand_height(),
            },
        )?;
        let value = self
            .function(frame.function())?
            .stack_map()
            .entries()
            .get(instruction.get() as usize)
            .filter(|entry| entry.instruction() == instruction)
            .and_then(|entry| entry.stack_before().get(position))
            .ok_or(VmError::OperandStackShapeMismatch {
                function: frame.function(),
                expected: position + 1,
                actual: frame.operand_height(),
            })?;
        Ok((value.ty(), value.plan().clone()))
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
        let slot_plans = self
            .function(frame.function())?
            .frame()
            .slot_plans()
            .to_vec();
        let slot_count = slot_plans.len();
        for ordinal in 0..slot_count {
            let index = frame.slot_base() + ordinal;
            if self.live_values.get(index).copied() == Some(true) {
                let value = self.values[index];
                let slot = FrameSlotIndex::new(u32::try_from(ordinal).map_err(|_| {
                    VmError::VerifiedEntryInvariant {
                        invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                    }
                })?);
                if self.retained_slots.contains(&slot) {
                    self.clear_value(index);
                    continue;
                }
                let plan = slot_plans[ordinal].clone();
                if !is_discardable_root(&value) {
                    executor.release(&value, &plan).map_err(|error| {
                        error.into_vm_error(frame.function(), frame.instruction(), opcode)
                    })?;
                }
                self.clear_value(index);
            }
        }
        for position in 0..frame.operand_height() {
            let index = frame.operand_base() + position;
            if self.live_values.get(index).copied() == Some(true) {
                let plan =
                    self.stack_map_operand_plan(frame.function(), frame.instruction(), position)?;
                let value = self.values[index];
                if !is_discardable_root(&value) {
                    executor.release(&value, &plan).map_err(|error| {
                        error.into_vm_error(frame.function(), frame.instruction(), opcode)
                    })?;
                }
                self.clear_value(index);
            }
        }
        // Caught envelopes whose catch slot lives in this frame die with the
        // frame. Their retained payload authority is released exactly once;
        // a rethrow has already moved the envelope out of this map.
        let range = frame.slot_base()..frame.slot_base().saturating_add(slot_count);
        let caught_indices: Vec<usize> = self
            .caught_exceptions
            .range(range.clone())
            .map(|(index, _)| *index)
            .collect();
        for index in caught_indices {
            let entry = self
                .caught_exceptions
                .get(&index)
                .cloned()
                .expect("caught index came from the same map");
            if let Some(slot) = entry.envelope.vm_local_slot() {
                if !is_discardable_root(&slot) {
                    executor.release(&slot, &entry.plan).map_err(|error| {
                        error.into_vm_error(frame.function(), frame.instruction(), opcode)
                    })?;
                }
            }
            self.caught_exceptions.remove(&index);
            self.caught_by_payload.remove(&entry.payload_handle);
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

    fn borrow_operands(&self, count: usize) -> Result<(VmFrame, usize, Vec<ValueSlot>), VmError> {
        let frame = self.current_frame()?.clone();
        let function = frame.function();
        if frame.operand_height() < count {
            return Err(VmError::OperandStackUnderflow {
                function,
                needed: count,
                available: frame.operand_height(),
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
        Ok((frame, start, values))
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

    fn reserve_operand_push(&self) -> Result<OperandPushReservation, VmError> {
        let frame_ordinal = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        let frame = self
            .frames
            .get(frame_ordinal)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        let function = frame.function();
        if frame.operand_height() >= frame.operand_capacity() {
            return Err(VmError::OperandStackOverflow {
                function,
                capacity: frame.operand_capacity(),
            });
        }
        let next_height =
            frame
                .operand_height()
                .checked_add(1)
                .ok_or(VmError::OperandStackOverflow {
                    function,
                    capacity: frame.operand_capacity(),
                })?;
        let value_index = frame
            .operand_base()
            .checked_add(frame.operand_height())
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        let destination_live =
            self.live_values
                .get(value_index)
                .copied()
                .ok_or(VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
        self.values
            .get(value_index)
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        if destination_live {
            return Err(VmError::LiveDestination {
                function: frame.function(),
                instruction: frame.instruction(),
                location: VmValueLocation::Operand(frame.operand_height()),
            });
        }
        Ok(OperandPushReservation {
            frame_ordinal,
            value_index,
            next_height,
        })
    }

    fn commit_operand_push(&mut self, reservation: OperandPushReservation, value: ValueSlot) {
        // `reserve_operand_push` checked all three indices and there is no
        // intervening fiber mutation at any call site. Commit therefore has
        // no fallible tail after the owner becomes live.
        self.frames[reservation.frame_ordinal].set_operand_height(reservation.next_height);
        self.values[reservation.value_index] = value;
        self.live_values[reservation.value_index] = true;
    }

    fn push_operand(&mut self, value: ValueSlot) -> Result<(), VmError> {
        let reservation = self.reserve_operand_push()?;
        self.commit_operand_push(reservation, value);
        Ok(())
    }

    fn reserve_operand_window_replacement(
        &self,
        count: usize,
    ) -> Result<(OperandWindowReplacementReservation, Vec<ValueSlot>), VmError> {
        let (frame, start, values) = self.borrow_operands(count)?;
        let frame_ordinal = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        let remaining_height =
            frame
                .operand_height()
                .checked_sub(count)
                .ok_or(VmError::OperandStackUnderflow {
                    function: frame.function(),
                    needed: count,
                    available: frame.operand_height(),
                })?;
        let next_height = remaining_height
            .checked_add(1)
            .ok_or(VmError::OperandStackOverflow {
                function: frame.function(),
                capacity: frame.operand_capacity(),
            })?;
        if next_height > frame.operand_capacity() {
            return Err(VmError::OperandStackOverflow {
                function: frame.function(),
                capacity: frame.operand_capacity(),
            });
        }
        let end = start
            .checked_add(count)
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        if end > self.values.len() || end > self.live_values.len() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        if count == 0 {
            let destination_live =
                self.live_values
                    .get(start)
                    .copied()
                    .ok_or(VmError::VerifiedEntryInvariant {
                        invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                    })?;
            self.values
                .get(start)
                .ok_or(VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::FrameLayoutOverflow,
                })?;
            if destination_live {
                return Err(VmError::LiveDestination {
                    function: frame.function(),
                    instruction: frame.instruction(),
                    location: VmValueLocation::Operand(frame.operand_height()),
                });
            }
        }
        Ok((
            OperandWindowReplacementReservation {
                frame_ordinal,
                start,
                end,
                next_height,
            },
            values,
        ))
    }

    fn commit_operand_window_replacement(
        &mut self,
        reservation: OperandWindowReplacementReservation,
        value: ValueSlot,
    ) {
        // Reservation validated every index. Allocation has adopted the
        // window's owners, so commit clears their old storage and installs the
        // sole record owner without any fallible tail.
        for index in reservation.start..reservation.end {
            self.values[index] = ValueSlot::null();
            self.live_values[index] = false;
        }
        self.frames[reservation.frame_ordinal].set_operand_height(reservation.next_height);
        self.values[reservation.start] = value;
        self.live_values[reservation.start] = true;
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
        for owner in &self.terminal_escrow {
            visitor.visit_root(&owner.value)?;
        }
        if let Some(unwind) = &self.unwind {
            if let Some(slot) = unwind.envelope.vm_local_slot() {
                visitor.visit_root(&slot)?;
            }
        }
        for caught in self.caught_exceptions.values() {
            if let Some(slot) = caught.envelope.vm_local_slot() {
                visitor.visit_root(&slot)?;
            }
        }
        if let Some(terminal) = &self.terminal_handoff {
            terminal.visit_roots(visitor)?;
        }
        Ok(())
    }
}

enum SegmentResult {
    Continue,
    Complete(VmOwnedValues),
    Throw(VmOwnedException),
    Handoff(VmControl),
}

enum DispatchOutcome {
    Continue,
    Complete(VmOwnedValues),
    Throw(VmOwnedException),
    Handoff(VmControl),
}

fn pending_matches(pending: &PendingResume, token: &VmResumeToken) -> bool {
    Arc::ptr_eq(&pending.binding, token.binding())
}

fn compact_type_tag(
    function: FunctionIndex,
    instruction: InstructionIndex,
    type_index: TypeIndex,
) -> Result<CompactTypeTag, VmError> {
    CompactTypeTag::try_from_type_index(type_index.get()).ok_or(VmError::CompactTypeTagOutOfRange {
        function,
        instruction,
        type_index,
    })
}

fn compact_record_type_tags(
    function: FunctionIndex,
    instruction: InstructionIndex,
    nominal_type: TypeIndex,
    field_types: impl IntoIterator<Item = TypeIndex>,
) -> Result<(CompactTypeTag, Vec<CompactTypeTag>), VmError> {
    let record_tag = compact_type_tag(function, instruction, nominal_type)?;
    let field_tags = field_types
        .into_iter()
        .map(|field_type| compact_type_tag(function, instruction, field_type))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((record_tag, field_tags))
}

fn materialize_new_record_const_string(
    executor: &mut LifecycleExecutor<'_>,
    value: String,
    field_tag: CompactTypeTag,
) -> Result<ValueSlot, VmError> {
    executor
        .heap()
        .alloc_typed_string(value, field_tag, ValueFlags::new(0))
        .map_err(VmError::Heap)
}

fn nominal_type_index(value: &ValueSlot) -> Option<TypeIndex> {
    match value.kind() {
        Some(
            ValueKind::RequestHeapRef
            | ValueKind::ActorStateRef
            | ValueKind::ConstRef
            | ValueKind::ResourceRef
            | ValueKind::CallbackClosureRef,
        ) => value
            .compact_type_tag()
            .map(CompactTypeTag::type_index)
            .map(TypeIndex::new),
        _ => None,
    }
}

fn interface_carrier_type(
    image: &DeploymentExecutionImage,
    table: &LinkedInterfaceTable,
) -> Option<TypeIndex> {
    image.types().iter().find_map(|row| {
        let TypeRefIr::AnyInterface { interface } = row.type_ref() else {
            return None;
        };
        (interface == table.interface().artifact()).then_some(row.index())
    })
}

#[allow(clippy::too_many_arguments)]
fn allocate_store_string_constant(
    heap: &mut dyn VmHeap,
    value: String,
    operand_type_tag: CompactTypeTag,
    constant_type: &TypeRefIr,
    operand_type: &TypeRefIr,
    operand_plan: &LinkedValueTransferPlan,
    destination_type: &TypeRefIr,
    destination_plan: &LinkedValueTransferPlan,
) -> Result<Option<ValueSlot>, VmHeapError> {
    if !store_slot_string_constant_authorized(
        constant_type,
        &value,
        operand_type,
        operand_plan,
        destination_type,
        destination_plan,
    ) {
        return Ok(None);
    }
    heap.alloc_typed_string(value, operand_type_tag, ValueFlags::new(0))
        .map(Some)
}

enum IntrinsicResultPayload {
    EmptyArray,
    EmptyMap,
    String(String),
    Bytes(Vec<u8>),
    Number(f64),
}

fn materialize_intrinsic_result(
    heap: &mut dyn VmHeap,
    payload: IntrinsicResultPayload,
    result_type_tag: CompactTypeTag,
) -> Result<ValueSlot, VmError> {
    match payload {
        IntrinsicResultPayload::EmptyArray => heap
            .allocate_array(&[], result_type_tag, ValueFlags::new(0))
            .map_err(VmError::Heap),
        IntrinsicResultPayload::EmptyMap => heap
            .allocate_map(&[], result_type_tag, ValueFlags::new(0))
            .map_err(VmError::Heap),
        IntrinsicResultPayload::String(value) => heap
            .alloc_typed_string(value, result_type_tag, ValueFlags::new(0))
            .map_err(VmError::Heap),
        IntrinsicResultPayload::Bytes(value) => heap
            .alloc_typed_bytes(value, result_type_tag, ValueFlags::new(0))
            .map_err(VmError::Heap),
        IntrinsicResultPayload::Number(value) => Ok(ValueSlot::number(value)),
    }
}

fn store_slot_string_constant_authorized(
    constant_type: &TypeRefIr,
    constant_value: &str,
    operand_type: &TypeRefIr,
    operand_plan: &LinkedValueTransferPlan,
    destination_type: &TypeRefIr,
    destination_plan: &LinkedValueTransferPlan,
) -> bool {
    let exact_string = |ty: &TypeRefIr| {
        matches!(
            ty,
            TypeRefIr::Builtin { name, args } if name == "string" && args.is_empty()
        )
    };
    let exact_constant = matches!(
        constant_type,
        TypeRefIr::Literal {
            value: LiteralIr::String { value },
        } if value == constant_value
    ) || exact_string(constant_type);
    let exact_destination = exact_string(destination_type)
        || matches!(
            destination_type,
            TypeRefIr::Literal {
                value: LiteralIr::String { value },
            } if value == constant_value
        );
    let exact_owned_plan = |plan: &LinkedValueTransferPlan| {
        matches!(
            plan,
            LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::SnapshotRelease,
            }
        )
    };
    exact_constant
        && exact_string(operand_type)
        && exact_destination
        && exact_owned_plan(operand_plan)
        && operand_plan == destination_plan
}

/// The actual concrete leaf identity of one runtime value, read from the
/// value's own runtime type tag plus the immutable linked type facts. This is
/// deliberately not the throw instruction's static operand type: two values
/// flowing through the same union-typed site carry different tags and yield
/// different identities.
pub(crate) fn runtime_leaf_catch_identity(
    image: &DeploymentExecutionImage,
    value: &ValueSlot,
) -> Option<CatchIdentity> {
    match value.kind()? {
        ValueKind::RequestHeapRef
        | ValueKind::ActorStateRef
        | ValueKind::ConstRef
        | ValueKind::ResourceRef
        | ValueKind::CallbackClosureRef => linked_type_catch_identity(
            image,
            TypeIndex::new(value.compact_type_tag()?.type_index()),
        ),
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
            Some(TypeIndex::new(slot.compact_type_tag()?.type_index()))
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
            Some(CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
                LocalExecutionTypeIdentity {
                    addr: TypeAddr {
                        unit: UnitAddr::Package(package_slot),
                        file: FileAddr::FileIrIdentity(package_id.clone()),
                        type_index: leaf.get() as usize,
                    },
                    type_arguments: Vec::new(),
                },
            )))
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

fn linked_plan_matches_native(
    linked: &LinkedValueTransferPlan,
    native: &NativeValueLifecycleConcrete,
) -> bool {
    match (linked, native) {
        (
            LinkedValueTransferPlan::SnapshotShare { drop: linked },
            NativeValueLifecycleConcrete::SnapshotShare { drop: native },
        )
        | (
            LinkedValueTransferPlan::MoveOnly { drop: linked },
            NativeValueLifecycleConcrete::MoveOnly { drop: native },
        ) => match (linked, native) {
            (LinkedValueDropPlan::Trivial, NativeValueDropPlan::Trivial)
            | (LinkedValueDropPlan::SnapshotRelease, NativeValueDropPlan::SnapshotRelease) => true,
            (
                LinkedValueDropPlan::NativeAdapter { adapter: linked },
                NativeValueDropPlan::NativeAdapter { adapter: native },
            ) => linked == native,
            _ => false,
        },
        (
            LinkedValueTransferPlan::AffineResource { drop: linked },
            NativeValueLifecycleConcrete::AffineResource { drop: native },
        ) => match (linked, native) {
            (
                LinkedResourceDropPlan::ResourceTableRelease,
                NativeResourceDropPlan::ResourceTableRelease,
            ) => true,
            (
                LinkedResourceDropPlan::NativeAdapter { adapter: linked },
                NativeResourceDropPlan::NativeAdapter { adapter: native },
            ) => linked == native,
            _ => false,
        },
        (
            LinkedValueTransferPlan::ExplicitCloneLease {
                clone_adapter: linked_clone,
                drop: linked_drop,
            },
            NativeValueLifecycleConcrete::ExplicitCloneLease {
                clone_adapter: native_clone,
                drop: native_drop,
            },
        ) => {
            linked_clone == native_clone
                && match (linked_drop, native_drop) {
                    (
                        LinkedResourceDropPlan::ResourceTableRelease,
                        NativeResourceDropPlan::ResourceTableRelease,
                    ) => true,
                    (
                        LinkedResourceDropPlan::NativeAdapter { adapter: linked },
                        NativeResourceDropPlan::NativeAdapter { adapter: native },
                    ) => linked == native,
                    _ => false,
                }
        }
        _ => false,
    }
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
            | Opcode::TakeDenseField
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
