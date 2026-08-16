use skiff_artifact_model::{BytecodeConstantRef, BytecodePoolEntry, LiteralIr, TypeRefIr};
use skiff_runtime_linked_bytecode::{ConstantIndex, FrozenConstantNodeIndex};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{
    BytecodeLinkError, BytecodeLinkLimit, BytecodeLinkLocation, BytecodeLinkObligation,
};

use super::super::{unsatisfied, DeploymentLinker};

impl DeploymentLinker<'_> {
    pub(super) fn preflight_constant_authority(&mut self) -> Result<(), BytecodeLinkError> {
        let mut constant_count = 0_u64;
        let mut root_count = 0_u64;
        let mut node_count = 0_u64;
        for package in self
            .deployment
            .packages()
            .values()
            .filter(|package| package.has_bytecode())
        {
            let view = package
                .bytecode()
                .ok_or_else(|| unavailable(self.package_location(package)))?
                .view();
            let pools = view.pools();
            let location = self.package_location(package);
            let package_constants = count(pools.constants.len(), location.clone())?;
            let package_roots = count(view.constant_roots().len(), location.clone())?;
            let package_nodes = count(view.frozen_constant_graph().nodes.len(), location.clone())?;
            let package_edges =
                view.frozen_constant_graph()
                    .nodes
                    .iter()
                    .try_fold(0_u64, |edges, node| {
                        checked_add(
                            edges,
                            count(node.children().len(), location.clone())?,
                            location.clone(),
                            "summing package constant graph edges",
                        )
                    })?;
            self.tracker
                .add_constant_graph(package_nodes, package_edges, location.clone())?;
            constant_count = checked_add(
                constant_count,
                package_constants,
                location.clone(),
                "summing constant rows",
            )?;
            root_count = checked_add(
                root_count,
                package_roots,
                location.clone(),
                "summing constant roots",
            )?;
            node_count = checked_add(
                node_count,
                package_nodes,
                location,
                "summing constant nodes",
            )?;
        }
        let deployment_location = self.deployment_location();
        self.tracker
            .add_image_table(constant_count, deployment_location.clone())?;
        self.tracker
            .add_image_table(root_count, deployment_location.clone())?;
        self.tracker
            .add_image_table(node_count, deployment_location)?;
        for package in self
            .deployment
            .packages()
            .values()
            .filter(|package| package.has_bytecode())
        {
            self.require_local_constant_pool(package)?;
        }
        Ok(())
    }

    fn require_local_constant_pool(
        &self,
        package: &HydratedBytecodePackage,
    ) -> Result<(), BytecodeLinkError> {
        let view = package
            .bytecode()
            .ok_or_else(|| unavailable(self.package_location(package)))?
            .view();
        let pools = view.pools();
        for (position, entry) in pools.constants.iter().enumerate() {
            let _ = artifact_constant_index(
                position,
                pools.constants.len(),
                self.package_location(package),
            )?;
            if !matches!(
                entry,
                BytecodePoolEntry::ConstantRef {
                    reference: BytecodeConstantRef::LocalNode { .. },
                    ..
                }
            ) {
                return Err(unavailable(self.package_location(package)));
            }
        }
        Ok(())
    }
}

pub(super) fn require_literal_carrier(
    literal: &LiteralIr,
    ty: &TypeRefIr,
    physical_carrier: Option<&TypeRefIr>,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    match ty {
        TypeRefIr::Literal { value } if value == literal => Ok(()),
        TypeRefIr::Literal { .. } => Err(constant_error(
            location,
            "literal carrier value differs from the frozen literal".to_string(),
        )),
        TypeRefIr::Builtin { name, args } if args.is_empty() => {
            let expected = match literal {
                LiteralIr::Null => "null",
                LiteralIr::Bool { .. } => "bool",
                LiteralIr::Number { .. } => "number",
                LiteralIr::String { .. } => "string",
            };
            if name == expected {
                Ok(())
            } else if matches!(name.as_str(), "null" | "bool" | "number" | "string") {
                Err(constant_error(
                    location,
                    format!("frozen {expected} literal is declared as builtin {name}"),
                ))
            } else {
                Err(unavailable(location))
            }
        }
        _ => match physical_carrier {
            Some(TypeRefIr::Builtin { name, args })
                if args.is_empty()
                    && matches!(
                        (literal, name.as_str()),
                        (LiteralIr::Number { .. }, "number")
                    ) =>
            {
                Ok(())
            }
            Some(_) => Err(constant_error(
                location,
                "frozen literal differs from its exact physical representation carrier".to_string(),
            )),
            None => Err(unavailable(location)),
        },
    }
}

