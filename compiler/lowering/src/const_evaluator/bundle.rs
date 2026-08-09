//! Owned constant graphs and their exact pool-index owners.

use std::collections::BTreeMap;

use skiff_artifact_model::{
    bytecode::dto::{FrozenConstantGraph, FrozenConstantNode},
    TypeRefIr,
};
use thiserror::Error;

use super::ConstEvaluatorError;

/// One File IR unit's frozen constants and the pools referenced by every
/// graph in that unit.
///
/// Index ownership is exact:
///
/// - graph child indices are local to that graph's `nodes` vector;
/// - `FrozenConstantNode::Representation::type_ref` indexes [`Self::types`];
/// - `FrozenConstantNode::Record::shape_index` indexes [`Self::shapes`];
/// - every [`FrozenConstantShape`] owner and field type indexes
///   [`Self::types`].
///
/// Graphs are keyed and iterated by ascending constant symbol. Types are
/// deduplicated and ordered by their canonical JSON encoding. Shapes are
/// deduplicated only when owner type, ordered field names, and field types are
/// all equal, then ordered lexicographically by those complete facts.
///
/// When an emitter combines graphs from this bundle into one artifact graph,
/// it must append graphs in symbol order and checked-add the current node
/// count to graph-local child indices. When it combines multiple unit
/// bundles, it must exact-match `module_path` to the owning MIR unit before
/// resolving `TypeRefIr::LocalType`; only then may it canonicalize image-wide
/// type and shape pools. It must never infer either pool from graph nodes or
/// reopen File IR.
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenConstantBundle {
    module_path: String,
    graphs: BTreeMap<String, FrozenConstantGraph>,
    types: Vec<TypeRefIr>,
    shapes: Vec<FrozenConstantShape>,
}

