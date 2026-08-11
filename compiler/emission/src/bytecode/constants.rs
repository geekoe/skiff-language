use std::collections::BTreeMap;

use skiff_artifact_model::{
    bytecode::limits, BytecodeConstantRef, BytecodePoolEntry, BytecodePools, ExprIr,
    FrozenConstantGraph, FrozenConstantNode, NominalTypeRefBaseIr, ResumeDescriptor,
    ShapeDeclaration, ShapeFieldDeclaration, TypeRefIr, ValueTransferPlan, WritablePathDeclaration,
    WritablePathSegment,
};
use skiff_compiler_core::type_ref::{map_type_ref, walk_type_ref};
use skiff_compiler_lowering::mir::{MirFunction, MirUnit};

use super::{inputs::ValidatedEmissionInputs, BytecodeEmissionError};

pub(crate) struct ConstantImage {
    pub(crate) pools: BytecodePools,
    pub(crate) roots: BTreeMap<String, u32>,
    pub(crate) graph: FrozenConstantGraph,
    type_indices: BTreeMap<String, u32>,
    shape_indices: BTreeMap<String, u32>,
    writable_path_indices: BTreeMap<String, u32>,
}

impl ConstantImage {
    pub(crate) fn type_index(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        context: &str,
    ) -> Result<u32, BytecodeEmissionError> {
        let qualified = qualify_local_types(module_path, ty);
        let key = type_key(&qualified, context)?;
        self.type_indices.get(&key).copied().ok_or_else(|| {
            BytecodeEmissionError::CanonicalSerialization {
                context: context.to_string(),
                message: "qualified type disappeared from the canonical pool".to_string(),
            }
        })
    }

    pub(crate) fn intern_type(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
        context: &str,
    ) -> Result<u32, BytecodeEmissionError> {
        let qualified = qualify_local_types(module_path, ty);
        let key = type_key(&qualified, context)?;
        if let Some(index) = self.type_indices.get(&key) {
            return Ok(*index);
        }
        let index = checked_index(self.pools.types.len(), "indexing canonical types")?;
        self.pools
            .types
            .push(BytecodePoolEntry::TypeRef { ty: qualified });
        self.type_indices.insert(key, index);
        Ok(index)
    }

    pub(crate) fn add_literal_constant(
        &mut self,
        module_path: &str,
        literal: &skiff_artifact_model::LiteralIr,
        ty: &TypeRefIr,
        context: &str,
    ) -> Result<u32, BytecodeEmissionError> {
        let type_ref = self.intern_type(module_path, ty, context)?;
        let node_index = checked_index(
            self.graph.nodes.len(),
            "indexing function literal constant graph nodes",
        )?;
        self.graph.nodes.push(FrozenConstantNode::Literal {
            literal: literal.clone(),
        });
        let plan = ValueTransferPlan::FromType { ty: ty.clone() };
        let pool_index = checked_index(
            self.pools.constants.len(),
            "indexing function literal constant pool",
        )?;
        self.pools.constants.push(BytecodePoolEntry::ConstantRef {
            reference: BytecodeConstantRef::LocalNode { node_index },
            type_ref,
            plan,
        });
        Ok(pool_index)
    }

    pub(crate) fn intern_shape(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
        fields: &BTreeMap<String, TypeRefIr>,
        context: &str,
    ) -> Result<u32, BytecodeEmissionError> {
        let type_ref = self.intern_type(module_path, ty, context)?;
        let field_declarations = fields
            .iter()
            .map(|(name, field_ty)| {
                let field_type_ref = self.intern_type(module_path, field_ty, context)?;
                let qualified = qualify_local_types(module_path, field_ty);
                Ok(ShapeFieldDeclaration {
                    name: name.clone(),
                    type_ref: field_type_ref,
                    plan: ValueTransferPlan::FromType { ty: qualified },
                })
            })
            .collect::<Result<Vec<_>, BytecodeEmissionError>>()?;
        let shape = ShapeDeclaration {
            type_ref,
            fields: field_declarations,
        };
        let key = serde_json::to_string(&shape).map_err(|error| {
            BytecodeEmissionError::CanonicalSerialization {
                context: context.to_string(),
                message: error.to_string(),
            }
        })?;
        if let Some(index) = self.shape_indices.get(&key) {
            return Ok(*index);
        }
        let index = checked_index(self.pools.shapes.len(), "indexing canonical record shapes")?;
        self.pools
            .shapes
            .push(BytecodePoolEntry::ShapeRef { shape });
        check_limit(
            "MAX_POOL_ENTRIES",
            "image.pools.shapes",
            self.pools.shapes.len(),
            limits::MAX_POOL_ENTRIES,
        )?;
        self.shape_indices.insert(key, index);
        Ok(index)
    }

