use serde::{Serialize, Serializer};

use super::*;

/// Complete immutable semantics of one persisted instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpcodeContract {
    pub kind: Opcode,
    pub opcode: u8,
    pub mnemonic: &'static str,
    pub operands: &'static [OperandSpec],
    pub typed: TypedTransition,
    pub control: ControlContract,
    pub pending: PendingContract,
    pub checkpoint: CheckpointContract,
    pub exception: ExceptionContract,
    pub statement: StatementContract,
    pub source: SourceContract,
    pub region: RegionContract,
    pub capabilities: &'static [CapabilityRequirement],
}

impl OpcodeContract {
    pub const fn operand_word_count(&self) -> u32 {
        self.operands.len() as u32
    }

    pub const fn instruction_word_count(&self) -> u32 {
        self.operand_word_count() + 1
    }

    pub fn operand(&self, role: OperandRole) -> Option<&'static OperandSpec> {
        self.operands.iter().find(|operand| operand.role == role)
    }

    pub fn operand_position(&self, role: OperandRole) -> Option<usize> {
        self.operands
            .iter()
            .position(|operand| operand.role == role)
    }

    pub fn operand_word(&self, role: OperandRole, operand_words: &[u32]) -> Option<u32> {
        self.operand_position(role)
            .and_then(|position| operand_words.get(position).copied())
    }
}

impl std::fmt::Display for OpcodeContract {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} (0x{:02x})", self.mnemonic, self.opcode)
    }
}

