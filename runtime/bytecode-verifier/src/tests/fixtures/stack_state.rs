use skiff_artifact_model::{
    CallableEffectSummary, Opcode, PackageCallableId, ParamModeIr, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{
    ArtifactFunctionKey, ArtifactTypeIndex, FrameSlotIndex, FunctionIndex, InstructionIndex,
    LinkedArtifactPoolOrigin, LinkedBytecodeCandidate, LinkedCallableEffectDeclaration,
    LinkedFrameLayout, LinkedFunction, LinkedFunctionTables, LinkedInstruction,
    LinkedInstructionTarget, LinkedParameterSlot, LinkedProgramPointState, LinkedResolvedOperand,
    LinkedSlotState, LinkedStackMapCandidate, LinkedStackValue, LinkedTypeEntry,
    LinkedValueDropPlan, LinkedValueTransferPlan, SpecializationKey, TypeIndex,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::{candidate_parts, exact_hydration_with_types};

mod calls;
mod constants;

pub(crate) use calls::{
    call_argument_mismatch_fixture, call_result_fixture, tail_argument_mismatch_fixture,
    tail_copy_live_fixture, tail_live_cleanup_fixture, tail_residue_fixture, tail_result_fixture,
};
pub(crate) use constants::constant_fixture;

pub(crate) struct StackFixture {
    pub(crate) hydrated: HydratedDeploymentBytecode,
    pub(crate) candidate: LinkedBytecodeCandidate,
}

pub(crate) struct FunctionSpec {
    pub(crate) slots: Vec<u32>,
    pub(crate) parameters: Vec<(u32, ParamModeIr)>,
    pub(crate) writable: Vec<u32>,
    pub(crate) results: Vec<u32>,
    pub(crate) instructions: Vec<LinkedInstruction>,
    pub(crate) declared_max: u32,
    pub(crate) hints: Option<Vec<Hint>>,
}

#[derive(Clone)]
pub(crate) struct Hint {
    pub(crate) stack: Vec<u32>,
    pub(crate) slots: Vec<HintSlot>,
}

#[derive(Clone, Copy)]
pub(crate) enum HintSlot {
    Uninitialized,
    Moved,
    Live(u32),
}

pub(crate) fn fixture(types: Vec<TypeRefIr>, spec: FunctionSpec) -> StackFixture {
    let hydrated = exact_hydration_with_types(types.clone());
    let mut parts = candidate_parts(&hydrated, None, None);
    let build = hydrated
        .packages()
        .values()
        .next()
        .expect("stack fixture package is hydrated")
        .reference()
        .package_build_id
        .clone();
    parts.types = linked_types(&build, &types);
    parts.functions = vec![linked_function(
        FunctionIndex::new(0),
        ordinary_key(&build, 0),
        PackageCallableId::new("fixture:stack:0"),
        CallableEffectSummary::analysis_pending(),
        spec,
        &types,
    )];
    StackFixture {
        hydrated,
        candidate: LinkedBytecodeCandidate::try_from_parts(parts)
            .expect("stack fixture passes candidate-local validation"),
    }
}

pub(crate) fn plain(opcode: Opcode) -> LinkedInstruction {
    LinkedInstruction::new(opcode, Box::new([]), Box::new([]), 0)
        .expect("operand-free test instruction is valid")
}

pub(crate) fn slot_instruction(opcode: Opcode, slot: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        opcode,
        Box::new([slot]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::FrameSlot(FrameSlotIndex::new(slot)),
        )]),
        0,
    )
    .expect("one-slot test instruction is valid")
}

pub(crate) fn two_slot_instruction(
    opcode: Opcode,
    source: u32,
    destination: u32,
) -> LinkedInstruction {
    LinkedInstruction::new(
        opcode,
        Box::new([source, destination]),
        Box::new([
            LinkedResolvedOperand::new(
                0,
                LinkedInstructionTarget::FrameSlot(FrameSlotIndex::new(source)),
            ),
            LinkedResolvedOperand::new(
                1,
                LinkedInstructionTarget::FrameSlot(FrameSlotIndex::new(destination)),
            ),
        ]),
        0,
    )
    .expect("two-slot test instruction is valid")
}

pub(crate) fn branch(opcode: Opcode, target: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        opcode,
        Box::new([target]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Branch(InstructionIndex::new(target)),
        )]),
        0,
    )
    .expect("branch test instruction is valid")
}

fn call_local(argument_count: u32, result_count: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        Opcode::CallLocal,
        Box::new([0, argument_count, result_count]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Function(FunctionIndex::new(1)),
        )]),
        0,
    )
    .expect("local-call test instruction is valid")
}

fn tail_call(argument_count: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        Opcode::TailCallLocal,
        Box::new([0, argument_count]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Function(FunctionIndex::new(1)),
        )]),
        0,
    )
    .expect("tail-call test instruction is valid")
}

fn linked_types(
    build: &skiff_artifact_model::PackageBuildId,
    types: &[TypeRefIr],
) -> Vec<LinkedTypeEntry> {
    types
        .iter()
        .cloned()
        .enumerate()
        .map(|(ordinal, ty)| {
            let index = u32::try_from(ordinal).expect("test type ordinal fits u32");
            LinkedTypeEntry::new(
                TypeIndex::new(index),
                LinkedArtifactPoolOrigin::new(build.clone(), ArtifactTypeIndex::new(index), None)
                    .expect("test type origin is valid"),
                ty,
                None,
            )
        })
        .collect()
}

