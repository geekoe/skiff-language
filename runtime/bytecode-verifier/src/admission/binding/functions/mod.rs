mod effects;
mod instructions;
mod tables;

use skiff_artifact_model::ValidatedFunction;
use skiff_runtime_linked_bytecode::{
    CandidateTable, InstructionBoundaryIndex, InstructionIndex, LinkedBytecodeCandidate,
    LinkedFunction, SpecializationKey, TypeIndex,
};
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

use crate::admission::facts::ExactFunctionEffectBinding;
use crate::admission::facts::ExactFunctionStatementBinding;
use crate::{VerificationError, VerificationLocation};

use super::{row_u32, semantic_violation, table_location, TargetCoverage};

pub(super) struct ProvedFunctionBindings {
    pub(super) coverage: TargetCoverage,
    pub(super) statements: Vec<ExactFunctionStatementBinding>,
    pub(super) effects: Vec<ExactFunctionEffectBinding>,
}

pub(super) fn prove_functions(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<ProvedFunctionBindings, VerificationError> {
    prove_exact_local_target_coverage(hydrated, candidate)?;
    let mut coverage = TargetCoverage::default();
    let mut statement_functions = Vec::with_capacity(candidate.functions().len());
    let mut effect_functions = Vec::with_capacity(candidate.functions().len());
    for function in candidate.functions() {
        let location = VerificationLocation::Function {
            function: function.index(),
        };
        let package = hydrated
            .packages()
            .get(function.key().package_build_id())
            .ok_or_else(|| {
                semantic_violation(location, "function owner package is not hydrated")
            })?;
        let source = source_function(package, function.key()).ok_or_else(|| {
            semantic_violation(
                location,
                format!(
                    "artifact function {:?} is absent from exact package {}",
                    function.key().artifact_function_key().as_str(),
                    package.reference().package_build_id
                ),
            )
        })?;
        prove_function_identity(package, function, source, candidate)?;
        effect_functions.push(effects::prove_exact_effect_binding(
            package, function, source,
        )?);
        prove_frame(function, source, candidate)?;
        statement_functions.push(tables::prove_function_tables(
            package, function, source, candidate,
        )?);
        tables::prove_resume_sites(package, function, source, candidate)?;
        instructions::prove_instructions(
            hydrated,
            package,
            function,
            source,
            candidate,
            &mut coverage,
        )?;
    }
    Ok(ProvedFunctionBindings {
        coverage,
        statements: statement_functions,
        effects: effect_functions,
    })
}

fn prove_exact_local_target_coverage(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let mut ordinary_count = 0_usize;
    for function in candidate.functions() {
        let location = VerificationLocation::Function {
            function: function.index(),
        };
        let source = hydrated
            .packages()
            .get(function.key().package_build_id())
            .and_then(|package| source_function(package, function.key()))
            .ok_or_else(|| {
                semantic_violation(
                    location,
                    "exact-local coverage cannot find the admitted function",
                )
            })?;
        if source.origin.ordinary_executable().is_some() {
            ordinary_count = ordinary_count.checked_add(1).ok_or_else(|| {
                semantic_violation(location, "ordinary function count overflowed usize")
            })?;
        }
    }
    if candidate.exact_local_targets().len() != ordinary_count {
        return Err(semantic_violation(
            VerificationLocation::Image,
            format!(
                "exact-local target table has {} rows for {ordinary_count} ordinary concrete functions",
                candidate.exact_local_targets().len(),
            ),
        ));
    }
    for (row_number, target) in candidate.exact_local_targets().iter().enumerate() {
        let location = table_location(
            CandidateTable::ExactLocalTargets,
            row_u32(CandidateTable::ExactLocalTargets, row_number)?,
        );
        let function = candidate
            .functions()
            .get(target.function().get() as usize)
            .ok_or_else(|| {
                semantic_violation(location, "exact-local function index is out of bounds")
            })?;
        if function.key() != target.key() {
            return Err(semantic_violation(
                location,
                "exact-local target key disagrees with its concrete function",
            ));
        }
    }
    for function in candidate.functions() {
        let is_ordinary = hydrated
            .packages()
            .get(function.key().package_build_id())
            .and_then(|package| source_function(package, function.key()))
            .is_some_and(|source| source.origin.ordinary_executable().is_some());
        if !is_ordinary {
            continue;
        }
        if !candidate
            .exact_local_targets()
            .iter()
            .any(|target| target.function() == function.index() && target.key() == function.key())
        {
            return Err(semantic_violation(
                VerificationLocation::Function {
                    function: function.index(),
                },
                "concrete function has no exact-local target row",
            ));
        }
    }
    Ok(())
}

fn prove_function_identity(
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    source: &ValidatedFunction,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let location = VerificationLocation::Function {
        function: function.index(),
    };
    if source.origin.ordinary_executable().is_none() {
        return Err(VerificationError::ProofUnavailable {
            obligation: crate::VerificationObligation::ExactHydrationBinding,
            location,
        });
    }
    let canonical_callable = package
        .canonical_implementation_callable_for_function_key(&source.function_key)
        .ok_or_else(|| {
            semantic_violation(
                location,
                "ordinary function has no canonical implementation callable authority",
            )
        })?;
    let selected_function =
        package.function_key_for_callable(function.key().template_function_key());
    let exact = function.key().artifact_function_key().as_str() == source.function_key
        && selected_function == Some(source.function_key.as_str())
        && function.key().concrete_type_arguments().len() == source.type_parameters.len()
        && function.key().concrete_receiver().is_some() == source.self_type_ref.is_some();
    if !exact {
        return Err(semantic_violation(
            location,
            "function key or specialization shape differs from the admitted artifact",
        ));
    }
    if package.function_key_for_canonical_implementation_callable(canonical_callable)
        != Some(source.function_key.as_str())
    {
        return Err(semantic_violation(
            location,
            "ordinary function specialization is not bound to its exact callable manifest",
        ));
    }
    if let (Some(receiver), Some(source_receiver)) =
        (function.key().concrete_receiver(), source.self_type_ref)
    {
        prove_type_origin(
            candidate,
            receiver,
            package,
            source_receiver,
            function.key(),
            location,
        )?;
    }
    Ok(())
}

fn prove_frame(
    function: &LinkedFunction,
    source: &ValidatedFunction,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let location = VerificationLocation::Function {
        function: function.index(),
    };
    let linked = function.frame();
    let artifact = &source.frame_layout;
    let exact_counts = linked.slot_types().len() == artifact.slot_count as usize
        && linked.slot_types().len() == artifact.slot_type_refs.len()
        && linked.parameters().len() == artifact.parameter_slots.len()
        && linked.writable_local_slots().len() == artifact.writable_local_slots.len()
        && linked.result_types().len() == artifact.result_count as usize
        && linked.result_types().len() == artifact.result_type_refs.len()
        && function.max_operand_depth() == source.max_operand_depth;
    if !exact_counts {
        return Err(semantic_violation(
            location,
            "linked frame shape or declared operand depth differs from the admitted function",
        ));
    }
    for (linked, artifact) in linked.parameters().iter().zip(&artifact.parameter_slots) {
        if linked.slot().get() != artifact.slot || linked.mode() != artifact.mode {
            return Err(semantic_violation(
                location,
                "linked parameter slot or calling mode differs from the admitted frame",
            ));
        }
    }
    if linked
        .writable_local_slots()
        .iter()
        .map(|slot| slot.get())
        .ne(artifact.writable_local_slots.iter().copied())
    {
        return Err(semantic_violation(
            location,
            "linked writable-local slots differ from the admitted frame",
        ));
    }
    let package = function.key().package_build_id();
    for (ty, artifact_index) in linked.slot_types().iter().zip(&artifact.slot_type_refs) {
        prove_type_origin_by_build(
            candidate,
            *ty,
            package,
            *artifact_index,
            function.key(),
            location,
        )?;
    }
    for (ty, artifact_index) in linked.result_types().iter().zip(&artifact.result_type_refs) {
        prove_type_origin_by_build(
            candidate,
            *ty,
            package,
            *artifact_index,
            function.key(),
            location,
        )?;
    }
    Ok(())
}

pub(super) fn source_function<'a>(
    package: &'a HydratedBytecodePackage,
    key: &SpecializationKey,
) -> Option<&'a ValidatedFunction> {
    package
        .bytecode()
        .view()
        .functions()
        .iter()
        .find(|function| function.function_key == key.artifact_function_key().as_str())
}

