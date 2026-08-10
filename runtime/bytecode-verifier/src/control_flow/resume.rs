use std::collections::BTreeSet;

use skiff_artifact_model::{
    contract_for_opcode, CapabilityRequirement, ExceptionBehavior, Opcode, OperandRole,
    PendingContract, PendingMode, RegionEffect, ResumeErrorMode, SourceContract,
    SourceOriginConstraint, SourceUse, StatementContract,
};
use skiff_runtime_linked_bytecode::{
    FrameSlotIndex, FunctionIndex, InstructionIndex, LinkedBytecodeCandidate, LinkedFunction,
    LinkedInstruction, LinkedInstructionTarget, ResumeSiteIndex,
};

use super::{
    AbstractSlotState, AbstractValue, ControlFlowEdgeKind, ControlFlowFacts, ProgramPointState,
};
use crate::{
    admission::{ExactResumeBinding, ExactResumeEntry},
    attribution::SourceAttributionFacts,
    concrete_values::ConcreteValueFacts,
    resume::{VerifiedResumeKind, VerifiedResumeSite, VerifiedResumeSites},
    VerificationError, VerificationLocation, VerificationObligation,
};

/// Resume operand/mode proof established before generic instruction transfer.
#[derive(Debug)]
pub(super) struct ResumePreflight {
    rows: Box<[StreamResumeCoordinate]>,
}

#[derive(Debug, Clone, Copy)]
struct StreamResumeCoordinate {
    descriptor: ResumeSiteIndex,
    function: FunctionIndex,
    site: InstructionIndex,
    endpoint_slot: FrameSlotIndex,
}

pub(super) fn prove_contracts(
    candidate: &LinkedBytecodeCandidate,
    exact: &ExactResumeBinding,
) -> Result<ResumePreflight, VerificationError> {
    let mut rows = Vec::with_capacity(exact.rows().len());
    let mut used = BTreeSet::new();
    for function in candidate.functions() {
        for (ordinal, instruction) in function.instructions().iter().enumerate() {
            let site = instruction_index(function, ordinal)?;
            let location = instruction_location(function.index(), site);
            let contract = contract_for_opcode(instruction.opcode());
            let PendingContract::ActualWithResume { resume, mode } = contract.pending else {
                continue;
            };
            if instruction.opcode() != Opcode::StreamNext
                || mode != PendingMode::StreamRead
                || contract.exception.behavior != ExceptionBehavior::RaiseAtCurrentSite
                || !contract.exception.failures.is_empty()
                || contract.statement != StatementContract::None
                || contract.source
                    != (SourceContract::Required {
                        use_kind: SourceUse::StreamSite,
                        origin: SourceOriginConstraint::SourceOrSynthetic,
                    })
                || contract.region.normal != RegionEffect::Preserve
                || contract.region.raised != RegionEffect::Unwind
                || contract.capabilities != [CapabilityRequirement::StreamConsumer]
            {
                return Err(unavailable(location));
            }
            let descriptor = resume_target(instruction, resume, location)?;
            let row = exact
                .row(descriptor)
                .ok_or_else(|| violation(location, "resume target has no exact P1 descriptor"))?;
            if row.function() != function.index() || row.site() != site {
                return Err(violation(
                    location,
                    "resume target is not bound to this exact function and instruction",
                ));
            }
            if !used.insert(descriptor) {
                return Err(violation(
                    location,
                    "resume descriptor is consumed by more than one pending instruction",
                ));
            }
            rows.push(StreamResumeCoordinate {
                descriptor,
                function: function.index(),
                site,
                endpoint_slot: slot_target(instruction, OperandRole::Slot, location)?,
            });
        }
    }
    if used.len() != exact.rows().len() {
        let missing = exact
            .rows()
            .iter()
            .find(|row| !used.contains(&row.index()))
            .map(|row| row.index().get())
            .unwrap_or(0);
        return Err(VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ResumeSite,
            location: VerificationLocation::Table {
                table: skiff_runtime_linked_bytecode::CandidateTable::ResumeSites,
                row: missing,
            },
        });
    }
    rows.sort_unstable_by_key(|row| row.descriptor.get());
    Ok(ResumePreflight {
        rows: rows.into_boxed_slice(),
    })
}

