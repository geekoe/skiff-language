use std::collections::BTreeMap;

use skiff_artifact_model::{contract_for_opcode, Opcode, OperandRole, TypeRefIr, ValueSource};
use skiff_runtime_linked_bytecode::{
    ConstantIndex, LinkedInstruction, LinkedInstructionTarget, LinkedResumeSite, LinkedStackValue,
    TypeIndex,
};

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation};

use super::{obligation_error, StackMapContext};

#[allow(clippy::too_many_lines)]
pub(super) fn source_values(
    context: &mut StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    source: ValueSource,
    inputs: &[Vec<LinkedStackValue>],
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    match source {
        ValueSource::Bool => scalar_value(context, "bool", location),
        ValueSource::Number => scalar_value(context, "number", location),
        ValueSource::CollectionIndex => scalar_value(context, "number", location),
        ValueSource::Constant { operand } => {
            let constant = constant_target(instruction, operand, location.clone())?;
            let position = usize::try_from(constant.get()).map_err(|_| {
                obligation_error(
                    location.clone(),
                    format!("constant index {} does not fit usize", constant.get()),
                )
            })?;
            let row = context
                .constants
                .get(position)
                .filter(|row| row.index() == constant)
                .ok_or_else(|| {
                    obligation_error(
                        location,
                        format!("constant target {} is out of bounds", constant.get()),
                    )
                })?;
            Ok(vec![LinkedStackValue::new(row.ty(), row.plan().clone())])
        }
        ValueSource::Slot { operand } => {
            let slot = operand_word(instruction, operand, location.clone())? as usize;
            let ty = context
                .frame
                .slot_types()
                .get(slot)
                .copied()
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        format!("frame slot {slot} is out of bounds"),
                    )
                })?;
            let plan = context
                .frame
                .slot_plans()
                .get(slot)
                .cloned()
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        format!("frame slot plan {slot} is out of bounds"),
                    )
                })?;
            Ok(vec![LinkedStackValue::new(ty, plan)])
        }
        ValueSource::StackInput { group } => inputs.get(group as usize).cloned().ok_or_else(|| {
            obligation_error(
                location,
                format!("typed transition references missing stack input group {group}"),
            )
        }),
        ValueSource::TargetParameters { target } => {
            target_parameter_values(context, instruction, target, location)
        }
        ValueSource::TargetResults { target } => {
            target_result_values(context, instruction, target, location)
        }
        ValueSource::FunctionResults => Ok(context
            .frame
            .result_types()
            .iter()
            .copied()
            .zip(context.frame.result_plans().iter().cloned())
            .map(|(ty, plan)| LinkedStackValue::new(ty, plan))
            .collect()),
        ValueSource::InterfaceReceiver { interface } => {
            let table = interface_table(context, instruction, interface, location.clone())?;
            let local_or_remote = match table.kind() {
                skiff_runtime_linked_bytecode::LinkedInterfaceTableKind::Local(local) => {
                    Some(local.concrete_type())
                }
                _ => None,
            };
            let ty = local_or_remote.ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    "interface receiver requires a local interface table".to_string(),
                )
            })?;
            Ok(vec![value_with_linked_plan(context, ty, location)?])
        }
        ValueSource::InterfaceCarrier { interface } => {
            let table = interface_table(context, instruction, interface, location.clone())?;
            let interface_ref = table.interface().artifact().clone();
            let ty = context.type_linker.intern_concrete_type(
                context.source.package,
                context.source.specialization,
                &TypeRefIr::AnyInterface {
                    interface: interface_ref,
                },
                context.substitutions,
                location.clone(),
            )?;
            Ok(vec![value_with_linked_plan(context, ty, location)?])
        }
        ValueSource::CallbackCaptures { layout } => {
            let index = pool_index(instruction, layout, location.clone())?;
            let row = context.type_linker.intern_callback_capture_layout(
                context.source.package,
                index,
                location.clone(),
            )?;
            Ok(context
                .type_linker
                .callback_capture_layout(row)
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        "callback capture layout is absent".to_string(),
                    )
                })?
                .captures()
                .iter()
                .map(|capture| LinkedStackValue::new(capture.ty(), capture.plan().clone()))
                .collect())
        }
        ValueSource::CallbackClosure { target } => {
            let index = relocation_index(instruction, target, location.clone())?;
            let relocation = context
                .source
                .function
                .relocations
                .get(index as usize)
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        "callback relocation is absent".to_string(),
                    )
                })?;
            let skiff_artifact_model::BytecodeRelocation::SyntheticCallbackRef { function_key } =
                relocation
            else {
                return Err(obligation_error(
                    location.clone(),
                    "callback target is not synthetic".to_string(),
                ));
            };
            let callback = context
                .dispatch_tables
                .synthetic_callback_index(context.source.package, function_key)
                .and_then(|index| {
                    context
                        .dispatch_tables
                        .synthetic_callbacks
                        .get(index.get() as usize)
                })
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        "synthetic callback target is absent".to_string(),
                    )
                })?;
            if let Some(binding) = callback.interface_method() {
                let table = context
                    .dispatch_tables
                    .interface_tables
                    .get(binding.interface_table().get() as usize)
                    .ok_or_else(|| {
                        obligation_error(
                            location.clone(),
                            "callback interface table is absent".to_string(),
                        )
                    })?;
                let interface_ref = table.interface().artifact().clone();
                let ty = context.type_linker.intern_concrete_type(
                    context.source.package,
                    context.source.specialization,
                    &TypeRefIr::AnyInterface {
                        interface: interface_ref,
                    },
                    context.substitutions,
                    location.clone(),
                )?;
                return Ok(vec![value_with_linked_plan(context, ty, location)?]);
            }
            Err(obligation_error(
                location,
                "synthetic callback has no interface carrier type".to_string(),
            ))
        }
        ValueSource::ShapeFields { shape } => {
            let index = pool_index(instruction, shape, location.clone())?;
            let row = context.type_linker.intern_pool_shape(
                context.source.package,
                context.source.specialization,
                index,
                context.substitutions,
                location.clone(),
            )?;
            Ok(context
                .type_linker
                .shape(row)
                .ok_or_else(|| {
                    obligation_error(location.clone(), "shape row is absent".to_string())
                })?
                .fields()
                .iter()
                .map(|field| LinkedStackValue::new(field.ty(), field.plan().clone()))
                .collect())
        }
        ValueSource::ShapeValue { shape } => {
            let index = pool_index(instruction, shape, location.clone())?;
            let row = context.type_linker.intern_pool_shape(
                context.source.package,
                context.source.specialization,
                index,
                context.substitutions,
                location.clone(),
            )?;
            let entry = context.type_linker.shape(row).ok_or_else(|| {
                obligation_error(location.clone(), "shape row is absent".to_string())
            })?;
            Ok(vec![LinkedStackValue::new(
                entry.nominal_type(),
                entry.plan().clone(),
            )])
        }
        ValueSource::ShapeField { shape, ordinal } => {
            let index = pool_index(instruction, shape, location.clone())?;
            let row = context.type_linker.intern_pool_shape(
                context.source.package,
                context.source.specialization,
                index,
                context.substitutions,
                location.clone(),
            )?;
            let field_index = operand_word(instruction, ordinal, location.clone())? as usize;
            let field = context
                .type_linker
                .shape(row)
                .and_then(|shape| shape.fields().get(field_index))
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        format!("shape field {field_index} is absent"),
                    )
                })?;
            Ok(vec![LinkedStackValue::new(
                field.ty(),
                field.plan().clone(),
            )])
        }
        ValueSource::WritablePathSelectors { path } => {
            let index = pool_index(instruction, path, location.clone())?;
            let row = context.type_linker.intern_writable_path(
                context.source.package,
                context.source.specialization,
                index,
                context.substitutions,
                location.clone(),
            )?;
            let entry = context.type_linker.writable_path(row).ok_or_else(|| {
                obligation_error(location.clone(), "writable path row is absent".to_string())
            })?;
            let mut values = Vec::new();
            let segments = entry.segments().to_vec();
            for segment in segments.iter() {
                match segment {
                    skiff_runtime_linked_bytecode::LinkedWritablePathSegment::ArrayIndex {
                        ..
                    } => {
                        // The selector is the index value itself, not the array
                        // element: array indices are number-typed exactly like
                        // the canonical `CollectionIndex` input class.
                        values.extend(scalar_value(context, "number", location.clone())?);
                    }
                    skiff_runtime_linked_bytecode::LinkedWritablePathSegment::MapKey {
                        key_type,
                        ..
                    } => {
                        values.push(value_with_linked_plan(
                            context,
                            *key_type,
                            location.clone(),
                        )?);
                    }
                    skiff_runtime_linked_bytecode::LinkedWritablePathSegment::DenseField {
                        ..
                    } => {}
                }
            }
            Ok(values)
        }
        ValueSource::WritablePathLeaf { path } => {
            let index = pool_index(instruction, path, location.clone())?;
            let row = context.type_linker.intern_writable_path(
                context.source.package,
                context.source.specialization,
                index,
                context.substitutions,
                location.clone(),
            )?;
            let entry = context.type_linker.writable_path(row).ok_or_else(|| {
                obligation_error(location.clone(), "writable path row is absent".to_string())
            })?;
            Ok(vec![value_with_linked_plan(
                context,
                entry.leaf_type(),
                location,
            )?])
        }
        ValueSource::RepresentationPayload { ty } => {
            let type_index = pool_index(instruction, ty, location.clone())?;
            let linked_ty = context.type_linker.intern_pool_type(
                context.source.package,
                context.source.specialization,
                type_index,
                context.substitutions,
                location.clone(),
            )?;
            let concrete = context
                .type_linker
                .linked_type_ref(linked_ty)
                .cloned()
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        "representation type is absent".to_string(),
                    )
                })?;
            let payload = representation_payload(
                context.source.package,
                &concrete,
                context.substitutions,
                &location,
            )?;
            let payload_ty = context.type_linker.intern_concrete_type(
                context.source.package,
                context.source.specialization,
                &payload,
                context.substitutions,
                location.clone(),
            )?;
            Ok(vec![value_with_linked_plan(context, payload_ty, location)?])
        }
        ValueSource::RepresentationValue { ty } => {
            let type_index = pool_index(instruction, ty, location.clone())?;
            let linked_ty = context.type_linker.intern_pool_type(
                context.source.package,
                context.source.specialization,
                type_index,
                context.substitutions,
                location.clone(),
            )?;
            Ok(vec![value_with_linked_plan(context, linked_ty, location)?])
        }
        ValueSource::ArrayBuilder { element_type } => {
            container_builder(context, instruction, element_type, "Array", 0, location)
        }
        ValueSource::ArrayValue | ValueSource::MapValue => Err(obligation_error(
            location,
            format!(
                "input-only value source {} cannot establish a stack output",
                source.name()
            ),
        )),
        ValueSource::ArrayFromBuilder { builder_input } => {
            container_from_builder(context, inputs, builder_input, "Array", location)
        }
        ValueSource::ArrayElement { array_input } => {
            container_element(context, inputs, array_input, false, location)
        }
        ValueSource::ArrayElementFromSlot { slot } => {
            container_element_from_slot(context, instruction, slot, false, location)
        }
        ValueSource::MapBuilder {
            key_type,
            value_type,
        } => {
            let key = pool_index(instruction, key_type, location.clone())?;
            let value = pool_index(instruction, value_type, location.clone())?;
            let key_index = context.type_linker.intern_pool_type(
                context.source.package,
                context.source.specialization,
                key,
                context.substitutions,
                location.clone(),
            )?;
            let value_index = context.type_linker.intern_pool_type(
                context.source.package,
                context.source.specialization,
                value,
                context.substitutions,
                location.clone(),
            )?;
            let map = TypeRefIr::Builtin {
                name: "Map".to_string(),
                args: vec![
                    context
                        .type_linker
                        .linked_type_ref(key_index)
                        .cloned()
                        .ok_or_else(|| {
                            obligation_error(location.clone(), "map key type is absent".to_string())
                        })?,
                    context
                        .type_linker
                        .linked_type_ref(value_index)
                        .cloned()
                        .ok_or_else(|| {
                            obligation_error(
                                location.clone(),
                                "map value type is absent".to_string(),
                            )
                        })?,
                ],
            };
            let ty = context.type_linker.intern_concrete_type(
                context.source.package,
                context.source.specialization,
                &map,
                context.substitutions,
                location.clone(),
            )?;
            Ok(vec![value_with_linked_plan(context, ty, location)?])
        }
        ValueSource::MapFromBuilder { builder_input } => {
            container_from_builder(context, inputs, builder_input, "Map", location)
        }
        ValueSource::MapKey { map_input } | ValueSource::MapElement { map_input } => {
            container_element(
                context,
                inputs,
                map_input,
                source == ValueSource::MapKey { map_input },
                location,
            )
        }
        ValueSource::MapKeyFromSlot { slot } => {
            container_element_from_slot(context, instruction, slot, true, location)
        }
        ValueSource::MapElementFromSlot { slot } => {
            container_element_from_slot(context, instruction, slot, false, location)
        }
        ValueSource::StreamItem { .. } => stream_next_item(context, instruction, location),
        ValueSource::FunctionStreamItem => function_stream_item(context, instruction, location),
        ValueSource::ExceptionPayload { type_ref } => {
            let type_index = pool_index(instruction, type_ref, location.clone())?;
            let ty = context.type_linker.intern_pool_type(
                context.source.package,
                context.source.specialization,
                type_index,
                context.substitutions,
                location.clone(),
            )?;
            Ok(vec![value_with_linked_plan(context, ty, location)?])
        }
        ValueSource::ExceptionEnvelope { source_slot } => {
            let slot = operand_word(instruction, source_slot, location.clone())? as usize;
            let ty = context
                .frame
                .slot_types()
                .get(slot)
                .copied()
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        format!("exception envelope slot {slot} is absent"),
                    )
                })?;
            let plan = context
                .frame
                .slot_plans()
                .get(slot)
                .cloned()
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        format!("exception envelope slot plan {slot} is absent"),
                    )
                })?;
            Ok(vec![LinkedStackValue::new(ty, plan)])
        }
        ValueSource::AnyStackValue | ValueSource::TaggedValue | ValueSource::ComparablePair => {
            Err(obligation_error(
                location,
                format!(
                    "typed output source {} cannot establish a concrete value",
                    source.name()
                ),
            ))
        }
        ValueSource::InOutCallInputs { .. } => Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::ControlFlowAndStackMap,
            location,
        }),
    }
}

