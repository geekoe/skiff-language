use std::collections::BTreeSet;

use skiff_artifact_model::{
    contract_for_opcode, Arity, CapabilityRequirement, ExceptionBehavior, Opcode, OperandRole,
    PendingContract, PendingMode, RegionEffect, ResumeErrorMode, SourceContract,
    SourceOriginConstraint, SourceUse, StatementContract, TypedStackGroup,
};
use skiff_runtime_linked_bytecode::{
    FrameSlotIndex, FunctionIndex, InstructionIndex, LinkedBytecodeCandidate, LinkedFunction,
    LinkedInstruction, LinkedInstructionTarget, ResumeSiteIndex, TypeIndex,
};

use super::{
    targets::ExactTargetAndCallFacts, AbstractSlotState, AbstractValue, ControlFlowEdgeKind,
    ControlFlowFacts, ProgramPointState,
};
use crate::{
    admission::{ExactResumeBinding, ExactResumeEntry},
    attribution::SourceAttributionFacts,
    concrete_values::ConcreteValueFacts,
    resume::{VerifiedResumeKind, VerifiedResumeSite, VerifiedResumeSites},
    VerificationError, VerificationLocation, VerificationObligation,
};

#[derive(Debug)]
pub(super) struct ResumePreflight {
    rows: Box<[PendingResumeCoordinate]>,
}

