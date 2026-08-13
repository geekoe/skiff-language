use std::collections::BTreeMap;

use skiff_artifact_model::{TypeRefIr, ValidatedFunction};
use skiff_runtime_linked_bytecode::{
    ActiveRegionIndex, CallLoanLayoutIndex, FrameSlotIndex, InstructionBoundaryIndex,
    InstructionIndex, LinkedActiveRegion, LinkedActiveRegionKind, LinkedCallLoanBinding,
    LinkedCallLoanLayout, LinkedCatchMatcher, LinkedExceptionRegion, LinkedFunctionTables,
    LinkedSourceMapEntry, LinkedStatementEntry, LinkedSwitchCase, LinkedSwitchTable,
    SpecializationKey,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{
    types::TypeLinker, BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation,
};

use super::{unsatisfied, DeploymentLinker};

impl DeploymentLinker<'_> {
    pub(super) fn link_function_tables(
        &self,
        package: &HydratedBytecodePackage,
        function: &ValidatedFunction,
        specialization: &SpecializationKey,
        type_linker: &mut TypeLinker<'_>,
        substitutions: &BTreeMap<String, TypeRefIr>,
    ) -> Result<LinkedFunctionTables, BytecodeLinkError> {
        let location = self.function_location(package, function);
        let exception_regions = link_exception_regions(
            package,
            function,
            specialization,
            type_linker,
            substitutions,
            location.clone(),
        )?;
        let active_regions = link_active_regions(function, location.clone())?;
        let switch_tables = link_switch_tables(
            package,
            function,
            specialization,
            type_linker,
            substitutions,
            location.clone(),
        )?;
        let call_loan_layouts = link_call_loan_layouts(
            package,
            function,
            specialization,
            type_linker,
            substitutions,
            location.clone(),
        )?;
        let statements = link_statement_entries(function, location.clone())?;
        let source_map = link_source_map(function, location)?;
        Ok(LinkedFunctionTables::new(
            exception_regions.into_boxed_slice(),
            active_regions.into_boxed_slice(),
            switch_tables.into_boxed_slice(),
            call_loan_layouts.into_boxed_slice(),
            statements.into_boxed_slice(),
            source_map.into_boxed_slice(),
        ))
    }
}

fn link_exception_regions(
    package: &HydratedBytecodePackage,
    function: &ValidatedFunction,
    specialization: &SpecializationKey,
    type_linker: &mut TypeLinker<'_>,
    substitutions: &BTreeMap<String, TypeRefIr>,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedExceptionRegion>, BytecodeLinkError> {
    function
        .exception_regions
        .iter()
        .map(|region| {
            let catch_matchers = region
                .catch_matchers
                .iter()
                .map(|matcher| match matcher {
                    skiff_artifact_model::CatchMatcher::TypeRef { type_ref } => {
                        Ok(LinkedCatchMatcher::Type(type_linker.intern_pool_type(
                            package,
                            specialization,
                            *type_ref,
                            substitutions,
                            location.clone(),
                        )?))
                    }
                    skiff_artifact_model::CatchMatcher::CatchAll => {
                        Ok(LinkedCatchMatcher::CatchAll)
                    }
                })
                .collect::<Result<Vec<_>, BytecodeLinkError>>()?;
            Ok(LinkedExceptionRegion::new(
                instruction_index(function, region.start_pc, location.clone())?,
                instruction_boundary(function, region.end_pc, location.clone())?,
                instruction_index(function, region.handler_pc, location.clone())?,
                region.handler_stack_height,
                catch_matchers.into_boxed_slice(),
                FrameSlotIndex::new(region.catch_slot),
                type_linker.intern_pool_type(
                    package,
                    specialization,
                    region.catch_slot_type_ref,
                    substitutions,
                    location.clone(),
                )?,
                region.cleanup_depth,
            ))
        })
        .collect()
}

fn link_active_regions(
    function: &ValidatedFunction,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedActiveRegion>, BytecodeLinkError> {
    function
        .active_regions
        .iter()
        .enumerate()
        .map(|(index, region)| {
            let index = u32::try_from(index).map_err(|_| {
                unsatisfied(
                    BytecodeLinkObligation::ExceptionAndResumePlan,
                    location.clone(),
                    "active region index does not fit u32".to_string(),
                )
            })?;
            Ok(LinkedActiveRegion::new(
                ActiveRegionIndex::new(index),
                instruction_index(function, region.start_pc, location.clone())?,
                instruction_boundary(function, region.end_pc, location.clone())?,
                match &region.kind {
                    skiff_artifact_model::ActiveRegionKind::Timeout { duration_ms, site } => {
                        LinkedActiveRegionKind::Timeout {
                            duration_ms: *duration_ms,
                            site: site.clone(),
                        }
                    }
                },
            ))
        })
        .collect()
}

fn link_call_loan_layouts(
    package: &HydratedBytecodePackage,
    function: &ValidatedFunction,
    specialization: &SpecializationKey,
    type_linker: &mut TypeLinker<'_>,
    substitutions: &BTreeMap<String, TypeRefIr>,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedCallLoanLayout>, BytecodeLinkError> {
    function
        .call_loan_layouts
        .iter()
        .enumerate()
        .map(|(index, layout)| {
            let index = u32::try_from(index).map_err(|_| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    "call loan layout index does not fit u32".to_string(),
                )
            })?;
            let loans = layout
                .loans
                .iter()
                .map(|loan| {
                    Ok(LinkedCallLoanBinding::new(
                        loan.parameter_ordinal,
                        FrameSlotIndex::new(loan.root_slot),
                        type_linker.intern_writable_path(
                            package,
                            specialization,
                            loan.writable_path_ref,
                            substitutions,
                            location.clone(),
                        )?,
                    ))
                })
                .collect::<Result<Vec<_>, BytecodeLinkError>>()?;
            LinkedCallLoanLayout::try_new(CallLoanLayoutIndex::new(index), loans.into_boxed_slice())
                .map_err(|error| {
                    unsatisfied(
                        BytecodeLinkObligation::ConcreteTargetTables,
                        location.clone(),
                        error.to_string(),
                    )
                })
        })
        .collect()
}

fn link_switch_tables(
    package: &HydratedBytecodePackage,
    function: &ValidatedFunction,
    specialization: &SpecializationKey,
    type_linker: &mut TypeLinker<'_>,
    substitutions: &BTreeMap<String, TypeRefIr>,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedSwitchTable>, BytecodeLinkError> {
    function
        .switch_tables
        .iter()
        .map(|table| {
            let cases = table
                .cases
                .iter()
                .map(|case| {
                    Ok(LinkedSwitchCase::new(
                        type_linker.intern_pool_type(
                            package,
                            specialization,
                            case.tag_type_ref,
                            substitutions,
                            location.clone(),
                        )?,
                        instruction_index(function, case.target_pc, location.clone())?,
                    ))
                })
                .collect::<Result<Vec<_>, BytecodeLinkError>>()?;
            // Binding proof compares rows positionally. `try_new` therefore
            // rejects a final TypeIndex order that would require reordering.
            LinkedSwitchTable::try_new(
                cases.into_boxed_slice(),
                instruction_index(function, table.default_pc, location.clone())?,
            )
            .map_err(|error| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location.clone(),
                    error.to_string(),
                )
            })
        })
        .collect()
}