pub(super) fn prove_type_origin(
    candidate: &LinkedBytecodeCandidate,
    ty: TypeIndex,
    package: &HydratedBytecodePackage,
    artifact_index: u32,
    specialization: &SpecializationKey,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    prove_type_origin_by_build(
        candidate,
        ty,
        &package.reference().package_build_id,
        artifact_index,
        specialization,
        location,
    )
}

pub(super) fn prove_type_origin_by_build(
    candidate: &LinkedBytecodeCandidate,
    ty: TypeIndex,
    package_build_id: &skiff_artifact_model::PackageBuildId,
    artifact_index: u32,
    specialization: &SpecializationKey,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let linked = candidate
        .types()
        .get(ty.get() as usize)
        .ok_or_else(|| semantic_violation(location, "linked type index is out of bounds"))?;
    let exact_specialization = linked
        .origin()
        .specialization()
        .is_none_or(|origin| origin == specialization);
    if linked.origin().package_build_id() != package_build_id
        || linked.origin().artifact_index().get() != artifact_index
        || !exact_specialization
    {
        return Err(semantic_violation(
            location,
            "linked type does not carry the exact artifact row and specialization origin",
        ));
    }
    Ok(())
}

pub(super) fn instruction_index_for_pc(
    source: &ValidatedFunction,
    pc: u32,
    location: VerificationLocation,
) -> Result<InstructionIndex, VerificationError> {
    let position = source.header_pcs.binary_search(&pc).map_err(|_| {
        semantic_violation(
            location,
            format!("artifact pc {pc} is not an instruction header"),
        )
    })?;
    let position = u32::try_from(position)
        .map_err(|_| semantic_violation(location, "instruction index does not fit u32"))?;
    Ok(InstructionIndex::new(position))
}

pub(super) fn boundary_index_for_pc(
    source: &ValidatedFunction,
    pc: u32,
    location: VerificationLocation,
) -> Result<InstructionBoundaryIndex, VerificationError> {
    let word_count = u32::try_from(source.words.len())
        .map_err(|_| semantic_violation(location, "function word count does not fit u32"))?;
    if pc == word_count {
        let end = u32::try_from(source.instructions.len())
            .map_err(|_| semantic_violation(location, "instruction boundary does not fit u32"))?;
        return Ok(InstructionBoundaryIndex::new(end));
    }
    Ok(InstructionBoundaryIndex::new(
        instruction_index_for_pc(source, pc, location)?.get(),
    ))
}
