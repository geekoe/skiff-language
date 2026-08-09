use skiff_artifact_model::FrozenConstantNode;

use crate::bytecode::{
    BytecodeLinkError, BytecodeLinkLimit, BytecodeLinkLocation, BytecodeLinkObligation,
};

use super::DeploymentLinker;

impl DeploymentLinker<'_> {
    /// Rejects package-wide facts whose exact candidate plans are outside this
    /// narrow checkpoint. This runs before root discovery so unreachable
    /// constant or ABI facts cannot be silently erased from the image.
    pub(super) fn reject_unsupported_global_authorities(&self) -> Result<(), BytecodeLinkError> {
        if let Some(dependency) = self.deployment.service_dependencies().values().next() {
            return Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::ConcreteTargetTables,
                location: BytecodeLinkLocation::ServiceDependency {
                    key: dependency.key().clone(),
                },
            });
        }
        for package in self.deployment.packages().values() {
            if !package.artifact().actor_implementations.is_empty()
                || !package.artifact().local_interface_conformances.is_empty()
            {
                return Err(BytecodeLinkError::ImplementationUnavailable {
                    obligation: BytecodeLinkObligation::ConcreteTargetTables,
                    location: self.package_location(package),
                });
            }
            let view = package.bytecode().view();
            if let Some((_, constant_index)) = view.constant_roots().first_key_value() {
                return Err(BytecodeLinkError::ImplementationUnavailable {
                    obligation: BytecodeLinkObligation::ConstantInitializationPlan,
                    location: BytecodeLinkLocation::Constant {
                        package: Box::new(package.reference().clone()),
                        node_index: *constant_index,
                    },
                });
            }
            if let Some((node_index, _)) = view
                .frozen_constant_graph()
                .nodes
                .iter()
                .enumerate()
                .find(|(_, node)| matches!(node, FrozenConstantNode::Implementation { .. }))
            {
                let node_index =
                    u32::try_from(node_index).map_err(|_| BytecodeLinkError::LimitExceeded {
                        limit: BytecodeLinkLimit::ConstantGraphNodes,
                        actual: view.frozen_constant_graph().nodes.len() as u64,
                        max: u32::MAX as u64,
                        location: self.package_location(package),
                    })?;
                return Err(BytecodeLinkError::ImplementationUnavailable {
                    obligation: BytecodeLinkObligation::ConstantInitializationPlan,
                    location: BytecodeLinkLocation::Constant {
                        package: Box::new(package.reference().clone()),
                        node_index,
                    },
                });
            }
        }
        Ok(())
    }
}