    pub(crate) fn intern_writable_path(
        &mut self,
        module_path: &str,
        root_ty: &TypeRefIr,
        leaf_ty: &TypeRefIr,
        segments: Vec<WritablePathSegment>,
        context: &str,
    ) -> Result<u32, BytecodeEmissionError> {
        let root_type_ref = self.intern_type(module_path, root_ty, context)?;
        let leaf_type_ref = self.intern_type(module_path, leaf_ty, context)?;
        let path = WritablePathDeclaration {
            root_type_ref,
            leaf_type_ref,
            segments,
        };
        let key = serde_json::to_string(&path).map_err(|error| {
            BytecodeEmissionError::CanonicalSerialization {
                context: context.to_string(),
                message: error.to_string(),
            }
        })?;
        if let Some(index) = self.writable_path_indices.get(&key) {
            return Ok(*index);
        }
        let index = checked_index(
            self.pools.writable_paths.len(),
            "indexing canonical writable paths",
        )?;
        self.pools
            .writable_paths
            .push(BytecodePoolEntry::WritablePath(path));
        check_limit(
            "MAX_POOL_ENTRIES",
            "image.pools.writablePaths",
            self.pools.writable_paths.len(),
            limits::MAX_POOL_ENTRIES,
        )?;
        self.writable_path_indices.insert(key, index);
        Ok(index)
    }

    pub(crate) fn add_resume_descriptor(
        &mut self,
        descriptor: ResumeDescriptor,
    ) -> Result<u32, BytecodeEmissionError> {
        let index = checked_index(self.pools.resume.len(), "indexing resume descriptors")?;
        self.pools
            .resume
            .push(BytecodePoolEntry::ResumeDescriptor(descriptor));
        check_limit(
            "MAX_POOL_ENTRIES",
            "image.pools.resume",
            self.pools.resume.len(),
            limits::MAX_POOL_ENTRIES,
        )?;
        Ok(index)
    }
}

pub(crate) fn build_constant_image(
    inputs: &ValidatedEmissionInputs<'_>,
) -> Result<ConstantImage, BytecodeEmissionError> {
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
                checked_index(index, "indexing canonical constant types")?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, BytecodeEmissionError>>()?;
    let pools = BytecodePools {
        types: canonical_types
            .into_values()
            .map(|ty| BytecodePoolEntry::TypeRef { ty })
            .collect(),
        ..BytecodePools::default()
    };

    merge_graphs(inputs, pools, &type_indices)
}

fn collect_canonical_types(
    inputs: &ValidatedEmissionInputs<'_>,
) -> Result<BTreeMap<String, TypeRefIr>, BytecodeEmissionError> {
    let mut types = BTreeMap::new();
    for (symbol, validated) in &inputs.constants {
        validate_local_types(
            validated.module_path,
            validated.type_count,
            &format!("constant `{symbol}` type"),
            &validated.constant.ty,
        )?;
        insert_type(
            &mut types,
            qualify_local_types(validated.module_path, &validated.constant.ty),
            format!("constant `{symbol}` type"),
        )?;
        if let ValueTransferPlan::FromType { ty } = validated.plan {
            validate_local_types(
                validated.module_path,
                validated.type_count,
                &format!("constant `{symbol}` transfer plan"),
                ty,
            )?;
        }
    }
    for (function_key, function) in &inputs.functions {
        let module_path = function.origin.module_path.as_str();
        let unit = inputs.units.get(module_path).ok_or_else(|| {
            BytecodeEmissionError::CanonicalSerialization {
                context: format!("function `{function_key}` owner"),
                message: "MIR unit disappeared from validated inputs".to_string(),
            }
        })?;
        collect_function_types(
            &mut types,
            unit,
            function_key,
            function,
            module_path,
            unit.type_table.len(),
        )?;
    }
    Ok(types)
}

fn collect_function_types(
    types: &mut BTreeMap<String, TypeRefIr>,
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    module_path: &str,
    type_count: usize,
) -> Result<(), BytecodeEmissionError> {
    let mut insert = |ty: &TypeRefIr, label: &str| -> Result<(), BytecodeEmissionError> {
        let context = format!("function `{function_key}` {label}");
        validate_local_types(module_path, type_count, &context, ty)?;
        insert_type(types, qualify_local_types(module_path, ty), context)
    };
    insert(&function.return_type, "return type")?;
    for parameter in &function.params {
        insert(&parameter.ty, &format!("parameter `{}`", parameter.name))?;
    }
    if let Some(ty) = &function.self_type {
        insert(ty, "self type")?;
    }
    if let Some(receiver) = &function.receiver {
        insert(&receiver.ty, "receiver type")?;
    }
    for slot in &function.slots {
        if let Some(ty) = &slot.ty {
            insert(ty, &format!("slot `{}` type", slot.name))?;
        }
    }
    for expression in &function.expressions {
        insert(
            &expression.ty,
            &format!("expression {} type", expression.index),
        )?;
        if let ExprIr::Call { call } = &expression.expression {
            if let Some(ty) = &call.concrete_receiver {
                insert(
                    ty,
                    &format!("expression {} concrete receiver", expression.index),
                )?;
            }
            for ty in call.type_args.values() {
                insert(
                    ty,
                    &format!("expression {} type argument", expression.index),
                )?;
            }
        }
    }
    let _ = unit;
    Ok(())
}