#[derive(Debug, Clone, Copy)]
struct PendingResumeCoordinate {
    descriptor: ResumeSiteIndex,
    function: FunctionIndex,
    site: InstructionIndex,
    endpoint_slot: Option<FrameSlotIndex>,
    mode: PendingMode,
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
            prove_pending_contract_shape(contract, mode, location)?;
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
            let endpoint_slot = (instruction.opcode() == Opcode::StreamNext)
                .then(|| slot_target(instruction, OperandRole::Slot, location))
                .transpose()?;
            rows.push(PendingResumeCoordinate {
                descriptor,
                function: function.index(),
                site,
                endpoint_slot,
                mode,
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

fn prove_pending_contract_shape(
    contract: &skiff_artifact_model::OpcodeContract,
    mode: PendingMode,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    if contract.exception.behavior != ExceptionBehavior::RaiseAtCurrentSite
        || !contract.exception.failures.is_empty()
        || contract.statement != StatementContract::None
        || contract.region.normal != RegionEffect::Preserve
        || contract.region.raised != RegionEffect::Unwind
    {
        return Err(unavailable(location));
    }
    let source_use = match mode {
        PendingMode::StreamRead | PendingMode::StreamBackpressure => SourceUse::StreamSite,
        PendingMode::ServiceBoundary
        | PendingMode::ActorBoundary
        | PendingMode::InterfaceBoundary
        | PendingMode::CallbackBoundary => SourceUse::CallSite,
        PendingMode::HostEffect => SourceUse::EffectSite,
    };
    let expected_capability = match mode {
        PendingMode::StreamRead => Some(CapabilityRequirement::StreamConsumer),
        PendingMode::StreamBackpressure => Some(CapabilityRequirement::StreamProducer),
        PendingMode::HostEffect => Some(CapabilityRequirement::TrustedHostAdapter),
        _ => None,
    };
    if contract.source
        != (SourceContract::Required {
            use_kind: source_use,
            origin: SourceOriginConstraint::SourceOrSynthetic,
        })
        || expected_capability
            .is_some_and(|capability| !contract.capabilities.contains(&capability))
    {
        return Err(unavailable(location));
    }
    Ok(())
}

pub(super) fn prove_resume_states(
    candidate: &LinkedBytecodeCandidate,
    exact: &ExactResumeBinding,
    preflight: &ResumePreflight,
    concrete: &ConcreteValueFacts,
    control_flow: &ControlFlowFacts,
    targets: &ExactTargetAndCallFacts,
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
        let verified_site = match coordinate.mode {
            PendingMode::StreamRead => {
                prove_stream_read(candidate, row, *coordinate, concrete, control_flow, source)?
            }
            _ => prove_pending_boundary(
                candidate,
                row,
                *coordinate,
                concrete,
                control_flow,
                targets,
                source,
            )?,
        };
        verified.push(verified_site);
    }
    Ok(VerifiedResumeSites::new(verified.into_boxed_slice()))
}

fn prove_stream_read(
    candidate: &LinkedBytecodeCandidate,
    row: &ExactResumeEntry,
    coordinate: PendingResumeCoordinate,
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
    let end_location = row
        .end_resume()
        .map(|end| instruction_location(coordinate.function, end));
    let resumed = flow
        .states_before
        .get(row.resume().get() as usize)
        .ok_or_else(|| violation(resume_location, "resume PC has no input state"))?;
    let end_resume = row.end_resume().ok_or_else(|| {
        violation(location, "StreamNext descriptor has no natural-end resume path")
    })?;
    if end_resume == row.resume() {
        return Err(violation(
            location,
            "StreamNext item and natural-end resume PCs are equal",
        ));
    }
    let ended = flow
        .states_before
        .get(end_resume.get() as usize)
        .ok_or_else(|| {
            violation(
                end_location.unwrap_or(location),
                "StreamNext end-resume PC has no input state",
            )
        })?;

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
    let mut expected_successors = [row.resume(), end_resume];
    expected_successors.sort_unstable_by_key(|target| target.get());
    let ordinary_successors = successor
        .iter()
        .filter(|edge| matches!(edge.kind, ControlFlowEdgeKind::Ordinary))
        .map(|edge| edge.target)
        .collect::<Vec<_>>();
    if row.resume() != expected_resume || ordinary_successors != expected_successors {
        return Err(violation(
            location,
            "StreamNext item/end CFG successors are not the exact resume PCs",
        ));
    }

    let endpoint = coordinate
        .endpoint_slot
        .ok_or_else(|| violation(location, "StreamNext has no endpoint slot"))?;
    let endpoint_value = live_slot(before, endpoint, location)?;
    let item = concrete.stream_item_type(endpoint_value, location)?;
    let declared_item = row.result_types()[0];
    if concrete.semantically_equal(item, declared_item) != Some(true)
        || !concrete.matches_declared_plan(item, &row.result_plans()[0])
    {
        return Err(violation(
            location,
            "resume result is not the independently derived Stream<T> item",
        ));
    }
    prove_stream_next_path_isomorphism(
        before,
        resumed,
        endpoint,
        item,
        true,
        concrete,
        resume_location,
    )?;
    prove_stream_next_path_isomorphism(
        before,
        ended,
        endpoint,
        item,
        false,
        concrete,
        end_location.unwrap_or(location),
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
            end_resume: Some(end_resume),
            expected_stack_height_before_result: row.expected_stack_height_before_result(),
            result_types: Box::new([item]),
            result_plans: Box::new([row.result_plans()[0].clone()]),
            error_mode: row.error_mode(),
            original_site,
            kind: VerifiedResumeKind::StreamRead {
                endpoint_slot: endpoint,
                item_type: item,
                end_resume,
            },
        },
    ))
}