fn pool_index(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<u32, BytecodeLinkError> {
    operand_word(instruction, role, location)
}

fn relocation_index(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<u32, BytecodeLinkError> {
    operand_word(instruction, role, location)
}

fn interface_table<'a>(
    context: &'a StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<&'a skiff_runtime_linked_bytecode::LinkedInterfaceTable, BytecodeLinkError> {
    let index = target_index(instruction, role, location.clone())?;
    context
        .dispatch_tables
        .interface_tables
        .get(index as usize)
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("interface table {index} is absent"),
            )
        })
}

fn target_index(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<u32, BytecodeLinkError> {
    let ordinal = contract_for_opcode(instruction.opcode())
        .operand_position(role)
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("operand role {} is absent", role.name()),
            )
        })?;
    instruction
        .resolved_operands()
        .iter()
        .find(|resolved| resolved.operand_ordinal() == ordinal as u32)
        .map(|resolved| match resolved.target() {
            LinkedInstructionTarget::ServiceOperation(index) => index.get(),
            LinkedInstructionTarget::ActorMethod(index) => index.get(),
            LinkedInstructionTarget::InterfaceTable(index) => index.get(),
            LinkedInstructionTarget::HostEffectAdapter(index) => index.get(),
            LinkedInstructionTarget::Intrinsic(index) => index.get(),
            _ => 0,
        })
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("target operand role {} is unresolved", role.name()),
            )
        })
}

