use std::collections::BTreeMap;

use skiff_artifact_model::{
    native_value_lifecycle_registry, LiteralIr, NativeResourceDropPlan, NativeValueAdapterRole,
    NativeValueDropPlan, NativeValueEmbedding, NativeValueLifecycleConcrete, ResourceDropPlan,
    TypeRefIr, ValueDropPlan, ValueTransferPlan,
};
use skiff_runtime_linked_bytecode::{
    LinkedResourceDropPlan, LinkedValueDropPlan, LinkedValueTransferPlan,
};

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation};

use super::interner::TypeLinker;

impl TypeLinker<'_> {
    pub(in crate::bytecode) fn link_transfer_plan(
        &self,
        plan: &ValueTransferPlan,
        _substitutions: &BTreeMap<String, TypeRefIr>,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
        match plan {
            ValueTransferPlan::SnapshotShare { drop } => {
                Ok(LinkedValueTransferPlan::SnapshotShare {
                    drop: self.link_value_drop(drop, location)?,
                })
            }
            ValueTransferPlan::MoveOnly { drop } => Ok(LinkedValueTransferPlan::MoveOnly {
                drop: self.link_value_drop(drop, location)?,
            }),
            ValueTransferPlan::AffineResource { drop } => {
                Ok(LinkedValueTransferPlan::AffineResource {
                    drop: self.link_resource_drop(drop, location)?,
                })
            }
            ValueTransferPlan::ExplicitCloneLease {
                clone_adapter,
                drop,
            } => Ok(LinkedValueTransferPlan::ExplicitCloneLease {
                clone_adapter: lifecycle_adapter(
                    &clone_adapter.binding_key,
                    NativeValueAdapterRole::CloneLease,
                    location.clone(),
                )?,
                drop: self.link_resource_drop(drop, location)?,
            }),
            ValueTransferPlan::FromType { .. } => {
                Err(BytecodeLinkError::ImplementationUnavailable {
                    obligation: BytecodeLinkObligation::FrameAndValueTransferPlan,
                    location,
                })
            }
        }
    }

    pub(in crate::bytecode) fn plan_for_concrete_type(
        &self,
        ty: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
        let resolution = native_value_lifecycle_registry()
            .lookup(ty)
            .map_err(|error| {
                obligation_error(
                    BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location,
                    format!("container position has no authoritative lifecycle: {error}"),
                )
            })?;
        Ok(link_native_lifecycle(resolution.lifecycle))
    }

    /// Eliminates a constant-local `FromType` only after checking that it names
    /// the exact linked type and that the authoritative lifecycle is an
    /// ordinary snapshot. Frame-local `FromType` remains unsupported.
    pub(in crate::bytecode) fn link_constant_plan(
        &self,
        declared: &ValueTransferPlan,
        concrete_type: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
        let registry_type = lifecycle_registry_type(concrete_type);
        let resolution = native_value_lifecycle_registry()
            .lookup(&registry_type)
            .map_err(|error| constant_plan_error(location.clone(), error.to_string()))?;
        if resolution.embedding != NativeValueEmbedding::Ordinary
            || !matches!(
                &resolution.lifecycle,
                NativeValueLifecycleConcrete::SnapshotShare { .. }
            )
        {
            return Err(constant_plan_error(
                location,
                "frozen constant type is not an Ordinary SnapshotShare value".to_string(),
            ));
        }
        let expected = link_native_lifecycle(resolution.lifecycle);
        let actual = match declared {
            ValueTransferPlan::FromType { ty } if ty == concrete_type => expected.clone(),
            ValueTransferPlan::FromType { .. } => {
                return Err(constant_plan_error(
                    location,
                    "constant FromType plan does not name its exact linked type".to_string(),
                ));
            }
            concrete => self
                .link_transfer_plan(concrete, &BTreeMap::new(), location.clone())
                .map_err(|error| constant_plan_error(location.clone(), error.to_string()))?,
        };
        if actual != expected {
            return Err(constant_plan_error(
                location,
                "constant plan differs from its authoritative concrete lifecycle".to_string(),
            ));
        }
        Ok(actual)
    }

    fn link_value_drop(
        &self,
        drop: &ValueDropPlan,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueDropPlan, BytecodeLinkError> {
        match drop {
            ValueDropPlan::Trivial => Ok(LinkedValueDropPlan::Trivial),
            ValueDropPlan::SnapshotRelease => Ok(LinkedValueDropPlan::SnapshotRelease),
            ValueDropPlan::NativeAdapter { adapter } => Ok(LinkedValueDropPlan::NativeAdapter {
                adapter: lifecycle_adapter(
                    &adapter.binding_key,
                    NativeValueAdapterRole::ValueDrop,
                    location,
                )?,
            }),
            ValueDropPlan::RecursiveShape { .. } => {
                Err(BytecodeLinkError::ImplementationUnavailable {
                    obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location,
                })
            }
        }
    }

    fn link_resource_drop(
        &self,
        drop: &ResourceDropPlan,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedResourceDropPlan, BytecodeLinkError> {
        match drop {
            ResourceDropPlan::ResourceTableRelease => {
                Ok(LinkedResourceDropPlan::ResourceTableRelease)
            }
            ResourceDropPlan::NativeAdapter { adapter } => {
                Ok(LinkedResourceDropPlan::NativeAdapter {
                    adapter: lifecycle_adapter(
                        &adapter.binding_key,
                        NativeValueAdapterRole::ResourceDrop,
                        location,
                    )?,
                })
            }
            ResourceDropPlan::RecursiveShape { .. } => {
                Err(BytecodeLinkError::ImplementationUnavailable {
                    obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location,
                })
            }
        }
    }
}

fn lifecycle_registry_type(ty: &TypeRefIr) -> TypeRefIr {
    match ty {
        TypeRefIr::Literal { value } => TypeRefIr::builtin(match value {
            LiteralIr::Null => "null",
            LiteralIr::Bool { .. } => "bool",
            LiteralIr::Number { .. } => "number",
            LiteralIr::String { .. } => "string",
        }),
        _ => ty.clone(),
    }
}

fn constant_plan_error(location: BytecodeLinkLocation, detail: String) -> BytecodeLinkError {
    BytecodeLinkError::UnsatisfiedObligation {
        obligation: BytecodeLinkObligation::ConstantInitializationPlan,
        location,
        detail,
    }
}

pub(super) fn type_metrics(
    ty: &TypeRefIr,
    location: &BytecodeLinkLocation,
) -> Result<(u64, u64), BytecodeLinkError> {
    let mut nodes = 0;
    let mut max_depth = 0;
    visit_type(ty, 1, &mut nodes, &mut max_depth, location)?;
    Ok((nodes, max_depth))
}

fn visit_type(
    ty: &TypeRefIr,
    depth: u64,
    nodes: &mut u64,
    max_depth: &mut u64,
    location: &BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    *nodes = nodes.checked_add(1).ok_or_else(|| {
        obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location.clone(),
            "arithmetic overflow while measuring a concrete type".to_string(),
        )
    })?;
    *max_depth = (*max_depth).max(depth);
    match ty {
        TypeRefIr::Builtin { args, .. } => visit_types(args, depth, nodes, max_depth, location)?,
        TypeRefIr::AppliedNominal { arguments, .. } => {
            visit_types(arguments, depth, nodes, max_depth, location)?;
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                visit_type(field, depth + 1, nodes, max_depth, location)?;
            }
        }
        TypeRefIr::Union { items } => visit_types(items, depth, nodes, max_depth, location)?,
        TypeRefIr::Nullable { inner } => {
            visit_type(inner, depth + 1, nodes, max_depth, location)?;
        }
        TypeRefIr::AnyInterface { interface } => visit_types(
            &interface.canonical_type_args,
            depth,
            nodes,
            max_depth,
            location,
        )?,
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for parameter in params {
                visit_type(&parameter.ty, depth + 1, nodes, max_depth, location)?;
            }
            visit_type(return_type, depth + 1, nodes, max_depth, location)?;
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
    Ok(())
}