impl FrozenConstantBundle {
    pub(super) fn from_evaluated(
        module_path: String,
        evaluated: BTreeMap<String, EvaluatedConstant>,
    ) -> Result<Self, ConstEvaluatorError> {
        let mut types = Vec::new();
        for constant in evaluated.values() {
            for ty in &constant.types {
                if !types.contains(ty) {
                    types.push(ty.clone());
                }
            }
        }
        types.sort_by_key(type_pool_key);
        ensure_u32_len(&module_path, "types", types.len())?;

        let mut shapes = Vec::<FrozenConstantShape>::new();
        for (symbol, constant) in &evaluated {
            let type_map = local_type_map(&module_path, symbol, &constant.types, &types)?;
            for shape in &constant.shapes {
                let relocated = relocate_shape(&module_path, symbol, shape, &type_map)?;
                ensure_u32_len(&module_path, "shape fields", relocated.fields.len())?;
                if !shapes.contains(&relocated) {
                    shapes.push(relocated);
                }
            }
        }
        shapes.sort();
        ensure_u32_len(&module_path, "shapes", shapes.len())?;

        let mut graphs = BTreeMap::new();
        for (symbol, constant) in evaluated {
            let type_map = local_type_map(&module_path, &symbol, &constant.types, &types)?;
            let local_shape_keys = constant
                .shapes
                .iter()
                .map(|shape| relocate_shape(&module_path, &symbol, shape, &type_map))
                .collect::<Result<Vec<_>, _>>()?;
            let shape_map = local_shape_keys
                .iter()
                .map(|shape| {
                    shapes
                        .iter()
                        .position(|candidate| candidate == shape)
                        .ok_or_else(|| ConstEvaluatorError::BundleContract {
                            module_path: module_path.clone(),
                            symbol: symbol.clone(),
                            message: "relocated shape is absent from the canonical bundle pool"
                                .to_string(),
                        })
                        .and_then(|index| {
                            u32::try_from(index).map_err(|_| ConstEvaluatorError::BundleContract {
                                module_path: module_path.clone(),
                                symbol: symbol.clone(),
                                message: "shape index exceeds u32::MAX".to_string(),
                            })
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let graph =
                relocate_graph(&module_path, &symbol, constant.graph, &type_map, &shape_map)?;
            graphs.insert(symbol, graph);
        }

        Ok(Self {
            module_path,
            graphs,
            types,
            shapes,
        })
    }

    /// Module that owns local type indices in this bundle.
    pub fn module_path(&self) -> &str {
        &self.module_path
    }

    /// Frozen graphs in canonical ascending-symbol order.
    pub fn graphs(&self) -> &BTreeMap<String, FrozenConstantGraph> {
        &self.graphs
    }

    /// Resolves one full constant symbol to its owned graph.
    pub fn graph(&self, symbol: &str) -> Result<&FrozenConstantGraph, FrozenConstantLookupError> {
        self.graphs
            .get(symbol)
            .ok_or_else(|| FrozenConstantLookupError::MissingGraph {
                module_path: self.module_path.clone(),
                symbol: symbol.to_string(),
            })
    }

    /// Resolves a graph-local node index.
    pub fn node(
        &self,
        symbol: &str,
        node_index: u32,
    ) -> Result<&FrozenConstantNode, FrozenConstantLookupError> {
        let graph = self.graph(symbol)?;
        graph
            .nodes
            .get(node_index as usize)
            .ok_or_else(|| FrozenConstantLookupError::MissingNode {
                module_path: self.module_path.clone(),
                symbol: symbol.to_string(),
                node_index,
                node_count: graph.nodes.len(),
            })
    }

    /// Returns the graph-local root index. Evaluator roots are always the
    /// final node; an empty graph is rejected structurally.
    pub fn root(&self, symbol: &str) -> Result<u32, FrozenConstantLookupError> {
        let graph = self.graph(symbol)?;
        let Some(root) = graph.nodes.len().checked_sub(1) else {
            return Err(FrozenConstantLookupError::EmptyGraph {
                module_path: self.module_path.clone(),
                symbol: symbol.to_string(),
            });
        };
        u32::try_from(root).map_err(|_| FrozenConstantLookupError::NodeIndexOverflow {
            module_path: self.module_path.clone(),
            symbol: symbol.to_string(),
        })
    }

    /// Canonical bundle-owned type pool.
    pub fn types(&self) -> &[TypeRefIr] {
        &self.types
    }

    /// Resolves a representation node or shape owner/field index against this
    /// bundle.
    pub fn type_ref(&self, type_ref: u32) -> Result<&TypeRefIr, FrozenConstantLookupError> {
        self.types
            .get(type_ref as usize)
            .ok_or_else(|| FrozenConstantLookupError::MissingType {
                module_path: self.module_path.clone(),
                type_ref,
                type_count: self.types.len(),
            })
    }

    /// Canonical bundle-owned shape pool.
    pub fn shapes(&self) -> &[FrozenConstantShape] {
        &self.shapes
    }

    /// Resolves a `Record` node's shape index against this bundle.
    pub fn shape(
        &self,
        shape_index: u32,
    ) -> Result<&FrozenConstantShape, FrozenConstantLookupError> {
        self.shapes.get(shape_index as usize).ok_or_else(|| {
            FrozenConstantLookupError::MissingShape {
                module_path: self.module_path.clone(),
                shape_index,
                shape_count: self.shapes.len(),
            }
        })
    }

    /// Resolves a shape's exact nominal/structural owner type.
    pub fn shape_type(&self, shape_index: u32) -> Result<&TypeRefIr, FrozenConstantLookupError> {
        self.type_ref(self.shape(shape_index)?.type_ref)
    }

    /// Resolves one ordered field declaration in a bundle-owned shape.
    pub fn shape_field(
        &self,
        shape_index: u32,
        field_ordinal: u32,
    ) -> Result<&FrozenConstantShapeField, FrozenConstantLookupError> {
        let shape = self.shape(shape_index)?;
        shape.fields.get(field_ordinal as usize).ok_or_else(|| {
            FrozenConstantLookupError::MissingShapeField {
                module_path: self.module_path.clone(),
                shape_index,
                field_ordinal,
                field_count: shape.field_count(),
            }
        })
    }

    /// Resolves one shape field all the way to its bundle-owned type fact.
    pub fn shape_field_type(
        &self,
        shape_index: u32,
        field_ordinal: u32,
    ) -> Result<&TypeRefIr, FrozenConstantLookupError> {
        self.type_ref(self.shape_field(shape_index, field_ordinal)?.type_ref)
    }
}

/// One canonical dense-record shape. Owner and field types are indices into
/// the owning [`FrozenConstantBundle::types`] pool. Transfer/drop plans are
/// deliberately not invented here; the emitter must join its separate
/// source-owned transfer-plan input by this exact nominal shape.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FrozenConstantShape {
    type_ref: u32,
    fields: Vec<FrozenConstantShapeField>,
}

impl FrozenConstantShape {
    /// Exact nominal or structural owner type in the bundle pool.
    pub fn type_ref(&self) -> u32 {
        self.type_ref
    }

    /// Exact number of fields in this shape.
    pub fn field_count(&self) -> u32 {
        u32::try_from(self.fields.len()).expect("bundle construction checked shape field count")
    }

    /// Strict field-name order and bundle-owned type indices.
    pub fn fields(&self) -> &[FrozenConstantShapeField] {
        &self.fields
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FrozenConstantShapeField {
    name: String,
    type_ref: u32,
}

impl FrozenConstantShapeField {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn type_ref(&self) -> u32 {
        self.type_ref
    }
}

/// Structured checked-lookup failure for an already-built constant bundle.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FrozenConstantLookupError {
    #[error("constant bundle `{module_path}` has no graph for `{symbol}`")]
    MissingGraph { module_path: String, symbol: String },
    #[error("constant graph `{symbol}` in `{module_path}` has no root node")]
    EmptyGraph { module_path: String, symbol: String },
    #[error("constant graph `{symbol}` in `{module_path}` has more than u32::MAX nodes")]
    NodeIndexOverflow { module_path: String, symbol: String },
    #[error(
        "constant graph `{symbol}` in `{module_path}` has no node {node_index} (node count {node_count})"
    )]
    MissingNode {
        module_path: String,
        symbol: String,
        node_index: u32,
        node_count: usize,
    },
    #[error("constant bundle `{module_path}` has no type {type_ref} (type count {type_count})")]
    MissingType {
        module_path: String,
        type_ref: u32,
        type_count: usize,
    },
    #[error(
        "constant bundle `{module_path}` has no shape {shape_index} (shape count {shape_count})"
    )]
    MissingShape {
        module_path: String,
        shape_index: u32,
        shape_count: usize,
    },
    #[error(
        "constant bundle `{module_path}` shape {shape_index} has no field {field_ordinal} (field count {field_count})"
    )]
    MissingShapeField {
        module_path: String,
        shape_index: u32,
        field_ordinal: u32,
        field_count: u32,
    },
}