fn target_parameter_values(
    context: &mut StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    if let Some(values) =
        local_target_parameter_values(context, instruction, role, location.clone())?
    {
        return Ok(values);
    }
    let signature = target_signature(context, instruction, role, location)?;
    if is_interface_target(instruction, role) {
        Ok(signature
            .parameter_types()
            .iter()
            .skip(1)
            .copied()
            .zip(signature.parameter_plans().iter().skip(1).cloned())
            .map(|(ty, plan)| LinkedStackValue::new(ty, plan))
            .collect())
    } else {
        Ok(signature
            .parameter_types()
            .iter()
            .copied()
            .zip(signature.parameter_plans().iter().cloned())
            .map(|(ty, plan)| LinkedStackValue::new(ty, plan))
            .collect())
    }
}

fn is_interface_target(instruction: &LinkedInstruction, role: OperandRole) -> bool {
    let Some(ordinal) = contract_for_opcode(instruction.opcode()).operand_position(role) else {
        return false;
    };
    instruction.resolved_operands().iter().any(|resolved| {
        resolved.operand_ordinal() == ordinal as u32
            && matches!(
                resolved.target(),
                LinkedInstructionTarget::InterfaceTable(_)
            )
    })
}

fn target_result_values(
    context: &mut StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    if let Some(values) = local_target_result_values(context, instruction, role, location.clone())?
    {
        return Ok(values);
    }
    target_signature(context, instruction, role, location).map(|signature| {
        signature
            .result_types()
            .iter()
            .copied()
            .zip(signature.result_plans().iter().cloned())
            .map(|(ty, plan)| LinkedStackValue::new(ty, plan))
            .collect()
    })
}

