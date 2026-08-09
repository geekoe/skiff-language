use crate::bytecode::dto::limits;
use crate::bytecode::dto::{BytecodePools, ResourceDropPlan, ValueDropPlan, ValueTransferPlan};
use crate::bytecode::opcodes::PoolCategory;

use super::{
    entry_is_kind, header_error, index_out_of_bounds, limit_error, type_ref_nesting_depth,
    StructuralValidationError,
};

pub(super) fn validate_transfer_plan(
    plan: &ValueTransferPlan,
    pools: &BytecodePools,
    enclosing_shape_index: Option<usize>,
    location: &str,
) -> Result<(), StructuralValidationError> {
    match plan {
        ValueTransferPlan::SnapshotShare { drop } | ValueTransferPlan::MoveOnly { drop } => {
            validate_value_drop_plan(drop, pools, enclosing_shape_index, location)?;
        }
        ValueTransferPlan::AffineResource { drop }
        | ValueTransferPlan::ExplicitCloneLease { drop, .. } => {
            validate_resource_drop_plan(drop, pools, enclosing_shape_index, location)?;
        }
        ValueTransferPlan::FromType { ty } => {
            let depth = type_ref_nesting_depth(ty);
            if depth as u64 > limits::MAX_NESTING_DEPTH {
                return Err(limit_error(
                    "MAX_NESTING_DEPTH",
                    limits::MAX_NESTING_DEPTH,
                    depth as u64,
                    location,
                ));
            }
        }
    }
    if let ValueTransferPlan::ExplicitCloneLease { clone_adapter, .. } = plan {
        validate_adapter_key(
            &clone_adapter.binding_key,
            &format!("{location}.cloneAdapter"),
        )?;
    }
    Ok(())
}

fn validate_value_drop_plan(
    drop: &ValueDropPlan,
    pools: &BytecodePools,
    enclosing_shape_index: Option<usize>,
    location: &str,
) -> Result<(), StructuralValidationError> {
    match drop {
        ValueDropPlan::Trivial | ValueDropPlan::SnapshotRelease => Ok(()),
        ValueDropPlan::RecursiveShape { shape_ref } => {
            validate_recursive_shape_ref(*shape_ref, pools, enclosing_shape_index, location)
        }
        ValueDropPlan::NativeAdapter { adapter } => {
            validate_adapter_key(&adapter.binding_key, &format!("{location}.drop.adapter"))
        }
    }
}

fn validate_resource_drop_plan(
    drop: &ResourceDropPlan,
    pools: &BytecodePools,
    enclosing_shape_index: Option<usize>,
    location: &str,
) -> Result<(), StructuralValidationError> {
    match drop {
        ResourceDropPlan::ResourceTableRelease => Ok(()),
        ResourceDropPlan::RecursiveShape { shape_ref } => {
            validate_recursive_shape_ref(*shape_ref, pools, enclosing_shape_index, location)
        }
        ResourceDropPlan::NativeAdapter { adapter } => {
            validate_adapter_key(&adapter.binding_key, &format!("{location}.drop.adapter"))
        }
    }
}

fn validate_recursive_shape_ref(
    shape_ref: u32,
    pools: &BytecodePools,
    enclosing_shape_index: Option<usize>,
    location: &str,
) -> Result<(), StructuralValidationError> {
    let Some(entry) = pools.shapes.get(shape_ref as usize) else {
        return Err(index_out_of_bounds("shapes pool", shape_ref, location));
    };
    if !entry_is_kind(entry, PoolCategory::Shapes) {
        return Err(header_error(format!(
            "{location} recursive shape must reference a ShapeRef entry"
        )));
    }
    if enclosing_shape_index.is_some_and(|parent| shape_ref as usize >= parent) {
        return Err(header_error(format!(
            "{location} recursive shape {shape_ref} must precede its enclosing shape (acyclic plan graph)"
        )));
    }
    Ok(())
}

pub(super) fn validate_adapter_key(
    binding_key: &str,
    location: &str,
) -> Result<(), StructuralValidationError> {
    if binding_key.is_empty() {
        return Err(header_error(format!(
            "{location}.bindingKey must not be empty"
        )));
    }
    Ok(())
}
