//! K6 DB intrinsic boundary helpers.
//!
//! This module owns the checked VM intrinsic seam that turns an
//! `InvokeIntrinsic` DB operation into the flat request child lifecycle. It
//! does not implement DB provider behavior: the D6R leaf still owns store
//! admission, prepared runtime operations, transaction tokens and pending
//! cleanup. The helpers here only translate exact linked DB facts into the
//! existing request composition and materialize logical runtime values under
//! the exact linked result plan.

use serde_json::{Number, Value};
use skiff_artifact_model::{bytecode::dto::DbOperationKind, LiteralIr};
use skiff_runtime_capability_context::{DbCapabilityTarget, DbCapabilityTargetId, DbKey};
use skiff_runtime_linked_bytecode::{
    FrozenConstantNodeIndex, LinkedDbOperation, LinkedFrozenConstantNode,
    LinkedFrozenConstantValue, LinkedShapeEntry, LinkedTypeEntry, ShapeIndex, TypeIndex,
};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapHandle, RuntimeObject, RuntimeObjectFields, RuntimeValue},
    value::HeapNode,
    vm_heap::{VmHeap, VmHeapError, VmHeapOperation, VmRecordField},
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
};

use crate::vm_heap::RequestVmHeap;

pub(crate) fn linked_db_target(
    operation: &LinkedDbOperation,
) -> Result<DbCapabilityTarget, String> {
    let target_id = operation
        .target_id()
        .ok_or_else(|| "DB read/write operation is missing its exact target".to_string())?;
    Ok(DbCapabilityTarget::new(
        DbCapabilityTargetId {
            package_artifact_ref: target_id.package_artifact_ref().clone(),
            file_ir_ref: target_id.file_ir_ref().clone(),
            type_index: usize::try_from(target_id.type_index())
                .expect("linked DB type index fits usize"),
        },
        operation.type_name(),
    ))
}

