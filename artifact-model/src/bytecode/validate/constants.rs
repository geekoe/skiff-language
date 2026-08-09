use crate::bytecode::dto::limits;
use crate::bytecode::dto::{
    BytecodeArtifact, BytecodeConstantRef, BytecodePoolEntry, FrozenConstantNode,
};
use crate::bytecode::opcodes::PoolCategory;

use super::{
    constant_graph_error, entry_is_kind, header_error, index_out_of_bounds, limit_error,
    validate_type_pool_ref, StructuralValidationError,
};

/// C2: frozen constant graph node/byte bounds (C8 detail checks run in
/// `validate_constant_graph`).
pub(super) fn validate_constant_graph_limits(
    artifact: &BytecodeArtifact,
) -> Result<(), StructuralValidationError> {
    let graph = &artifact.image.frozen_constant_graph;
    let node_count = graph.nodes.len() as u64;
    if node_count > limits::MAX_CONSTANT_GRAPH_NODES {
        return Err(limit_error(
            "MAX_CONSTANT_GRAPH_NODES",
            limits::MAX_CONSTANT_GRAPH_NODES,
            node_count,
            "image.frozenConstantGraph.nodes",
        ));
    }
    let graph_bytes = skiff_canonical_json::canonical_json_bytes(graph)
        .map_err(|error| header_error(format!("constant graph is not canonical JSON: {error}")))?;
    if graph_bytes.len() as u64 > limits::MAX_CONSTANT_GRAPH_BYTES {
        return Err(limit_error(
            "MAX_CONSTANT_GRAPH_BYTES",
            limits::MAX_CONSTANT_GRAPH_BYTES,
            graph_bytes.len() as u64,
            "image.frozenConstantGraph",
        ));
    }
    Ok(())
}

/// C8: constant graph encoding (child < parent, in-bounds, compatible kinds,
/// existing behavior function) and nesting depth.
pub(super) fn validate_constant_graph(
    artifact: &BytecodeArtifact,
) -> Result<(), StructuralValidationError> {
    let graph = &artifact.image.frozen_constant_graph;
    let nodes = &graph.nodes;
    let mut depths = vec![1u32; nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        let index_u32 = index as u32;
        for child in node.children() {
            if *child >= index_u32 {
                return Err(constant_graph_error(format!(
                    "node[{index}].children contains {child}; child index must be strictly less than parent index (acyclicity encoding)"
                )));
            }
        }
        for child in node.children() {
            let child_depth = depths[*child as usize];
            depths[index] = depths[index].max(child_depth.checked_add(1).unwrap_or(u32::MAX));
        }
        if depths[index] as u64 > limits::MAX_NESTING_DEPTH {
            return Err(limit_error(
                "MAX_NESTING_DEPTH",
                limits::MAX_NESTING_DEPTH,
                depths[index] as u64,
                &format!("image.frozenConstantGraph.nodes[{index}]"),
            ));
        }
        match node {
            FrozenConstantNode::Representation { type_ref, .. } => {
                validate_type_pool_ref(
                    &artifact.image.pools,
                    *type_ref,
                    &format!("image.frozenConstantGraph.nodes[{index}].typeRef"),
                )?;
            }
            FrozenConstantNode::Record {
                shape_index,
                children,
            } => {
                if *shape_index as usize >= artifact.image.pools.shapes.len() {
                    return Err(index_out_of_bounds(
                        "shapes pool",
                        *shape_index,
                        &format!("image.frozenConstantGraph.nodes[{index}].shapeIndex"),
                    ));
                }
                if !entry_is_kind(
                    &artifact.image.pools.shapes[*shape_index as usize],
                    PoolCategory::Shapes,
                ) {
                    return Err(constant_graph_error(format!(
                        "node[{index}] shapeIndex must reference a ShapeRef entry"
                    )));
                }
                let BytecodePoolEntry::ShapeRef { shape } =
                    &artifact.image.pools.shapes[*shape_index as usize]
                else {
                    return Err(constant_graph_error(format!(
                        "node[{index}] shapeIndex must reference a ShapeRef entry"
                    )));
                };
                if children.len() != shape.fields.len() {
                    return Err(constant_graph_error(format!(
                        "node[{index}] has {} record children but shape declares {} fields",
                        children.len(),
                        shape.fields.len()
                    )));
                }
            }
            FrozenConstantNode::Implementation { record, behaviors } => {
                if !matches!(
                    nodes.get(*record as usize),
                    Some(FrozenConstantNode::Record { .. })
                ) {
                    return Err(constant_graph_error(format!(
                        "node[{index}] implementation record {record} is not a Record node"
                    )));
                }
                if behaviors.is_empty() {
                    return Err(constant_graph_error(format!(
                        "node[{index}] implementation must declare at least one behavior"
                    )));
                }
                let mut previous_function: Option<&str> = None;
                for (behavior_index, behavior) in behaviors.iter().enumerate() {
                    if behavior.function_key.is_empty()
                        || previous_function
                            .is_some_and(|previous| previous >= behavior.function_key.as_str())
                        || !artifact
                            .image
                            .functions
                            .contains_key(&behavior.function_key)
                    {
                        return Err(constant_graph_error(format!(
                            "node[{index}].behaviors[{behavior_index}] is not strictly ordered or references a missing function"
                        )));
                    }
                    previous_function = Some(behavior.function_key.as_str());
                }
            }
            FrozenConstantNode::Literal { .. } | FrozenConstantNode::Array { .. } => {}
        }
    }
    let mut reachable = vec![false; nodes.len()];
    let mut pending = Vec::new();
    for entry in &artifact.image.pools.constants {
        let BytecodePoolEntry::ConstantRef {
            reference: BytecodeConstantRef::LocalNode { node_index, .. },
            ..
        } = entry
        else {
            continue;
        };
        pending.push(*node_index);
    }
    while let Some(index) = pending.pop() {
        let slot = &mut reachable[index as usize];
        if *slot {
            continue;
        }
        *slot = true;
        pending.extend(nodes[index as usize].children().iter().copied());
    }
    if let Some(orphan) = reachable.iter().position(|reachable| !reachable) {
        return Err(constant_graph_error(format!(
            "node[{orphan}] is unreachable from every local constant pool root"
        )));
    }
    Ok(())
}