pub(super) fn prove_resume_states(
    candidate: &LinkedBytecodeCandidate,
    exact: &ExactResumeBinding,
    preflight: &ResumePreflight,
    concrete: &ConcreteValueFacts,
    control_flow: &ControlFlowFacts,
    source: &SourceAttributionFacts,
) -> Result<VerifiedResumeSites, VerificationError> {
    if preflight.rows.len() != exact.rows().len() {
        return Err(violation(
            VerificationLocation::Image,
            "resume preflight and exact descriptor counts differ",
        ));
    }
    let mut verified = Vec::with_capacity(preflight.rows.len());
    for coordinate in &preflight.rows {
        let row = exact.row(coordinate.descriptor).ok_or_else(|| {
            violation(
                instruction_location(coordinate.function, coordinate.site),
                "resume preflight lost its exact descriptor",
            )
        })?;
        verified.push(prove_stream_read(
            candidate,
            row,
            *coordinate,
            concrete,
            control_flow,
            source,
        )?);
    }
    Ok(VerifiedResumeSites::new(verified.into_boxed_slice()))
}

fn prove_stream_read(
    candidate: &LinkedBytecodeCandidate,
    row: &ExactResumeEntry,
    coordinate: StreamResumeCoordinate,
    concrete: &ConcreteValueFacts,
    control_flow: &ControlFlowFacts,
    source: &SourceAttributionFacts,
) -> Result<VerifiedResumeSite, VerificationError> {
    let location = instruction_location(coordinate.function, coordinate.site);
    if candidate
        .functions()
        .get(coordinate.function.get() as usize)
        .is_none_or(|function| function.index() != coordinate.function)
    {
        return Err(violation(location, "resume function is not dense"));
    }
    let flow = control_flow
        .functions
        .get(coordinate.function.get() as usize)
        .ok_or_else(|| violation(location, "resume function has no CFG facts"))?;
    let before = flow
        .states_before
        .get(coordinate.site.get() as usize)
        .ok_or_else(|| violation(location, "resume site has no input state"))?;
    let resume_location = instruction_location(coordinate.function, row.resume());
    let resumed = flow
        .states_before
        .get(row.resume().get() as usize)
        .ok_or_else(|| violation(resume_location, "resume PC has no input state"))?;

    let expected_height = u32::try_from(before.stack.len())
        .map_err(|_| violation(location, "resume stack height does not fit u32"))?;
    if row.expected_stack_height_before_result() != expected_height
        || row.result_types().len() != 1
        || row.result_plans().len() != 1
        || row.error_mode() != ResumeErrorMode::RaiseAtSite
    {
        return Err(violation(
            location,
            "StreamNext descriptor has the wrong stack arity or error route",
        ));
    }
    let expected_resume = coordinate
        .site
        .get()
        .checked_add(1)
        .map(InstructionIndex::new)
        .ok_or_else(|| violation(location, "resume instruction arithmetic overflowed"))?;
    let successor = flow
        .successors
        .get(coordinate.site.get() as usize)
        .ok_or_else(|| violation(location, "StreamNext site has no CFG successor row"))?;
    if row.resume() != expected_resume
        || successor.as_ref()
            != [super::ControlFlowEdge {
                target: row.resume(),
                kind: ControlFlowEdgeKind::Ordinary,
            }]
    {
        return Err(violation(
            location,
            "StreamNext ready path is not the exact immediate resume PC",
        ));
    }

    let endpoint = live_slot(before, coordinate.endpoint_slot, location)?;
    let item = concrete.stream_item_type(endpoint, location)?;
    let declared_item = row.result_types()[0];
    if concrete.semantically_equal(item, declared_item) != Some(true)
        || !concrete.matches_declared_plan(item, &row.result_plans()[0])
    {
        return Err(violation(
            location,
            "resume result is not the independently derived Stream<T> item",
        ));
    }
    prove_ready_resume_isomorphism(
        before,
        resumed,
        coordinate.endpoint_slot,
        item,
        concrete,
        resume_location,
    )?;
    let original_site = source
        .current_site(coordinate.function, coordinate.site)
        .cloned()
        .ok_or_else(|| violation(location, "StreamNext has no verified original source site"))?;

    Ok(VerifiedResumeSite::from_parts(
        crate::resume::VerifiedResumeSiteParts {
            index: row.index(),
            function: row.function(),
            site: row.site(),
            resume: row.resume(),
            expected_stack_height_before_result: row.expected_stack_height_before_result(),
            result_type: item,
            result_plan: row.result_plans()[0].clone(),
            error_mode: row.error_mode(),
            original_site,
            kind: VerifiedResumeKind::StreamRead {
                endpoint_slot: coordinate.endpoint_slot,
                item_type: item,
            },
        },
    ))
}

