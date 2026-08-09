use std::collections::BTreeMap;

use skiff_artifact_model::{CallableEffectSummary, ValidatedFunction};
use skiff_runtime_linked_bytecode::{
    FrameSlotIndex, FunctionIndex, LinkedCallableEffectDeclaration, LinkedFrameLayout,
    LinkedFunction, LinkedParameterSlot, SpecializationKey,
};

use crate::bytecode::{
    stack_map::{build_stack_map, StackMapLinked, StackMapSource},
    types::TypeLinker,
    BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation,
};

use super::{
    relocations::{RelocationContext, RelocationSource},
    unsatisfied, DeploymentLinker,
};

impl DeploymentLinker<'_> {
    pub(super) fn link_frame(
        &self,
        key: &SpecializationKey,
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<LinkedFrameLayout, BytecodeLinkError> {
        let (package, function) = self.source_function(key)?;
        let location = self.function_location(package, function);
        let substitutions = BTreeMap::new();
        let slot_types = function
            .frame_layout
            .slot_type_refs
            .iter()
            .map(|artifact_index| {
                type_linker.intern_pool_type(
                    package,
                    key,
                    *artifact_index,
                    &substitutions,
                    location.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let slot_plans = function
            .frame_layout
            .slot_plans
            .iter()
            .map(|plan| type_linker.link_transfer_plan(plan, &substitutions, location.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let parameters = function
            .frame_layout
            .parameter_slots
            .iter()
            .map(|parameter| {
                Ok(LinkedParameterSlot::new(
                    FrameSlotIndex::new(parameter.slot),
                    parameter.mode,
                    type_linker.link_transfer_plan(
                        &parameter.plan,
                        &substitutions,
                        location.clone(),
                    )?,
                ))
            })
            .collect::<Result<Vec<_>, BytecodeLinkError>>()?;
        let result_types = function
            .frame_layout
            .result_type_refs
            .iter()
            .map(|artifact_index| {
                type_linker.intern_pool_type(
                    package,
                    key,
                    *artifact_index,
                    &substitutions,
                    location.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let result_plans = function
            .frame_layout
            .result_plans
            .iter()
            .map(|plan| type_linker.link_transfer_plan(plan, &substitutions, location.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        LinkedFrameLayout::new(
            slot_types.into_boxed_slice(),
            parameters.into_boxed_slice(),
            function
                .frame_layout
                .writable_local_slots
                .iter()
                .copied()
                .map(FrameSlotIndex::new)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            result_types.into_boxed_slice(),
            slot_plans.into_boxed_slice(),
            result_plans.into_boxed_slice(),
        )
        .map_err(|error| {
            unsatisfied(
                BytecodeLinkObligation::FrameAndValueTransferPlan,
                location,
                error.to_string(),
            )
        })
    }

    pub(super) fn link_function(
        &mut self,
        key: &SpecializationKey,
        index: FunctionIndex,
        function_indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        frames: &[LinkedFrameLayout],
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<LinkedFunction, BytecodeLinkError> {
        let (package, source) = self.source_function(key)?;
        let location = self.function_location(package, source);
        self.tracker.add_function(
            source.words.len() as u64,
            source.relocations.len() as u64,
            function_table_entry_count(source, location.clone())?,
            location.clone(),
        )?;
        let substitutions = BTreeMap::new();
        let tables =
            self.link_function_tables(package, source, key, type_linker, &substitutions)?;
        let instructions = {
            let relocation_source = RelocationSource::new(package, source, key, &substitutions);
            let mut relocation_context =
                RelocationContext::new(self, relocation_source, function_indices, type_linker);
            source
                .instructions
                .iter()
                .map(|instruction| relocation_context.link(instruction))
                .collect::<Result<Vec<_>, _>>()?
        };
        let frame = frames.get(index.get() as usize).cloned().ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::ConcreteSpecialization,
                location.clone(),
                format!("function index {} has no linked frame", index.get()),
            )
        })?;
        let stack_map = build_stack_map(
            StackMapSource::new(package, key, source),
            StackMapLinked::new(&instructions, &frame, frames, tables.switch_tables()),
            type_linker,
            &substitutions,
        )?;
        let effect = effect_declaration(package, source, location)?;
        Ok(LinkedFunction::new(
            index,
            key.clone(),
            instructions.into_boxed_slice(),
            frame,
            source.max_operand_depth,
            effect,
            tables,
            stack_map,
        ))
    }
}

fn function_table_entry_count(
    source: &ValidatedFunction,
    location: BytecodeLinkLocation,
) -> Result<u64, BytecodeLinkError> {
    [
        source.exception_regions.len(),
        source.active_regions.len(),
        source.switch_tables.len(),
        source.call_loan_layouts.len(),
        source.statement_entries.len(),
        source.source_map.len(),
    ]
    .into_iter()
    .try_fold(0_u64, |total, count| {
        total.checked_add(count as u64).ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::SourceAndStatementTables,
                location.clone(),
                "function-local table row count overflowed".to_string(),
            )
        })
    })
}

fn effect_declaration(
    package: &skiff_runtime_loader::HydratedBytecodePackage,
    source: &ValidatedFunction,
    location: BytecodeLinkLocation,
) -> Result<LinkedCallableEffectDeclaration, BytecodeLinkError> {
    let facts = package
        .artifact()
        .callable_semantic_facts
        .get(&source.effect_summary_ref)
        .ok_or_else(|| {
            unsatisfied(
                BytecodeLinkObligation::CallableEffectPlan,
                location.clone(),
                format!(
                    "canonical callable {} has no hydrated effect authority",
                    source.effect_summary_ref
                ),
            )
        })?;
    if matches!(&facts.effects, CallableEffectSummary::Unknown { .. }) {
        return Err(unsatisfied(
            BytecodeLinkObligation::CallableEffectPlan,
            location,
            "canonical callable effect analysis is unknown".to_string(),
        ));
    }
    Ok(LinkedCallableEffectDeclaration::new(
        source.effect_summary_ref.clone(),
        facts.effects.clone(),
    ))
}