fn local_target_parameter_values(
    context: &StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<Option<Vec<LinkedStackValue>>, BytecodeLinkError> {
    let ordinal = contract_for_opcode(instruction.opcode())
        .operand_position(role)
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("operand role {} is absent", role.name()),
            )
        })?;
    let Some(LinkedInstructionTarget::Function(function)) = instruction
        .resolved_operands()
        .iter()
        .find(|resolved| resolved.operand_ordinal() == ordinal as u32)
        .map(|resolved| resolved.target())
    else {
        return Ok(None);
    };
    let frame = context
        .all_frames
        .get(function.get() as usize)
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("function target {} is absent", function.get()),
            )
        })?;
    Ok(Some(
        frame
            .parameters()
            .iter()
            .map(|parameter| {
                frame
                    .slot_types()
                    .get(parameter.slot().get() as usize)
                    .copied()
                    .map(|ty| LinkedStackValue::new(ty, parameter.plan().clone()))
                    .ok_or_else(|| {
                        obligation_error(
                            location.clone(),
                            "target parameter type is absent".to_string(),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

fn local_target_result_values(
    context: &StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<Option<Vec<LinkedStackValue>>, BytecodeLinkError> {
    let ordinal = contract_for_opcode(instruction.opcode())
        .operand_position(role)
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("operand role {} is absent", role.name()),
            )
        })?;
    let Some(LinkedInstructionTarget::Function(function)) = instruction
        .resolved_operands()
        .iter()
        .find(|resolved| resolved.operand_ordinal() == ordinal as u32)
        .map(|resolved| resolved.target())
    else {
        return Ok(None);
    };
    let frame = context
        .all_frames
        .get(function.get() as usize)
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("function target {} is absent", function.get()),
            )
        })?;
    Ok(Some(
        frame
            .result_types()
            .iter()
            .copied()
            .zip(frame.result_plans().iter().cloned())
            .map(|(ty, plan)| LinkedStackValue::new(ty, plan))
            .collect(),
    ))
}

enum TargetSignature<'a> {
    Callable(&'a skiff_runtime_linked_bytecode::LinkedCallableSignature),
    Native(&'a skiff_runtime_linked_bytecode::LinkedNativeCallableSignature),
}

impl TargetSignature<'_> {
    fn parameter_types(&self) -> &[skiff_runtime_linked_bytecode::TypeIndex] {
        match self {
            Self::Callable(signature) => signature.parameter_types(),
            Self::Native(signature) => signature.parameter_types(),
        }
    }

    fn parameter_plans(&self) -> &[skiff_runtime_linked_bytecode::LinkedValueTransferPlan] {
        match self {
            Self::Callable(signature) => signature.parameter_plans(),
            Self::Native(signature) => signature.parameter_plans(),
        }
    }

    fn result_types(&self) -> &[skiff_runtime_linked_bytecode::TypeIndex] {
        match self {
            Self::Callable(signature) => signature.result_types(),
            Self::Native(signature) => signature.result_types(),
        }
    }

    fn result_plans(&self) -> &[skiff_runtime_linked_bytecode::LinkedValueTransferPlan] {
        match self {
            Self::Callable(signature) => signature.result_plans(),
            Self::Native(signature) => signature.result_plans(),
        }
    }
}

fn target_signature<'a>(
    context: &'a StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<TargetSignature<'a>, BytecodeLinkError> {
    let index = target_index(instruction, role, location.clone())? as usize;
    let ordinal = contract_for_opcode(instruction.opcode())
        .operand_position(role)
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("operand role {} is absent", role.name()),
            )
        })?;
    let target = instruction
        .resolved_operands()
        .iter()
        .find(|resolved| resolved.operand_ordinal() == ordinal as u32)
        .map(|resolved| resolved.target())
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("target operand role {} is unresolved", role.name()),
            )
        })?;
    match target {
        LinkedInstructionTarget::ServiceOperation(_) => context
            .dispatch_tables
            .service_operations
            .get(index)
            .map(|target| TargetSignature::Callable(target.signature()))
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    "service operation target is absent".to_string(),
                )
            }),
        LinkedInstructionTarget::ActorMethod(_) => context
            .dispatch_tables
            .actor_methods
            .get(index)
            .map(|target| TargetSignature::Callable(target.signature()))
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    "actor method target is absent".to_string(),
                )
            }),
        LinkedInstructionTarget::InterfaceTable(_) => {
            let table = context
                .dispatch_tables
                .interface_tables
                .get(index)
                .ok_or_else(|| {
                    obligation_error(location.clone(), "interface table is absent".to_string())
                })?;
            let method_ordinal =
                operand_word(instruction, OperandRole::MethodOrdinal, location.clone())? as usize;
            let signature = match table.kind() {
                skiff_runtime_linked_bytecode::LinkedInterfaceTableKind::Requirement(methods)
                | skiff_runtime_linked_bytecode::LinkedInterfaceTableKind::Callback(methods) => {
                    methods
                        .methods()
                        .get(method_ordinal)
                        .map(|method| method.signature())
                }
                skiff_runtime_linked_bytecode::LinkedInterfaceTableKind::Local(methods) => methods
                    .methods()
                    .get(method_ordinal)
                    .map(|method| method.signature()),
                skiff_runtime_linked_bytecode::LinkedInterfaceTableKind::Remote(methods) => methods
                    .methods()
                    .get(method_ordinal)
                    .map(|method| method.signature()),
            }
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    format!("interface method ordinal {method_ordinal} is absent"),
                )
            })?;
            Ok(TargetSignature::Callable(signature))
        }
        LinkedInstructionTarget::HostEffectAdapter(_) => context
            .dispatch_tables
            .host_effect_adapters
            .get(index)
            .map(|target| TargetSignature::Native(target.signature()))
            .ok_or_else(|| {
                obligation_error(location.clone(), "host effect target is absent".to_string())
            }),
        LinkedInstructionTarget::Intrinsic(_) => context
            .dispatch_tables
            .intrinsics
            .get(index)
            .map(|target| TargetSignature::Native(target.signature()))
            .ok_or_else(|| {
                obligation_error(location.clone(), "intrinsic target is absent".to_string())
            }),
        _ => Err(obligation_error(
            location.clone(),
            "target is not callable".to_string(),
        )),
    }
}