fn visit_types(
    types: &[TypeRefIr],
    depth: u64,
    nodes: &mut u64,
    max_depth: &mut u64,
    location: &BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    for ty in types {
        visit_type(ty, depth + 1, nodes, max_depth, location)?;
    }
    Ok(())
}

fn lifecycle_adapter(
    binding_key: &str,
    expected_role: NativeValueAdapterRole,
    location: BytecodeLinkLocation,
) -> Result<skiff_artifact_model::NativeValueLifecycleAdapter, BytecodeLinkError> {
    native_value_lifecycle_registry()
        .adapter(binding_key)
        .filter(|adapter| adapter.role == expected_role)
        .cloned()
        .ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::FrameAndValueTransferPlan,
                location,
                format!(
                    "lifecycle adapter {binding_key:?} is absent or has the wrong authoritative role"
                ),
            )
        })
}

fn link_native_lifecycle(lifecycle: NativeValueLifecycleConcrete) -> LinkedValueTransferPlan {
    match lifecycle {
        NativeValueLifecycleConcrete::SnapshotShare { drop } => {
            LinkedValueTransferPlan::SnapshotShare {
                drop: link_native_value_drop(drop),
            }
        }
        NativeValueLifecycleConcrete::MoveOnly { drop } => LinkedValueTransferPlan::MoveOnly {
            drop: link_native_value_drop(drop),
        },
        NativeValueLifecycleConcrete::AffineResource { drop } => {
            LinkedValueTransferPlan::AffineResource {
                drop: link_native_resource_drop(drop),
            }
        }
        NativeValueLifecycleConcrete::ExplicitCloneLease {
            clone_adapter,
            drop,
        } => LinkedValueTransferPlan::ExplicitCloneLease {
            clone_adapter,
            drop: link_native_resource_drop(drop),
        },
    }
}

fn link_native_value_drop(drop: NativeValueDropPlan) -> LinkedValueDropPlan {
    match drop {
        NativeValueDropPlan::Trivial => LinkedValueDropPlan::Trivial,
        NativeValueDropPlan::SnapshotRelease => LinkedValueDropPlan::SnapshotRelease,
        NativeValueDropPlan::NativeAdapter { adapter } => {
            LinkedValueDropPlan::NativeAdapter { adapter }
        }
    }
}

fn link_native_resource_drop(drop: NativeResourceDropPlan) -> LinkedResourceDropPlan {
    match drop {
        NativeResourceDropPlan::ResourceTableRelease => {
            LinkedResourceDropPlan::ResourceTableRelease
        }
        NativeResourceDropPlan::NativeAdapter { adapter } => {
            LinkedResourceDropPlan::NativeAdapter { adapter }
        }
    }
}

fn obligation_error(
    obligation: BytecodeLinkObligation,
    location: BytecodeLinkLocation,
    detail: String,
) -> BytecodeLinkError {
    BytecodeLinkError::UnsatisfiedObligation {
        obligation,
        location,
        detail,
    }
}