macro_rules! opcode_rows {
    ($emit:ident) => {
        $emit! {
            {
                kind: Const, opcode: 0x00, mnemonic: "const",
                operands: [(OperandKind::Pool, OperandRole::ConstantRef, LinkedOperandKind::Constant, [])],
                stack_in: [],
                stack_out: [(Arity::Fixed(1), ValueSource::Constant { operand: OperandRole::ConstantRef })],
                slots: SlotContract::None,
                control: ControlContract::Fallthrough,
                pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: []
            },
            {
                kind: CopySlot, opcode: 0x01, mnemonic: "copy_slot",
                operands: [
                    (OperandKind::Slot, OperandRole::SourceSlot, LinkedOperandKind::FrameSlot, []),
                    (OperandKind::Slot, OperandRole::DestinationSlot, LinkedOperandKind::FrameSlot, [])
                ],
                stack_in: [], stack_out: [],
                slots: SlotContract::Effects(&[
                    SlotEffectContract::new(OperandRole::SourceSlot, SlotAction::ReadShare, ValueSource::Slot { operand: OperandRole::SourceSlot }),
                    SlotEffectContract::new(OperandRole::DestinationSlot, SlotAction::Write, ValueSource::Slot { operand: OperandRole::SourceSlot })
                ]),
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::ShareableValue]
            },
            {
                kind: MoveSlot, opcode: 0x02, mnemonic: "move_slot",
                operands: [
                    (OperandKind::Slot, OperandRole::SourceSlot, LinkedOperandKind::FrameSlot, []),
                    (OperandKind::Slot, OperandRole::DestinationSlot, LinkedOperandKind::FrameSlot, [])
                ],
                stack_in: [], stack_out: [],
                slots: SlotContract::Effects(&[
                    SlotEffectContract::new(OperandRole::SourceSlot, SlotAction::Take, ValueSource::Slot { operand: OperandRole::SourceSlot }),
                    SlotEffectContract::new(OperandRole::DestinationSlot, SlotAction::Write, ValueSource::Slot { operand: OperandRole::SourceSlot })
                ]),
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::MovableValue]
            },
            {
                kind: StoreSlot, opcode: 0x03, mnemonic: "store_slot",
                operands: [(OperandKind::Slot, OperandRole::DestinationSlot, LinkedOperandKind::FrameSlot, [])],
                stack_in: [(Arity::Fixed(1), ValueSource::AnyStackValue)], stack_out: [],
                slots: SlotContract::Effects(&[
                    SlotEffectContract::new(OperandRole::DestinationSlot, SlotAction::Write, ValueSource::StackInput { group: 0 })
                ]),
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::MovableValue]
            },
            {
                kind: Drop, opcode: 0x04, mnemonic: "drop",
                operands: [(OperandKind::Slot, OperandRole::Slot, LinkedOperandKind::FrameSlot, [])],
                stack_in: [], stack_out: [],
                slots: SlotContract::Effects(&[
                    SlotEffectContract::new(OperandRole::Slot, SlotAction::Drop, ValueSource::Slot { operand: OperandRole::Slot })
                ]),
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::DroppableValue]
            },
            {
                kind: Dup, opcode: 0x05, mnemonic: "dup",
                operands: [],
                stack_in: [(Arity::Fixed(1), ValueSource::AnyStackValue)],
                stack_out: [(Arity::Fixed(2), ValueSource::StackInput { group: 0 })],
                slots: SlotContract::None,
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::ShareableValue]
            },
            {
                kind: LoadSlot, opcode: 0x06, mnemonic: "load_slot",
                operands: [(OperandKind::Slot, OperandRole::SourceSlot, LinkedOperandKind::FrameSlot, [])],
                stack_in: [],
                stack_out: [(Arity::Fixed(1), ValueSource::Slot { operand: OperandRole::SourceSlot })],
                slots: SlotContract::Effects(&[
                    SlotEffectContract::new(OperandRole::SourceSlot, SlotAction::ReadShare, ValueSource::Slot { operand: OperandRole::SourceSlot })
                ]),
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::ShareableValue]
            },
            {
                kind: TakeSlot, opcode: 0x07, mnemonic: "take_slot",
                operands: [(OperandKind::Slot, OperandRole::SourceSlot, LinkedOperandKind::FrameSlot, [])],
                stack_in: [],
                stack_out: [(Arity::Fixed(1), ValueSource::Slot { operand: OperandRole::SourceSlot })],
                slots: SlotContract::Effects(&[
                    SlotEffectContract::new(OperandRole::SourceSlot, SlotAction::Take, ValueSource::Slot { operand: OperandRole::SourceSlot })
                ]),
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::MovableValue]
            },
            {
                kind: Pop, opcode: 0x08, mnemonic: "pop", operands: [],
                stack_in: [(Arity::Fixed(1), ValueSource::AnyStackValue)], stack_out: [],
                slots: SlotContract::None,
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::DroppableValue]
            },
            {
                kind: Jump, opcode: 0x10, mnemonic: "jump",
                operands: [(OperandKind::Branch, OperandRole::BranchTarget, LinkedOperandKind::Instruction, [])],
                stack_in: [], stack_out: [], slots: SlotContract::None,
                control: ControlContract::Jump { target: OperandRole::BranchTarget },
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: []
            },
            {
                kind: JumpIfTrue, opcode: 0x11, mnemonic: "jump_if_true",
                operands: [(OperandKind::Branch, OperandRole::BranchTarget, LinkedOperandKind::Instruction, [])],
                stack_in: [(Arity::Fixed(1), ValueSource::Bool)], stack_out: [], slots: SlotContract::None,
                control: ControlContract::Branch { target: OperandRole::BranchTarget, when: BranchWhen::True },
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: []
            },
            {
                kind: JumpIfFalse, opcode: 0x12, mnemonic: "jump_if_false",
                operands: [(OperandKind::Branch, OperandRole::BranchTarget, LinkedOperandKind::Instruction, [])],
                stack_in: [(Arity::Fixed(1), ValueSource::Bool)], stack_out: [], slots: SlotContract::None,
                control: ControlContract::Branch { target: OperandRole::BranchTarget, when: BranchWhen::False },
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: []
            },
            {
                kind: SwitchTag, opcode: 0x13, mnemonic: "switch_tag",
                operands: [(OperandKind::Table, OperandRole::SwitchTable, LinkedOperandKind::SwitchTable, [])],
                stack_in: [(Arity::Fixed(1), ValueSource::TaggedValue)], stack_out: [], slots: SlotContract::None,
                control: ControlContract::Switch { table: OperandRole::SwitchTable },
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::NominalTag]
            },
            {
                kind: BudgetCheckpoint, opcode: 0x14, mnemonic: "budget_checkpoint", operands: [],
                stack_in: [], stack_out: [], slots: SlotContract::None,
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::Budget {
                    budget_stop: FailureDisposition::UncatchableTerminal,
                    timeout_attribution: TimeoutAttribution::ActiveRegionSite
                },
                exception: ExceptionContract::new(ExceptionBehavior::None, &[
                    FailureContract::terminal(FailureKind::BudgetStop, FailureTrigger::BudgetExceeded)
                ]),
                statement: StatementContract::RequiredEvent {
                    charge_kind: StatementChargeKind::LoopCheck,
                    attribution: crate::StatementAttributionClass::Generated,
                },
                source: SourceContract::Required {
                    use_kind: SourceUse::GeneratedFailure,
                    origin: SourceOriginConstraint::SyntheticOnly
                },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind),
                capabilities: []
            },
            {
                kind: Trap, opcode: 0x15, mnemonic: "trap",
                operands: [(OperandKind::Immediate, OperandRole::FailureKind, LinkedOperandKind::Immediate, [])],
                stack_in: [(Arity::Fixed(1), ValueSource::Bool)], stack_out: [], slots: SlotContract::None,
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[
                    FailureContract::terminal(FailureKind::Assertion, FailureTrigger::AssertionFalse)
                ]),
                statement: StatementContract::None, source: SourceContract::Required {
                    use_kind: SourceUse::Assertion,
                    origin: SourceOriginConstraint::SourceOrSynthetic
                },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind),
                capabilities: []
            },
            {
                kind: CallLocal, opcode: 0x20, mnemonic: "call_local",
                operands: [
                    (OperandKind::Reloc, OperandRole::LocalTarget, LinkedOperandKind::Function, [RelocationKind::LocalExecutableRef, RelocationKind::PackageCallableRef]),
                    (OperandKind::Immediate, OperandRole::ArgCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Immediate, OperandRole::ResultCount, LinkedOperandKind::Immediate, [])
                ],
                stack_in: [(Arity::Declared(OperandRole::ArgCount), ValueSource::TargetParameters { target: OperandRole::LocalTarget })],
                stack_out: [(Arity::Declared(OperandRole::ResultCount), ValueSource::TargetResults { target: OperandRole::LocalTarget })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::TransitiveTarget { target: OperandRole::LocalTarget },
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::PropagateTarget { target: OperandRole::LocalTarget }, &[]),
                statement: StatementContract::RequiredEvent {
                    charge_kind: StatementChargeKind::LocalCall,
                    attribution: crate::StatementAttributionClass::Expression,
                },
                source: SourceContract::Required { use_kind: SourceUse::CallSite, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind),
                capabilities: [CapabilityRequirement::ExactLocalTarget]
            },
            {
                kind: TailCallLocal, opcode: 0x21, mnemonic: "tail_call_local",
                operands: [
                    (OperandKind::Reloc, OperandRole::LocalTarget, LinkedOperandKind::Function, [RelocationKind::LocalExecutableRef, RelocationKind::PackageCallableRef]),
                    (OperandKind::Immediate, OperandRole::ArgCount, LinkedOperandKind::Immediate, [])
                ],
                stack_in: [(Arity::Declared(OperandRole::ArgCount), ValueSource::TargetParameters { target: OperandRole::LocalTarget })],
                stack_out: [], slots: SlotContract::None,
                control: ControlContract::TailCall,
                pending: PendingContract::TransitiveTarget { target: OperandRole::LocalTarget },
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::PropagateTarget { target: OperandRole::LocalTarget }, &[]),
                statement: StatementContract::RequiredEvent {
                    charge_kind: StatementChargeKind::TailHop,
                    attribution: crate::StatementAttributionClass::Expression,
                },
                source: SourceContract::Required { use_kind: SourceUse::CallSite, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::TailReplace, RegionEffect::Unwind),
                capabilities: [CapabilityRequirement::ExactLocalTarget, CapabilityRequirement::TailEligible]
            },
            {
                kind: CallService, opcode: 0x22, mnemonic: "call_service",
                operands: [
                    (OperandKind::Reloc, OperandRole::ServiceTarget, LinkedOperandKind::ServiceOperation, [RelocationKind::ServiceOperationRef]),
                    (OperandKind::Immediate, OperandRole::ArgCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Immediate, OperandRole::ResultCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Pool, OperandRole::ResumeRef, LinkedOperandKind::ResumeSite, [])
                ],
                stack_in: [(Arity::Declared(OperandRole::ArgCount), ValueSource::TargetParameters { target: OperandRole::ServiceTarget })],
                stack_out: [(Arity::Declared(OperandRole::ResultCount), ValueSource::TargetResults { target: OperandRole::ServiceTarget })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::ActualWithResume { resume: OperandRole::ResumeRef, mode: PendingMode::ServiceBoundary },
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::RaiseAtCurrentSite, &[]),
                statement: StatementContract::None,
                source: SourceContract::Required { use_kind: SourceUse::CallSite, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind),
                capabilities: []
            },
            {
                kind: CallActor, opcode: 0x23, mnemonic: "call_actor",
                operands: [
                    (OperandKind::Reloc, OperandRole::ActorTarget, LinkedOperandKind::ActorMethod, [RelocationKind::ActorMethodRef]),
                    (OperandKind::Immediate, OperandRole::ArgCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Immediate, OperandRole::ResultCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Pool, OperandRole::ResumeRef, LinkedOperandKind::ResumeSite, [])
                ],
                stack_in: [(Arity::Declared(OperandRole::ArgCount), ValueSource::TargetParameters { target: OperandRole::ActorTarget })],
                stack_out: [(Arity::Declared(OperandRole::ResultCount), ValueSource::TargetResults { target: OperandRole::ActorTarget })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::ActualWithResume { resume: OperandRole::ResumeRef, mode: PendingMode::ActorBoundary },
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::RaiseAtCurrentSite, &[]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::CallSite, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind), capabilities: []
            },
            {
                kind: CallInterface, opcode: 0x24, mnemonic: "call_interface",
                operands: [
                    (OperandKind::Reloc, OperandRole::InterfaceTarget, LinkedOperandKind::InterfaceTable, [RelocationKind::InterfaceRequirementRef]),
                    (OperandKind::Immediate, OperandRole::MethodOrdinal, LinkedOperandKind::Immediate, []),
                    (OperandKind::Immediate, OperandRole::ArgCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Immediate, OperandRole::ResultCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Pool, OperandRole::ResumeRef, LinkedOperandKind::ResumeSite, [])
                ],
                stack_in: [
                    (Arity::Fixed(1), ValueSource::InterfaceCarrier { interface: OperandRole::InterfaceTarget }),
                    (Arity::Declared(OperandRole::ArgCount), ValueSource::TargetParameters { target: OperandRole::InterfaceTarget })
                ],
                stack_out: [(Arity::Declared(OperandRole::ResultCount), ValueSource::TargetResults { target: OperandRole::InterfaceTarget })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::ActualWithResume { resume: OperandRole::ResumeRef, mode: PendingMode::InterfaceBoundary },
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::RaiseAtCurrentSite, &[]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::CallSite, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind), capabilities: []
            },
            {
                kind: Return, opcode: 0x25, mnemonic: "return", operands: [],
                stack_in: [(Arity::FunctionResultCount, ValueSource::FunctionResults)], stack_out: [],
                slots: SlotContract::None, control: ControlContract::Return,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::ExitFunction, RegionEffect::NotApplicable), capabilities: []
            },
            {
                kind: CallLocalInOut, opcode: 0x26, mnemonic: "call_local_inout",
                operands: [
                    (OperandKind::Reloc, OperandRole::LocalTarget, LinkedOperandKind::Function, [RelocationKind::LocalExecutableRef, RelocationKind::PackageCallableRef]),
                    (OperandKind::Immediate, OperandRole::InputCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Immediate, OperandRole::ResultCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Table, OperandRole::CallLoanLayout, LinkedOperandKind::CallLoanLayout, [])
                ],
                stack_in: [(Arity::Declared(OperandRole::InputCount), ValueSource::InOutCallInputs { target: OperandRole::LocalTarget, layout: OperandRole::CallLoanLayout })],
                stack_out: [(Arity::Declared(OperandRole::ResultCount), ValueSource::TargetResults { target: OperandRole::LocalTarget })],
                slots: SlotContract::InOutCallLoans { target: OperandRole::LocalTarget, layout: OperandRole::CallLoanLayout },
                control: ControlContract::Fallthrough,
                pending: PendingContract::NoPendingTarget { target: OperandRole::LocalTarget, loan_layout: OperandRole::CallLoanLayout },
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::PropagateTarget { target: OperandRole::LocalTarget }, &[]),
                statement: StatementContract::RequiredEvent {
                    charge_kind: StatementChargeKind::LocalCall,
                    attribution: crate::StatementAttributionClass::Expression,
                },
                source: SourceContract::Required { use_kind: SourceUse::CallSite, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind),
                capabilities: [CapabilityRequirement::ExactLocalTarget, CapabilityRequirement::InOutLoan, CapabilityRequirement::NoPendingTarget]
            },
            {
                kind: InterfaceBoxLocal, opcode: 0x30, mnemonic: "interface_box_local",
                operands: [(OperandKind::Reloc, OperandRole::InterfaceTarget, LinkedOperandKind::InterfaceTable, [RelocationKind::LocalInterfaceRef])],
                stack_in: [(Arity::Fixed(1), ValueSource::InterfaceReceiver { interface: OperandRole::InterfaceTarget })],
                stack_out: [(Arity::Fixed(1), ValueSource::InterfaceCarrier { interface: OperandRole::InterfaceTarget })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::LocalInterfaceTable]
            },
            {
                kind: InterfaceBoxRemote, opcode: 0x31, mnemonic: "interface_box_remote",
                operands: [(OperandKind::Reloc, OperandRole::InterfaceTarget, LinkedOperandKind::InterfaceTable, [RelocationKind::RemoteInterfaceRef])],
                stack_in: [],
                stack_out: [(Arity::Fixed(1), ValueSource::InterfaceCarrier { interface: OperandRole::InterfaceTarget })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::RemoteInterfaceTable]
            },
            {
                kind: MakeCallback, opcode: 0x32, mnemonic: "make_callback",
                operands: [
                    (OperandKind::Reloc, OperandRole::CallbackTarget, LinkedOperandKind::SyntheticCallback, [RelocationKind::SyntheticCallbackRef]),
                    (OperandKind::Pool, OperandRole::CaptureLayoutRef, LinkedOperandKind::CallbackCaptureLayout, []),
                    (OperandKind::Immediate, OperandRole::CaptureCount, LinkedOperandKind::Immediate, [])
                ],
                stack_in: [(Arity::Declared(OperandRole::CaptureCount), ValueSource::CallbackCaptures { layout: OperandRole::CaptureLayoutRef })],
                stack_out: [(Arity::Fixed(1), ValueSource::CallbackClosure { target: OperandRole::CallbackTarget })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::CallbackCapture]
            },
            {
                kind: InvokeCallback, opcode: 0x33, mnemonic: "invoke_callback",
                operands: [
                    (OperandKind::Reloc, OperandRole::InterfaceTarget, LinkedOperandKind::InterfaceTable, [RelocationKind::InterfaceRequirementRef]),
                    (OperandKind::Immediate, OperandRole::MethodOrdinal, LinkedOperandKind::Immediate, []),
                    (OperandKind::Immediate, OperandRole::ArgCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Immediate, OperandRole::ResultCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Pool, OperandRole::ResumeRef, LinkedOperandKind::ResumeSite, [])
                ],
                stack_in: [
                    (Arity::Fixed(1), ValueSource::InterfaceCarrier { interface: OperandRole::InterfaceTarget }),
                    (Arity::Declared(OperandRole::ArgCount), ValueSource::TargetParameters { target: OperandRole::InterfaceTarget })
                ],
                stack_out: [(Arity::Declared(OperandRole::ResultCount), ValueSource::TargetResults { target: OperandRole::InterfaceTarget })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::ActualWithResume { resume: OperandRole::ResumeRef, mode: PendingMode::CallbackBoundary },
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::RaiseAtCurrentSite, &[]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::CallSite, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind),
                capabilities: [CapabilityRequirement::CallbackInvocation]
            },
            {
                kind: NewRecord, opcode: 0x40, mnemonic: "new_record",
                operands: [
                    (OperandKind::Pool, OperandRole::ShapeRef, LinkedOperandKind::Shape, []),
                    (OperandKind::Immediate, OperandRole::FieldCount, LinkedOperandKind::Immediate, [])
                ],
                stack_in: [(Arity::Declared(OperandRole::FieldCount), ValueSource::ShapeFields { shape: OperandRole::ShapeRef })],
                stack_out: [(Arity::Fixed(1), ValueSource::ShapeValue { shape: OperandRole::ShapeRef })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::VerifiedShape]
            },
            {
                kind: GetDenseField, opcode: 0x41, mnemonic: "get_dense_field",
                operands: [
                    (OperandKind::Pool, OperandRole::ShapeRef, LinkedOperandKind::Shape, []),
                    (OperandKind::Immediate, OperandRole::FieldOrdinal, LinkedOperandKind::Immediate, [])
                ],
                stack_in: [(Arity::Fixed(1), ValueSource::ShapeValue { shape: OperandRole::ShapeRef })],
                stack_out: [(Arity::Fixed(1), ValueSource::ShapeField { shape: OperandRole::ShapeRef, ordinal: OperandRole::FieldOrdinal })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::VerifiedShape]
            },
            {
                kind: SetWritablePath, opcode: 0x42, mnemonic: "set_writable_path",
                operands: [
                    (OperandKind::Slot, OperandRole::Slot, LinkedOperandKind::FrameSlot, []),
                    (OperandKind::Pool, OperandRole::WritablePathRef, LinkedOperandKind::WritablePath, []),
                    (OperandKind::Immediate, OperandRole::SelectorCount, LinkedOperandKind::Immediate, [])
                ],
                stack_in: [
                    (Arity::Declared(OperandRole::SelectorCount), ValueSource::WritablePathSelectors { path: OperandRole::WritablePathRef }),
                    (Arity::Fixed(1), ValueSource::WritablePathLeaf { path: OperandRole::WritablePathRef })
                ],
                stack_out: [],
                slots: SlotContract::Effects(&[
                    SlotEffectContract::new(OperandRole::Slot, SlotAction::Mutate, ValueSource::Slot { operand: OperandRole::Slot })
                ]),
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[
                    FailureContract::catchable(
                        FailureKind::CollectionIndexOutOfBounds,
                        FailureTrigger::IndexOutOfBounds,
                        crate::PlatformErrorProjectionKey::StdCollectionArrayIndexOutOfBoundsError
                    ),
                    FailureContract::catchable(
                        FailureKind::WritablePathIntermediateMissingKey,
                        FailureTrigger::IntermediateMissingKey,
                        crate::PlatformErrorProjectionKey::StdCollectionMapKeyNotFoundError
                    ),
                    FailureContract::invariant(
                        FailureKind::WritablePathTypeInvariant,
                        FailureTrigger::InternalTypeInvariant
                    ),
                    FailureContract::invariant(
                        FailureKind::WritablePathCowInvariant,
                        FailureTrigger::InternalCowInvariant
                    )
                ]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::InstructionFailure, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind),
                capabilities: [
                    CapabilityRequirement::VerifiedWritablePath,
                    CapabilityRequirement::WritablePathFinalMapUpsert
                ]
            },
            {
                kind: RepresentationWrap, opcode: 0x43, mnemonic: "representation_wrap",
                operands: [(OperandKind::Pool, OperandRole::TypeRef, LinkedOperandKind::Type, [])],
                stack_in: [(Arity::Fixed(1), ValueSource::RepresentationPayload { ty: OperandRole::TypeRef })],
                stack_out: [(Arity::Fixed(1), ValueSource::RepresentationValue { ty: OperandRole::TypeRef })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::RepresentationType]
            },
            {
                kind: TakeDenseField, opcode: 0x44, mnemonic: "take_dense_field",
                operands: [
                    (OperandKind::Pool, OperandRole::ShapeRef, LinkedOperandKind::Shape, []),
                    (OperandKind::Immediate, OperandRole::FieldOrdinal, LinkedOperandKind::Immediate, [])
                ],
                stack_in: [(Arity::Fixed(1), ValueSource::ShapeValue { shape: OperandRole::ShapeRef })],
                stack_out: [(Arity::Fixed(1), ValueSource::ShapeField { shape: OperandRole::ShapeRef, ordinal: OperandRole::FieldOrdinal })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::VerifiedShape, CapabilityRequirement::AffineFieldTake]
            },
            {
                kind: NewArrayBuilder, opcode: 0x50, mnemonic: "new_array_builder",
                operands: [(OperandKind::Pool, OperandRole::ElementTypeRef, LinkedOperandKind::Type, [])],
                stack_in: [],
                stack_out: [(Arity::Fixed(1), ValueSource::ArrayBuilder { element_type: OperandRole::ElementTypeRef })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::ArrayBuilderToken]
            },
            {
                kind: ArrayBuilderPush, opcode: 0x51, mnemonic: "array_builder_push", operands: [],
                stack_in: [
                    (Arity::Fixed(1), ValueSource::AnyStackValue),
                    (Arity::Fixed(1), ValueSource::ArrayElement { array_input: 0 })
                ],
                stack_out: [(Arity::Fixed(1), ValueSource::StackInput { group: 0 })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::ArrayBuilderToken]
            },
            {
                kind: FreezeArray, opcode: 0x52, mnemonic: "freeze_array", operands: [],
                stack_in: [(Arity::Fixed(1), ValueSource::AnyStackValue)],
                stack_out: [(Arity::Fixed(1), ValueSource::ArrayFromBuilder { builder_input: 0 })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::ArrayBuilderToken]
            },
            {
                kind: ArrayGet, opcode: 0x53, mnemonic: "array_get", operands: [],
                stack_in: [(Arity::Fixed(1), ValueSource::ArrayValue), (Arity::Fixed(1), ValueSource::CollectionIndex)],
                stack_out: [(Arity::Fixed(1), ValueSource::ArrayElement { array_input: 0 })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[
                    FailureContract::catchable(
                        FailureKind::CollectionIndexOutOfBounds,
                        FailureTrigger::IndexOutOfBounds,
                        crate::PlatformErrorProjectionKey::StdCollectionArrayIndexOutOfBoundsError
                    )
                ]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::InstructionFailure, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind), capabilities: []
            },
            {
                kind: ArrayPushOwned, opcode: 0x54, mnemonic: "array_push_owned",
                operands: [(OperandKind::Slot, OperandRole::Slot, LinkedOperandKind::FrameSlot, [])],
                stack_in: [(Arity::Fixed(1), ValueSource::ArrayElementFromSlot { slot: OperandRole::Slot })], stack_out: [],
                slots: SlotContract::Effects(&[
                    SlotEffectContract::new(OperandRole::Slot, SlotAction::Mutate, ValueSource::Slot { operand: OperandRole::Slot })
                ]),
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable), capabilities: []
            },
            {
                kind: NewMapBuilder, opcode: 0x55, mnemonic: "new_map_builder",
                operands: [
                    (OperandKind::Pool, OperandRole::KeyTypeRef, LinkedOperandKind::Type, []),
                    (OperandKind::Pool, OperandRole::ValueTypeRef, LinkedOperandKind::Type, [])
                ],
                stack_in: [],
                stack_out: [(Arity::Fixed(1), ValueSource::MapBuilder { key_type: OperandRole::KeyTypeRef, value_type: OperandRole::ValueTypeRef })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::MapBuilderToken]
            },
            {
                kind: MapBuilderPut, opcode: 0x56, mnemonic: "map_builder_put", operands: [],
                stack_in: [
                    (Arity::Fixed(1), ValueSource::AnyStackValue),
                    (Arity::Fixed(1), ValueSource::MapKey { map_input: 0 }),
                    (Arity::Fixed(1), ValueSource::MapElement { map_input: 0 })
                ],
                stack_out: [(Arity::Fixed(1), ValueSource::StackInput { group: 0 })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::MapBuilderToken]
            },
            {
                kind: FreezeMap, opcode: 0x57, mnemonic: "freeze_map", operands: [],
                stack_in: [(Arity::Fixed(1), ValueSource::AnyStackValue)],
                stack_out: [(Arity::Fixed(1), ValueSource::MapFromBuilder { builder_input: 0 })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable),
                capabilities: [CapabilityRequirement::MapBuilderToken]
            },
            {
                kind: MapGet, opcode: 0x58, mnemonic: "map_get", operands: [],
                stack_in: [(Arity::Fixed(1), ValueSource::MapValue), (Arity::Fixed(1), ValueSource::MapKey { map_input: 0 })],
                stack_out: [(Arity::Fixed(1), ValueSource::MapElement { map_input: 0 })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[
                    FailureContract::catchable(
                        FailureKind::CollectionMissingKey,
                        FailureTrigger::MissingKey,
                        crate::PlatformErrorProjectionKey::StdCollectionMapKeyNotFoundError
                    )
                ]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::InstructionFailure, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind), capabilities: []
            },
            {
                kind: MapPutOwned, opcode: 0x59, mnemonic: "map_put_owned",
                operands: [(OperandKind::Slot, OperandRole::Slot, LinkedOperandKind::FrameSlot, [])],
                stack_in: [
                    (Arity::Fixed(1), ValueSource::MapKeyFromSlot { slot: OperandRole::Slot }),
                    (Arity::Fixed(1), ValueSource::MapElementFromSlot { slot: OperandRole::Slot })
                ],
                stack_out: [],
                slots: SlotContract::Effects(&[
                    SlotEffectContract::new(OperandRole::Slot, SlotAction::Mutate, ValueSource::Slot { operand: OperandRole::Slot })
                ]),
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable), capabilities: []
            },
            {
                kind: ArrayLen, opcode: 0x5A, mnemonic: "array_len", operands: [],
                stack_in: [(Arity::Fixed(1), ValueSource::ArrayValue)],
                stack_out: [(Arity::Fixed(1), ValueSource::Number)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable), capabilities: []
            },
            {
                kind: MapLen, opcode: 0x5B, mnemonic: "map_len", operands: [],
                stack_in: [(Arity::Fixed(1), ValueSource::MapValue)],
                stack_out: [(Arity::Fixed(1), ValueSource::Number)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable), capabilities: []
            },
            {
                kind: MapEntryAt, opcode: 0x5C, mnemonic: "map_entry_at", operands: [],
                stack_in: [(Arity::Fixed(1), ValueSource::MapValue), (Arity::Fixed(1), ValueSource::CollectionIndex)],
                stack_out: [
                    (Arity::Fixed(1), ValueSource::MapKey { map_input: 0 }),
                    (Arity::Fixed(1), ValueSource::MapElement { map_input: 0 })
                ],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[
                    FailureContract::terminal(
                        FailureKind::MapEntryIndexOutOfBounds,
                        FailureTrigger::IndexOutOfBounds
                    )
                ]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::GeneratedFailure, origin: SourceOriginConstraint::SyntheticOnly },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind),
                capabilities: [
                    CapabilityRequirement::InternalGenerated,
                    CapabilityRequirement::CanonicalMapSnapshot
                ]
            },
            {
                kind: StreamNext, opcode: 0x60, mnemonic: "stream_next",
                operands: [
                    (OperandKind::Slot, OperandRole::Slot, LinkedOperandKind::FrameSlot, []),
                    (OperandKind::Pool, OperandRole::ResumeRef, LinkedOperandKind::ResumeSite, [])
                ],
                stack_in: [],
                stack_out: [(Arity::Fixed(1), ValueSource::StreamItem { endpoint_slot: OperandRole::Slot })],
                slots: SlotContract::Effects(&[
                    SlotEffectContract::new(OperandRole::Slot, SlotAction::Mutate, ValueSource::Slot { operand: OperandRole::Slot })
                ]),
                control: ControlContract::Fallthrough,
                pending: PendingContract::ActualWithResume { resume: OperandRole::ResumeRef, mode: PendingMode::StreamRead },
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::RaiseAtCurrentSite, &[]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::StreamSite, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind),
                capabilities: [CapabilityRequirement::StreamConsumer]
            },
            {
                kind: EmitStream, opcode: 0x61, mnemonic: "emit_stream",
                operands: [(OperandKind::Pool, OperandRole::ResumeRef, LinkedOperandKind::ResumeSite, [])],
                stack_in: [(Arity::Fixed(1), ValueSource::FunctionStreamItem)], stack_out: [],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::ActualWithResume { resume: OperandRole::ResumeRef, mode: PendingMode::StreamBackpressure },
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::RaiseAtCurrentSite, &[]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::StreamSite, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind),
                capabilities: [CapabilityRequirement::StreamProducer]
            },
            {
                kind: Throw, opcode: 0x70, mnemonic: "throw",
                operands: [(OperandKind::Pool, OperandRole::TypeRef, LinkedOperandKind::Type, [])],
                stack_in: [(Arity::Fixed(1), ValueSource::ExceptionPayload { type_ref: OperandRole::TypeRef })], stack_out: [],
                slots: SlotContract::None, control: ControlContract::Raise,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::ThrowValue { type_ref: OperandRole::TypeRef }, &[]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::ThrowOrigin, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::NotApplicable, RegionEffect::Unwind), capabilities: []
            },
            {
                kind: Rethrow, opcode: 0x71, mnemonic: "rethrow",
                operands: [(OperandKind::Slot, OperandRole::SourceSlot, LinkedOperandKind::FrameSlot, [])],
                stack_in: [], stack_out: [],
                slots: SlotContract::Effects(&[
                    SlotEffectContract::new(OperandRole::SourceSlot, SlotAction::Read, ValueSource::ExceptionEnvelope { source_slot: OperandRole::SourceSlot })
                ]),
                control: ControlContract::Rethrow, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::PreserveOriginal { source_slot: OperandRole::SourceSlot }, &[]),
                statement: StatementContract::None, source: SourceContract::PreserveOriginal,
                region: RegionContract::new(RegionEffect::NotApplicable, RegionEffect::Unwind), capabilities: []
            },
            {
                kind: EnterRegion, opcode: 0x72, mnemonic: "enter_region",
                operands: [(OperandKind::Table, OperandRole::ActiveRegion, LinkedOperandKind::ActiveRegion, [])],
                stack_in: [], stack_out: [], slots: SlotContract::None,
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::ActiveRegion { operand: OperandRole::ActiveRegion },
                region: RegionContract::new(RegionEffect::Enter { operand: OperandRole::ActiveRegion }, RegionEffect::NotApplicable),
                capabilities: []
            },
            {
                kind: LeaveRegion, opcode: 0x73, mnemonic: "leave_region",
                operands: [(OperandKind::Table, OperandRole::ActiveRegion, LinkedOperandKind::ActiveRegion, [])],
                stack_in: [], stack_out: [], slots: SlotContract::None,
                control: ControlContract::Fallthrough, pending: PendingContract::Never,
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]),
                statement: StatementContract::None, source: SourceContract::ActiveRegion { operand: OperandRole::ActiveRegion },
                region: RegionContract::new(RegionEffect::Leave { operand: OperandRole::ActiveRegion }, RegionEffect::NotApplicable),
                capabilities: []
            },
            {
                kind: InvokeHost, opcode: 0x80, mnemonic: "invoke_host",
                operands: [
                    (OperandKind::Reloc, OperandRole::HostTarget, LinkedOperandKind::HostEffectAdapter, [RelocationKind::HostEffectRef]),
                    (OperandKind::Immediate, OperandRole::ArgCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Immediate, OperandRole::ResultCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Pool, OperandRole::ResumeRef, LinkedOperandKind::ResumeSite, [])
                ],
                stack_in: [(Arity::Declared(OperandRole::ArgCount), ValueSource::TargetParameters { target: OperandRole::HostTarget })],
                stack_out: [(Arity::Declared(OperandRole::ResultCount), ValueSource::TargetResults { target: OperandRole::HostTarget })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::ActualWithResume { resume: OperandRole::ResumeRef, mode: PendingMode::HostEffect },
                checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::RaiseAtCurrentSite, &[]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::EffectSite, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind),
                capabilities: [CapabilityRequirement::TrustedHostAdapter]
            },
            {
                kind: InvokeIntrinsic, opcode: 0x81, mnemonic: "invoke_intrinsic",
                operands: [
                    (OperandKind::Reloc, OperandRole::IntrinsicTarget, LinkedOperandKind::Intrinsic, [RelocationKind::IntrinsicRef, RelocationKind::TaskSubmitRef]),
                    (OperandKind::Immediate, OperandRole::ArgCount, LinkedOperandKind::Immediate, []),
                    (OperandKind::Immediate, OperandRole::ResultCount, LinkedOperandKind::Immediate, [])
                ],
                stack_in: [(Arity::Declared(OperandRole::ArgCount), ValueSource::TargetParameters { target: OperandRole::IntrinsicTarget })],
                stack_out: [(Arity::Declared(OperandRole::ResultCount), ValueSource::TargetResults { target: OperandRole::IntrinsicTarget })],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::RaiseAtCurrentSite, &[]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::EffectSite, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind),
                capabilities: [CapabilityRequirement::TrustedIntrinsic]
            },
            {
                kind: Not, opcode: 0x90, mnemonic: "not", operands: [],
                stack_in: [(Arity::Fixed(1), ValueSource::Bool)], stack_out: [(Arity::Fixed(1), ValueSource::Bool)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable), capabilities: []
            },
            {
                kind: Negate, opcode: 0x91, mnemonic: "negate", operands: [],
                stack_in: [(Arity::Fixed(1), ValueSource::Number)], stack_out: [(Arity::Fixed(1), ValueSource::Number)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[
                    FailureContract::terminal(
                        FailureKind::ScalarNonFinite,
                        FailureTrigger::NonFiniteResult
                    )
                ]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::GeneratedFailure, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind), capabilities: []
            },
            {
                kind: Add, opcode: 0x92, mnemonic: "add", operands: [],
                stack_in: [(Arity::Fixed(2), ValueSource::Number)], stack_out: [(Arity::Fixed(1), ValueSource::Number)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[
                    FailureContract::terminal(
                        FailureKind::ScalarNonFinite,
                        FailureTrigger::NonFiniteResult
                    )
                ]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::GeneratedFailure, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind), capabilities: []
            },
            {
                kind: Subtract, opcode: 0x93, mnemonic: "subtract", operands: [],
                stack_in: [(Arity::Fixed(2), ValueSource::Number)], stack_out: [(Arity::Fixed(1), ValueSource::Number)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[
                    FailureContract::terminal(
                        FailureKind::ScalarNonFinite,
                        FailureTrigger::NonFiniteResult
                    )
                ]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::GeneratedFailure, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind), capabilities: []
            },
            {
                kind: Multiply, opcode: 0x94, mnemonic: "multiply", operands: [],
                stack_in: [(Arity::Fixed(2), ValueSource::Number)], stack_out: [(Arity::Fixed(1), ValueSource::Number)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[
                    FailureContract::terminal(
                        FailureKind::ScalarNonFinite,
                        FailureTrigger::NonFiniteResult
                    )
                ]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::GeneratedFailure, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind), capabilities: []
            },
            {
                kind: Divide, opcode: 0x95, mnemonic: "divide", operands: [],
                stack_in: [(Arity::Fixed(2), ValueSource::Number)], stack_out: [(Arity::Fixed(1), ValueSource::Number)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[
                    FailureContract::terminal(
                        FailureKind::DivideByZero,
                        FailureTrigger::ZeroDivisorIncludingNegativeZero
                    ),
                    FailureContract::terminal(
                        FailureKind::ScalarNonFinite,
                        FailureTrigger::NonFiniteResult
                    )
                ]),
                statement: StatementContract::None, source: SourceContract::Required { use_kind: SourceUse::GeneratedFailure, origin: SourceOriginConstraint::SourceOrSynthetic },
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::Unwind), capabilities: []
            },
            {
                kind: Equal, opcode: 0x96, mnemonic: "equal", operands: [],
                stack_in: [(Arity::Fixed(2), ValueSource::ComparablePair)], stack_out: [(Arity::Fixed(1), ValueSource::Bool)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable), capabilities: []
            },
            {
                kind: NotEqual, opcode: 0x97, mnemonic: "not_equal", operands: [],
                stack_in: [(Arity::Fixed(2), ValueSource::ComparablePair)], stack_out: [(Arity::Fixed(1), ValueSource::Bool)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable), capabilities: []
            },
            {
                kind: LessThan, opcode: 0x98, mnemonic: "less_than", operands: [],
                stack_in: [(Arity::Fixed(2), ValueSource::Number)], stack_out: [(Arity::Fixed(1), ValueSource::Bool)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable), capabilities: []
            },
            {
                kind: LessOrEqual, opcode: 0x99, mnemonic: "less_or_equal", operands: [],
                stack_in: [(Arity::Fixed(2), ValueSource::Number)], stack_out: [(Arity::Fixed(1), ValueSource::Bool)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable), capabilities: []
            },
            {
                kind: GreaterThan, opcode: 0x9A, mnemonic: "greater_than", operands: [],
                stack_in: [(Arity::Fixed(2), ValueSource::Number)], stack_out: [(Arity::Fixed(1), ValueSource::Bool)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable), capabilities: []
            },
            {
                kind: GreaterOrEqual, opcode: 0x9B, mnemonic: "greater_or_equal", operands: [],
                stack_in: [(Arity::Fixed(2), ValueSource::Number)], stack_out: [(Arity::Fixed(1), ValueSource::Bool)],
                slots: SlotContract::None, control: ControlContract::Fallthrough,
                pending: PendingContract::Never, checkpoint: CheckpointContract::None,
                exception: ExceptionContract::new(ExceptionBehavior::None, &[]), statement: StatementContract::None, source: SourceContract::None,
                region: RegionContract::new(RegionEffect::Preserve, RegionEffect::NotApplicable), capabilities: []
            }
        }
    };
}