fn link_statement_entries(
    function: &ValidatedFunction,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStatementEntry>, BytecodeLinkError> {
    let mut linked = Vec::with_capacity(function.statement_entries.len());
    let mut header_cursor = 0_usize;

    for (entry_index, entry) in function.statement_entries.iter().enumerate() {
        while function
            .header_pcs
            .get(header_cursor)
            .is_some_and(|header| *header < entry.pc)
        {
            header_cursor += 1;
        }
        if function.header_pcs.get(header_cursor) != Some(&entry.pc) {
            return Err(unsatisfied(
                BytecodeLinkObligation::SourceAndStatementTables,
                location,
                format!(
                    "statement entry {entry_index} pc {} is not an admitted instruction header",
                    entry.pc
                ),
            ));
        }
        let instruction = u32::try_from(header_cursor).map_err(|_| {
            unsatisfied(
                BytecodeLinkObligation::SourceAndStatementTables,
                location.clone(),
                "statement instruction index does not fit u32".to_string(),
            )
        })?;
        linked.push(LinkedStatementEntry::new(
            InstructionIndex::new(instruction),
            entry.sequence_ordinal,
            entry.attribution_id,
            entry.site.clone(),
        ));
    }

    Ok(linked)
}

fn link_source_map(
    function: &ValidatedFunction,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedSourceMapEntry>, BytecodeLinkError> {
    function
        .source_map
        .iter()
        .map(|entry| {
            Ok(LinkedSourceMapEntry::new(
                instruction_index(function, entry.start_pc, location.clone())?,
                instruction_boundary(function, entry.end_pc, location.clone())?,
                entry.site.clone(),
            ))
        })
        .collect()
}

pub(super) fn instruction_index(
    function: &ValidatedFunction,
    pc: u32,
    location: BytecodeLinkLocation,
) -> Result<InstructionIndex, BytecodeLinkError> {
    function
        .header_pcs
        .binary_search(&pc)
        .ok()
        .and_then(|index| u32::try_from(index).ok())
        .map(InstructionIndex::new)
        .ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteTargetTables,
                location,
                format!("artifact pc {pc} is not an admitted instruction header"),
            )
        })
}

fn instruction_boundary(
    function: &ValidatedFunction,
    pc: u32,
    location: BytecodeLinkLocation,
) -> Result<InstructionBoundaryIndex, BytecodeLinkError> {
    let end_pc = u32::try_from(function.words.len()).map_err(|_| {
        unsatisfied(
            BytecodeLinkObligation::ConcreteTargetTables,
            location.clone(),
            "function word count does not fit u32".to_string(),
        )
    })?;
    if pc == end_pc {
        return u32::try_from(function.instructions.len())
            .map(InstructionBoundaryIndex::new)
            .map_err(|_| {
                unsatisfied(
                    BytecodeLinkObligation::ConcreteTargetTables,
                    location,
                    "instruction boundary count does not fit u32".to_string(),
                )
            });
    }
    instruction_index(function, pc, location)
        .map(|index| InstructionBoundaryIndex::new(index.get()))
}