fn container_builder(
    context: &mut StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    element_role: OperandRole,
    name: &str,
    _element_count: usize,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    let type_index = pool_index(instruction, element_role, location.clone())?;
    let element = context.type_linker.intern_pool_type(
        context.source.package,
        context.source.specialization,
        type_index,
        context.substitutions,
        location.clone(),
    )?;
    let element_ty = context
        .type_linker
        .linked_type_ref(element)
        .cloned()
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                "container element type is absent".to_string(),
            )
        })?;
    let builtin = TypeRefIr::Builtin {
        name: name.to_string(),
        args: vec![element_ty],
    };
    let ty = context.type_linker.intern_concrete_type(
        context.source.package,
        context.source.specialization,
        &builtin,
        context.substitutions,
        location.clone(),
    )?;
    Ok(vec![value_with_linked_plan(context, ty, location)?])
}

fn container_from_builder(
    _context: &mut StackMapContext<'_, '_>,
    inputs: &[Vec<LinkedStackValue>],
    builder_input: u8,
    _name: &str,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    let builder = inputs
        .get(builder_input as usize)
        .and_then(|values| values.first())
        .cloned()
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                "container builder input is absent".to_string(),
            )
        })?;
    Ok(vec![builder])
}

fn container_element(
    context: &mut StackMapContext<'_, '_>,
    inputs: &[Vec<LinkedStackValue>],
    input_group: u8,
    key: bool,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    let container = inputs
        .get(input_group as usize)
        .and_then(|values| values.first())
        .cloned()
        .ok_or_else(|| {
            obligation_error(location.clone(), "container input is absent".to_string())
        })?;
    let layout = context
        .type_linker
        .container_layout(container.ty())
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                "container input has no layout".to_string(),
            )
        })?;
    let position = if key {
        layout.key()
    } else {
        layout.element().or_else(|| layout.value())
    }
    .ok_or_else(|| {
        obligation_error(location.clone(), "container position is absent".to_string())
    })?;
    Ok(vec![LinkedStackValue::new(
        position.ty(),
        position.plan().clone(),
    )])
}

