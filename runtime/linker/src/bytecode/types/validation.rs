use std::collections::BTreeMap;

use skiff_artifact_model::{
    native_value_lifecycle_registry, CallableRegistryTypeExpression, NativeValueAdapterRole,
    PackageRefIr, PrivilegedAffineCompositeIdentity, ResourceDropPlan, TypeRefIr, ValueDropPlan,
    ValueTransferPlan,
};
use skiff_runtime_linked_bytecode::{
    LinkedResourceDropPlan, LinkedShapeEntry, LinkedShapeField, LinkedValueDropPlan,
    LinkedValueTransferPlan, ShapeIndex, SpecializationKey, TypeIndex,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation};

use super::interner::TypeLinker;

impl TypeLinker<'_> {
    /// Links the compiler plan stored on one exact TypeRef row while that row
    /// is reserved. The explicit index closes the only recursive-shape cycle;
    /// no nominal or registry lookup is used to recover a plan.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::bytecode) fn link_type_entry_plan_at(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        substitutions: &BTreeMap<String, TypeRefIr>,
        type_index: TypeIndex,
        declared: &ValueTransferPlan,
        concrete_type: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
        let ValueTransferPlan::MoveOnly {
            drop: ValueDropPlan::RecursiveShape { shape_ref },
        } = declared
        else {
            return self.link_plan_for_type_at(
                package,
                specialization,
                substitutions,
                declared,
                concrete_type,
                location,
            );
        };
        let shape = self.intern_pool_shape(
            package,
            specialization,
            *shape_ref,
            substitutions,
            location.clone(),
        )?;
        let row = self.shape(shape).ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                format!("recursive type plan shape {} is absent", shape.get()),
            )
        })?;
        if row.nominal_type() != type_index || row.privileged_affine_composite().is_none() {
            return Err(obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location,
                format!(
                    "recursive compiler plan for {concrete_type:?} does not bind its exact TypeRef row"
                ),
            ));
        }
        Ok(LinkedValueTransferPlan::MoveOnly {
            drop: LinkedValueDropPlan::RecursiveShape { shape },
        })
    }

    /// Links a lifecycle plan at an exact artifact position. Recursive shape
    /// references are never accepted without the current package and
    /// specialization, so a linked `ShapeIndex` cannot be manufactured from a
    /// type name or a registry lifecycle alone.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::bytecode) fn link_plan_for_type_at(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        substitutions: &BTreeMap<String, TypeRefIr>,
        declared: &ValueTransferPlan,
        concrete_type: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
        match declared {
            ValueTransferPlan::FromType { .. } => {
                Err(BytecodeLinkError::ImplementationUnavailable {
                    obligation: BytecodeLinkObligation::FrameAndValueTransferPlan,
                    location,
                })
            }
            ValueTransferPlan::MoveOnly {
                drop: ValueDropPlan::RecursiveShape { shape_ref },
            } => {
                let shape = self.intern_pool_shape(
                    package,
                    specialization,
                    *shape_ref,
                    substitutions,
                    location.clone(),
                )?;
                self.require_privileged_root_shape(shape, concrete_type, location)?;
                Ok(LinkedValueTransferPlan::MoveOnly {
                    drop: LinkedValueDropPlan::RecursiveShape { shape },
                })
            }
            concrete => self.link_transfer_plan(concrete, substitutions, location),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::bytecode) fn link_exact_plan_for_type_at(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        substitutions: &BTreeMap<String, TypeRefIr>,
        type_index: TypeIndex,
        declared: &ValueTransferPlan,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
        let concrete_type = self.linked_type_ref(type_index).cloned().ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                format!("linked type row {} is absent", type_index.get()),
            )
        })?;
        let linked = self.link_plan_for_type_at(
            package,
            specialization,
            substitutions,
            declared,
            &concrete_type,
            location.clone(),
        )?;
        let exact = self.linked_type_plan(type_index).ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::FrameAndValueTransferPlan,
                location.clone(),
                format!(
                    "linked type row {} has no compiler-owned plan",
                    type_index.get()
                ),
            )
        })?;
        if &linked != exact {
            return Err(obligation_error(
                BytecodeLinkObligation::FrameAndValueTransferPlan,
                location,
                format!(
                    "declared value plan differs from exact TypeRef row {} plan",
                    type_index.get()
                ),
            ));
        }
        Ok(linked)
    }

    pub(in crate::bytecode) fn validate_privileged_shape_authority(
        &self,
        identity: Option<PrivilegedAffineCompositeIdentity>,
        nominal_type: skiff_runtime_linked_bytecode::TypeIndex,
        fields: &[LinkedShapeField],
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        let nominal = self.linked_type_ref(nominal_type).ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                "linked privileged shape nominal type is absent".to_string(),
            )
        })?;
        let registry = native_value_lifecycle_registry();
        let symbol_schema = match nominal {
            TypeRefIr::PackageSymbol { symbol } => {
                registry.privileged_affine_composite_for_symbol(symbol)
            }
            _ => None,
        };
        let Some(identity) = identity else {
            if symbol_schema.is_some() {
                return Err(obligation_error(
                    BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location,
                    "privileged affine nominal type lacks explicit linked shape authority"
                        .to_string(),
                ));
            }
            return Ok(());
        };
        let schema = registry
            .privileged_affine_composite(identity)
            .ok_or_else(|| {
                obligation_error(
                    BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location.clone(),
                    format!("linked privileged affine identity {identity:?} is not pinned"),
                )
            })?;
        if symbol_schema.map(|row| row.identity) != Some(identity) {
            return Err(obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                "linked privileged affine identity does not match its exact nominal symbol"
                    .to_string(),
            ));
        }
        if fields.len() != schema.fields.len() {
            return Err(obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location,
                "privileged affine linked field count differs from the pinned schema".to_string(),
            ));
        }
        for (ordinal, (field, expected)) in fields.iter().zip(&schema.fields).enumerate() {
            let actual_type = self.linked_type_ref(field.ty()).ok_or_else(|| {
                obligation_error(
                    BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location.clone(),
                    format!("privileged affine field {ordinal} type is absent"),
                )
            })?;
            let exact_plan = self.linked_type_plan(field.ty()).ok_or_else(|| {
                obligation_error(
                    BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location.clone(),
                    format!("privileged affine field {ordinal} type plan is absent"),
                )
            })?;
            if field.name() != expected.name
                || !matches_registry_type(&expected.ty, actual_type)
                || field.plan() != exact_plan
            {
                return Err(obligation_error(
                    BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location.clone(),
                    format!(
                        "privileged affine linked field {ordinal} differs from its exact pinned name/type or compiler plan"
                    ),
                ));
            }
        }
        Ok(())
    }

    pub(in crate::bytecode) fn validate_dense_result_materialization(
        &self,
        result_type: TypeIndex,
        result_plan: &LinkedValueTransferPlan,
        shape: &LinkedShapeEntry,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        self.validate_dense_record_materialization(
            result_type,
            result_plan,
            shape,
            "dense result materialization",
            "its exact resume result",
            location,
        )
    }

    pub(in crate::bytecode) fn validate_dense_parameter_materialization(
        &self,
        parameter_type: TypeIndex,
        parameter_plan: &LinkedValueTransferPlan,
        shape: &LinkedShapeEntry,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        self.validate_dense_record_materialization(
            parameter_type,
            parameter_plan,
            shape,
            "dense parameter materialization",
            "its exact frame parameter",
            location,
        )
    }

    fn validate_dense_record_materialization(
        &self,
        value_type: TypeIndex,
        value_plan: &LinkedValueTransferPlan,
        shape: &LinkedShapeEntry,
        subject: &str,
        nominal_owner: &str,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        let exact_value_plan = self.linked_type_plan(value_type).ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::FrameAndValueTransferPlan,
                location.clone(),
                format!("{subject} value type plan is absent"),
            )
        })?;
        if value_plan != exact_value_plan || shape.plan() != exact_value_plan {
            return Err(obligation_error(
                BytecodeLinkObligation::FrameAndValueTransferPlan,
                location,
                format!("{subject} value and shape plans must equal the exact TypeRef plan"),
            ));
        }
        let value_type_ref = self.linked_type_ref(value_type).ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                format!("{subject} value type {} is absent", value_type.get()),
            )
        })?;
        let nominal_type_ref = self.linked_type_ref(shape.nominal_type()).ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                format!(
                    "{subject} shape {} nominal type is absent",
                    shape.index().get()
                ),
            )
        })?;
        if value_type_ref != nominal_type_ref {
            return Err(obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location,
                format!("{subject} shape nominal TypeRef/ABI differs from {nominal_owner}"),
            ));
        }
        for (ordinal, field) in shape.fields().iter().enumerate() {
            let expected_plan = self.linked_type_plan(field.ty()).ok_or_else(|| {
                obligation_error(
                    BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location.clone(),
                    format!("{subject} field {ordinal} type plan is absent"),
                )
            })?;
            if field.plan() != expected_plan {
                return Err(obligation_error(
                    BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location,
                    format!("{subject} field {ordinal} differs from its exact TypeRef plan"),
                ));
            }
        }
        Ok(())
    }

    fn require_privileged_root_shape(
        &self,
        shape: ShapeIndex,
        concrete_type: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        let row = self.shape(shape).ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                format!("privileged affine shape {} is absent", shape.get()),
            )
        })?;
        let nominal = self.linked_type_ref(row.nominal_type()).ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                "privileged affine shape nominal type is absent".to_string(),
            )
        })?;
        if nominal != concrete_type || row.privileged_affine_composite().is_none() {
            return Err(obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location,
                "recursive root plan does not bind the exact privileged nominal shape".to_string(),
            ));
        }
        Ok(())
    }

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

fn matches_registry_type(expected: &CallableRegistryTypeExpression, actual: &TypeRefIr) -> bool {
    match (expected, actual) {
        (
            CallableRegistryTypeExpression::Builtin { name, arguments },
            TypeRefIr::Builtin {
                name: actual_name,
                args,
            },
        ) => {
            name == actual_name
                && arguments.len() == args.len()
                && arguments
                    .iter()
                    .zip(args)
                    .all(|(expected, actual)| matches_registry_type(expected, actual))
        }
        (
            CallableRegistryTypeExpression::PackageSymbol {
                package_id,
                symbol_path,
            },
            TypeRefIr::PackageSymbol { symbol },
        ) => {
            symbol.symbol_path == *symbol_path
                && matches!(
                    &symbol.package,
                    PackageRefIr::PackageId {
                        package_id: actual_package_id,
                    } if actual_package_id == package_id
                )
        }
        _ => false,
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
