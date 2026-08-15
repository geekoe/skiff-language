mod validation;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BytecodeConstantRef, BytecodePoolEntry, FrozenConstantNode, PackageBuildId,
};
use skiff_runtime_linked_bytecode::{
    ArtifactConstantIndex, ArtifactConstantNodeIndex, ConstantIndex, FrozenConstantNodeIndex,
    LinkedArtifactPoolOrigin, LinkedConstantEntry, LinkedConstantReference, LinkedConstantRoot,
    LinkedConstantSymbolPath, LinkedFrozenConstantNode, LinkedFrozenConstantValue,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{
    types::TypeLinker, BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation,
};

use self::validation::{
    artifact_constant_index, artifact_node_index, constant_error, constant_index,
    constant_location, frozen_node_index, require_literal_carrier, source_literal, unavailable,
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
            self.link_literal_nodes(package, &mut nodes, &mut node_origins)?;
            self.link_package_constants(
                package,
                type_linker,
                &node_origins,
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

    fn link_literal_nodes(
        &self,
        package: &HydratedBytecodePackage,
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
            let FrozenConstantNode::Literal { literal } = node else {
                return Err(unavailable(location));
            };
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
            linked.push(LinkedFrozenConstantNode::new(
                index,
                origin,
                LinkedFrozenConstantValue::Literal(literal.clone()),
            ));
        }
        Ok(())
    }

    fn link_package_constants(
        &self,
        package: &HydratedBytecodePackage,
        type_linker: &mut TypeLinker<'_>,
        node_origins: &BTreeMap<NodeOriginKey, FrozenConstantNodeIndex>,
        linked: &mut Vec<LinkedConstantEntry>,
        origins: &mut BTreeMap<ConstantOriginKey, ConstantIndex>,
    ) -> Result<(), BytecodeLinkError> {
        let view = package
            .bytecode()
            .ok_or_else(|| unavailable(self.package_location(package)))?
            .view();
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
            let literal = source_literal(package, *node_index, location.clone())?;
            let (ty, concrete_type) =
                type_linker.intern_package_global_type(package, *type_ref, location.clone())?;
            let physical_carrier = type_linker
                .linked_representation_carrier(ty)
                .and_then(|carrier| type_linker.linked_type_ref(carrier.physical_carrier_type()));
            require_literal_carrier(literal, &concrete_type, physical_carrier, location.clone())?;
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