fn container_element_from_slot(
    context: &mut StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    slot_role: OperandRole,
    key: bool,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    let slot = operand_word(instruction, slot_role, location.clone())? as usize;
    let ty = context
        .frame
        .slot_types()
        .get(slot)
        .copied()
        .ok_or_else(|| {
            obligation_error(location.clone(), format!("container slot {slot} is absent"))
        })?;
    let layout = context.type_linker.container_layout(ty).ok_or_else(|| {
        obligation_error(location.clone(), "container slot has no layout".to_string())
    })?;
    let position = if key {
        layout.key()
    } else {
        layout.element().or_else(|| layout.value())
    }
    .ok_or_else(|| {
        obligation_error(location.clone(), "container position is absent".to_string())
    })?;
    Ok(vec![LinkedStackValue::new(
        position.ty(),
        position.plan().clone(),
    )])
}

fn stream_next_item(
    context: &StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    let site = exact_resume_site(context, instruction, Opcode::StreamNext, location.clone())?;
    let ([ty], [plan]) = (site.result_types(), site.result_plans()) else {
        return Err(obligation_error(
            location,
            "StreamNext resume site must carry exactly one result type and plan".to_string(),
        ));
    };
    Ok(vec![LinkedStackValue::new(*ty, plan.clone())])
}

