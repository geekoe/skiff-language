use crate::{
    bytecode::{ResourceDropPlan, ValueDropPlan, ValueTransferPlan},
    NativeResourceDropPlan, NativeValueAdapterRole, NativeValueDropPlan,
    NativeValueLifecycleConcrete, NativeValueLifecycleResolution, TypeRefIr,
};

use super::{
    classify::classify_type,
    contract::{
        PositionalTypeEnvironment, ValueLifecycleFactResolver, ValueLifecyclePolicyBudget,
        ValueLifecyclePolicyError,
    },
    normalize::normalize_type,
    traversal::ClassificationContext,
};

pub fn verify_value_transfer_plan<R: ValueLifecycleFactResolver>(
    ty: &TypeRefIr,
    declared: &ValueTransferPlan,
    environment: &PositionalTypeEnvironment,
    resolver: &mut R,
    budget: &mut ValueLifecyclePolicyBudget,
) -> Result<NativeValueLifecycleResolution, ValueLifecyclePolicyError> {
    let normalized_type = normalize_type(ty, environment, budget, 1)?;
    let mut context = ClassificationContext::new(resolver, budget);
    let expected = classify_type(
        &normalized_type,
        &PositionalTypeEnvironment::empty(),
        &mut context,
        1,
    )?;
    drop(context);
    if let ValueTransferPlan::FromType { ty: plan_type } = declared {
        let normalized_plan = normalize_type(plan_type, environment, budget, 1)?;
        if normalized_plan != normalized_type {
            return Err(ValueLifecyclePolicyError::PlanMismatch {
                message: "FromType does not name the value position's exact normalized type"
                    .to_string(),
            });
        }
        return Ok(expected);
    }
    let actual = concrete_plan(declared)?;
    if actual != expected.lifecycle {
        return Err(ValueLifecyclePolicyError::PlanMismatch {
            message: format!("declared {actual:?}, recomputed {:?}", expected.lifecycle),
        });
    }
    Ok(expected)
}

fn concrete_plan(
    plan: &ValueTransferPlan,
) -> Result<NativeValueLifecycleConcrete, ValueLifecyclePolicyError> {
    Ok(match plan {
        ValueTransferPlan::SnapshotShare { drop } => NativeValueLifecycleConcrete::SnapshotShare {
            drop: value_drop(drop)?,
        },
        ValueTransferPlan::MoveOnly { drop } => NativeValueLifecycleConcrete::MoveOnly {
            drop: value_drop(drop)?,
        },
        ValueTransferPlan::AffineResource { drop } => {
            NativeValueLifecycleConcrete::AffineResource {
                drop: resource_drop(drop)?,
            }
        }
        ValueTransferPlan::ExplicitCloneLease {
            clone_adapter,
            drop,
        } => NativeValueLifecycleConcrete::ExplicitCloneLease {
            clone_adapter: adapter(
                &clone_adapter.binding_key,
                NativeValueAdapterRole::CloneLease,
            )?,
            drop: resource_drop(drop)?,
        },
        ValueTransferPlan::FromType { .. } => unreachable!("FromType handled before conversion"),
    })
}

fn value_drop(drop: &ValueDropPlan) -> Result<NativeValueDropPlan, ValueLifecyclePolicyError> {
    Ok(match drop {
        ValueDropPlan::Trivial => NativeValueDropPlan::Trivial,
        ValueDropPlan::SnapshotRelease => NativeValueDropPlan::SnapshotRelease,
        ValueDropPlan::RecursiveShape { .. } => NativeValueDropPlan::PrivilegedRecursiveShape,
        ValueDropPlan::NativeAdapter { adapter: reference } => NativeValueDropPlan::NativeAdapter {
            adapter: adapter(&reference.binding_key, NativeValueAdapterRole::ValueDrop)?,
        },
    })
}

fn resource_drop(
    drop: &ResourceDropPlan,
) -> Result<NativeResourceDropPlan, ValueLifecyclePolicyError> {
    Ok(match drop {
        ResourceDropPlan::ResourceTableRelease => NativeResourceDropPlan::ResourceTableRelease,
        ResourceDropPlan::RecursiveShape { .. } => {
            return Err(ValueLifecyclePolicyError::RecursiveShapePlan);
        }
        ResourceDropPlan::NativeAdapter { adapter: reference } => {
            NativeResourceDropPlan::NativeAdapter {
                adapter: adapter(&reference.binding_key, NativeValueAdapterRole::ResourceDrop)?,
            }
        }
    })
}

fn adapter(
    binding_key: &str,
    expected_role: NativeValueAdapterRole,
) -> Result<crate::NativeValueLifecycleAdapter, ValueLifecyclePolicyError> {
    let adapter = crate::native_value_lifecycle_registry()
        .adapter(binding_key)
        .ok_or_else(|| ValueLifecyclePolicyError::UnknownAdapter {
            binding_key: binding_key.to_string(),
        })?;
    if adapter.role != expected_role {
        return Err(ValueLifecyclePolicyError::AdapterRoleMismatch {
            binding_key: binding_key.to_string(),
        });
    }
    Ok(adapter.clone())
}
