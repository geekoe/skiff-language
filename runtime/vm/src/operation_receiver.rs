//! Exact frozen-constant receiver materialization for provider operations.

use std::fmt;

use skiff_artifact_model::{LiteralIr, TypeRefIr};
use skiff_runtime_linked_bytecode::{
    ConstantIndex, FrozenConstantNodeIndex, LinkedConstantReference, LinkedFrozenConstantValue,
    LinkedValueTransferPlan, TypeIndex,
};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    vm_heap::{VmHeap, VmHeapError, VmRecordField},
    vm_value::{CompactTypeTag, ValueFlags, ValueSlot},
};

use crate::local_interface::catch_identity_for_type;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationReceiverMaterializeError {
    MissingConstant {
        constant: u32,
    },
    InvalidConstantReference {
        constant: u32,
    },
    ConstantTypeMismatch {
        constant: u32,
        expected: u32,
        actual: u32,
    },
    MissingConstantNode {
        constant: u32,
        node: u32,
    },
    MissingType {
        type_index: u32,
    },
    TypeIndexMismatch {
        type_index: u32,
    },
    PlanMismatch {
        type_index: u32,
    },
    UnsupportedType {
        type_ref: String,
    },
    MissingShape {
        shape: u32,
    },
    ShapeTypeMismatch {
        shape: u32,
        expected: u32,
        actual: u32,
    },
    ShapeFieldCountMismatch {
        shape: u32,
        child_count: usize,
        field_count: usize,
    },
    MissingCatchIdentity {
        type_index: u32,
    },
    Heap(VmHeapError),
}

impl fmt::Display for OperationReceiverMaterializeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingConstant { constant } => {
                write!(formatter, "linked constant {constant} is absent")
            }
            Self::InvalidConstantReference { constant } => write!(
                formatter,
                "linked constant {constant} is not a local frozen constant node"
            ),
            Self::ConstantTypeMismatch {
                constant,
                expected,
                actual,
            } => write!(
                formatter,
                "linked constant {constant} type is {actual}, expected {expected}"
            ),
            Self::MissingConstantNode { constant, node } => write!(
                formatter,
                "linked constant {constant} node {node} is absent"
            ),
            Self::MissingType { type_index } => {
                write!(formatter, "linked type {type_index} is absent")
            }
            Self::TypeIndexMismatch { type_index } => write!(
                formatter,
                "linked type row {type_index} does not match its index"
            ),
            Self::PlanMismatch { type_index } => write!(
                formatter,
                "linked type {type_index} transfer plan differs from the receiver fact"
            ),
            Self::UnsupportedType { type_ref } => {
                write!(formatter, "receiver type is unsupported: {type_ref}")
            }
            Self::MissingShape { shape } => {
                write!(formatter, "linked shape {shape} is absent")
            }
            Self::ShapeTypeMismatch {
                shape,
                expected,
                actual,
            } => write!(
                formatter,
                "linked shape {shape} nominal type is {actual}, expected {expected}"
            ),
            Self::ShapeFieldCountMismatch {
                shape,
                child_count,
                field_count,
            } => write!(
                formatter,
                "linked shape {shape} declares {field_count} fields but constant has {child_count} children"
            ),
            Self::MissingCatchIdentity { type_index } => write!(
                formatter,
                "receiver type {type_index} has no exact catch identity"
            ),
            Self::Heap(error) => write!(formatter, "receiver heap operation failed: {error}"),
        }
    }
}

