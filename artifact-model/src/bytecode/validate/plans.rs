use crate::bytecode::dto::limits;
use crate::bytecode::dto::{
    BytecodePoolEntry, BytecodePools, ResourceDropPlan, ShapeDeclaration, ValueDropPlan,
    ValueTransferPlan,
};
use crate::bytecode::opcodes::PoolCategory;
use crate::{
    CallableRegistryTypeExpression, NativeResourceDropPlan, NativeValueAdapterRole,
    NativeValueDropPlan, NativeValueLifecycleConcrete, PackageRefIr,
    PrivilegedAffineCompositeIdentity, PrivilegedAffineFieldAccess, TypeRefIr,
};

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
        ValueTransferPlan::SnapshotShare { drop } => {
            if matches!(drop, ValueDropPlan::RecursiveShape { .. }) {
                return Err(header_error(format!(
                    "{location} SnapshotShare may not carry a recursive shape"
                )));
            }
            validate_value_drop_plan(drop, pools, enclosing_shape_index, location)?;
        }
        ValueTransferPlan::MoveOnly { drop } => {
            validate_value_drop_plan(drop, pools, enclosing_shape_index, location)?;
            if let ValueDropPlan::RecursiveShape { shape_ref } = drop {
                require_privileged_shape(*shape_ref, pools, location)?;
            }
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
        validate_lifecycle_adapter_key(
            &clone_adapter.binding_key,
            NativeValueAdapterRole::CloneLease,
            &format!("{location}.cloneAdapter"),
        )?;
    }
    Ok(())
}

pub(super) fn validate_privileged_shape_declaration(
    shape_index: usize,
    shape: &ShapeDeclaration,
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    let Some(identity) = shape.privileged_affine_composite else {
        if shape.fields.iter().any(|field| {
            matches!(
                field.plan,
                ValueTransferPlan::AffineResource { .. }
                    | ValueTransferPlan::ExplicitCloneLease { .. }
            )
        }) {
            return Err(header_error(format!(
                "image.pools.shapes[{shape_index}] ordinary shape may not contain an affine resource field"
            )));
        }
        return Ok(());
    };
    let location = format!("image.pools.shapes[{shape_index}]");
    let schema = crate::native_value_lifecycle_registry()
        .privileged_affine_composite(identity)
        .ok_or_else(|| {
            header_error(format!(
                "{location}.privilegedAffineComposite is absent from the pinned native lifecycle registry"
            ))
        })?;
    let ty = type_for_ref(pools, shape.type_ref, &format!("{location}.typeRef"))?;
    if !matches_schema_symbol(schema, ty) {
        return Err(header_error(format!(
            "{location}.typeRef does not match the privileged composite registry symbol"
        )));
    }
    if shape.fields.len() != schema.fields.len() {
        return Err(header_error(format!(
            "{location}.fields count {} does not match privileged registry count {}",
            shape.fields.len(),
            schema.fields.len()
        )));
    }
    for (ordinal, (field, expected)) in shape.fields.iter().zip(&schema.fields).enumerate() {
        if field.name != expected.name {
            return Err(header_error(format!(
                "{location}.fields[{ordinal}].name {:?} does not match privileged registry field {:?}",
                field.name, expected.name
            )));
        }
        let ty = type_for_ref(
            pools,
            field.type_ref,
            &format!("{location}.fields[{ordinal}].typeRef"),
        )?;
        if !matches_type_expression(&expected.ty, ty) {
            return Err(header_error(format!(
                "{location}.fields[{ordinal}].typeRef does not match the privileged registry field type"
            )));
        }
        if !plan_matches_lifecycle(&field.plan, &expected.lifecycle) {
            return Err(header_error(format!(
                "{location}.fields[{ordinal}].plan does not match the privileged registry lifecycle"
            )));
        }
    }
    Ok(())
}

pub(super) fn privileged_field_access(
    shape: &ShapeDeclaration,
    ordinal: usize,
) -> Option<PrivilegedAffineFieldAccess> {
    let identity = shape.privileged_affine_composite?;
    crate::native_value_lifecycle_registry()
        .privileged_affine_composite(identity)
        .and_then(|schema| schema.fields.get(ordinal))
        .map(|field| field.access)
}

