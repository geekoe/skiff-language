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
    constants::LinkedConstantTables,
    dispatch::LinkedDispatchTables,
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
            .enumerate()
            .map(|(ordinal, plan)| {
                let ty = slot_types.get(ordinal).copied().ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::FrameAndValueTransferPlan,
                        location.clone(),
                        format!("frame slot plan ordinal {ordinal} has no exact slot type"),
                    )
                })?;
                let concrete = type_linker.linked_type_ref(ty).cloned().ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::FrameAndValueTransferPlan,
                        location.clone(),
                        format!("frame slot type {} is absent", ty.get()),
                    )
                })?;
                type_linker.link_plan_for_type_at(
                    package,
                    key,
                    &substitutions,
                    plan,
                    &concrete,
                    location.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let parameters = function
            .frame_layout
            .parameter_slots
            .iter()
            .map(|parameter| {
                let slot = FrameSlotIndex::new(parameter.slot);
                let ty = slot_types
                    .get(slot.get() as usize)
                    .copied()
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::FrameAndValueTransferPlan,
                            location.clone(),
                            format!("frame parameter slot {} has no exact type", slot.get()),
                        )
                    })?;
                let concrete = type_linker.linked_type_ref(ty).cloned().ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::FrameAndValueTransferPlan,
                        location.clone(),
                        format!("frame parameter type {} is absent", ty.get()),
                    )
                })?;
                let plan = type_linker.link_plan_for_type_at(
                    package,
                    key,
                    &substitutions,
                    &parameter.plan,
                    &concrete,
                    location.clone(),
                )?;
                let dense_record_shape = parameter
                    .dense_record_shape_ref
                    .map(|artifact_shape| {
                        let shape = type_linker.intern_pool_shape(
                            package,
                            key,
                            artifact_shape,
                            &substitutions,
                            location.clone(),
                        )?;
                        let row = type_linker.shape(shape).cloned().ok_or_else(|| {
                            unsatisfied(
                                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                                location.clone(),
                                format!("dense parameter shape {} is absent", shape.get()),
                            )
                        })?;
                        type_linker.validate_dense_parameter_materialization(
                            ty,
                            &plan,
                            &row,
                            location.clone(),
                        )?;
                        Ok(shape)
                    })
                    .transpose()?;
                Ok(LinkedParameterSlot::new(
                    slot,
                    parameter.mode,
                    plan,
                    dense_record_shape,
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
            .enumerate()
            .map(|(ordinal, plan)| {
                let ty = result_types.get(ordinal).copied().ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::FrameAndValueTransferPlan,
                        location.clone(),
                        format!("frame result plan ordinal {ordinal} has no exact result type"),
                    )
                })?;
                let concrete = type_linker.linked_type_ref(ty).cloned().ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::FrameAndValueTransferPlan,
                        location.clone(),
                        format!("frame result type {} is absent", ty.get()),
                    )
                })?;
                type_linker.link_plan_for_type_at(
                    package,
                    key,
                    &substitutions,
                    plan,
                    &concrete,
                    location.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let stream_result_type_ref = function
            .frame_layout
            .stream_result_type_ref
            .map(|artifact_index| {
                type_linker.intern_pool_type(
                    package,
                    key,
                    artifact_index,
                    &substitutions,
                    location.clone(),
                )
            })
            .transpose()?;
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
            stream_result_type_ref,
        )
        .map_err(|error| {
            unsatisfied(
                BytecodeLinkObligation::FrameAndValueTransferPlan,
                location,
                error.to_string(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn link_function(
        &mut self,
        key: &SpecializationKey,
        index: FunctionIndex,
        function_indices: &BTreeMap<SpecializationKey, FunctionIndex>,
        frames: &[LinkedFrameLayout],
        constant_tables: &LinkedConstantTables,
        dispatch_tables: &LinkedDispatchTables,
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
            let mut relocation_context = RelocationContext::new(
                self,
                relocation_source,
                function_indices,
                constant_tables,
                dispatch_tables,
                type_linker,
            );
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
            StackMapLinked::new(
                &instructions,
                &frame,
                frames,
                tables.switch_tables(),
                constant_tables.constants(),
                dispatch_tables,
            ),
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