macro_rules! define_opcode_contracts {
    ($({
        kind: $kind:ident, opcode: $opcode:literal, mnemonic: $mnemonic:literal,
        operands: [$(($operand_kind:expr, $operand_role:expr, $linked_kind:expr, [$($relocation:expr),*])),* $(,)?],
        stack_in: [$(($stack_in_arity:expr, $stack_in_value:expr)),* $(,)?],
        stack_out: [$(($stack_out_arity:expr, $stack_out_value:expr)),* $(,)?],
        slots: $slots:expr,
        control: $control:expr,
        pending: $pending:expr,
        checkpoint: $checkpoint:expr,
        exception: $exception:expr,
        statement: $statement:expr,
        source: $source:expr,
        region: $region:expr,
        capabilities: [$($capability:expr),* $(,)?]
    }),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Opcode {
            $($kind),*
        }

        impl Opcode {
            pub const ALL: &'static [Self] = &[$(Self::$kind),*];

            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$kind => $mnemonic),*
                }
            }
        }

        impl Serialize for Opcode {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.name())
            }
        }

        pub const ALL_OPCODES: &[Opcode] = Opcode::ALL;

        pub const OPCODE_CONTRACTS: &[OpcodeContract] = &[
            $(OpcodeContract {
                kind: Opcode::$kind,
                opcode: $opcode,
                mnemonic: $mnemonic,
                operands: &[
                    $(OperandSpec::new(
                        $operand_kind,
                        $operand_role,
                        $linked_kind,
                        &[$($relocation),*],
                    )),*
                ],
                typed: TypedTransition::new(
                    &[$(TypedStackGroup::new($stack_in_arity, $stack_in_value)),*],
                    &[$(TypedStackGroup::new($stack_out_arity, $stack_out_value)),*],
                    $slots,
                ),
                control: $control,
                pending: $pending,
                checkpoint: $checkpoint,
                exception: $exception,
                statement: $statement,
                source: $source,
                region: $region,
                capabilities: &[$($capability),*],
            }),*
        ];

        /// Compatibility view generated from `OPCODE_CONTRACTS`' sole row
        /// declaration. It is not an independent semantic table.
        pub const OPCODE_TABLE: &[OpcodeDescriptor] = &[
            $(OpcodeDescriptor {
                kind: Opcode::$kind,
                opcode: $opcode,
                mnemonic: $mnemonic,
                operand_layout: &[$($operand_kind),*],
                operand_roles: &[$($operand_role),*],
                stack_in: &[$(StackEffect::new($stack_in_arity)),*],
                stack_out: &[$(StackEffect::new($stack_out_arity)),*],
                allowed_relocations: &[$($($relocation,)*)*],
            }),*
        ];
    };
}

