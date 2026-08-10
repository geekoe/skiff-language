mod entry_admission;
#[cfg(test)]
mod tests;

use skiff_artifact_model::{descriptor_for_opcode, Opcode};
use skiff_runtime_bytecode_verifier::{VerifiedCodeEntry, VerifiedLinkedBytecodeImage};
use skiff_runtime_deployment_image::{DeploymentOwnerIdentity, PinnedDeploymentEntry};
use skiff_runtime_linked_bytecode::{FunctionIndex, LinkedFunction};
use skiff_runtime_model::{
    vm_heap::{VmHeap, VmHeapError},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::ValueSlot,
};

use crate::{
    admission::{is_self_describing_immediate, validate_entry_arguments},
    fiber::entry_admission::validate_entry_contract,
    frame::VmFrame,
    statement::{charge_frame_entry, charge_instruction_events},
    ResumeOutcome, VmBudget, VmControl, VmError, VmLimits, VmResumeToken, VmVerifiedInvariant,
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
        let frame = VmFrame::root(function_index, operand_base, operand_capacity);
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
            self.charge_statement_events(budget)?;
            self.dispatch_one()?;
        }
        Ok(())
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

    fn dispatch_one(&mut self) -> Result<(), VmError> {
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

        match opcode_execution_class(instruction.opcode()) {
            VmOpcodeExecutionClass::BudgetCheckpoint => {
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
    let function = program.functions().get(index.get() as usize)?;
    (function.index() == index).then_some(function)
}