pub(crate) fn require_db_operation(operation: &LinkedDbOperation) -> Result<(), String> {
    match operation.op() {
        DbOperationKind::Read | DbOperationKind::Write => {
            if operation.target_id().is_none() {
                return Err(
                    "DB read/write intrinsic is missing its exact DbObjectTargetId".to_string(),
                );
            }
            if operation.parameter_plans().len() != 1
                || operation.result_types().len() != 1
                || operation.result_plans().len() != 1
            {
                return Err(format!(
                    "DB {} intrinsic must carry exactly one operand and result; got {:?}/{:?}",
                    format!("{:?}", operation.op()).to_lowercase(),
                    operation.parameter_plans().len(),
                    operation.result_plans().len(),
                ));
            }
        }
        DbOperationKind::Commit | DbOperationKind::Abort => {
            if operation.target_id().is_some() {
                return Err(
                    "DB transaction control intrinsic must not carry an object target".to_string(),
                );
            }
            if !operation.parameter_plans().is_empty()
                || !operation.result_types().is_empty()
                || !operation.result_plans().is_empty()
            {
                return Err(
                    "DB transaction control intrinsic must carry zero operands and results"
                        .to_string(),
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn db_key_from_runtime(value: &RuntimeValue) -> Result<DbKey, String> {
    let json = match value {
        RuntimeValue::Null => Value::Null,
        RuntimeValue::Bool(value) => Value::Bool(*value),
        RuntimeValue::Number(value) => Number::from_f64(*value)
            .map(Value::Number)
            .ok_or_else(|| "DB read key number is not finite".to_string())?,
        RuntimeValue::Date(value) => Value::Number(Number::from(*value)),
        RuntimeValue::String(value) => Value::String(value.clone()),
        RuntimeValue::ActorRef(_) | RuntimeValue::Heap(_) => {
            return Err("DB read key must be a scalar runtime value".to_string());
        }
    };
    Ok(DbKey::new(json))
}

/// Projects one DB intrinsic argument to its logical runtime value.
///
/// Linked string literals are `ConstRef` image borrows rather than request
/// heap owners, so they cannot be validated or read through the physical heap.
/// This seam resolves them from the exact frozen constant graph; complex
/// constants are materialized into the caller heap for the D6R child clone.
pub(crate) fn db_argument_runtime_value(
    heap: &mut RequestVmHeap,
    image: &DeploymentExecutionImage,
    value: &ValueSlot,
) -> Result<RuntimeValue, VmHeapError> {
    match value.kind() {
        Some(ValueKind::Null) => Ok(RuntimeValue::Null),
        Some(ValueKind::Bool) => value
            .as_bool()
            .map(RuntimeValue::Bool)
            .ok_or(VmHeapError::InvalidValueMetadata),
        Some(ValueKind::Number) => value
            .as_number()
            .map(RuntimeValue::Number)
            .ok_or(VmHeapError::InvalidValueMetadata),
        Some(ValueKind::Integer) => value
            .as_integer()
            .map(|value| RuntimeValue::Number(value as f64))
            .ok_or(VmHeapError::InvalidValueMetadata),
        Some(ValueKind::Date) => value
            .as_date()
            .map(RuntimeValue::Date)
            .ok_or(VmHeapError::InvalidValueMetadata),
        Some(ValueKind::RequestHeapRef) => heap.runtime_value_for_slot(value),
        Some(ValueKind::ConstRef) => project_const_ref_to_runtime_value(heap, image, value),
        None => Err(VmHeapError::InvalidValueMetadata),
        Some(other) => Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::AllocateRecord,
            kind: other,
        }),
    }
}

fn project_const_ref_to_runtime_value(
    heap: &mut RequestVmHeap,
    image: &DeploymentExecutionImage,
    value: &ValueSlot,
) -> Result<RuntimeValue, VmHeapError> {
    let handle = value
        .as_const_ref()
        .ok_or(VmHeapError::InvalidValueMetadata)?;
    let node_index = FrozenConstantNodeIndex::new(
        u32::try_from(handle.get()).map_err(|_| VmHeapError::InvalidValueMetadata)?,
    );
    project_frozen_constant_node(
        heap,
        image.frozen_constant_nodes(),
        image.shapes(),
        node_index,
    )
}

fn project_frozen_constant_node(
    heap: &mut RequestVmHeap,
    nodes: &[LinkedFrozenConstantNode],
    shapes: &[LinkedShapeEntry],
    index: FrozenConstantNodeIndex,
) -> Result<RuntimeValue, VmHeapError> {
    let node = unique_frozen_constant_node(nodes, index)?;
    match node.value() {
        LinkedFrozenConstantValue::Literal(literal) => runtime_value_from_literal(literal),
        LinkedFrozenConstantValue::Array { children } => {
            project_constant_array(heap, nodes, shapes, children)
        }
        LinkedFrozenConstantValue::Record { shape, children } => {
            project_constant_record(heap, nodes, shapes, *shape, children)
        }
        LinkedFrozenConstantValue::Representation { value, .. }
        | LinkedFrozenConstantValue::Implementation { record: value, .. } => {
            project_frozen_constant_node(heap, nodes, shapes, *value)
        }
    }
}

fn runtime_value_from_literal(literal: &LiteralIr) -> Result<RuntimeValue, VmHeapError> {
    match literal {
        LiteralIr::Null => Ok(RuntimeValue::Null),
        LiteralIr::Bool { value } => Ok(RuntimeValue::Bool(*value)),
        LiteralIr::Number { value } => value.as_f64().map(RuntimeValue::Number).ok_or_else(|| {
            VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::AllocateRecord,
                message: "frozen constant number is not representable as f64".to_string(),
            }
        }),
        LiteralIr::String { value } => Ok(RuntimeValue::String(value.clone())),
    }
}

fn project_constant_array(
    heap: &mut RequestVmHeap,
    nodes: &[LinkedFrozenConstantNode],
    shapes: &[LinkedShapeEntry],
    children: &[FrozenConstantNodeIndex],
) -> Result<RuntimeValue, VmHeapError> {
    let checkpoint = heap.request_heap().checkpoint();
    let mut values = Vec::with_capacity(children.len());
    for child in children {
        match project_frozen_constant_node(heap, nodes, shapes, *child) {
            Ok(value) => values.push(value),
            Err(error) => {
                heap.request_heap_mut().rollback_to_checkpoint(checkpoint);
                return Err(error);
            }
        }
    }
    match heap.request_heap_mut().alloc_array(values) {
        Ok(handle) => Ok(RuntimeValue::Heap(handle)),
        Err(error) => {
            heap.request_heap_mut().rollback_to_checkpoint(checkpoint);
            Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::AllocateArray,
                message: error.to_string(),
            })
        }
    }
}

