use std::fmt;

use skiff_artifact_model::LiteralIr;
use skiff_runtime_linked_bytecode::{
    ConstantIndex, LinkedBytecodeCandidate, LinkedConstantReference, LinkedFrozenConstantValue,
};
use skiff_runtime_model::vm_value::{ValueFlags, ValueSlot, VmHandle};

use super::{compact_type_tag, ExecutionImageConstructionError};

/// Immutable image-local values materialized from the linked frozen-literal table.
pub struct ExecutionConstantHeap {
    values: Box<[ValueSlot]>,
}

impl fmt::Debug for ExecutionConstantHeap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutionConstantHeap")
            .field("len", &self.values.len())
            .finish_non_exhaustive()
    }
}

impl ExecutionConstantHeap {
    pub fn get(&self, index: ConstantIndex) -> Option<ValueSlot> {
        let index = usize::try_from(index.get()).ok()?;
        self.values.get(index).copied()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

pub(in crate::bytecode) fn build_constant_heap(
    linked: &LinkedBytecodeCandidate,
) -> Result<ExecutionConstantHeap, ExecutionImageConstructionError> {
    let mut values = Vec::with_capacity(linked.constants().len());
    for constant in linked.constants() {
        let LinkedConstantReference::LocalNode { node } = constant.reference() else {
            return Err(
                ExecutionImageConstructionError::UnsupportedConstantReference {
                    constant: constant.index(),
                },
            );
        };
        let linked_node = linked
            .frozen_constant_nodes()
            .get(node.get() as usize)
            .filter(|row| row.index() == *node)
            .ok_or(ExecutionImageConstructionError::ConstantNodeMissing {
                constant: constant.index(),
                node: *node,
            })?;
        let value = match linked_node.value() {
            LinkedFrozenConstantValue::Literal(literal) => {
                materialize_literal(constant.index(), *node, constant.ty(), literal)?
            }
            _ => ValueSlot::const_ref(
                VmHandle::new(u64::from(node.get())),
                compact_type_tag(constant.ty())?,
                ValueFlags::new(0),
            ),
        };
        values.push(value);
    }
    Ok(ExecutionConstantHeap {
        values: values.into_boxed_slice(),
    })
}

fn materialize_literal(
    constant: ConstantIndex,
    node: skiff_runtime_linked_bytecode::FrozenConstantNodeIndex,
    ty: skiff_runtime_linked_bytecode::TypeIndex,
    literal: &LiteralIr,
) -> Result<ValueSlot, ExecutionImageConstructionError> {
    match literal {
        LiteralIr::Null => Ok(ValueSlot::null()),
        LiteralIr::Bool { value } => Ok(ValueSlot::bool(*value)),
        LiteralIr::Number { value } => value
            .as_f64()
            .map(ValueSlot::number)
            .ok_or(ExecutionImageConstructionError::ConstantNumberNotRepresentable { constant }),
        LiteralIr::String { .. } => Ok(ValueSlot::const_ref(
            VmHandle::new(u64::from(node.get())),
            compact_type_tag(ty)?,
            ValueFlags::new(0),
        )),
    }
}