impl std::error::Error for OperationReceiverMaterializeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Heap(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VmHeapError> for OperationReceiverMaterializeError {
    fn from(error: VmHeapError) -> Self {
        Self::Heap(error)
    }
}

#[derive(Default)]
struct MaterializeSession {
    roots: Vec<ValueSlot>,
}

impl MaterializeSession {
    fn commit(&mut self, root: ValueSlot) {
        self.roots.push(root);
    }

    fn release_all(&mut self, heap: &mut dyn VmHeap) {
        while let Some(root) = self.roots.pop() {
            let _ = heap.release_snapshot(&root);
        }
    }
}

/// Materializes one exact linked const receiver into a provider child heap.
///
/// The constant index, node graph, destination type and destination plan are
/// all taken from the same execution image. No layout is reconstructed from
/// a runtime value, type name, or nominal registry.
pub fn materialize_operation_receiver(
    destination_heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    constant: ConstantIndex,
    destination_type: TypeIndex,
    destination_plan: &LinkedValueTransferPlan,
) -> Result<ValueSlot, OperationReceiverMaterializeError> {
    let position = usize::try_from(constant.get()).map_err(|_| {
        OperationReceiverMaterializeError::MissingConstant {
            constant: constant.get(),
        }
    })?;
    let entry = image
        .constants()
        .get(position)
        .filter(|entry| entry.index() == constant)
        .ok_or(OperationReceiverMaterializeError::MissingConstant {
            constant: constant.get(),
        })?;
    if entry.ty() != destination_type {
        return Err(OperationReceiverMaterializeError::ConstantTypeMismatch {
            constant: constant.get(),
            expected: destination_type.get(),
            actual: entry.ty().get(),
        });
    }
    if entry.plan() != destination_plan {
        return Err(OperationReceiverMaterializeError::PlanMismatch {
            type_index: destination_type.get(),
        });
    }
    let LinkedConstantReference::LocalNode { node } = entry.reference() else {
        return Err(
            OperationReceiverMaterializeError::InvalidConstantReference {
                constant: constant.get(),
            },
        );
    };
    let mut session = MaterializeSession::default();
    match materialize_node(
        destination_heap,
        image,
        constant,
        *node,
        destination_type,
        destination_plan,
        &mut session,
    ) {
        Ok(value) => Ok(value),
        Err(error) => {
            session.release_all(destination_heap);
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn materialize_node(
    heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    constant: ConstantIndex,
    node_index: FrozenConstantNodeIndex,
    destination_type: TypeIndex,
    destination_plan: &LinkedValueTransferPlan,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, OperationReceiverMaterializeError> {
    let entry = checked_type_entry(image, destination_type)?;
    if entry.plan() != destination_plan {
        return Err(OperationReceiverMaterializeError::PlanMismatch {
            type_index: destination_type.get(),
        });
    }
    let position = usize::try_from(node_index.get()).map_err(|_| {
        OperationReceiverMaterializeError::MissingConstantNode {
            constant: constant.get(),
            node: node_index.get(),
        }
    })?;
    let node = image
        .frozen_constant_nodes()
        .get(position)
        .filter(|node| node.index() == node_index)
        .ok_or(OperationReceiverMaterializeError::MissingConstantNode {
            constant: constant.get(),
            node: node_index.get(),
        })?;
    match node.value() {
        LinkedFrozenConstantValue::Literal(literal) => materialize_literal(
            heap,
            constant,
            node_index,
            destination_type,
            entry,
            literal,
            session,
        ),
        LinkedFrozenConstantValue::Array { children } => materialize_array(
            heap,
            image,
            constant,
            children,
            destination_type,
            entry,
            session,
        ),
        LinkedFrozenConstantValue::Record { shape, children } => materialize_record(
            heap,
            image,
            constant,
            *shape,
            children,
            destination_type,
            entry,
            session,
        ),
        LinkedFrozenConstantValue::Representation { ty, value } => materialize_representation(
            heap,
            image,
            constant,
            *ty,
            *value,
            destination_type,
            entry,
            session,
        ),
        LinkedFrozenConstantValue::Implementation { record, .. } => materialize_node(
            heap,
            image,
            constant,
            *record,
            destination_type,
            destination_plan,
            session,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn materialize_literal(
    heap: &mut dyn VmHeap,
    constant: ConstantIndex,
    node: FrozenConstantNodeIndex,
    destination_type: TypeIndex,
    entry: &skiff_runtime_linked_bytecode::LinkedTypeEntry,
    literal: &LiteralIr,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, OperationReceiverMaterializeError> {
    if !literal_type_matches(entry, literal) {
        return Err(OperationReceiverMaterializeError::UnsupportedType {
            type_ref: format!("{:?}", entry.type_ref()),
        });
    }
    match literal {
        LiteralIr::Null => Ok(ValueSlot::null()),
        LiteralIr::Bool { value } => Ok(ValueSlot::bool(*value)),
        LiteralIr::Number { value } => value.as_f64().map(ValueSlot::number).ok_or_else(|| {
            OperationReceiverMaterializeError::UnsupportedType {
                type_ref: format!("{:?}", entry.type_ref()),
            }
        }),
        LiteralIr::String { value } => {
            let TypeRefIr::Builtin { name, args } = entry.type_ref() else {
                return Err(OperationReceiverMaterializeError::UnsupportedType {
                    type_ref: format!("{:?}", entry.type_ref()),
                });
            };
            if name != "string" || !args.is_empty() {
                return Err(OperationReceiverMaterializeError::UnsupportedType {
                    type_ref: format!("{:?}", entry.type_ref()),
                });
            }
            let tag = CompactTypeTag::try_from_type_index(destination_type.get()).ok_or(
                OperationReceiverMaterializeError::TypeIndexMismatch {
                    type_index: destination_type.get(),
                },
            )?;
            let materialized = heap.alloc_typed_string(value.clone(), tag, ValueFlags::new(0))?;
            session.commit(materialized);
            let _ = constant;
            let _ = node;
            Ok(materialized)
        }
    }
}

fn literal_type_matches(
    entry: &skiff_runtime_linked_bytecode::LinkedTypeEntry,
    literal: &LiteralIr,
) -> bool {
    match literal {
        LiteralIr::Null => {
            matches!(
                entry.type_ref(),
                TypeRefIr::Builtin { name, args } if name == "null" && args.is_empty()
            ) || matches!(
                entry.type_ref(),
                TypeRefIr::Literal {
                    value: LiteralIr::Null
                }
            )
        }
        LiteralIr::Bool { .. } => matches!(
            entry.type_ref(),
            TypeRefIr::Builtin { name, args } if name == "bool" && args.is_empty()
        ),
        LiteralIr::Number { .. } => matches!(
            entry.type_ref(),
            TypeRefIr::Builtin { name, args } if name == "number" && args.is_empty()
        ),
        LiteralIr::String { .. } => matches!(
            entry.type_ref(),
            TypeRefIr::Builtin { name, args } if name == "string" && args.is_empty()
        ),
    }
}

fn materialize_array(
    heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    constant: ConstantIndex,
    children: &[FrozenConstantNodeIndex],
    destination_type: TypeIndex,
    entry: &skiff_runtime_linked_bytecode::LinkedTypeEntry,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, OperationReceiverMaterializeError> {
    let layout = entry
        .container_layout()
        .filter(|layout| {
            matches!(
                layout.kind(),
                skiff_runtime_linked_bytecode::LinkedContainerLayoutKind::Array
            )
        })
        .ok_or_else(|| OperationReceiverMaterializeError::UnsupportedType {
            type_ref: format!("{:?}", entry.type_ref()),
        })?;
    let element =
        layout
            .element()
            .ok_or_else(|| OperationReceiverMaterializeError::UnsupportedType {
                type_ref: format!("{:?}", entry.type_ref()),
            })?;
    let start_len = session.roots.len();
    let mut materialized = Vec::with_capacity(children.len());
    for child in children {
        let value = materialize_node(
            heap,
            image,
            constant,
            *child,
            element.ty(),
            element.plan(),
            session,
        )?;
        materialized.push(value);
    }
    let tag = CompactTypeTag::try_from_type_index(destination_type.get()).ok_or(
        OperationReceiverMaterializeError::TypeIndexMismatch {
            type_index: destination_type.get(),
        },
    )?;
    let array = heap.allocate_array(&materialized, tag, ValueFlags::new(0))?;
    session.roots.truncate(start_len);
    session.commit(array);
    Ok(array)
}

#[allow(clippy::too_many_arguments)]
fn materialize_record(
    heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    constant: ConstantIndex,
    shape_index: skiff_runtime_linked_bytecode::ShapeIndex,
    children: &[FrozenConstantNodeIndex],
    destination_type: TypeIndex,
    entry: &skiff_runtime_linked_bytecode::LinkedTypeEntry,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, OperationReceiverMaterializeError> {
    let shape_position = usize::try_from(shape_index.get()).map_err(|_| {
        OperationReceiverMaterializeError::MissingShape {
            shape: shape_index.get(),
        }
    })?;
    let shape = image
        .shapes()
        .get(shape_position)
        .filter(|shape| shape.index() == shape_index)
        .ok_or(OperationReceiverMaterializeError::MissingShape {
            shape: shape_index.get(),
        })?;
    if shape.nominal_type() != destination_type {
        return Err(OperationReceiverMaterializeError::ShapeTypeMismatch {
            shape: shape_index.get(),
            expected: destination_type.get(),
            actual: shape.nominal_type().get(),
        });
    }
    if shape.fields().len() != children.len() {
        return Err(OperationReceiverMaterializeError::ShapeFieldCountMismatch {
            shape: shape_index.get(),
            child_count: children.len(),
            field_count: shape.fields().len(),
        });
    }
    let start_len = session.roots.len();
    let mut fields = Vec::with_capacity(children.len());
    for (field, child) in shape.fields().iter().zip(children) {
        let value = materialize_node(
            heap,
            image,
            constant,
            *child,
            field.ty(),
            field.plan(),
            session,
        )?;
        fields.push(VmRecordField {
            name: field.name().to_string(),
            value,
        });
    }
    let tag = CompactTypeTag::try_from_type_index(destination_type.get()).ok_or(
        OperationReceiverMaterializeError::TypeIndexMismatch {
            type_index: destination_type.get(),
        },
    )?;
    let record = heap.allocate_record(&fields, tag, ValueFlags::new(0))?;
    session.roots.truncate(start_len);
    session.commit(record);
    let _ = entry;
    Ok(record)
}

#[allow(clippy::too_many_arguments)]
fn materialize_representation(
    heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    constant: ConstantIndex,
    representation_type: TypeIndex,
    value: FrozenConstantNodeIndex,
    destination_type: TypeIndex,
    entry: &skiff_runtime_linked_bytecode::LinkedTypeEntry,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, OperationReceiverMaterializeError> {
    if representation_type != destination_type {
        return Err(OperationReceiverMaterializeError::TypeIndexMismatch {
            type_index: representation_type.get(),
        });
    }
    let carrier = entry.representation_carrier().ok_or_else(|| {
        OperationReceiverMaterializeError::UnsupportedType {
            type_ref: format!("{:?}", entry.type_ref()),
        }
    })?;
    let physical = checked_type_entry(image, carrier.physical_carrier_type())?;
    let payload = materialize_node(
        heap,
        image,
        constant,
        value,
        carrier.physical_carrier_type(),
        physical.plan(),
        session,
    )?;
    let start_len = session.roots.len();
    let tag = CompactTypeTag::try_from_type_index(destination_type.get()).ok_or(
        OperationReceiverMaterializeError::TypeIndexMismatch {
            type_index: destination_type.get(),
        },
    )?;
    let identity = catch_identity_for_type(image, destination_type).ok_or_else(|| {
        OperationReceiverMaterializeError::MissingCatchIdentity {
            type_index: destination_type.get(),
        }
    })?;
    let representation =
        heap.allocate_representation(&payload, identity, tag, ValueFlags::new(0))?;
    session.roots.truncate(start_len);
    session.commit(representation);
    Ok(representation)
}

fn checked_type_entry(
    image: &DeploymentExecutionImage,
    type_index: TypeIndex,
) -> Result<&skiff_runtime_linked_bytecode::LinkedTypeEntry, OperationReceiverMaterializeError> {
    let position = usize::try_from(type_index.get()).map_err(|_| {
        OperationReceiverMaterializeError::TypeIndexMismatch {
            type_index: type_index.get(),
        }
    })?;
    image
        .types()
        .get(position)
        .filter(|entry| entry.index() == type_index)
        .ok_or(OperationReceiverMaterializeError::MissingType {
            type_index: type_index.get(),
        })
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::PackageBuildId;
    use skiff_runtime_linked_bytecode::{
        ArtifactTypeIndex, LinkedArtifactPoolOrigin, LinkedTypeEntry, LinkedValueDropPlan,
        LinkedValueTransferPlan,
    };

    use super::*;

    fn linked_type(type_ref: TypeRefIr) -> LinkedTypeEntry {
        let origin = LinkedArtifactPoolOrigin::new(
            PackageBuildId::new("build:receiver"),
            ArtifactTypeIndex::new(0),
            None,
        )
        .expect("test origin is canonical");
        LinkedTypeEntry::new(
            TypeIndex::new(0),
            origin,
            type_ref,
            LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::Trivial,
            },
            None,
            None,
        )
    }

    #[test]
    fn literal_type_facts_match_exact_builtin_rows() {
        assert!(literal_type_matches(
            &linked_type(TypeRefIr::builtin("null")),
            &LiteralIr::Null
        ));
        assert!(literal_type_matches(
            &linked_type(TypeRefIr::builtin("bool")),
            &LiteralIr::Bool { value: true }
        ));
        assert!(literal_type_matches(
            &linked_type(TypeRefIr::builtin("number")),
            &LiteralIr::Number {
                value: serde_json::Number::from(1)
            }
        ));
        assert!(literal_type_matches(
            &linked_type(TypeRefIr::builtin("string")),
            &LiteralIr::String {
                value: "receiver".to_string()
            }
        ));
    }

    #[test]
    fn literal_type_facts_reject_wrong_rows() {
        assert!(!literal_type_matches(
            &linked_type(TypeRefIr::builtin("string")),
            &LiteralIr::Number {
                value: serde_json::Number::from(1)
            }
        ));
        assert!(!literal_type_matches(
            &linked_type(TypeRefIr::builtin("number")),
            &LiteralIr::Bool { value: false }
        ));
    }
}
