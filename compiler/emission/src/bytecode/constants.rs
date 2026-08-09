use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    bytecode::limits, BytecodePoolEntry, BytecodePools, FrozenConstantGraph, FrozenConstantNode,
    NominalTypeRefBaseIr, ShapeDeclaration, TypeRefIr,
};
use skiff_compiler_core::type_ref::{map_type_ref, walk_type_ref};
use skiff_compiler_lowering::FrozenConstantBundle;

use super::{
    inputs::{is_void, ValidatedEmissionInputs},
    BytecodeEmissionError,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalShapeKey {
    field_count: u32,
    field_types: Vec<u32>,
}

pub(crate) fn build_constant_image(
    inputs: &ValidatedEmissionInputs<'_>,
) -> Result<(BytecodePools, FrozenConstantGraph), BytecodeEmissionError> {
    let canonical_types = collect_canonical_types(inputs)?;
    check_limit(
        "MAX_POOL_ENTRIES",
        "image.pools.types",
        canonical_types.len(),
        limits::MAX_POOL_ENTRIES,
    )?;
    let type_indices = canonical_types
        .keys()
        .enumerate()
        .map(|(index, key)| {
            Ok((
                key.clone(),
                checked_index(index, "indexing canonical types")?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, BytecodeEmissionError>>()?;

    let mut bundle_type_maps = BTreeMap::new();
    for (module_path, bundle) in &inputs.bundles {
        let unit = inputs
            .units
            .get(module_path)
            .expect("bundle coverage was checked before constant merging");
        let mut mapping = Vec::with_capacity(bundle.types().len());
        for (index, ty) in bundle.types().iter().enumerate() {
            validate_local_types(
                module_path,
                unit.type_table.len(),
                &format!("constant bundle type {index}"),
                ty,
            )?;
            let qualified = qualify_local_types(module_path, ty);
            let key = type_key(
                &qualified,
                &format!("constant bundle `{module_path}` type {index}"),
            )?;
            mapping.push(*type_indices.get(&key).ok_or_else(|| {
                BytecodeEmissionError::CanonicalSerialization {
                    context: format!("constant bundle `{module_path}` type {index}"),
                    message: "qualified type disappeared from the canonical pool".to_string(),
                }
            })?);
        }
        bundle_type_maps.insert(module_path.clone(), mapping);
    }

    let (shape_indices, bundle_shape_maps) = collect_canonical_shapes(inputs, &bundle_type_maps)?;
    check_limit(
        "MAX_POOL_ENTRIES",
        "image.pools.shapes",
        shape_indices.len(),
        limits::MAX_POOL_ENTRIES,
    )?;

    let pools = BytecodePools {
        constants: Vec::new(),
        types: canonical_types
            .into_values()
            .map(|ty| BytecodePoolEntry::TypeRef { ty })
            .collect(),
        shapes: shape_indices
            .keys()
            .map(|shape_key| BytecodePoolEntry::ShapeRef {
                shape: ShapeDeclaration {
                    field_count: shape_key.field_count,
                    field_types: shape_key.field_types.clone(),
                },
            })
            .collect(),
        effects: Vec::new(),
        resume: Vec::new(),
        callback_capture: Vec::new(),
    };

    merge_graphs(inputs, pools, &bundle_shape_maps)
}

fn collect_canonical_types(
    inputs: &ValidatedEmissionInputs<'_>,
) -> Result<BTreeMap<String, TypeRefIr>, BytecodeEmissionError> {
    let mut types = BTreeMap::new();
    for (module_path, bundle) in &inputs.bundles {
        let unit = inputs
            .units
            .get(module_path)
            .expect("bundle coverage was checked before type collection");
        for (index, ty) in bundle.types().iter().enumerate() {
            validate_local_types(
                module_path,
                unit.type_table.len(),
                &format!("constant bundle type {index}"),
                ty,
            )?;
            insert_type(
                &mut types,
                qualify_local_types(module_path, ty),
                format!("constant bundle `{module_path}` type {index}"),
            )?;
        }
    }
    for (function_key, validated) in &inputs.functions {
        for slot in &validated.function.slots {
            let ty = validated.function.slot_type(slot.slot)?;
            validate_local_types(
                &validated.unit.module_path,
                validated.unit.type_table.len(),
                &format!("function `{function_key}` slot {}", slot.slot),
                ty,
            )?;
            insert_type(
                &mut types,
                qualify_local_types(&validated.unit.module_path, ty),
                format!("function `{function_key}` slot {}", slot.slot),
            )?;
        }
        if !is_void(&validated.function.return_type) {
            validate_local_types(
                &validated.unit.module_path,
                validated.unit.type_table.len(),
                &format!("function `{function_key}` result"),
                &validated.function.return_type,
            )?;
            insert_type(
                &mut types,
                qualify_local_types(&validated.unit.module_path, &validated.function.return_type),
                format!("function `{function_key}` result"),
            )?;
        }
    }
    Ok(types)
}

fn collect_canonical_shapes(
    inputs: &ValidatedEmissionInputs<'_>,
    bundle_type_maps: &BTreeMap<String, Vec<u32>>,
) -> Result<(BTreeMap<CanonicalShapeKey, u32>, BTreeMap<String, Vec<u32>>), BytecodeEmissionError> {
    let mut shapes = BTreeSet::new();
    let mut relocated_by_bundle = BTreeMap::new();
    for (module_path, bundle) in &inputs.bundles {
        let type_map = bundle_type_maps
            .get(module_path)
            .expect("every bundle received a type relocation map");
        let mut relocated = Vec::with_capacity(bundle.shapes().len());
        for (shape_index, shape) in bundle.shapes().iter().enumerate() {
            if shape.field_types().len() != shape.field_count() as usize {
                return Err(BytecodeEmissionError::InvalidConstantGraph {
                    symbol: format!("{module_path}::<shape:{shape_index}>"),
                    message: format!(
                        "shape fieldCount {} differs from fieldTypes length {}",
                        shape.field_count(),
                        shape.field_types().len()
                    ),
                });
            }
            let mut field_types = Vec::with_capacity(shape.field_types().len());
            for type_ref in shape.field_types() {
                bundle.type_ref(*type_ref)?;
                field_types.push(*type_map.get(*type_ref as usize).ok_or_else(|| {
                    BytecodeEmissionError::InvalidConstantGraph {
                        symbol: format!("{module_path}::<shape:{shape_index}>"),
                        message: format!("type relocation {type_ref} is absent"),
                    }
                })?);
            }
            let key = shape_key(&field_types)?;
            shapes.insert(key.clone());
            relocated.push(key);
        }
        relocated_by_bundle.insert(module_path.clone(), relocated);
    }

    let shape_indices = shapes
        .into_iter()
        .enumerate()
        .map(|(index, shape)| {
            Ok((
                shape,
                checked_index(index, "indexing canonical constant shapes")?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, BytecodeEmissionError>>()?;
    let mut bundle_shape_maps = BTreeMap::new();
    for (module_path, shapes) in relocated_by_bundle {
        let mapping = shapes
            .iter()
            .map(|shape| {
                shape_indices.get(shape).copied().ok_or_else(|| {
                    BytecodeEmissionError::InvalidConstantGraph {
                        symbol: format!("{module_path}::<shape>"),
                        message: "relocated shape disappeared from the canonical pool".to_string(),
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        bundle_shape_maps.insert(module_path, mapping);
    }
    Ok((shape_indices, bundle_shape_maps))
}

fn merge_graphs(
    inputs: &ValidatedEmissionInputs<'_>,
    mut pools: BytecodePools,
    bundle_shape_maps: &BTreeMap<String, Vec<u32>>,
) -> Result<(BytecodePools, FrozenConstantGraph), BytecodeEmissionError> {
    let mut nodes = Vec::new();
    for module_path in inputs.units.keys() {
        let bundle = inputs
            .bundles
            .get(module_path)
            .expect("constant bundle coverage was checked before graph merging");
        let shape_map = bundle_shape_maps
            .get(module_path)
            .expect("every bundle received a shape relocation map");

        for (symbol, graph) in bundle.graphs() {
            let base = checked_index(nodes.len(), "offsetting a frozen constant graph")?;
            let root = bundle.root(symbol)?;
            let prospective = nodes.len().checked_add(graph.nodes.len()).ok_or(
                BytecodeEmissionError::ArithmeticOverflow {
                    context: "merging frozen constant graph nodes",
                },
            )?;
            check_limit(
                "MAX_CONSTANT_GRAPH_NODES",
                "image.frozenConstantGraph.nodes",
                prospective,
                limits::MAX_CONSTANT_GRAPH_NODES,
            )?;

            for local_index in 0..graph.nodes.len() {
                let local_index = checked_index(local_index, "relocating a constant node")?;
                let node = bundle.node(symbol, local_index)?;
                let relocated = relocate_node(bundle, symbol, local_index, node, base, shape_map)?;
                nodes.push(relocated);
            }
            let root = base
                .checked_add(root)
                .ok_or(BytecodeEmissionError::ArithmeticOverflow {
                    context: "relocating a frozen constant root",
                })?;
            pools
                .constants
                .push(BytecodePoolEntry::FrozenConstantRef { node_index: root });
        }
    }
    check_limit(
        "MAX_POOL_ENTRIES",
        "image.pools.constants",
        pools.constants.len(),
        limits::MAX_POOL_ENTRIES,
    )?;
    Ok((pools, FrozenConstantGraph { nodes }))
}

fn relocate_node(
    bundle: &FrozenConstantBundle,
    symbol: &str,
    node_index: u32,
    node: &FrozenConstantNode,
    base: u32,
    shape_map: &[u32],
) -> Result<FrozenConstantNode, BytecodeEmissionError> {
    let relocate_children = |children: &[u32]| {
        children
            .iter()
            .map(|child| {
                if *child >= node_index {
                    return Err(BytecodeEmissionError::InvalidConstantGraph {
                        symbol: symbol.to_string(),
                        message: format!(
                            "node {node_index} child {child} is not a strictly earlier graph-local index"
                        ),
                    });
                }
                base.checked_add(*child).ok_or(
                    BytecodeEmissionError::ArithmeticOverflow {
                        context: "relocating frozen constant child indices",
                    },
                )
            })
            .collect::<Result<Vec<_>, _>>()
    };

    match node {
        FrozenConstantNode::Literal { literal } => Ok(FrozenConstantNode::Literal {
            literal: literal.clone(),
        }),
        FrozenConstantNode::Array { children } => Ok(FrozenConstantNode::Array {
            children: relocate_children(children)?,
        }),
        FrozenConstantNode::Record {
            shape_index,
            children,
        } => {
            let shape = bundle.shape(*shape_index)?;
            if children.len() != shape.field_count() as usize {
                return Err(BytecodeEmissionError::ConstantShapeArityMismatch {
                    symbol: symbol.to_string(),
                    node_index,
                    shape_index: *shape_index,
                    child_count: children.len(),
                    field_count: shape.field_count(),
                });
            }
            let _ = shape_map.get(*shape_index as usize).ok_or_else(|| {
                BytecodeEmissionError::InvalidConstantGraph {
                    symbol: symbol.to_string(),
                    message: format!("shape relocation {shape_index} is absent"),
                }
            })?;
            let _ = relocate_children(children)?;
            Err(BytecodeEmissionError::UnsupportedConstantNode {
                symbol: symbol.to_string(),
                node_index,
                construct: "Record",
                reason: "the frozen shape sidecar has no nominal owner, field names, or explicit field transfer plans",
            })
        }
        FrozenConstantNode::TypeRef { .. } => Err(
            BytecodeEmissionError::UnsupportedConstantNode {
                symbol: symbol.to_string(),
                node_index,
                construct: "RepresentationWrap/TypeRef",
                reason: "the frozen graph has no explicit edge from the type annotation to its wrapped value",
            },
        ),
        FrozenConstantNode::Behavior { .. } => Err(
            BytecodeEmissionError::UnsupportedConstantNode {
                symbol: symbol.to_string(),
                node_index,
                construct: "implementation Behavior",
                reason: "the frozen graph has no explicit owner edge from the behavior to its implementation record",
            },
        ),
    }
}

fn insert_type(
    types: &mut BTreeMap<String, TypeRefIr>,
    ty: TypeRefIr,
    context: String,
) -> Result<(), BytecodeEmissionError> {
    let key = type_key(&ty, &context)?;
    types.entry(key).or_insert(ty);
    Ok(())
}

fn type_key(ty: &TypeRefIr, context: &str) -> Result<String, BytecodeEmissionError> {
    serde_json::to_string(ty).map_err(|error| BytecodeEmissionError::CanonicalSerialization {
        context: context.to_string(),
        message: error.to_string(),
    })
}

fn shape_key(field_types: &[u32]) -> Result<CanonicalShapeKey, BytecodeEmissionError> {
    Ok(CanonicalShapeKey {
        field_count: checked_index(
            field_types.len(),
            "encoding canonical constant shape field count",
        )?,
        field_types: field_types.to_vec(),
    })
}

fn validate_local_types(
    module_path: &str,
    type_count: usize,
    location: &str,
    ty: &TypeRefIr,
) -> Result<(), BytecodeEmissionError> {
    let mut failure = None;
    walk_type_ref(ty, &mut |node| {
        let local = match node {
            TypeRefIr::LocalType { type_index } => Some(*type_index),
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType { type_index },
                ..
            } => Some(*type_index),
            _ => None,
        };
        if let Some(type_index) = local.filter(|index| *index as usize >= type_count) {
            failure.get_or_insert(type_index);
        }
    });
    if let Some(type_index) = failure {
        return Err(BytecodeEmissionError::MissingLocalType {
            module_path: module_path.to_string(),
            location: location.to_string(),
            type_index,
            type_count,
        });
    }
    Ok(())
}

fn qualify_local_types(module_path: &str, ty: &TypeRefIr) -> TypeRefIr {
    map_type_ref(ty.clone(), &mut |node| match node {
        TypeRefIr::LocalType { type_index } => TypeRefIr::PublicationType {
            module_path: module_path.to_string(),
            type_index,
        },
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType { type_index },
            arguments,
        } => TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PublicationType {
                module_path: module_path.to_string(),
                type_index,
            },
            arguments,
        },
        other => other,
    })
}

fn checked_index(index: usize, context: &'static str) -> Result<u32, BytecodeEmissionError> {
    u32::try_from(index).map_err(|_| BytecodeEmissionError::ArithmeticOverflow { context })
}

fn check_limit(
    limit: &'static str,
    location: impl Into<String>,
    actual: usize,
    max: u64,
) -> Result<(), BytecodeEmissionError> {
    if actual as u64 > max {
        return Err(BytecodeEmissionError::LimitExceeded {
            limit,
            location: location.into(),
            actual: actual as u64,
            max,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{NominalTypeRefBaseIr, TypeRefIr};

    use super::qualify_local_types;

    #[test]
    fn local_type_qualification_includes_applied_nominal_bases() {
        let qualified = qualify_local_types(
            "alpha",
            &TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType { type_index: 3 },
                arguments: vec![TypeRefIr::LocalType { type_index: 4 }],
            },
        );
        assert_eq!(
            qualified,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PublicationType {
                    module_path: "alpha".to_string(),
                    type_index: 3,
                },
                arguments: vec![TypeRefIr::PublicationType {
                    module_path: "alpha".to_string(),
                    type_index: 4,
                }],
            }
        );
    }
}
