use skiff_artifact_model::{Opcode, ParamModeIr, TypeRefIr};
use skiff_runtime_linked_bytecode::{FunctionIndex, LinkedBytecodeCandidate};

use super::{
    call_local, candidate_parts, linked_function, linked_types, plain, slot_instruction,
    FunctionSpec, Hint, HintSlot, StackFixture,
};
use crate::tests::fixtures::{loader_backed_local_call, LocalCallCandidateCorruption};

pub(crate) fn call_argument_mismatch_fixture() -> StackFixture {
    authority_fixture(
        FunctionSpec {
            slots: vec![0],
            parameters: vec![(0, ParamModeIr::Value)],
            writable: Vec::new(),
            results: Vec::new(),
            instructions: vec![
                slot_instruction(Opcode::TakeSlot, 0),
                call_local(1, 0),
                plain(Opcode::Return),
            ],
            declared_max: 1,
            hints: None,
        },
        FunctionSpec {
            slots: vec![2],
            parameters: vec![(0, ParamModeIr::Value)],
            writable: Vec::new(),
            results: Vec::new(),
            instructions: vec![plain(Opcode::Return)],
            declared_max: 0,
            hints: None,
        },
    )
}

pub(crate) fn call_result_fixture() -> StackFixture {
    authority_fixture(
        FunctionSpec {
            slots: vec![0],
            parameters: vec![(0, ParamModeIr::Value)],
            writable: Vec::new(),
            results: vec![0],
            instructions: vec![
                slot_instruction(Opcode::TakeSlot, 0),
                call_local(1, 1),
                plain(Opcode::Return),
            ],
            declared_max: 1,
            hints: Some(vec![
                hint(&[], &[HintSlot::Live(0)]),
                hint(&[0], &[HintSlot::Moved]),
                hint(&[1], &[HintSlot::Moved]),
            ]),
        },
        FunctionSpec {
            slots: vec![1],
            parameters: vec![(0, ParamModeIr::Value)],
            writable: Vec::new(),
            results: vec![1],
            instructions: vec![slot_instruction(Opcode::TakeSlot, 0), plain(Opcode::Return)],
            declared_max: 1,
            hints: Some(vec![
                hint(&[], &[HintSlot::Live(1)]),
                hint(&[1], &[HintSlot::Moved]),
            ]),
        },
    )
}

fn authority_fixture(caller_spec: FunctionSpec, target_spec: FunctionSpec) -> StackFixture {
    let (hydrated, authority) = loader_backed_local_call(LocalCallCandidateCorruption::None);
    let types = vec![
        TypeRefIr::builtin("string"),
        TypeRefIr::builtin("string"),
        TypeRefIr::builtin("bytes"),
    ];
    let build = hydrated
        .packages()
        .values()
        .next()
        .expect("local-call package is hydrated")
        .reference()
        .package_build_id
        .clone();
    let caller = &authority.functions()[0];
    let target = &authority.functions()[1];
    let functions = vec![
        linked_function(
            FunctionIndex::new(0),
            caller.key().clone(),
            caller.effect_summary_ref().clone(),
            caller.declarative_effect_summary().clone(),
            caller_spec,
            &types,
        ),
        linked_function(
            FunctionIndex::new(1),
            target.key().clone(),
            target.effect_summary_ref().clone(),
            target.declarative_effect_summary().clone(),
            target_spec,
            &types,
        ),
    ];
    let mut parts = candidate_parts(&hydrated, None, None);
    parts.types = linked_types(&build, &types);
    parts.functions = functions;
    parts.exact_local_targets = authority.exact_local_targets().to_vec();
    StackFixture {
        hydrated,
        candidate: LinkedBytecodeCandidate::try_from_parts(parts)
            .expect("isolated local-call fixture passes local validation"),
    }
}

fn hint(stack: &[u32], slots: &[HintSlot]) -> Hint {
    Hint {
        stack: stack.to_vec(),
        slots: slots.to_vec(),
    }
}
