use skiff_runtime_linked_bytecode::{
    FunctionIndex, LinkedBytecodeCandidate, LinkedCallableEffectDeclaration,
    LinkedExactLocalTarget, LinkedFunction, LinkedFunctionTables, LinkedSourceMapEntry,
    LinkedStackMapCandidate, LinkedStatementEntry, SpecializationKey,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::{
    identities::{callable, specialization},
    inout, instructions, resume, statements, EffectGraphCallKind, EffectGraphFunction,
};
use crate::tests::fixtures::candidate_parts;

pub(super) fn candidate(
    hydrated: &HydratedDeploymentBytecode,
    functions: &[EffectGraphFunction],
    swap_resume_targets: bool,
) -> LinkedBytecodeCandidate {
    let package = hydrated.packages().values().next().unwrap();
    let build = package.reference().package_build_id.clone();
    let keys = (0..functions.len())
        .map(|ordinal| specialization(&build, ordinal))
        .collect::<Vec<_>>();
    let resume_count = resume::descriptor_count(functions);
    let linked_functions = functions
        .iter()
        .enumerate()
        .map(|(ordinal, function)| {
            linked_function(
                ordinal,
                &keys,
                function,
                resume::descriptor_index(functions, ordinal),
                resume_count,
                swap_resume_targets,
            )
        })
        .collect();
    let mut parts = candidate_parts(hydrated, None, None);
    parts.functions = linked_functions;
    parts.exact_local_targets = keys
        .into_iter()
        .enumerate()
        .map(|(ordinal, key)| LinkedExactLocalTarget::new(key, FunctionIndex::new(ordinal as u32)))
        .collect();
    parts.resume_sites = resume::linked_sites(functions);
    inout::extend_linked_parts(&mut parts, &build, functions);
    LinkedBytecodeCandidate::try_from_parts(parts).unwrap()
}

fn linked_function(
    ordinal: usize,
    keys: &[SpecializationKey],
    function: &EffectGraphFunction,
    resume_index: u32,
    resume_count: u32,
    swap_resume_targets: bool,
) -> LinkedFunction {
    let resume_target =
        resume::selected_descriptor(resume_index, resume_count, swap_resume_targets);
    let mut instructions = match (function.call_kind, function.target) {
        (EffectGraphCallKind::Ordinary, Some(target)) => vec![
            instructions::linked_call(target),
            instructions::linked_budget(),
            instructions::linked_return(5),
        ],
        (EffectGraphCallKind::Tail, Some(target)) => {
            vec![instructions::linked_tail_call(target)]
        }
        (EffectGraphCallKind::Resume, None) => vec![
            resume::linked_emit_stream(resume_index, resume_target),
            instructions::linked_return(2),
        ],
        (EffectGraphCallKind::StreamRead, None) => vec![
            resume::linked_stream_next(resume_index, resume_target, 0),
            instructions::linked_pop(3),
            instructions::linked_return(4),
        ],
        (EffectGraphCallKind::StreamReadTwice, None) => vec![
            resume::linked_stream_next(resume_index, resume_target, 0),
            resume::linked_stream_next(
                resume_index + 1,
                resume::selected_descriptor(resume_index + 1, resume_count, swap_resume_targets),
                3,
            ),
            instructions::linked_pop(6),
            instructions::linked_pop(7),
            instructions::linked_return(8),
        ],
        (EffectGraphCallKind::InOut, Some(target)) => {
            vec![inout::linked_call(target), instructions::linked_return(5)]
        }
        (EffectGraphCallKind::Ordinary, None)
        | (EffectGraphCallKind::Tail, None)
        | (EffectGraphCallKind::InOut, None) => vec![instructions::linked_return(0)],
        (EffectGraphCallKind::Resume, Some(_))
        | (EffectGraphCallKind::StreamRead, Some(_))
        | (EffectGraphCallKind::StreamReadTwice, Some(_)) => {
            panic!("resume fixture cannot have a target")
        }
    };
    if function.trailing_return && matches!(function.call_kind, EffectGraphCallKind::Tail) {
        instructions.push(instructions::linked_return(3));
    }
    let states = resume::linked_states(function.call_kind, instructions.len());
    let max_operand_depth = resume::max_operand_depth(function.call_kind);
    let stack_map = LinkedStackMapCandidate::try_new(
        states.into_boxed_slice(),
        instructions.len(),
        inout::slot_count(function.call_kind),
        max_operand_depth,
    )
    .unwrap();
    let statement_entries = statement_entries(function);
    let source_map = source_map(function);
    LinkedFunction::new(
        FunctionIndex::new(ordinal as u32),
        keys[ordinal].clone(),
        instructions.into_boxed_slice(),
        inout::linked_frame(function.call_kind),
        max_operand_depth,
        LinkedCallableEffectDeclaration::new(callable(ordinal), function.summary.clone()),
        LinkedFunctionTables::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            inout::linked_loan_layouts(function.call_kind),
            statement_entries,
            source_map,
        ),
        stack_map,
    )
}

fn statement_entries(function: &EffectGraphFunction) -> Box<[LinkedStatementEntry]> {
    match (function.call_kind, function.target) {
        (EffectGraphCallKind::Ordinary, Some(_)) => statements::linked_entries(),
        (EffectGraphCallKind::Tail, Some(_)) => statements::linked_tail_entries(),
        (EffectGraphCallKind::InOut, Some(_)) => statements::linked_call_only_entries(),
        (EffectGraphCallKind::Resume, None)
        | (EffectGraphCallKind::StreamRead, None)
        | (EffectGraphCallKind::StreamReadTwice, None)
        | (EffectGraphCallKind::Ordinary, None)
        | (EffectGraphCallKind::Tail, None)
        | (EffectGraphCallKind::InOut, None) => Vec::new().into_boxed_slice(),
        (EffectGraphCallKind::Resume, Some(_))
        | (EffectGraphCallKind::StreamRead, Some(_))
        | (EffectGraphCallKind::StreamReadTwice, Some(_)) => {
            panic!("resume fixture cannot have a target")
        }
    }
}

fn source_map(function: &EffectGraphFunction) -> Box<[LinkedSourceMapEntry]> {
    match (function.call_kind, function.target) {
        (EffectGraphCallKind::Resume, None) => statements::linked_resume_source_map(),
        (EffectGraphCallKind::StreamRead, None) => statements::linked_stream_source_map(),
        (EffectGraphCallKind::StreamReadTwice, None) => {
            statements::linked_double_stream_source_map()
        }
        (EffectGraphCallKind::Ordinary, Some(_)) => statements::linked_source_map(),
        (EffectGraphCallKind::Tail, Some(_)) => statements::linked_tail_source_map(),
        (EffectGraphCallKind::InOut, Some(_)) => statements::linked_inout_source_map(),
        (EffectGraphCallKind::Ordinary, None)
        | (EffectGraphCallKind::Tail, None)
        | (EffectGraphCallKind::InOut, None) => Vec::new().into_boxed_slice(),
        (EffectGraphCallKind::Resume, Some(_))
        | (EffectGraphCallKind::StreamRead, Some(_))
        | (EffectGraphCallKind::StreamReadTwice, Some(_)) => {
            panic!("resume fixture cannot have a target")
        }
    }
}