fn merge_graphs(
    inputs: &ValidatedEmissionInputs<'_>,
    mut pools: BytecodePools,
    type_indices: &BTreeMap<String, u32>,
) -> Result<ConstantImage, BytecodeEmissionError> {
    let mut nodes = Vec::new();
    let mut roots = BTreeMap::new();

    for (symbol, validated) in &inputs.constants {
        let graph = validated.bundle.graph(symbol)?;
        let base = checked_index(nodes.len(), "offsetting a frozen constant graph")?;
        let local_root = validated.bundle.root(symbol)?;
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
            let node = validated.bundle.node(symbol, local_index)?;
            nodes.push(relocate_node(symbol, local_index, node, base)?);
        }

        let node_index =
            base.checked_add(local_root)
                .ok_or(BytecodeEmissionError::ArithmeticOverflow {
                    context: "relocating a frozen constant root",
                })?;
        let qualified_type = qualify_local_types(validated.module_path, &validated.constant.ty);
        let type_key = type_key(&qualified_type, &format!("constant `{symbol}` type"))?;
        let type_ref = *type_indices.get(&type_key).ok_or_else(|| {
            BytecodeEmissionError::CanonicalSerialization {
                context: format!("constant `{symbol}` type"),
                message: "qualified type disappeared from the canonical pool".to_string(),
            }
        })?;
        let plan = qualify_transfer_plan(validated.module_path, validated.plan);
        let pool_index = checked_index(pools.constants.len(), "indexing canonical constant roots")?;
        pools.constants.push(BytecodePoolEntry::ConstantRef {
            reference: BytecodeConstantRef::LocalNode { node_index },
            type_ref,
            plan,
        });
        if roots.insert(symbol.clone(), pool_index).is_some() {
            return Err(BytecodeEmissionError::DuplicateConstantSymbol {
                symbol: symbol.clone(),
            });
        }
    }

    check_limit(
        "MAX_POOL_ENTRIES",
        "image.pools.constants",
        pools.constants.len(),
        limits::MAX_POOL_ENTRIES,
    )?;
    Ok(ConstantImage {
        pools,
        roots,
        graph: FrozenConstantGraph { nodes },
        type_indices: type_indices.clone(),
        shape_indices: BTreeMap::new(),
        writable_path_indices: BTreeMap::new(),
    })
}

fn relocate_node(
    symbol: &str,
    node_index: u32,
    node: &FrozenConstantNode,
    base: u32,
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
        FrozenConstantNode::Record { children, .. } => {
            let _ = relocate_children(children)?;
            Err(BytecodeEmissionError::UnsupportedConstantNode {
                symbol: symbol.to_string(),
                node_index,
                construct: "Record",
                reason: "the frozen shape sidecar has no nominal owner, field names, or explicit field transfer plans",
            })
        }
        FrozenConstantNode::Representation { value, .. } => {
            let _ = relocate_children(std::slice::from_ref(value))?;
            Err(BytecodeEmissionError::UnsupportedConstantNode {
                symbol: symbol.to_string(),
                node_index,
                construct: "Representation",
                reason: "producer-owned representation type/value facts are not yet connected to emission",
            })
        }
        FrozenConstantNode::Implementation { record, .. } => {
            let _ = relocate_children(std::slice::from_ref(record))?;
            Err(BytecodeEmissionError::UnsupportedConstantNode {
                symbol: symbol.to_string(),
                node_index,
                construct: "Implementation/Behavior",
                reason: "producer-owned implementation/behavior ownership facts are not yet connected to emission",
            })
        }
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

fn qualify_transfer_plan(module_path: &str, plan: &ValueTransferPlan) -> ValueTransferPlan {
    match plan {
        ValueTransferPlan::FromType { ty } => ValueTransferPlan::FromType {
            ty: qualify_local_types(module_path, ty),
        },
        other => other.clone(),
    }
}

pub(crate) fn qualify_local_types(module_path: &str, ty: &TypeRefIr) -> TypeRefIr {
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
    use skiff_artifact_model::{NominalTypeRefBaseIr, TypeRefIr, ValueTransferPlan};

    use super::{qualify_local_types, qualify_transfer_plan};

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

    #[test]
    fn from_type_plan_is_owner_qualified_without_changing_policy() {
        let plan = qualify_transfer_plan(
            "alpha",
            &ValueTransferPlan::FromType {
                ty: TypeRefIr::LocalType { type_index: 2 },
            },
        );
        assert_eq!(
            plan,
            ValueTransferPlan::FromType {
                ty: TypeRefIr::PublicationType {
                    module_path: "alpha".to_string(),
                    type_index: 2,
                },
            }
        );
    }
}