fn project_constant_record(
    heap: &mut RequestVmHeap,
    nodes: &[LinkedFrozenConstantNode],
    shapes: &[LinkedShapeEntry],
    shape_index: ShapeIndex,
    children: &[FrozenConstantNodeIndex],
) -> Result<RuntimeValue, VmHeapError> {
    let shape = unique_shape(shapes, shape_index)?;
    if shape.fields().len() != children.len() {
        return Err(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::AllocateRecord,
            message: format!(
                "linked frozen record shape {} declares {} fields but node has {} children",
                shape_index.get(),
                shape.fields().len(),
                children.len()
            ),
        });
    }
    let checkpoint = heap.request_heap().checkpoint();
    let mut fields = RuntimeObjectFields::new();
    for (field, child) in shape.fields().iter().zip(children) {
        match project_frozen_constant_node(heap, nodes, shapes, *child) {
            Ok(value) => {
                fields.insert(field.name().to_string(), value);
            }
            Err(error) => {
                heap.request_heap_mut().rollback_to_checkpoint(checkpoint);
                return Err(error);
            }
        }
    }
    match heap
        .request_heap_mut()
        .alloc_object(RuntimeObject::unshaped(fields))
    {
        Ok(handle) => Ok(RuntimeValue::Heap(handle)),
        Err(error) => {
            heap.request_heap_mut().rollback_to_checkpoint(checkpoint);
            Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::AllocateRecord,
                message: error.to_string(),
            })
        }
    }
}

fn unique_frozen_constant_node<'a>(
    nodes: &'a [LinkedFrozenConstantNode],
    index: FrozenConstantNodeIndex,
) -> Result<&'a LinkedFrozenConstantNode, VmHeapError> {
    let mut matches = nodes.iter().filter(|node| node.index() == index);
    let first = matches
        .next()
        .ok_or_else(|| VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::AllocateRecord,
            message: format!("linked frozen constant node {} is absent", index.get()),
        })?;
    if matches.next().is_some() {
        return Err(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::AllocateRecord,
            message: format!(
                "linked frozen constant node {} matches more than one exact node",
                index.get()
            ),
        });
    }
    Ok(first)
}

fn unique_shape<'a>(
    shapes: &'a [LinkedShapeEntry],
    index: ShapeIndex,
) -> Result<&'a LinkedShapeEntry, VmHeapError> {
    let mut matches = shapes.iter().filter(|shape| shape.index() == index);
    let first = matches
        .next()
        .ok_or_else(|| VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::AllocateRecord,
            message: format!("linked frozen record shape {} is absent", index.get()),
        })?;
    if matches.next().is_some() {
        return Err(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::AllocateRecord,
            message: format!(
                "linked frozen record shape {} matches more than one exact shape",
                index.get()
            ),
        });
    }
    Ok(first)
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
    let result_type = operation
        .result_types()
        .first()
        .copied()
        .ok_or_else(|| "linked DB data result type is absent".to_string())?;
    let result_plan = operation
        .result_plans()
        .first()
        .ok_or_else(|| "linked DB data result plan is absent".to_string())?;
    let mut session = Vec::new();
    match materialize_runtime_value(
        destination,
        source,
        image,
        value,
        result_type,
        result_plan,
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
        materialize_record_value(
            destination,
            source,
            image,
            handle,
            ty,
            plan,
            tag,
            flags,
            session,
        )
    }
}

fn materialize_record_value(
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
    let shape = shape_for_type(image, ty, plan)?;
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
    plan: &skiff_runtime_linked_bytecode::LinkedValueTransferPlan,
) -> Result<&'a LinkedShapeEntry, String> {
    let entry = type_entry(image, ty)?;
    unique_shape_for_type_index(image.shapes(), ty, plan, entry.origin().specialization())
}

fn unique_shape_for_type_index<'a>(
    shapes: &'a [LinkedShapeEntry],
    ty: TypeIndex,
    plan: &skiff_runtime_linked_bytecode::LinkedValueTransferPlan,
    specialization: Option<&skiff_runtime_linked_bytecode::SpecializationKey>,
) -> Result<&'a LinkedShapeEntry, String> {
    let mut matches = shapes.iter().filter(|shape| {
        shape.nominal_type() == ty
            && shape.plan() == plan
            && shape.origin().specialization() == specialization
    });
    if let Some(first) = matches.next() {
        if matches.next().is_some() {
            return Err(format!(
                "linked DB result type {} matches more than one exact shape",
                ty.get()
            ));
        }
        return Ok(first);
    }
    let mut canonical = shapes.iter().filter(|shape| {
        shape.nominal_type() == ty && shape.origin().specialization().is_none()
    });
    let first = canonical
        .next()
        .ok_or_else(|| format!("linked DB result type {} has no exact shape", ty.get()))?;
    if canonical.next().is_some() {
        return Err(format!(
            "linked DB result type {} matches more than one canonical exact shape",
            ty.get()
        ));
    }
    Ok(first)
}