fn require_privileged_shape(
    shape_ref: u32,
    pools: &BytecodePools,
    location: &str,
) -> Result<PrivilegedAffineCompositeIdentity, StructuralValidationError> {
    let Some(BytecodePoolEntry::ShapeRef { shape }) = pools.shapes.get(shape_ref as usize) else {
        return Err(header_error(format!(
            "{location} recursive shape must reference a ShapeRef entry"
        )));
    };
    shape.privileged_affine_composite.ok_or_else(|| {
        header_error(format!(
            "{location} recursive MoveOnly shape lacks privileged affine composite authority"
        ))
    })
}

fn type_for_ref<'a>(
    pools: &'a BytecodePools,
    type_ref: u32,
    location: &str,
) -> Result<&'a TypeRefIr, StructuralValidationError> {
    let Some(BytecodePoolEntry::TypeRef { ty }) = pools.types.get(type_ref as usize) else {
        return Err(header_error(format!(
            "{location} does not reference a TypeRef entry"
        )));
    };
    Ok(ty)
}

fn matches_schema_symbol(schema: &crate::PrivilegedAffineCompositeSchema, ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::PackageSymbol { symbol }
            if matches!(
                &symbol.package,
                PackageRefIr::PackageId { package_id } if package_id == &schema.package_id
            ) && symbol.symbol_path == schema.symbol_path
    )
}

fn matches_type_expression(expected: &CallableRegistryTypeExpression, actual: &TypeRefIr) -> bool {
    match (expected, actual) {
        (
            CallableRegistryTypeExpression::Builtin { name, arguments },
            TypeRefIr::Builtin {
                name: actual_name,
                args: actual_arguments,
            },
        ) => {
            name == actual_name
                && arguments.len() == actual_arguments.len()
                && arguments
                    .iter()
                    .zip(actual_arguments)
                    .all(|(expected, actual)| matches_type_expression(expected, actual))
        }
        (
            CallableRegistryTypeExpression::PackageSymbol {
                package_id,
                symbol_path,
            },
            TypeRefIr::PackageSymbol { symbol },
        ) => {
            matches!(
                &symbol.package,
                PackageRefIr::PackageId {
                    package_id: actual_package_id
                } if actual_package_id == package_id
            ) && symbol.symbol_path == *symbol_path
        }
        _ => false,
    }
}

fn plan_matches_lifecycle(
    plan: &ValueTransferPlan,
    expected: &NativeValueLifecycleConcrete,
) -> bool {
    match (plan, expected) {
        (
            ValueTransferPlan::SnapshotShare { drop },
            NativeValueLifecycleConcrete::SnapshotShare {
                drop: expected_drop,
            },
        )
        | (
            ValueTransferPlan::MoveOnly { drop },
            NativeValueLifecycleConcrete::MoveOnly {
                drop: expected_drop,
            },
        ) => matches!(
            (drop, expected_drop),
            (ValueDropPlan::Trivial, NativeValueDropPlan::Trivial)
                | (
                    ValueDropPlan::SnapshotRelease,
                    NativeValueDropPlan::SnapshotRelease
                )
                | (
                    ValueDropPlan::RecursiveShape { .. },
                    NativeValueDropPlan::PrivilegedRecursiveShape
                )
        ),
        (
            ValueTransferPlan::AffineResource { drop },
            NativeValueLifecycleConcrete::AffineResource {
                drop: expected_drop,
            },
        ) => matches!(
            (drop, expected_drop),
            (
                ResourceDropPlan::ResourceTableRelease,
                NativeResourceDropPlan::ResourceTableRelease
            )
        ),
        _ => false,
    }
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
        ValueDropPlan::NativeAdapter { adapter } => validate_lifecycle_adapter_key(
            &adapter.binding_key,
            NativeValueAdapterRole::ValueDrop,
            &format!("{location}.drop.adapter"),
        ),
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
        ResourceDropPlan::NativeAdapter { adapter } => validate_lifecycle_adapter_key(
            &adapter.binding_key,
            NativeValueAdapterRole::ResourceDrop,
            &format!("{location}.drop.adapter"),
        ),
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

fn validate_lifecycle_adapter_key(
    binding_key: &str,
    expected_role: NativeValueAdapterRole,
    location: &str,
) -> Result<(), StructuralValidationError> {
    validate_adapter_key(binding_key, location)?;
    let Some(adapter) = crate::native_value_lifecycle_registry().adapter(binding_key) else {
        return Err(header_error(format!(
            "{location}.bindingKey {binding_key:?} is absent from the pinned native lifecycle registry"
        )));
    };
    if adapter.role != expected_role {
        return Err(header_error(format!(
            "{location}.bindingKey {binding_key:?} has role {:?}, expected {expected_role:?}",
            adapter.role
        )));
    }
    Ok(())
}
