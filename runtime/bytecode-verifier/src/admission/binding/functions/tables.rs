use skiff_artifact_model::{ActiveRegionKind, CatchMatcher, ValidatedFunction};
use skiff_runtime_linked_bytecode::{
    CandidateTable, LinkedActiveRegionKind, LinkedBytecodeCandidate, LinkedCatchMatcher,
    LinkedFunction,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::{VerificationError, VerificationLocation};

use super::{boundary_index_for_pc, instruction_index_for_pc, prove_type_origin};
use crate::admission::binding::{semantic_violation, table_location};

pub(super) fn prove_function_tables(
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    source: &ValidatedFunction,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    prove_exception_regions(package, function, source, candidate)?;
    prove_active_regions(function, source)?;
    prove_switch_tables(package, function, source, candidate)?;
    prove_call_loan_layouts(function, source, candidate)?;
    prove_statement_entries(function, source)?;
    prove_source_map(function, source)
}

fn prove_exception_regions(
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    source: &ValidatedFunction,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let location = function_location(function);
    if function.exception_regions().len() != source.exception_regions.len() {
        return Err(semantic_violation(
            location,
            "linked exception-region coverage differs from the admitted function",
        ));
    }
    for (linked, artifact) in function
        .exception_regions()
        .iter()
        .zip(&source.exception_regions)
    {
        let exact = linked.start()
            == instruction_index_for_pc(source, artifact.start_pc, location)?
            && linked.end() == boundary_index_for_pc(source, artifact.end_pc, location)?
            && linked.handler() == instruction_index_for_pc(source, artifact.handler_pc, location)?
            && linked.handler_stack_height() == artifact.handler_stack_height
            && linked.catch_slot().get() == artifact.catch_slot
            && linked.cleanup_depth() == artifact.cleanup_depth
            && linked.catch_matchers().len() == artifact.catch_matchers.len();
        if !exact {
            return Err(semantic_violation(
                location,
                "linked exception-region row differs from its admitted artifact row",
            ));
        }
        prove_type_origin(
            candidate,
            linked.catch_slot_type(),
            package,
            artifact.catch_slot_type_ref,
            function.key(),
            location,
        )?;
        for (linked, artifact) in linked.catch_matchers().iter().zip(&artifact.catch_matchers) {
            match (linked, artifact) {
                (LinkedCatchMatcher::CatchAll, CatchMatcher::CatchAll) => {}
                (LinkedCatchMatcher::Type(ty), CatchMatcher::TypeRef { type_ref }) => {
                    prove_type_origin(
                        candidate,
                        *ty,
                        package,
                        *type_ref,
                        function.key(),
                        location,
                    )?;
                }
                _ => {
                    return Err(semantic_violation(
                        location,
                        "linked catch matcher differs from its admitted artifact matcher",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn prove_active_regions(
    function: &LinkedFunction,
    source: &ValidatedFunction,
) -> Result<(), VerificationError> {
    let location = function_location(function);
    if function.active_regions().len() != source.active_regions.len() {
        return Err(semantic_violation(
            location,
            "linked active-region coverage differs from the admitted function",
        ));
    }
    for (row, (linked, artifact)) in function
        .active_regions()
        .iter()
        .zip(&source.active_regions)
        .enumerate()
    {
        let index = u32::try_from(row)
            .map_err(|_| semantic_violation(location, "active-region index does not fit u32"))?;
        let exact_kind = matches!(
            (linked.kind(), &artifact.kind),
            (
                LinkedActiveRegionKind::Timeout {
                    duration_ms: linked_duration,
                    site: linked_site,
                },
                ActiveRegionKind::Timeout {
                    duration_ms: artifact_duration,
                    site: artifact_site,
                }
            ) if linked_duration == artifact_duration && linked_site == artifact_site
        );
        let exact = linked.index().get() == index
            && linked.start() == instruction_index_for_pc(source, artifact.start_pc, location)?
            && linked.end() == boundary_index_for_pc(source, artifact.end_pc, location)?
            && exact_kind;
        if !exact {
            return Err(semantic_violation(
                location,
                "linked active-region row differs from its admitted artifact row",
            ));
        }
    }
    Ok(())
}

fn prove_switch_tables(
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    source: &ValidatedFunction,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let location = function_location(function);
    if function.switch_tables().len() != source.switch_tables.len() {
        return Err(semantic_violation(
            location,
            "linked switch-table coverage differs from the admitted function",
        ));
    }
    for (linked, artifact) in function.switch_tables().iter().zip(&source.switch_tables) {
        if linked.cases().len() != artifact.cases.len()
            || linked.default_target()
                != instruction_index_for_pc(source, artifact.default_pc, location)?
        {
            return Err(semantic_violation(
                location,
                "linked switch table differs from its admitted artifact row",
            ));
        }
        for (linked, artifact) in linked.cases().iter().zip(&artifact.cases) {
            if linked.target() != instruction_index_for_pc(source, artifact.target_pc, location)? {
                return Err(semantic_violation(
                    location,
                    "linked switch target differs from its admitted artifact target",
                ));
            }
            prove_type_origin(
                candidate,
                linked.tag_type(),
                package,
                artifact.tag_type_ref,
                function.key(),
                location,
            )?;
        }
    }
    Ok(())
}

fn prove_call_loan_layouts(
    function: &LinkedFunction,
    source: &ValidatedFunction,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let location = function_location(function);
    if function.call_loan_layouts().len() != source.call_loan_layouts.len() {
        return Err(semantic_violation(
            location,
            "linked call-loan coverage differs from the admitted function",
        ));
    }
    for (row, (linked, artifact)) in function
        .call_loan_layouts()
        .iter()
        .zip(&source.call_loan_layouts)
        .enumerate()
    {
        let index = u32::try_from(row)
            .map_err(|_| semantic_violation(location, "call-loan index does not fit u32"))?;
        if linked.index().get() != index || linked.loans().len() != artifact.loans.len() {
            return Err(semantic_violation(
                location,
                "linked call-loan row differs from its admitted artifact row",
            ));
        }
        for (linked, artifact) in linked.loans().iter().zip(&artifact.loans) {
            let path = candidate
                .writable_paths()
                .get(linked.writable_path().get() as usize)
                .ok_or_else(|| {
                    semantic_violation(location, "call-loan writable path is out of bounds")
                })?;
            let exact = linked.parameter_ordinal() == artifact.parameter_ordinal
                && linked.root_slot().get() == artifact.root_slot
                && path.origin().package_build_id() == function.key().package_build_id()
                && path.origin().artifact_index().get() == artifact.writable_path_ref
                && path
                    .origin()
                    .specialization()
                    .is_none_or(|specialization| specialization == function.key());
            if !exact {
                return Err(semantic_violation(
                    location,
                    "linked call-loan binding differs from its admitted artifact binding",
                ));
            }
        }
    }
    Ok(())
}

fn prove_statement_entries(
    function: &LinkedFunction,
    source: &ValidatedFunction,
) -> Result<(), VerificationError> {
    let location = function_location(function);
    if function.statement_entries().len() != source.statement_entries.len() {
        return Err(semantic_violation(
            location,
            "linked statement-table coverage differs from the admitted function",
        ));
    }
    for (linked, artifact) in function
        .statement_entries()
        .iter()
        .zip(&source.statement_entries)
    {
        let exact = linked.instruction()
            == instruction_index_for_pc(source, artifact.pc, location)?
            && linked.statement_id() == artifact.statement_id
            && linked.charge_kind() == artifact.charge_kind;
        if !exact {
            return Err(semantic_violation(
                location,
                "linked statement entry differs from its admitted artifact row",
            ));
        }
    }
    Ok(())
}

fn prove_source_map(
    function: &LinkedFunction,
    source: &ValidatedFunction,
) -> Result<(), VerificationError> {
    let location = function_location(function);
    if function.source_map().len() != source.source_map.len() {
        return Err(semantic_violation(
            location,
            "linked source-map coverage differs from the admitted function",
        ));
    }
    for (linked, artifact) in function.source_map().iter().zip(&source.source_map) {
        let exact = linked.start()
            == instruction_index_for_pc(source, artifact.start_pc, location)?
            && linked.end() == boundary_index_for_pc(source, artifact.end_pc, location)?
            && linked.site() == &artifact.site;
        if !exact {
            return Err(semantic_violation(
                location,
                "linked source-map entry differs from its admitted artifact row",
            ));
        }
    }
    Ok(())
}

pub(super) fn prove_resume_sites(
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    source: &ValidatedFunction,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let artifact_rows = package
        .bytecode()
        .view()
        .resume_sites()
        .iter()
        .filter(|resume| resume.function_key == source.function_key)
        .collect::<Vec<_>>();
    let linked_rows = candidate
        .resume_sites()
        .iter()
        .filter(|resume| resume.function() == function.index())
        .collect::<Vec<_>>();
    let location = function_location(function);
    if linked_rows.len() != artifact_rows.len() {
        return Err(semantic_violation(
            location,
            "linked resume-site coverage differs from the admitted function",
        ));
    }
    for artifact in artifact_rows {
        let site = instruction_index_for_pc(source, artifact.site_pc, location)?;
        let linked = linked_rows
            .iter()
            .copied()
            .find(|resume| resume.site() == site)
            .ok_or_else(|| {
                semantic_violation(location, "admitted resume site has no linked row")
            })?;
        let row_location = table_location(CandidateTable::ResumeSites, linked.index().get());
        let exact = linked.resume()
            == instruction_index_for_pc(source, artifact.resume_pc, row_location)?
            && linked.expected_stack_height_before_result()
                == artifact.expected_stack_height_before_result
            && linked.error_mode() == artifact.error_mode
            && linked.result_types().len() == artifact.result_type_refs.len()
            && linked.result_plans().len() == artifact.result_plans.len();
        if !exact {
            return Err(semantic_violation(
                row_location,
                "linked resume-site row differs from its admitted descriptor",
            ));
        }
        for (ty, artifact_index) in linked.result_types().iter().zip(&artifact.result_type_refs) {
            prove_type_origin(
                candidate,
                *ty,
                package,
                *artifact_index,
                function.key(),
                row_location,
            )?;
        }
    }
    Ok(())
}

const fn function_location(function: &LinkedFunction) -> VerificationLocation {
    VerificationLocation::Function {
        function: function.index(),
    }
}