fn function_stream_item(
    context: &StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    let site = exact_resume_site(context, instruction, Opcode::EmitStream, location.clone())?;
    let shape_index = site.emit_stream_item_shape().ok_or_else(|| {
        obligation_error(
            location.clone(),
            "EmitStream resume site has no exact item shape".to_string(),
        )
    })?;
    let shape = context
        .type_linker
        .shape(shape_index)
        .filter(|shape| shape.index() == shape_index)
        .ok_or_else(|| {
            obligation_error(
                location,
                format!("EmitStream item shape {} is absent", shape_index.get()),
            )
        })?;
    Ok(vec![LinkedStackValue::new(
        shape.nominal_type(),
        shape.plan().clone(),
    )])
}

fn exact_resume_site<'a>(
    context: &'a StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    expected_opcode: Opcode,
    location: BytecodeLinkLocation,
) -> Result<&'a LinkedResumeSite, BytecodeLinkError> {
    if instruction.opcode() != expected_opcode {
        return Err(obligation_error(
            location,
            format!(
                "{} value authority cannot be used by opcode {}",
                expected_opcode.name(),
                instruction.opcode().name()
            ),
        ));
    }
    let ordinal = contract_for_opcode(expected_opcode)
        .operand_position(OperandRole::ResumeRef)
        .and_then(|ordinal| u32::try_from(ordinal).ok())
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("{} has no resume operand", expected_opcode.name()),
            )
        })?;
    let resume_index = instruction
        .resolved_operands()
        .iter()
        .find(|resolved| resolved.operand_ordinal() == ordinal)
        .and_then(|resolved| match resolved.target() {
            LinkedInstructionTarget::ResumeSite(index) => Some(index),
            _ => None,
        })
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!(
                    "{} has no exact resolved resume site",
                    expected_opcode.name()
                ),
            )
        })?;
    let site = context
        .type_linker
        .resume_site(resume_index)
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("resolved resume site {} is absent", resume_index.get()),
            )
        })?;
    let function = context
        .type_linker
        .function_index(context.source.specialization)
        .ok_or_else(|| BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::ControlFlowAndStackMap,
            location: location.clone(),
        })?;
    if site.function() != function {
        return Err(obligation_error(
            location.clone(),
            format!(
                "resume site {} belongs to function {}, expected {}",
                resume_index.get(),
                site.function().get(),
                function.get()
            ),
        ));
    }
    let instruction_index = context
        .source
        .function
        .header_pcs
        .binary_search(&instruction.artifact_pc())
        .map_err(|_| {
            obligation_error(
                location.clone(),
                format!(
                    "instruction pc {} is absent from its function headers",
                    instruction.artifact_pc()
                ),
            )
        })?;
    if site.site().get() as usize != instruction_index {
        return Err(obligation_error(
            location,
            format!(
                "resume site {} belongs to instruction {}, expected {instruction_index}",
                resume_index.get(),
                site.site().get()
            ),
        ));
    }
    Ok(site)
}

