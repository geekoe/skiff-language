pub(super) mod validation;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BytecodeConstantRef, BytecodePoolEntry, FrozenConstantNode, PackageBuildId,
};
use skiff_runtime_linked_bytecode::{
    ArtifactConstantIndex, ArtifactConstantNodeIndex, ArtifactFunctionKey, ConstantIndex,
    FrozenConstantNodeIndex, LinkedArtifactPoolOrigin, LinkedConstantEntry,
    LinkedConstantReference, LinkedConstantRoot, LinkedConstantSymbolPath,
    LinkedContainerLayoutKind, LinkedFrozenBehaviorBinding, LinkedFrozenConstantNode,
    LinkedFrozenConstantValue, LinkedValueTransferPlan, TypeIndex,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{
    types::TypeLinker, BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation,
};

use self::validation::{
    artifact_constant_index, artifact_node_index, constant_error, constant_index,
    constant_location, frozen_node_index, require_literal_carrier, unavailable,
};
use super::{unsatisfied, DeploymentLinker};

type ConstantOriginKey = (PackageBuildId, u32);
type NodeOriginKey = (PackageBuildId, u32);

pub(super) struct LinkedConstantTables {
    constants: Vec<LinkedConstantEntry>,
    roots: Vec<LinkedConstantRoot>,
    nodes: Vec<LinkedFrozenConstantNode>,
    constant_origins: BTreeMap<ConstantOriginKey, ConstantIndex>,
}

impl LinkedConstantTables {
    pub(super) fn constants(&self) -> &[LinkedConstantEntry] {
        &self.constants
    }

    pub(super) fn resolve(
        &self,
        package: &HydratedBytecodePackage,
        artifact_index: u32,
        location: BytecodeLinkLocation,
    ) -> Result<ConstantIndex, BytecodeLinkError> {
        self.constant_origins
            .get(&(package.reference().package_build_id.clone(), artifact_index))
            .copied()
            .ok_or_else(|| {
                unsatisfied(
                    BytecodeLinkObligation::ConstantInitializationPlan,
                    location,
                    format!(
                        "constant pool row {artifact_index} has no exact package-global linked row"
                    ),
                )
            })
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        Vec<LinkedConstantEntry>,
        Vec<LinkedConstantRoot>,
        Vec<LinkedFrozenConstantNode>,
    ) {
        (self.constants, self.roots, self.nodes)
    }
}

impl DeploymentLinker<'_> {
    pub(super) fn link_constant_tables(
        &mut self,
        type_linker: &mut TypeLinker<'_>,
    ) -> Result<LinkedConstantTables, BytecodeLinkError> {
        self.preflight_constant_authority()?;
        let mut constants = Vec::new();
        let mut roots = Vec::new();
        let mut nodes = Vec::new();
        let mut constant_origins = BTreeMap::new();
        let mut node_origins = BTreeMap::new();

        for package in self
            .deployment
            .packages()
            .values()
            .filter(|package| package.has_bytecode())
        {
            self.link_frozen_nodes(package, type_linker, &mut nodes, &mut node_origins)?;
            self.link_package_constants(
                package,
                type_linker,
                &node_origins,
                &nodes,
                &mut constants,
                &mut constant_origins,
            )?;
            self.link_package_constant_roots(package, &constant_origins, &mut roots)?;
        }

        Ok(LinkedConstantTables {
            constants,
            roots,
            nodes,
            constant_origins,
        })
    }

    fn link_frozen_nodes(
        &self,
        package: &HydratedBytecodePackage,
        type_linker: &mut TypeLinker<'_>,
        linked: &mut Vec<LinkedFrozenConstantNode>,
        origins: &mut BTreeMap<NodeOriginKey, FrozenConstantNodeIndex>,
    ) -> Result<(), BytecodeLinkError> {
        let source = &package
            .bytecode()
            .ok_or_else(|| unavailable(self.package_location(package)))?
            .view()
            .frozen_constant_graph()
            .nodes;
        for (position, node) in source.iter().enumerate() {
            let location = constant_location(package, position, source.len())?;
            let value =
                self.link_frozen_node(package, type_linker, node, origins, location.clone())?;
            let index = frozen_node_index(linked.len(), location.clone())?;
            let artifact_index = artifact_node_index(position, source.len(), location.clone())?;
            let origin = LinkedArtifactPoolOrigin::new(
                package.reference().package_build_id.clone(),
                ArtifactConstantNodeIndex::new(artifact_index),
                None,
            )
            .map_err(|error| constant_error(location.clone(), error.to_string()))?;
            let key = (package.reference().package_build_id.clone(), artifact_index);
            if origins.insert(key, index).is_some() {
                return Err(constant_error(
                    location,
                    "duplicate frozen constant node origin".to_string(),
                ));
            }
            linked.push(LinkedFrozenConstantNode::new(index, origin, value));
        }
        Ok(())
    }

    fn link_frozen_node(
        &self,
        package: &HydratedBytecodePackage,
        type_linker: &mut TypeLinker<'_>,
        node: &FrozenConstantNode,
        origins: &BTreeMap<NodeOriginKey, FrozenConstantNodeIndex>,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedFrozenConstantValue, BytecodeLinkError> {
        let build_id = package.reference().package_build_id.clone();
        let resolve_child = |artifact_index: u32| {
            origins
                .get(&(build_id.clone(), artifact_index))
                .copied()
                .ok_or_else(|| {
                    constant_error(
                        location.clone(),
                        format!(
                            "frozen constant node {artifact_index} was not linked before its parent"
                        ),
                    )
                })
        };
        match node {
            FrozenConstantNode::Literal { literal } => {
                Ok(LinkedFrozenConstantValue::Literal(literal.clone()))
            }
            FrozenConstantNode::Array { children } => {
                let children = children
                    .iter()
                    .map(|child| resolve_child(*child))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_boxed_slice();
                Ok(LinkedFrozenConstantValue::Array { children })
            }
            FrozenConstantNode::Record {
                shape_index,
                children,
            } => {
                let shape = type_linker
                    .intern_package_global_shape(package, *shape_index, location.clone())
                    .map_err(|error| graph_error(location.clone(), error))?;
                let row = type_linker.shape(shape).ok_or_else(|| {
                    constant_error(
                        location.clone(),
                        format!("linked shape {} is absent", shape.get()),
                    )
                })?;
                if row.fields().len() != children.len() {
                    return Err(constant_error(
                        location,
                        format!(
                            "frozen record shape {} declares {} fields but node has {} children",
                            shape.get(),
                            row.fields().len(),
                            children.len()
                        ),
                    ));
                }
                let children = children
                    .iter()
                    .map(|child| resolve_child(*child))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_boxed_slice();
                Ok(LinkedFrozenConstantValue::Record { shape, children })
            }
            FrozenConstantNode::Representation { type_ref, value } => {
                let ty = type_linker
                    .intern_package_global_type(package, *type_ref, location.clone())
                    .map_err(|error| graph_error(location.clone(), error))?
                    .0;
                let value = resolve_child(*value)?;
                Ok(LinkedFrozenConstantValue::Representation { ty, value })
            }
            FrozenConstantNode::Implementation { record, behaviors } => {
                let record = resolve_child(*record)?;
                let behaviors = behaviors
                    .iter()
                    .map(|behavior| {
                        let key = self
                            .key_for_receiver_function(package, &behavior.function_key, type_linker)
                            .map_err(|error| graph_error(location.clone(), error))?;
                        let function = type_linker.function_index(&key).ok_or_else(|| {
                            constant_error(
                                location.clone(),
                                format!(
                                    "frozen behavior {:?} has no linked function index",
                                    behavior.function_key
                                ),
                            )
                        })?;
                        let artifact_function_key = ArtifactFunctionKey::parse(
                            behavior.function_key.clone(),
                        )
                        .map_err(|error| constant_error(location.clone(), error.to_string()))?;
                        Ok(LinkedFrozenBehaviorBinding::new(
                            artifact_function_key,
                            function,
                        ))
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_boxed_slice();
                Ok(LinkedFrozenConstantValue::Implementation { record, behaviors })
            }
        }
    }

    fn link_package_constants(
        &self,
        package: &HydratedBytecodePackage,
        type_linker: &mut TypeLinker<'_>,
        node_origins: &BTreeMap<NodeOriginKey, FrozenConstantNodeIndex>,
        nodes: &[LinkedFrozenConstantNode],
        linked: &mut Vec<LinkedConstantEntry>,
        origins: &mut BTreeMap<ConstantOriginKey, ConstantIndex>,
    ) -> Result<(), BytecodeLinkError> {
        let view = package
            .bytecode()
            .ok_or_else(|| unavailable(self.package_location(package)))?
            .view();
        let mut expected_nodes = BTreeMap::new();
        for (position, entry) in view.pools().constants.iter().enumerate() {
            let package_location = self.package_location(package);
            let artifact_index = artifact_constant_index(
                position,
                view.pools().constants.len(),
                package_location.clone(),
            )?;
            let BytecodePoolEntry::ConstantRef {
                reference: BytecodeConstantRef::LocalNode { node_index },
                type_ref,
                plan,
            } = entry
            else {
                return Err(unavailable(package_location));
            };
            let location = BytecodeLinkLocation::Constant {
                package: Box::new(package.reference().clone()),
                node_index: *node_index,
            };
            let node = node_origins
                .get(&(package.reference().package_build_id.clone(), *node_index))
                .copied()
                .ok_or_else(|| {
                    constant_error(
                        location.clone(),
                        format!("local constant node {node_index} was not linked"),
                    )
                })?;
            let (ty, concrete_type) =
                type_linker.intern_package_global_type(package, *type_ref, location.clone())?;
            let physical_carrier = type_linker
                .linked_representation_carrier(ty)
                .and_then(|carrier| type_linker.linked_type_ref(carrier.physical_carrier_type()));
            let linked_plan = type_linker
                .link_transfer_plan(plan, &BTreeMap::new(), location.clone())
                .map_err(|error| constant_error(location.clone(), error.to_string()))?;
            let exact_type_plan = type_linker.linked_type_plan(ty).ok_or_else(|| {
                constant_error(
                    location.clone(),
                    format!("constant type row {} has no compiler-owned plan", ty.get()),
                )
            })?;
            if &linked_plan != exact_type_plan {
                return Err(constant_error(
                    location,
                    "constant plan differs from its exact TypeRef plan".to_string(),
                ));
            }
            let linked_node = linked_node(nodes, node, location.clone())?;
            if let LinkedFrozenConstantValue::Literal(literal) = linked_node.value() {
                require_literal_carrier(
                    literal,
                    &concrete_type,
                    physical_carrier,
                    location.clone(),
                )?;
            }
            validate_constant_node(
                type_linker,
                nodes,
                node,
                ty,
                &linked_plan,
                &mut expected_nodes,
                location.clone(),
            )?;
            let index = constant_index(linked.len(), location.clone())?;
            let origin = LinkedArtifactPoolOrigin::new(
                package.reference().package_build_id.clone(),
                ArtifactConstantIndex::new(artifact_index),
                None,
            )
            .map_err(|error| constant_error(location.clone(), error.to_string()))?;
            let key = (package.reference().package_build_id.clone(), artifact_index);
            if origins.insert(key, index).is_some() {
                return Err(constant_error(
                    location,
                    "duplicate constant pool origin".to_string(),
                ));
            }
            linked.push(LinkedConstantEntry::new(
                index,
                origin,
                LinkedConstantReference::LocalNode { node },
                ty,
                linked_plan,
            ));
        }
        Ok(())
    }

    fn link_package_constant_roots(
        &self,
        package: &HydratedBytecodePackage,
        origins: &BTreeMap<ConstantOriginKey, ConstantIndex>,
        linked: &mut Vec<LinkedConstantRoot>,
    ) -> Result<(), BytecodeLinkError> {
        for (symbol, artifact_index) in package
            .bytecode()
            .ok_or_else(|| unavailable(self.package_location(package)))?
            .view()
            .constant_roots()
        {
            let location = self.package_location(package);
            let constant = origins
                .get(&(
                    package.reference().package_build_id.clone(),
                    *artifact_index,
                ))
                .copied()
                .ok_or_else(|| {
                    constant_error(
                        location.clone(),
                        format!("constant root {symbol:?} has no linked pool row"),
                    )
                })?;
            let symbol = LinkedConstantSymbolPath::parse(symbol.clone())
                .map_err(|error| constant_error(location, error.to_string()))?;
            linked.push(LinkedConstantRoot::new(
                package.reference().package_build_id.clone(),
                symbol,
                constant,
            ));
        }
        Ok(())
    }
}

fn linked_node(
    nodes: &[LinkedFrozenConstantNode],
    node: FrozenConstantNodeIndex,
    location: BytecodeLinkLocation,
) -> Result<&LinkedFrozenConstantNode, BytecodeLinkError> {
    let position = usize::try_from(node.get()).map_err(|_| {
        constant_error(
            location.clone(),
            format!("linked frozen node {} does not fit usize", node.get()),
        )
    })?;
    nodes
        .get(position)
        .filter(|row| row.index() == node)
        .ok_or_else(|| {
            constant_error(
                location,
                format!("linked frozen node {} is absent", node.get()),
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn validate_constant_node(
    type_linker: &TypeLinker<'_>,
    nodes: &[LinkedFrozenConstantNode],
    node: FrozenConstantNodeIndex,
    expected_type: TypeIndex,
    expected_plan: &LinkedValueTransferPlan,
    expected: &mut BTreeMap<FrozenConstantNodeIndex, (TypeIndex, LinkedValueTransferPlan)>,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    if let Some((existing_type, existing_plan)) = expected.get(&node) {
        if *existing_type != expected_type || existing_plan != expected_plan {
            return Err(constant_error(
                location,
                format!(
                    "frozen constant node {} has ambiguous type/plan initialization",
                    node.get()
                ),
            ));
        }
        return Ok(());
    }

    let row = linked_node(nodes, node, location.clone())?;
    match row.value() {
        LinkedFrozenConstantValue::Literal(literal) => {
            let concrete_type = type_linker.linked_type_ref(expected_type).ok_or_else(|| {
                constant_error(
                    location.clone(),
                    format!("constant type row {} is absent", expected_type.get()),
                )
            })?;
            let physical_carrier = type_linker
                .linked_representation_carrier(expected_type)
                .and_then(|carrier| type_linker.linked_type_ref(carrier.physical_carrier_type()));
            require_literal_carrier(literal, concrete_type, physical_carrier, location.clone())?;
        }
        LinkedFrozenConstantValue::Array { children } => {
            let layout = type_linker
                .container_layout(expected_type)
                .filter(|layout| matches!(layout.kind(), LinkedContainerLayoutKind::Array))
                .ok_or_else(|| {
                    constant_error(
                        location.clone(),
                        format!(
                            "frozen array node {} has no exact Array carrier for type {}",
                            node.get(),
                            expected_type.get()
                        ),
                    )
                })?;
            let element = layout.element().ok_or_else(|| {
                constant_error(
                    location.clone(),
                    "linked Array layout has no exact element position".to_string(),
                )
            })?;
            for child in children {
                validate_constant_node(
                    type_linker,
                    nodes,
                    *child,
                    element.ty(),
                    element.plan(),
                    expected,
                    location.clone(),
                )?;
            }
        }
        LinkedFrozenConstantValue::Record { shape, children } => {
            let shape_row = type_linker.shape(*shape).ok_or_else(|| {
                constant_error(
                    location.clone(),
                    format!("linked shape {} is absent", shape.get()),
                )
            })?;
            if shape_row.nominal_type() != expected_type || shape_row.plan() != expected_plan {
                return Err(constant_error(
                    location.clone(),
                    format!(
                        "frozen record node {} shape {} disagrees with its exact constant type/plan",
                        node.get(),
                        shape.get()
                    ),
                ));
            }
            if shape_row.fields().len() != children.len() {
                return Err(constant_error(
                    location.clone(),
                    format!(
                        "frozen record node {} has {} children but shape {} declares {} fields",
                        node.get(),
                        children.len(),
                        shape.get(),
                        shape_row.fields().len()
                    ),
                ));
            }
            for (field, child) in shape_row.fields().iter().zip(children) {
                validate_constant_node(
                    type_linker,
                    nodes,
                    *child,
                    field.ty(),
                    field.plan(),
                    expected,
                    location.clone(),
                )?;
            }
        }
        LinkedFrozenConstantValue::Representation { ty, value } => {
            if *ty != expected_type {
                return Err(constant_error(
                    location.clone(),
                    format!(
                        "frozen representation node {} declares type {}, expected {}",
                        node.get(),
                        ty.get(),
                        expected_type.get()
                    ),
                ));
            }
            let carrier = type_linker
                .linked_representation_carrier(expected_type)
                .ok_or_else(|| {
                    constant_error(
                        location.clone(),
                        format!(
                            "frozen representation node {} type {} has no exact representation carrier",
                            node.get(),
                            expected_type.get()
                        ),
                    )
                })?;
            let physical_type = carrier.physical_carrier_type();
            let physical_plan = type_linker
                .linked_type_plan(physical_type)
                .cloned()
                .ok_or_else(|| {
                    constant_error(
                        location.clone(),
                        format!(
                            "frozen representation node {} physical carrier type {} has no plan",
                            node.get(),
                            physical_type.get()
                        ),
                    )
                })?;
            validate_constant_node(
                type_linker,
                nodes,
                *value,
                physical_type,
                &physical_plan,
                expected,
                location.clone(),
            )?;
        }
        LinkedFrozenConstantValue::Implementation { record, .. } => {
            validate_constant_node(
                type_linker,
                nodes,
                *record,
                expected_type,
                expected_plan,
                expected,
                location.clone(),
            )?;
        }
    }

    expected.insert(node, (expected_type, expected_plan.clone()));
    Ok(())
}

fn graph_error(location: BytecodeLinkLocation, error: BytecodeLinkError) -> BytecodeLinkError {
    match error {
        BytecodeLinkError::LimitExceeded { .. } => error,
        _ => constant_error(location, error.to_string()),
    }
}