fn heap_error(error: VmHeapError) -> String {
    format!("DB result materialization failed: {error}")
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        FileIrRef, LiteralIr, PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity,
    };
    use skiff_runtime_linked_bytecode::{
        ArtifactConstantNodeIndex, ArtifactShapeIndex, FrozenConstantNodeIndex,
        LinkedArtifactPoolOrigin, LinkedDbObjectTargetId, LinkedFrozenConstantNode,
        LinkedFrozenConstantValue, LinkedShapeEntry, LinkedShapeField, LinkedValueDropPlan,
        LinkedValueTransferPlan, ShapeIndex, TypeIndex,
    };
    use skiff_runtime_model::{
        request_heap::RequestHeapLimits,
        runtime_value::{HeapNode, RuntimeValue},
    };

    use super::*;

    fn const_origin() -> LinkedArtifactPoolOrigin<ArtifactConstantNodeIndex> {
        LinkedArtifactPoolOrigin::new(
            PackageBuildId::new("build:db"),
            ArtifactConstantNodeIndex::new(0),
            None,
        )
        .expect("test constant origin is canonical")
    }

    fn shape_origin() -> LinkedArtifactPoolOrigin<ArtifactShapeIndex> {
        LinkedArtifactPoolOrigin::new(
            PackageBuildId::new("build:db"),
            ArtifactShapeIndex::new(0),
            None,
        )
        .expect("test shape origin is canonical")
    }

    fn plan() -> LinkedValueTransferPlan {
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        }
    }

    fn heap() -> RequestVmHeap {
        RequestVmHeap::new(RequestHeapLimits::default())
    }

    fn string_node(value: &str, index: u32) -> LinkedFrozenConstantNode {
        LinkedFrozenConstantNode::new(
            FrozenConstantNodeIndex::new(index),
            const_origin(),
            LinkedFrozenConstantValue::Literal(LiteralIr::String {
                value: value.to_string(),
            }),
        )
    }

    fn record_shape(index: u32) -> LinkedShapeEntry {
        LinkedShapeEntry::new(
            ShapeIndex::new(index),
            shape_origin(),
            TypeIndex::new(0),
            plan(),
            None,
            Box::new([LinkedShapeField::new("value", TypeIndex::new(0), plan())
                .expect("test shape field is canonical")]),
        )
        .expect("test shape is canonical")
    }

    fn record_node(index: u32, shape: u32, children: &[u32]) -> LinkedFrozenConstantNode {
        LinkedFrozenConstantNode::new(
            FrozenConstantNodeIndex::new(index),
            const_origin(),
            LinkedFrozenConstantValue::Record {
                shape: ShapeIndex::new(shape),
                children: children
                    .iter()
                    .map(|index| FrozenConstantNodeIndex::new(*index))
                    .collect(),
            },
        )
    }

    fn target_id() -> LinkedDbObjectTargetId {
        LinkedDbObjectTargetId::new(
            PackageArtifactRef {
                package_id: "test.skiff/db".to_string(),
                package_version: "1.0.0".to_string(),
                package_build_id: PackageBuildId::new("build:db"),
                package_local_abi_identity: PackageLocalAbiIdentity::new("abi:db"),
            },
            FileIrRef::new("file:db", "main.skiff"),
            0,
        )
    }

    fn scalar_plan() -> skiff_runtime_linked_bytecode::LinkedValueTransferPlan {
        skiff_runtime_linked_bytecode::LinkedValueTransferPlan::SnapshotShare {
            drop: skiff_runtime_linked_bytecode::LinkedValueDropPlan::Trivial,
        }
    }

    fn data_operation(op: DbOperationKind) -> LinkedDbOperation {
        let plan = scalar_plan();
        LinkedDbOperation::new(
            Some(target_id()),
            "Doc",
            op,
            Box::new([plan.clone()]),
            Box::new([TypeIndex::new(0)]),
            Box::new([plan]),
        )
        .expect("linked data operation is valid")
    }

    fn control_operation(op: DbOperationKind) -> LinkedDbOperation {
        LinkedDbOperation::new(
            None,
            "db.transaction",
            op,
            Box::new([]),
            Box::new([]),
            Box::new([]),
        )
        .expect("linked transaction control is valid")
    }

    #[test]
    fn require_db_operation_accepts_normalized_lanes() {
        for op in [
            DbOperationKind::Read,
            DbOperationKind::Write,
            DbOperationKind::Commit,
            DbOperationKind::Abort,
        ] {
            let operation = match op {
                DbOperationKind::Read | DbOperationKind::Write => data_operation(op),
                DbOperationKind::Commit | DbOperationKind::Abort => control_operation(op),
            };
            require_db_operation(&operation).expect("normalized DB lane is admitted");
        }
    }

    #[test]
    fn linked_db_operation_rejects_transaction_control_with_arity() {
        let plan = scalar_plan();
        assert!(LinkedDbOperation::new(
            None,
            "db.transaction",
            DbOperationKind::Commit,
            Box::new([plan.clone()]),
            Box::new([TypeIndex::new(0)]),
            Box::new([plan]),
        )
        .is_err());
    }

    #[test]
    fn db_key_from_runtime_accepts_scalar_and_rejects_heap() {
        assert_eq!(
            db_key_from_runtime(&RuntimeValue::String("key".to_string())).unwrap(),
            DbKey::new(serde_json::json!("key"))
        );
        assert!(db_key_from_runtime(&RuntimeValue::Null).is_ok());
        assert!(db_key_from_runtime(&RuntimeValue::Number(1.0)).is_ok());
    }

    #[test]
    fn db_argument_projects_string_constant() {
        let nodes = vec![string_node("recoverable-restore", 0)];
        let mut heap = heap();
        let value =
            project_frozen_constant_node(&mut heap, &nodes, &[], FrozenConstantNodeIndex::new(0))
                .expect("string constant must project");
        assert_eq!(
            value,
            RuntimeValue::String("recoverable-restore".to_string())
        );
    }

    #[test]
    fn db_argument_projects_number_constant() {
        let nodes = vec![LinkedFrozenConstantNode::new(
            FrozenConstantNodeIndex::new(0),
            const_origin(),
            LinkedFrozenConstantValue::Literal(LiteralIr::Number {
                value: serde_json::Number::from(7),
            }),
        )];
        let mut heap = heap();
        let value =
            project_frozen_constant_node(&mut heap, &nodes, &[], FrozenConstantNodeIndex::new(0))
                .expect("number constant must project");
        assert_eq!(value, RuntimeValue::Number(7.0));
    }

    #[test]
    fn db_argument_projects_record_constant() {
        let string = string_node("ok", 0);
        let record = record_node(1, 0, &[0]);
        let shapes = vec![record_shape(0)];
        let mut heap = heap();
        let value = project_frozen_constant_node(
            &mut heap,
            &[string, record],
            &shapes,
            FrozenConstantNodeIndex::new(1),
        )
        .expect("record constant must project");
        let handle = value.as_heap_handle().expect("record projects to heap");
        let HeapNode::Object(object) = heap
            .request_heap()
            .get(handle)
            .expect("record heap handle is readable")
        else {
            panic!("projected record is not an object node");
        };
        assert_eq!(
            object.fields().get("value"),
            Some(&RuntimeValue::String("ok".to_string()))
        );
    }

    #[test]
    fn db_argument_missing_constant_fails_closed() {
        let mut heap = heap();
        let error =
            project_frozen_constant_node(&mut heap, &[], &[], FrozenConstantNodeIndex::new(9))
                .expect_err("absent constant node must fail closed");
        assert!(error.to_string().contains("node 9 is absent"));
    }

    #[test]
    fn db_argument_ambiguous_constant_fails_closed() {
        let node = string_node("ambiguous", 0);
        let mut heap = heap();
        let error = project_frozen_constant_node(
            &mut heap,
            &[node.clone(), node],
            &[],
            FrozenConstantNodeIndex::new(0),
        )
        .expect_err("duplicate constant node must fail closed");
        assert!(error.to_string().contains("more than one exact node"));
    }

    #[test]
    fn db_argument_ambiguous_shape_fails_closed() {
        let nodes = vec![record_node(0, 0, &[])];
        let shape = record_shape(0);
        let shapes = vec![shape.clone(), shape];
        let mut heap = heap();
        let error = project_frozen_constant_node(
            &mut heap,
            &nodes,
            &shapes,
            FrozenConstantNodeIndex::new(0),
        )
        .expect_err("duplicate record shape must fail closed");
        assert!(error.to_string().contains("more than one exact shape"));
    }
}