#[allow(clippy::too_many_arguments)]
fn prove_pending_boundary(
    candidate: &LinkedBytecodeCandidate,
    row: &ExactResumeEntry,
    coordinate: PendingResumeCoordinate,
    concrete: &ConcreteValueFacts,
    control_flow: &ControlFlowFacts,
    targets: &ExactTargetAndCallFacts,
    source: &SourceAttributionFacts,
) -> Result<VerifiedResumeSite, VerificationError> {
    let location = instruction_location(coordinate.function, coordinate.site);
    if row.end_resume().is_some() {
        return Err(violation(
            location,
            "end resume is only valid for a StreamNext descriptor",
        ));
    }
    if row.error_mode() != ResumeErrorMode::RaiseAtSite {
        return Err(violation(
            location,
            "pending descriptor has the wrong error route",
        ));
    }
    let function = candidate
        .functions()
        .get(coordinate.function.get() as usize)
        .filter(|function| function.index() == coordinate.function)
        .ok_or_else(|| violation(location, "resume function is not dense"))?;
    let instruction = function
        .instructions()
        .get(coordinate.site.get() as usize)
        .ok_or_else(|| violation(location, "resume instruction is out of bounds"))?;
    let contract = contract_for_opcode(instruction.opcode());
    let consumed = input_arity(contract.typed.stack_in, function, instruction, location)?;
    let expected_results = expected_resume_results(
        targets,
        coordinate.function,
        coordinate.site,
        coordinate.mode,
        location,
    )?;
    if row.result_types().len() != expected_results.len()
        || row.result_plans().len() != expected_results.len()
    {
        return Err(violation(
            location,
            "pending descriptor result arity differs from the exact target signature",
        ));
    }
    for (ordinal, (actual, expected)) in row.result_types().iter().zip(expected_results).enumerate()
    {
        if concrete.semantically_equal(*actual, expected) != Some(true)
            || !concrete.matches_declared_plan(*actual, &row.result_plans()[ordinal])
        {
            return Err(violation(
                location,
                format!("pending result {ordinal} differs from the exact target result"),
            ));
        }
    }
    let flow = control_flow
        .functions
        .get(coordinate.function.get() as usize)
        .ok_or_else(|| violation(location, "resume function has no CFG facts"))?;
    let before = flow
        .states_before
        .get(coordinate.site.get() as usize)
        .ok_or_else(|| violation(location, "resume site has no input state"))?;
    let expected_height = before
        .stack
        .len()
        .checked_sub(consumed)
        .ok_or_else(|| violation(location, "resume stack height underflowed"))?;
    if row.expected_stack_height_before_result() as usize != expected_height {
        return Err(violation(
            location,
            "pending descriptor expected stack height is not the input prefix height",
        ));
    }
    let resumed = flow
        .states_before
        .get(row.resume().get() as usize)
        .ok_or_else(|| {
            violation(
                instruction_location(coordinate.function, row.resume()),
                "resume PC has no input state",
            )
        })?;
    prove_boundary_resume_isomorphism(
        before,
        resumed,
        consumed,
        row.result_types(),
        concrete,
        instruction_location(coordinate.function, row.resume()),
    )?;
    let original_site = source
        .current_site(coordinate.function, coordinate.site)
        .cloned()
        .ok_or_else(|| {
            violation(
                location,
                "pending site has no verified original source site",
            )
        })?;
    let kind = match coordinate.mode {
        PendingMode::StreamBackpressure => VerifiedResumeKind::StreamBackpressure,
        PendingMode::ServiceBoundary => VerifiedResumeKind::ServiceBoundary,
        PendingMode::ActorBoundary => VerifiedResumeKind::ActorBoundary,
        PendingMode::InterfaceBoundary => VerifiedResumeKind::InterfaceBoundary,
        PendingMode::CallbackBoundary => VerifiedResumeKind::CallbackBoundary,
        PendingMode::HostEffect => VerifiedResumeKind::HostEffect,
        PendingMode::StreamRead => {
            return Err(violation(
                location,
                "StreamRead must use the stream-read proof",
            ))
        }
    };
    Ok(VerifiedResumeSite::from_parts(
        crate::resume::VerifiedResumeSiteParts {
            index: row.index(),
            function: row.function(),
            site: row.site(),
            resume: row.resume(),
            end_resume: row.end_resume(),
            expected_stack_height_before_result: row.expected_stack_height_before_result(),
            result_types: row.result_types().into(),
            result_plans: row.result_plans().into(),
            error_mode: row.error_mode(),
            original_site,
            kind,
        },
    ))
}

fn expected_resume_results(
    targets: &ExactTargetAndCallFacts,
    function: FunctionIndex,
    site: InstructionIndex,
    mode: PendingMode,
    location: VerificationLocation,
) -> Result<Vec<TypeIndex>, VerificationError> {
    if mode == PendingMode::StreamBackpressure {
        return Ok(Vec::new());
    }
    targets
        .call_plan(function, site)
        .map(|plan| plan.results().iter().map(|result| result.ty()).collect())
        .ok_or_else(|| violation(location, "pending boundary has no exact call plan"))
}