fn prove_ready_resume_isomorphism(
    before: &ProgramPointState,
    resumed: &ProgramPointState,
    endpoint_slot: FrameSlotIndex,
    item: skiff_runtime_linked_bytecode::TypeIndex,
    concrete: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let expected_stack_len =
        before.stack.len().checked_add(1).ok_or_else(|| {
            violation(location, "StreamNext result stack height overflowed usize")
        })?;
    if resumed.stack.len() != expected_stack_len
        || resumed.slots.len() != before.slots.len()
        || resumed.active_regions != before.active_regions
        || resumed.writable_loans != before.writable_loans
    {
        return Err(violation(
            location,
            "ready and resumed StreamNext states have different shapes",
        ));
    }
    for (ready, resumed) in before.stack.iter().zip(resumed.stack.iter()) {
        if !same_value(*ready, *resumed, concrete) {
            return Err(violation(
                location,
                "resume stack prefix differs from ready state",
            ));
        }
    }
    if !same_value(
        AbstractValue::Concrete(item),
        resumed.stack[before.stack.len()],
        concrete,
    ) {
        return Err(violation(
            location,
            "resume stack result is not Stream<T> item T",
        ));
    }
    for (ready, resumed) in before.slots.iter().zip(resumed.slots.iter()) {
        if !same_slot(*ready, *resumed, concrete) {
            return Err(violation(
                location,
                "resume slot state differs from ready state",
            ));
        }
    }
    let resumed_endpoint = live_slot(resumed, endpoint_slot, location)?;
    concrete.stream_item_type(resumed_endpoint, location)?;
    Ok(())
}

fn same_value(left: AbstractValue, right: AbstractValue, concrete: &ConcreteValueFacts) -> bool {
    let (AbstractValue::Concrete(left), AbstractValue::Concrete(right)) = (left, right);
    concrete.semantically_equal(left, right) == Some(true)
}

fn same_slot(
    left: AbstractSlotState,
    right: AbstractSlotState,
    concrete: &ConcreteValueFacts,
) -> bool {
    match (left, right) {
        (AbstractSlotState::Uninitialized, AbstractSlotState::Uninitialized)
        | (AbstractSlotState::Moved, AbstractSlotState::Moved) => true,
        (AbstractSlotState::Live(left), AbstractSlotState::Live(right)) => {
            concrete.semantically_equal(left, right) == Some(true)
        }
        _ => false,
    }
}

fn live_slot(
    state: &ProgramPointState,
    slot: FrameSlotIndex,
    location: VerificationLocation,
) -> Result<skiff_runtime_linked_bytecode::TypeIndex, VerificationError> {
    match state.slots.get(slot.get() as usize) {
        Some(AbstractSlotState::Live(ty)) => Ok(*ty),
        Some(AbstractSlotState::Moved) => Err(violation(location, "stream endpoint is moved")),
        Some(AbstractSlotState::Uninitialized) => {
            Err(violation(location, "stream endpoint is uninitialized"))
        }
        None => Err(violation(location, "stream endpoint slot is out of bounds")),
    }
}

fn resume_target(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: VerificationLocation,
) -> Result<ResumeSiteIndex, VerificationError> {
    match resolved_target(instruction, role, location)? {
        LinkedInstructionTarget::ResumeSite(index) => Ok(index),
        _ => Err(violation(location, "resume role has a non-resume target")),
    }
}

fn slot_target(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: VerificationLocation,
) -> Result<FrameSlotIndex, VerificationError> {
    match resolved_target(instruction, role, location)? {
        LinkedInstructionTarget::FrameSlot(index) => Ok(index),
        _ => Err(violation(
            location,
            "stream endpoint role has a non-slot target",
        )),
    }
}

fn resolved_target(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: VerificationLocation,
) -> Result<LinkedInstructionTarget, VerificationError> {
    let contract = contract_for_opcode(instruction.opcode());
    let ordinal = contract
        .operand_position(role)
        .and_then(|ordinal| u32::try_from(ordinal).ok())
        .ok_or_else(|| violation(location, "resume operand role is absent"))?;
    instruction
        .resolved_operands()
        .iter()
        .find(|operand| operand.operand_ordinal() == ordinal)
        .map(|operand| operand.target())
        .ok_or_else(|| violation(location, "resume typed operand is absent"))
}

fn instruction_index(
    function: &LinkedFunction,
    ordinal: usize,
) -> Result<InstructionIndex, VerificationError> {
    u32::try_from(ordinal)
        .map(InstructionIndex::new)
        .map_err(|_| {
            violation(
                VerificationLocation::Function {
                    function: function.index(),
                },
                "resume instruction index does not fit u32",
            )
        })
}

const fn instruction_location(
    function: FunctionIndex,
    instruction: InstructionIndex,
) -> VerificationLocation {
    VerificationLocation::Instruction {
        function,
        instruction,
    }
}

const fn unavailable(location: VerificationLocation) -> VerificationError {
    VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ResumeSite,
        location,
    }
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ResumeSite,
        location,
        detail: detail.into(),
    }
}
