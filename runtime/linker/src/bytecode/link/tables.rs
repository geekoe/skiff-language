use std::collections::BTreeMap;

use skiff_artifact_model::{TypeRefIr, ValidatedFunction};
use skiff_runtime_linked_bytecode::{
    InstructionBoundaryIndex, InstructionIndex, LinkedFunctionTables, LinkedSourceMapEntry,
    LinkedStatementEntry, LinkedSwitchCase, LinkedSwitchTable, SpecializationKey,
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
        reject_unsupported_tables(package, function, location.clone())?;
        let switch_tables = link_switch_tables(
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
            Box::new([]),
            Box::new([]),
            switch_tables.into_boxed_slice(),
            Box::new([]),
            statements.into_boxed_slice(),
            source_map.into_boxed_slice(),
        ))
    }
}

fn reject_unsupported_tables(
    package: &HydratedBytecodePackage,
    function: &ValidatedFunction,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    if !function.exception_regions.is_empty() || !function.active_regions.is_empty() {
        return Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::ExceptionAndResumePlan,
            location,
        });
    }
    if !function.call_loan_layouts.is_empty() {
        return Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::ConcreteTargetTables,
            location,
        });
    }
    if package
        .bytecode()
        .view()
        .resume_sites()
        .iter()
        .any(|site| site.function_key == function.function_key)
    {
        return Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::ExceptionAndResumePlan,
            location,
        });
    }
    Ok(())
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
    function
        .statement_entries
        .iter()
        .map(|entry| {
            LinkedStatementEntry::new(
                instruction_index(function, entry.pc, location.clone())?,
                entry.statement_id.clone(),
                entry.charge_kind,
            )
            .map_err(|error| {
                unsatisfied(
                    BytecodeLinkObligation::SourceAndStatementTables,
                    location.clone(),
                    error.to_string(),
                )
            })
        })
        .collect()
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