fn input_arity(
    groups: &[TypedStackGroup],
    function: &LinkedFunction,
    instruction: &LinkedInstruction,
    location: VerificationLocation,
) -> Result<usize, VerificationError> {
    let contract = contract_for_opcode(instruction.opcode());
    let mut total = 0_usize;
    for group in groups {
        let count = match group.arity {
            Arity::Fixed(count) => usize::from(count),
            Arity::Declared(role) => contract
                .operand_word(role, instruction.operands())
                .and_then(|count| usize::try_from(count).ok())
                .ok_or_else(|| violation(location, "declared stack arity is absent"))?,
            Arity::FunctionResultCount => function.frame().result_types().len(),
        };
        total = total
            .checked_add(count)
            .ok_or_else(|| violation(location, "input arity overflowed usize"))?;
    }
    Ok(total)
}

fn prove_stream_next_path_isomorphism(
    before: &ProgramPointState,
    resumed: &ProgramPointState,
    endpoint_slot: FrameSlotIndex,
    item: TypeIndex,
    pushes_item: bool,
    concrete: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let result_count = if pushes_item { 1 } else { 0 };
    let expected_stack_len = before
        .stack
        .len()
        .checked_add(result_count)
        .ok_or_else(|| violation(location, "resume result stack height overflowed usize"))?;
    if resumed.stack.len() != expected_stack_len
        || resumed.slots.len() != before.slots.len()
        || resumed.active_regions != before.active_regions
        || resumed.writable_loans != before.writable_loans
    {
        return Err(violation(
            location,
            "StreamNext resume state shape differs from its ready state",
        ));
    }
    for (ready, resumed) in before.stack.iter().zip(resumed.stack.iter()) {
        if !same_value(*ready, *resumed, concrete) {
            return Err(violation(
                location,
                "StreamNext resume stack prefix differs from ready state",
            ));
        }
    }
    if pushes_item
        && !same_value(
            AbstractValue::Concrete(item),
            resumed.stack[before.stack.len()],
            concrete,
        )
    {
        return Err(violation(
            location,
            "StreamNext item resume result is not Stream<T> item T",
        ));
    }
    for (ordinal, (ready, resumed)) in before.slots.iter().zip(resumed.slots.iter()).enumerate() {
        if ordinal == endpoint_slot.get() as usize {
            if !matches!(resumed, AbstractSlotState::Moved) {
                return Err(violation(
                    location,
                    "stream endpoint is not moved/dead on every StreamNext successor",
                ));
            }
        } else if !same_slot(*ready, *resumed, concrete) {
            return Err(violation(
                location,
                "StreamNext resume slot state differs from ready state",
            ));
        }
    }
    Ok(())
}

fn prove_boundary_resume_isomorphism(
    before: &ProgramPointState,
    resumed: &ProgramPointState,
    consumed: usize,
    results: &[TypeIndex],
    concrete: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let expected_stack_len = before
        .stack
        .len()
        .checked_sub(consumed)
        .and_then(|prefix| prefix.checked_add(results.len()))
        .ok_or_else(|| violation(location, "boundary resume stack height overflowed usize"))?;
    if resumed.stack.len() != expected_stack_len
        || resumed.slots.len() != before.slots.len()
        || resumed.active_regions != before.active_regions
        || resumed.writable_loans != before.writable_loans
    {
        return Err(violation(
            location,
            "ready and resumed boundary states have different shapes",
        ));
    }
    let prefix = before.stack.len() - consumed;
    for (ready, resumed) in before.stack[..prefix]
        .iter()
        .zip(resumed.stack[..prefix].iter())
    {
        if !same_value(*ready, *resumed, concrete) {
            return Err(violation(
                location,
                "resume stack prefix differs from ready state",
            ));
        }
    }
    for (ordinal, result) in results.iter().enumerate() {
        let position = prefix + ordinal;
        if !same_value(
            AbstractValue::Concrete(*result),
            resumed.stack[position],
            concrete,
        ) {
            return Err(violation(
                location,
                format!("boundary resume result {ordinal} is not the exact target result"),
            ));
        }
    }
    for (ready, resumed) in before.slots.iter().zip(resumed.slots.iter()) {
        if !same_slot(*ready, *resumed, concrete) {
            return Err(violation(
                location,
                "boundary resume slot state differs from ready state",
            ));
        }
    }
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
) -> Result<TypeIndex, VerificationError> {
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