fn representation_payload(
    package: &skiff_runtime_loader::HydratedBytecodePackage,
    ty: &TypeRefIr,
    _substitutions: &BTreeMap<String, TypeRefIr>,
    location: &BytecodeLinkLocation,
) -> Result<TypeRefIr, BytecodeLinkError> {
    match ty {
        TypeRefIr::PackageSymbol { symbol } => {
            let symbol = package
                .artifact()
                .package_local_abi
                .implementation_symbols
                .get(&symbol.symbol_path)
                .or_else(|| {
                    package
                        .artifact()
                        .package_local_abi
                        .public_symbols
                        .get(&symbol.symbol_path)
                })
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        "representation package symbol is absent".to_string(),
                    )
                })?;
            match symbol {
                skiff_artifact_model::PackageLocalAbiSymbol::Type { descriptor, .. } => {
                    match descriptor {
                        skiff_artifact_model::TypeDescriptorIr::Representation {
                            representation,
                        } => Ok(representation.clone()),
                        _ => Err(obligation_error(
                            location.clone(),
                            "representation type is not a representation descriptor".to_string(),
                        )),
                    }
                }
                _ => Err(obligation_error(
                    location.clone(),
                    "representation type is not a type symbol".to_string(),
                )),
            }
        }
        _ => Err(obligation_error(
            location.clone(),
            "representation payload requires an exact package nominal type".to_string(),
        )),
    }
}

fn constant_target(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<ConstantIndex, BytecodeLinkError> {
    let ordinal = contract_for_opcode(instruction.opcode())
        .operand_position(role)
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("operand role {} is absent", role.name()),
            )
        })?;
    let ordinal = u32::try_from(ordinal).map_err(|_| {
        obligation_error(
            location.clone(),
            "operand ordinal does not fit u32".to_string(),
        )
    })?;
    let Some(LinkedInstructionTarget::Constant(constant)) = instruction
        .resolved_operands()
        .iter()
        .find(|resolved| resolved.operand_ordinal() == ordinal)
        .map(|resolved| resolved.target())
    else {
        return Err(obligation_error(
            location,
            format!("constant operand role {} is unresolved", role.name()),
        ));
    };
    Ok(constant)
}

pub(super) fn operand_word(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<u32, BytecodeLinkError> {
    contract_for_opcode(instruction.opcode())
        .operand_word(role, instruction.operands())
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("operand role {} is absent", role.name()),
            )
        })
}

fn scalar_value(
    context: &mut StackMapContext<'_, '_>,
    name: &str,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    let ty = context.type_linker.intern_builtin(
        context.source.package,
        context.source.specialization,
        name,
        context.substitutions,
        location.clone(),
    )?;
    Ok(vec![value_with_linked_plan(context, ty, location)?])
}

fn value_with_linked_plan(
    context: &StackMapContext<'_, '_>,
    ty: TypeIndex,
    location: BytecodeLinkLocation,
) -> Result<LinkedStackValue, BytecodeLinkError> {
    let plan = context.type_linker.linked_type_plan(ty).cloned().ok_or(
        BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::ControlFlowAndStackMap,
            location,
        },
    )?;
    Ok(LinkedStackValue::new(ty, plan))
}
