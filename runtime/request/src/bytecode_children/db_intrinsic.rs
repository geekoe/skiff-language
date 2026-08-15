//! K6 DB intrinsic boundary helpers.
//!
//! This module owns the checked VM intrinsic seam that turns an
//! `InvokeIntrinsic` DB operation into the flat request child lifecycle. It
//! does not implement DB provider behavior: the D6R leaf still owns store
//! admission, prepared runtime operations, transaction tokens and pending
//! cleanup. The helpers here only translate exact linked DB facts into the
//! existing request composition and materialize logical runtime values under
//! the exact linked result plan.

use skiff_artifact_model::bytecode::dto::DbOperationKind;
use skiff_runtime_capability_context::{DbCapabilityTarget, DbCapabilityTargetId};
use skiff_runtime_linked_bytecode::{
    LinkedDbOperation, LinkedShapeEntry, LinkedTypeEntry, TypeIndex,
};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapHandle, RuntimeValue},
    value::HeapNode,
    vm_heap::{VmHeap, VmHeapError, VmRecordField},
    vm_value::{CompactTypeTag, ValueFlags, ValueSlot},
};

use crate::vm_heap::RequestVmHeap;

pub(crate) fn linked_db_target(operation: &LinkedDbOperation) -> DbCapabilityTarget {
    DbCapabilityTarget::new(
        DbCapabilityTargetId {
            package_artifact_ref: operation.target_id().package_artifact_ref().clone(),
            file_ir_ref: operation.target_id().file_ir_ref().clone(),
            type_index: usize::try_from(operation.target_id().type_index())
                .expect("linked DB type index fits usize"),
        },
        operation.type_name(),
    )
}

pub(crate) fn require_db_operation(operation: &LinkedDbOperation) -> Result<(), String> {
    if operation.op() != DbOperationKind::Insert {
        return Err(format!(
            "K6 DB intrinsic seam currently admits linked DB insert only; got {:?}",
            operation.op()
        ));
    }
    Ok(())
}

/// Materializes one provider runtime value into the caller VM heap using only
/// the exact linked result type and plan. The source heap may be a child DB
/// heap; all allocations happen in `destination` and roll back on error.
pub(crate) fn materialize_db_result_to_vm(
    destination: &mut RequestVmHeap,
    source: &RequestHeap,
    image: &DeploymentExecutionImage,
    value: &RuntimeValue,
    operation: &LinkedDbOperation,
) -> Result<ValueSlot, String> {
    let mut session = Vec::new();
    match materialize_runtime_value(
        destination,
        source,
        image,
        value,
        operation.result_type(),
        operation.result_plan(),
        &mut session,
    ) {
        Ok(value) => Ok(value),
        Err(error) => {
            for root in session.iter().rev() {
                let _ = destination.release_snapshot(root);
            }
            Err(error)
        }
    }
}