pub(super) fn unavailable(location: BytecodeLinkLocation) -> BytecodeLinkError {
    BytecodeLinkError::ImplementationUnavailable {
        obligation: BytecodeLinkObligation::ConstantInitializationPlan,
        location,
    }
}

pub(in crate::bytecode) fn constant_error(
    location: BytecodeLinkLocation,
    detail: String,
) -> BytecodeLinkError {
    unsatisfied(
        BytecodeLinkObligation::ConstantInitializationPlan,
        location,
        detail,
    )
}

fn checked_add(
    current: u64,
    additional: u64,
    location: BytecodeLinkLocation,
    context: &'static str,
) -> Result<u64, BytecodeLinkError> {
    current
        .checked_add(additional)
        .ok_or_else(|| constant_error(location, format!("arithmetic overflow while {context}")))
}

fn count(value: usize, location: BytecodeLinkLocation) -> Result<u64, BytecodeLinkError> {
    u64::try_from(value)
        .map_err(|_| constant_error(location, "table count does not fit u64".to_string()))
}

pub(in crate::bytecode) fn constant_location(
    package: &HydratedBytecodePackage,
    position: usize,
    node_count: usize,
) -> Result<BytecodeLinkLocation, BytecodeLinkError> {
    let package_location = BytecodeLinkLocation::Package {
        package: Box::new(package.reference().clone()),
    };
    let actual = count(node_count, package_location.clone())?;
    let node_index = u32::try_from(position).map_err(|_| BytecodeLinkError::LimitExceeded {
        limit: BytecodeLinkLimit::ConstantGraphNodes,
        actual,
        max: u64::from(u32::MAX),
        location: package_location,
    })?;
    Ok(BytecodeLinkLocation::Constant {
        package: Box::new(package.reference().clone()),
        node_index,
    })
}

pub(super) fn artifact_node_index(
    position: usize,
    node_count: usize,
    location: BytecodeLinkLocation,
) -> Result<u32, BytecodeLinkError> {
    let actual = count(node_count, location.clone())?;
    u32::try_from(position).map_err(|_| BytecodeLinkError::LimitExceeded {
        limit: BytecodeLinkLimit::ConstantGraphNodes,
        actual,
        max: u64::from(u32::MAX),
        location,
    })
}

pub(super) fn artifact_constant_index(
    position: usize,
    constant_count: usize,
    location: BytecodeLinkLocation,
) -> Result<u32, BytecodeLinkError> {
    let actual = count(constant_count, location.clone())?;
    u32::try_from(position).map_err(|_| BytecodeLinkError::LimitExceeded {
        limit: BytecodeLinkLimit::ImageTableEntries,
        actual,
        max: u64::from(u32::MAX),
        location,
    })
}

pub(super) fn frozen_node_index(
    position: usize,
    location: BytecodeLinkLocation,
) -> Result<FrozenConstantNodeIndex, BytecodeLinkError> {
    let actual = checked_add(
        count(position, location.clone())?,
        1,
        location.clone(),
        "computing linked frozen-node table size",
    )?;
    u32::try_from(position)
        .map(FrozenConstantNodeIndex::new)
        .map_err(|_| BytecodeLinkError::LimitExceeded {
            limit: BytecodeLinkLimit::ConstantGraphNodes,
            actual,
            max: u64::from(u32::MAX),
            location,
        })
}

pub(super) fn constant_index(
    position: usize,
    location: BytecodeLinkLocation,
) -> Result<ConstantIndex, BytecodeLinkError> {
    let actual = checked_add(
        count(position, location.clone())?,
        1,
        location.clone(),
        "computing linked constant table size",
    )?;
    u32::try_from(position)
        .map(ConstantIndex::new)
        .map_err(|_| BytecodeLinkError::LimitExceeded {
            limit: BytecodeLinkLimit::ImageTableEntries,
            actual,
            max: u64::from(u32::MAX),
            location,
        })
}