opcode_rows!(define_opcode_contracts);

pub const OPCODE_COUNT: usize = 64;

pub const fn opcode_contract_for(value: u8) -> Option<&'static OpcodeContract> {
    let mut index = 0;
    while index < OPCODE_CONTRACTS.len() {
        let contract = &OPCODE_CONTRACTS[index];
        if contract.opcode == value {
            return Some(contract);
        }
        index += 1;
    }
    None
}

pub fn contract_for_opcode(kind: Opcode) -> &'static OpcodeContract {
    OPCODE_CONTRACTS
        .iter()
        .find(|contract| contract.kind == kind)
        .expect("every Opcode has one canonical contract")
}

/// Phase 1 compatibility lookup.
pub const fn opcode_for(value: u8) -> Option<&'static OpcodeDescriptor> {
    let mut index = 0;
    while index < OPCODE_TABLE.len() {
        let descriptor = &OPCODE_TABLE[index];
        if descriptor.opcode == value {
            return Some(descriptor);
        }
        index += 1;
    }
    None
}

pub fn opcode_kind(encoded: u8) -> Option<Opcode> {
    opcode_contract_for(encoded).map(|contract| contract.kind)
}

/// Phase 1 compatibility lookup.
pub fn descriptor_for_opcode(kind: Opcode) -> &'static OpcodeDescriptor {
    OPCODE_TABLE
        .iter()
        .find(|descriptor| descriptor.kind == kind)
        .expect("every Opcode has one generated compatibility descriptor")
}

pub const fn pool_operand_category(opcode: u8, position: usize) -> Option<PoolCategory> {
    let contract = match opcode_contract_for(opcode) {
        Some(contract) => contract,
        None => return None,
    };
    if position >= contract.operands.len() {
        return None;
    }
    contract.operands[position].linked_kind.pool_category()
}

pub const fn table_operand_category(opcode: u8, position: usize) -> Option<TableCategory> {
    let contract = match opcode_contract_for(opcode) {
        Some(contract) => contract,
        None => return None,
    };
    if position >= contract.operands.len() {
        return None;
    }
    contract.operands[position].linked_kind.table_category()
}