/// Private evaluator output before all graphs in one unit are relocated into
/// their shared canonical pools.
pub(super) struct EvaluatedConstant {
    pub graph: FrozenConstantGraph,
    pub types: Vec<TypeRefIr>,
    pub shapes: Vec<EvaluatedConstantShape>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EvaluatedConstantShape {
    pub type_ref: u32,
    pub fields: Vec<(String, u32)>,
}

fn ensure_u32_len(
    module_path: &str,
    pool: &'static str,
    len: usize,
) -> Result<(), ConstEvaluatorError> {
    if u32::try_from(len).is_err() {
        return Err(ConstEvaluatorError::BundleContract {
            module_path: module_path.to_string(),
            symbol: "<bundle>".to_string(),
            message: format!("{pool} pool length exceeds u32::MAX"),
        });
    }
    Ok(())
}

fn local_type_map(
    module_path: &str,
    symbol: &str,
    local_types: &[TypeRefIr],
    canonical_types: &[TypeRefIr],
) -> Result<Vec<u32>, ConstEvaluatorError> {
    local_types
        .iter()
        .map(|ty| {
            canonical_types
                .iter()
                .position(|candidate| candidate == ty)
                .ok_or_else(|| ConstEvaluatorError::BundleContract {
                    module_path: module_path.to_string(),
                    symbol: symbol.to_string(),
                    message: "local type is absent from the canonical bundle pool".to_string(),
                })
                .and_then(|index| {
                    u32::try_from(index).map_err(|_| ConstEvaluatorError::BundleContract {
                        module_path: module_path.to_string(),
                        symbol: symbol.to_string(),
                        message: "type index exceeds u32::MAX".to_string(),
                    })
                })
        })
        .collect()
}

fn relocate_shape(
    module_path: &str,
    symbol: &str,
    shape: &EvaluatedConstantShape,
    type_map: &[u32],
) -> Result<FrozenConstantShape, ConstEvaluatorError> {
    if shape
        .fields
        .windows(2)
        .any(|pair| pair[0].0.as_bytes() >= pair[1].0.as_bytes())
    {
        return Err(ConstEvaluatorError::BundleContract {
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
            message: "shape field names are not strictly ascending".to_string(),
        });
    }
    let relocate_type = |type_ref: u32| {
        type_map.get(type_ref as usize).copied().ok_or_else(|| {
            ConstEvaluatorError::BundleContract {
                module_path: module_path.to_string(),
                symbol: symbol.to_string(),
                message: format!(
                    "shape type index {type_ref} is out of bounds for local type count {}",
                    type_map.len()
                ),
            }
        })
    };
    let type_ref = relocate_type(shape.type_ref)?;
    let fields = shape
        .fields
        .iter()
        .map(|(name, type_ref)| {
            Ok(FrozenConstantShapeField {
                name: name.clone(),
                type_ref: relocate_type(*type_ref)?,
            })
        })
        .collect::<Result<Vec<_>, ConstEvaluatorError>>()?;
    Ok(FrozenConstantShape { type_ref, fields })
}

fn relocate_graph(
    module_path: &str,
    symbol: &str,
    graph: FrozenConstantGraph,
    type_map: &[u32],
    shape_map: &[u32],
) -> Result<FrozenConstantGraph, ConstEvaluatorError> {
    if graph.nodes.is_empty() {
        return Err(ConstEvaluatorError::BundleContract {
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
            message: "frozen constant graph is empty".to_string(),
        });
    }
    ensure_u32_len(module_path, "graph nodes", graph.nodes.len())?;
    let mut nodes = Vec::with_capacity(graph.nodes.len());
    for (node_index, mut node) in graph.nodes.into_iter().enumerate() {
        let node_index =
            u32::try_from(node_index).map_err(|_| ConstEvaluatorError::BundleContract {
                module_path: module_path.to_string(),
                symbol: symbol.to_string(),
                message: "graph node index exceeds u32::MAX".to_string(),
            })?;
        for child in node.children() {
            if *child >= node_index {
                return Err(ConstEvaluatorError::BundleContract {
                    module_path: module_path.to_string(),
                    symbol: symbol.to_string(),
                    message: format!(
                        "node {node_index} child {child} is not a strictly earlier graph-local index"
                    ),
                });
            }
        }
        match &mut node {
            FrozenConstantNode::Representation { type_ref, .. } => {
                let local_type_ref = *type_ref;
                *type_ref = type_map.get(local_type_ref as usize).copied().ok_or_else(|| {
                    ConstEvaluatorError::BundleContract {
                        module_path: module_path.to_string(),
                        symbol: symbol.to_string(),
                        message: format!(
                            "node {node_index} type index {local_type_ref} is out of bounds for local type count {}",
                            type_map.len()
                        ),
                    }
                })?;
            }
            FrozenConstantNode::Record { shape_index, .. } => {
                let local_shape_index = *shape_index;
                *shape_index = shape_map
                    .get(local_shape_index as usize)
                    .copied()
                    .ok_or_else(|| ConstEvaluatorError::BundleContract {
                        module_path: module_path.to_string(),
                        symbol: symbol.to_string(),
                        message: format!(
                            "node {node_index} shape index {local_shape_index} is out of bounds for local shape count {}",
                            shape_map.len()
                        ),
                    })?;
            }
            FrozenConstantNode::Literal { .. }
            | FrozenConstantNode::Array { .. }
            | FrozenConstantNode::Implementation { .. } => {}
        }
        nodes.push(node);
    }
    Ok(FrozenConstantGraph { nodes })
}

fn type_pool_key(ty: &TypeRefIr) -> String {
    serde_json::to_string(ty).expect("TypeRefIr serializes without failure")
}