fn linked_function(
    index: FunctionIndex,
    key: SpecializationKey,
    effect_owner: PackageCallableId,
    effect: CallableEffectSummary,
    spec: FunctionSpec,
    types: &[TypeRefIr],
) -> LinkedFunction {
    let slot_types = spec
        .slots
        .iter()
        .copied()
        .map(TypeIndex::new)
        .collect::<Vec<_>>();
    let slot_plans = spec
        .slots
        .iter()
        .map(|index| plan_for(&types[*index as usize]))
        .collect::<Vec<_>>();
    let parameters = spec
        .parameters
        .iter()
        .map(|(slot, mode)| {
            LinkedParameterSlot::new(
                FrameSlotIndex::new(*slot),
                *mode,
                slot_plans[*slot as usize].clone(),
            )
        })
        .collect::<Vec<_>>();
    let result_types = spec
        .results
        .iter()
        .copied()
        .map(TypeIndex::new)
        .collect::<Vec<_>>();
    let result_plans = spec
        .results
        .iter()
        .map(|index| plan_for(&types[*index as usize]))
        .collect::<Vec<_>>();
    let writable = spec
        .writable
        .iter()
        .copied()
        .map(FrameSlotIndex::new)
        .collect::<Vec<_>>();
    let hints = spec.hints.unwrap_or_else(|| {
        vec![
            Hint {
                stack: Vec::new(),
                slots: vec![HintSlot::Uninitialized; slot_types.len()],
            };
            spec.instructions.len()
        ]
    });
    let stack_map = stack_map(hints, &slot_plans, types, spec.declared_max);
    LinkedFunction::new(
        index,
        key,
        spec.instructions.into_boxed_slice(),
        LinkedFrameLayout::new(
            slot_types.into_boxed_slice(),
            parameters.into_boxed_slice(),
            writable.into_boxed_slice(),
            result_types.into_boxed_slice(),
            slot_plans.into_boxed_slice(),
            result_plans.into_boxed_slice(),
        )
        .expect("test frame layout is valid"),
        spec.declared_max,
        LinkedCallableEffectDeclaration::new(effect_owner, effect),
        LinkedFunctionTables::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
        ),
        stack_map,
    )
}

fn stack_map(
    hints: Vec<Hint>,
    slot_plans: &[LinkedValueTransferPlan],
    types: &[TypeRefIr],
    declared_max: u32,
) -> LinkedStackMapCandidate {
    let slot_count = slot_plans.len();
    let states = hints
        .into_iter()
        .enumerate()
        .map(|(ordinal, hint)| {
            let stack = hint
                .stack
                .into_iter()
                .map(|ty| stack_value(ty, types))
                .collect::<Vec<_>>();
            let slots = hint
                .slots
                .into_iter()
                .map(|slot| match slot {
                    HintSlot::Uninitialized => LinkedSlotState::Uninitialized,
                    HintSlot::Moved => LinkedSlotState::Moved,
                    HintSlot::Live(ty) => LinkedSlotState::Live(stack_value(ty, types)),
                })
                .collect::<Vec<_>>();
            LinkedProgramPointState::new(
                InstructionIndex::new(u32::try_from(ordinal).expect("test ordinal fits u32")),
                stack.into_boxed_slice(),
                slots.into_boxed_slice(),
                Box::new([]),
                Box::new([]),
            )
        })
        .collect::<Vec<_>>();
    let instruction_count = states.len();
    LinkedStackMapCandidate::try_new(
        states.into_boxed_slice(),
        instruction_count,
        slot_count,
        declared_max,
    )
    .expect("test stack map is structurally valid")
}

fn stack_value(ty: u32, types: &[TypeRefIr]) -> LinkedStackValue {
    LinkedStackValue::new(TypeIndex::new(ty), plan_for(&types[ty as usize]))
}

fn ordinary_key(build: &skiff_artifact_model::PackageBuildId, ordinal: u32) -> SpecializationKey {
    SpecializationKey::new(
        build.clone(),
        ArtifactFunctionKey::parse(format!("fixture::stack_{ordinal}"))
            .expect("test function key is valid"),
        PackageCallableId::new(format!("fixture:stack:{ordinal}")),
        Box::new([]),
        None,
    )
}

fn plan_for(ty: &TypeRefIr) -> LinkedValueTransferPlan {
    let TypeRefIr::Builtin { name, .. } = ty else {
        panic!("stack fixture supports only builtin types")
    };
    match name.as_str() {
        "null" | "bool" | "number" | "integer" | "Date" => LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        },
        "string" | "bytes" | "Json" | "JsonObject" | "Array" | "Map" => {
            LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::SnapshotRelease,
            }
        }
        "Stream" => LinkedValueTransferPlan::AffineResource {
            drop: skiff_runtime_linked_bytecode::LinkedResourceDropPlan::ResourceTableRelease,
        },
        _ => panic!("unsupported stack fixture builtin {name}"),
    }
}

pub(crate) fn stream_of_string() -> TypeRefIr {
    TypeRefIr::Builtin {
        name: "Stream".to_string(),
        args: vec![TypeRefIr::builtin("string")],
    }
}
