use skiff_artifact_model::{descriptor_for_opcode, Opcode, ParamModeIr};
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
    admission::{is_self_describing_immediate, validate_entry_arguments},
    frame::VmFrame,
    ResumeOutcome, VmBudget, VmControl, VmError, VmLimits, VmResumeToken, VmSemanticCharge,
    VmSemanticChargeKind, VmVerifiedInvariant,
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
        for (argument, parameter_slot) in arguments
            .into_vec()
            .into_iter()
            .zip(function.frame().parameter_slots().iter().copied())
        {
            let index = usize::try_from(parameter_slot.get()).map_err(|_| {
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

    /// Runs a bounded synchronous segment. External work is never performed
    /// through a callback/trait port; implemented effect opcodes will return a
    /// typed [`VmControl`] handoff to the scheduler.
    pub fn run_segment(&mut self, _heap: &mut dyn VmHeap, budget: &mut dyn VmBudget) -> VmControl {
        if self.state != VmFiberState::Runnable {
            return VmControl::Complete(Err(VmError::FiberNotRunnable { state: self.state }));
        }

        match self.run_segment_inner(budget) {
            Ok(()) => VmControl::Continue,
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

    /// Explicitly clears self-describing immediates still owned by an
    /// error-terminal fiber.
    ///
    /// `run_segment` never performs this cleanup implicitly: the fiber remains
    /// a root source after failure. A reference value remains owned and causes
    /// a structured failure until its full linked lifecycle plan is available.
    pub fn discard_terminal_roots(&mut self, _heap: &mut dyn VmHeap) -> Result<(), VmError> {
        if self.state != VmFiberState::Terminal {
            return Err(VmError::DiscardRequiresTerminal { state: self.state });
        }

        for (index, (value, live)) in self.values.iter().zip(&self.live_values).enumerate() {
            if *live && !is_self_describing_immediate(value) {
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

    fn run_segment_inner(&mut self, budget: &mut dyn VmBudget) -> Result<(), VmError> {
        for _ in 0..self.limits.max_segment_instructions().get() {
            self.charge_function_entry(budget)?;
            self.consume_raw_fuel(budget)?;
            self.charge_statement_entries(budget)?;
            self.dispatch_one(budget)?;
        }
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

    fn dispatch_one(&mut self, budget: &mut dyn VmBudget) -> Result<(), VmError> {
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

        match opcode_execution_class(instruction.opcode()) {
            VmOpcodeExecutionClass::BudgetCheckpoint => {
                budget.poll_interrupt()?;
                budget.charge_semantic(VmSemanticCharge::new(
                    function_index,
                    instruction_index,
                    VmSemanticChargeKind::BudgetCheckpoint,
                ))?;
                self.advance_current_instruction()?;
            }
            VmOpcodeExecutionClass::RequiresFullValueLifecyclePlan => {
                return Err(VmError::FullValueLifecyclePlanUnavailable {
                    function: function_index,
                    instruction: instruction_index,
                    opcode: instruction.opcode(),
                });
            }
            VmOpcodeExecutionClass::Unsupported => {
                return Err(VmError::UnsupportedOpcode {
                    function: function_index,
                    instruction: instruction_index,
                    opcode: instruction.opcode(),
                });
            }
        }
        Ok(())
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
    if signature.parameter_types().len() != signature.parameter_modes().len() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::EntryParameterCount,
        });
    }
    if signature.parameter_types().len() != signature.parameter_plans().len() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ParameterTransferPlan,
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
    }
    if frame.result_types() != signature.result_types() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ResultType,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VmOpcodeExecutionClass {
    BudgetCheckpoint,
    RequiresFullValueLifecyclePlan,
    Unsupported,
}

fn opcode_execution_class(opcode: Opcode) -> VmOpcodeExecutionClass {
    match opcode {
        Opcode::BudgetCheckpoint => VmOpcodeExecutionClass::BudgetCheckpoint,
        Opcode::Const
        | Opcode::CopySlot
        | Opcode::MoveSlot
        | Opcode::StoreSlot
        | Opcode::Drop
        | Opcode::Dup
        | Opcode::LoadSlot
        | Opcode::TakeSlot
        | Opcode::Pop
        | Opcode::Return => VmOpcodeExecutionClass::RequiresFullValueLifecyclePlan,
        _ => VmOpcodeExecutionClass::Unsupported,
    }
}

fn verified_function(
    program: &VerifiedLinkedBytecodeImage,
    index: FunctionIndex,
) -> Option<&LinkedFunction> {
    let function = program.functions().get(index_to_usize(index))?;
    (function.index() == index).then_some(function)
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
    use skiff_artifact_model::Opcode;
    use skiff_runtime_model::vm_value::ValueSlot;

    use super::{opcode_execution_class, VerifiedVmEntry, Vm, VmFiber, VmOpcodeExecutionClass};
    use crate::{VmBudget, VmError, VmLimits};

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

    #[test]
    fn opcode_dispatch_has_no_heap_port_without_full_lifecycle_plans() {
        let dispatch: fn(&mut VmFiber, &mut dyn VmBudget) -> Result<(), VmError> =
            VmFiber::dispatch_one;

        let _ = dispatch;
    }

    #[test]
    fn every_previous_plan_kind_execution_path_is_fail_closed() {
        for opcode in [
            Opcode::Const,
            Opcode::CopySlot,
            Opcode::MoveSlot,
            Opcode::StoreSlot,
            Opcode::Drop,
            Opcode::Dup,
            Opcode::LoadSlot,
            Opcode::TakeSlot,
            Opcode::Pop,
            Opcode::Return,
        ] {
            assert_eq!(
                opcode_execution_class(opcode),
                VmOpcodeExecutionClass::RequiresFullValueLifecyclePlan
            );
        }
    }
}