fn materialize_runtime_value(
    destination: &mut RequestVmHeap,
    source: &RequestHeap,
    image: &DeploymentExecutionImage,
    value: &RuntimeValue,
    ty: TypeIndex,
    plan: &skiff_runtime_linked_bytecode::LinkedValueTransferPlan,
    session: &mut Vec<ValueSlot>,
) -> Result<ValueSlot, String> {
    let tag = CompactTypeTag::try_from_type_index(ty.get()).ok_or_else(|| {
        format!(
            "linked DB result type {} does not fit compact tag",
            ty.get()
        )
    })?;
    let flags = ValueFlags::new(0);
    match value {
        RuntimeValue::Null => Ok(ValueSlot::null()),
        RuntimeValue::Bool(value) => Ok(ValueSlot::bool(*value)),
        RuntimeValue::Number(value) => Ok(ValueSlot::number(*value)),
        RuntimeValue::Date(value) => Ok(ValueSlot::date(*value)),
        RuntimeValue::String(value) => destination
            .alloc_typed_string(value.clone(), tag, flags)
            .map_err(heap_error),
        RuntimeValue::Heap(handle) => materialize_heap_value(
            destination,
            source,
            image,
            *handle,
            ty,
            plan,
            tag,
            flags,
            session,
        ),
        RuntimeValue::ActorRef(_) => {
            Err("linked DB result must not carry an actor ref".to_string())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn materialize_heap_value(
    destination: &mut RequestVmHeap,
    source: &RequestHeap,
    image: &DeploymentExecutionImage,
    handle: HeapHandle,
    ty: TypeIndex,
    plan: &skiff_runtime_linked_bytecode::LinkedValueTransferPlan,
    tag: CompactTypeTag,
    flags: ValueFlags,
    session: &mut Vec<ValueSlot>,
) -> Result<ValueSlot, String> {
    let entry = type_entry(image, ty)?;
    if entry.plan() != plan {
        return Err("linked DB result plan does not match its image type row".to_string());
    }
    if entry.representation_carrier().is_some() {
        return Err(
            "linked DB result representation carriers are not supported by the K6 seam".to_string(),
        );
    }
    if let Some(layout) = entry.container_layout() {
        match layout.kind() {
            skiff_runtime_linked_bytecode::LinkedContainerLayoutKind::Array => {
                let element = layout
                    .element()
                    .ok_or_else(|| "linked DB array layout has no element".to_string())?;
                let count = match source
                    .get(handle)
                    .map_err(|error| format!("DB result heap read failed: {error}"))?
                {
                    HeapNode::Array(items) => items.len(),
                    _ => return Err("linked DB result is not an array".to_string()),
                };
                let mut elements = Vec::with_capacity(count);
                for index in 0..count {
                    let carrier = source
                        .array_item_carrier(handle, index)
                        .map_err(|error| format!("DB result array read failed: {error}"))?
                        .ok_or_else(|| format!("linked DB array item {index} is absent"))?;
                    elements.push(materialize_runtime_value(
                        destination,
                        source,
                        image,
                        carrier.value(),
                        element.ty(),
                        element.plan(),
                        session,
                    )?);
                }
                let array = destination
                    .allocate_array(&elements, tag, flags)
                    .map_err(heap_error)?;
                let child_start = session.len().saturating_sub(elements.len());
                session.truncate(child_start);
                session.push(array);
                Ok(array)
            }
            _ => Err(
                "linked DB map/Json result materialization is not in the K6 write set".to_string(),
            ),
        }
    } else {
        materialize_record_value(destination, source, image, handle, ty, tag, flags, session)
    }
}

fn materialize_record_value(
    destination: &mut RequestVmHeap,
    source: &RequestHeap,
    image: &DeploymentExecutionImage,
    handle: HeapHandle,
    ty: TypeIndex,
    tag: CompactTypeTag,
    flags: ValueFlags,
    session: &mut Vec<ValueSlot>,
) -> Result<ValueSlot, String> {
    let shape = shape_for_type(image, ty)?;
    let node = source
        .get(handle)
        .map_err(|error| format!("DB result record read failed: {error}"))?;
    if !matches!(node, HeapNode::Object(_)) {
        return Err("linked DB result record does not map to an object node".to_string());
    }
    let mut fields = Vec::with_capacity(shape.fields().len());
    for field in shape.fields() {
        let carrier = source
            .object_field_carrier(handle, field.name())
            .map_err(|error| format!("DB result field read failed: {error}"))?
            .ok_or_else(|| format!("linked DB result field {} is absent", field.name()))?;
        fields.push(VmRecordField {
            name: field.name().to_string(),
            value: materialize_runtime_value(
                destination,
                source,
                image,
                carrier.value(),
                field.ty(),
                field.plan(),
                session,
            )?,
        });
    }
    let record = destination
        .allocate_record(&fields, tag, flags)
        .map_err(heap_error)?;
    let child_start = session.len().saturating_sub(fields.len());
    session.truncate(child_start);
    session.push(record);
    Ok(record)
}

fn type_entry<'a>(
    image: &'a DeploymentExecutionImage,
    ty: TypeIndex,
) -> Result<&'a LinkedTypeEntry, String> {
    let position = usize::try_from(ty.get())
        .map_err(|_| "linked DB result type index overflows".to_string())?;
    image
        .types()
        .get(position)
        .filter(|entry| entry.index() == ty)
        .ok_or_else(|| format!("linked DB result type {} is absent", ty.get()))
}

fn shape_for_type<'a>(
    image: &'a DeploymentExecutionImage,
    ty: TypeIndex,
) -> Result<&'a LinkedShapeEntry, String> {
    image
        .shapes()
        .iter()
        .find(|shape| shape.nominal_type() == ty)
        .ok_or_else(|| format!("linked DB result type {} has no exact shape", ty.get()))
}

fn heap_error(error: VmHeapError) -> String {
    format!("DB result materialization failed: {error}")
}
