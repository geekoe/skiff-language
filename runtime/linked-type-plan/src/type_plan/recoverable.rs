use super::linked::from_linked_descriptor;
use super::*;
use skiff_runtime_model::type_plan::RuntimeBuiltinShape;

impl RuntimeRecoverableExpectedTypePlanLinkedExt for RuntimeRecoverableExpectedTypePlan {
    fn from_linked(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self> {
        recoverable_expected_from_linked(type_ref, ctx)
    }

    fn from_linked_ref(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self> {
        recoverable_expected_from_linked_ref(type_ref, ctx)
    }
}

fn unresolved_recoverable_expected_from_type_ref(
    type_ref: &LinkedTypeRef,
) -> RuntimeRecoverableExpectedTypePlan {
    let runtime_plan = unknown_plan_for_type_ref(type_ref);
    RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
        &runtime_plan,
    )
}

fn unresolved_recoverable_expected_from_descriptor(
    descriptor: &LinkedTypeDescriptor,
) -> RuntimeRecoverableExpectedTypePlan {
    let runtime_plan = unknown_plan_for_descriptor(descriptor);
    RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
        &runtime_plan,
    )
}

pub fn linked_interface_instantiation_runtime_id(
    interface: &skiff_runtime_linked_program::LinkedInterfaceInstantiationRef,
) -> String {
    if interface.canonical_type_args.is_empty() {
        return interface.interface_abi_id.clone();
    }
    sorted_json_string(serde_json::to_value(interface).unwrap_or_else(|_| serde_json::Value::Null))
}

pub fn linked_type_ref_runtime_key(type_ref: &LinkedTypeRef) -> String {
    sorted_json_string(skiff_runtime_linked_program::type_ref_to_value(type_ref))
}

/// Stable recoverable interface projection identity for an expected `any I`.
///
/// Non-generic interfaces intentionally keep the compiler recoverable metadata
/// shape (`interface:{interfaceAbiId}`). Generic instantiations include the
/// canonical instantiation JSON so different `I<T>` projections cannot collide.
pub fn recoverable_interface_projection_identity(
    interface: &skiff_runtime_linked_program::LinkedInterfaceInstantiationRef,
) -> String {
    if interface.canonical_type_args.is_empty() {
        return format!("interface:{}", interface.interface_abi_id);
    }
    format!(
        "interface:{}",
        sorted_json_string(
            serde_json::to_value(interface).unwrap_or_else(|_| serde_json::Value::Null),
        )
    )
}

fn recoverable_expected_from_linked(
    type_ref: &LinkedTypeRef,
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypePlan> {
    let runtime_plan = RuntimeTypePlan::from_linked(type_ref, ctx)?;
    let mut expected =
        RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
            &runtime_plan,
        );
    expected.node = recoverable_expected_node_from_linked(type_ref, ctx)?;
    if let LinkedTypeRef::AnyInterface { interface } = type_ref {
        let interface_identity = linked_interface_instantiation_runtime_id(interface);
        expected.identity = Some(RuntimeRecoverableTypeIdentityRef::Interface(
            RuntimeRecoverableInterfaceTypeRef {
                interface_identity: interface_identity.clone(),
            },
        ));
        expected.label = format!("any {interface_identity}");
    }
    Ok(expected)
}

fn recoverable_expected_from_linked_ref(
    type_ref: &LinkedTypeRef,
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypePlan> {
    match type_ref {
        LinkedTypeRef::TypeParam { name } => {
            if let Some(bound) = ctx.substitution(name) {
                let closed = close_linked_type_ref(bound, ctx.substitutions)?;
                return recoverable_expected_from_linked_ref(&closed, &ctx.without_substitutions());
            }
        }
        LinkedTypeRef::Address { addr } => {
            return recoverable_expected_resolve_addr_or_bridge(type_ref, addr.clone(), ctx);
        }
        LinkedTypeRef::LocalType { type_index } => {
            let addr = TypeAddr {
                unit: ctx.current_addr.unit.clone(),
                file: ctx.current_addr.file.clone(),
                type_index: *type_index,
            };
            return recoverable_expected_resolve_addr_or_bridge(type_ref, addr, ctx);
        }
        LinkedTypeRef::ServiceSymbol { symbol } => {
            if let Some(addr) =
                program_service_symbol_type_addr(ctx.program, &ctx.current_addr.unit, symbol)?
            {
                return recoverable_expected_resolve_addr_or_bridge(type_ref, addr, ctx);
            }
        }
        LinkedTypeRef::PackageSymbol { symbol } => {
            if let Some(addr) = program_package_type_addr(ctx.program, symbol) {
                return recoverable_expected_resolve_addr_or_bridge(type_ref, addr, ctx);
            }
        }
        _ => {}
    }
    recoverable_expected_from_linked(type_ref, ctx)
}

fn recoverable_expected_resolve_addr_or_bridge(
    type_ref: &LinkedTypeRef,
    addr: TypeAddr,
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypePlan> {
    match ctx.program.types.declaration(&addr) {
        Some(declaration) => {
            recoverable_expected_from_linked_declaration(declaration, &ctx.deeper_by(1))
        }
        None => Ok(unresolved_recoverable_expected_from_type_ref(type_ref)),
    }
}

fn recoverable_expected_from_linked_declaration(
    declaration: &TypeDeclIr,
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypePlan> {
    if !declaration.type_params.is_empty() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "generic nominal {} requires an applied nominal wrapper with {} arguments",
            declaration.name,
            declaration.type_params.len()
        )));
    }
    let mut expected = recoverable_expected_from_linked_descriptor(&declaration.descriptor, ctx)?;
    expected.label = declaration.name.clone();
    if let RuntimeRecoverableExpectedTypeNode::Record {
        boundary_record_kind,
        ..
    } = &mut expected.node
    {
        *boundary_record_kind = Some(declaration.name.clone());
    }
    if let RuntimeRecoverableExpectedTypeNode::Union { items } = &mut expected.node {
        for (index, branch) in items.iter_mut().enumerate() {
            branch.label = format!("{}::branch[{index}]::{}", declaration.name, branch.label);
        }
    }
    Ok(expected)
}

fn recoverable_expected_from_linked_descriptor(
    descriptor: &LinkedTypeDescriptor,
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypePlan> {
    if ctx.over_depth_cap() {
        return Ok(unresolved_recoverable_expected_from_descriptor(descriptor));
    }

    let node = match descriptor {
        LinkedTypeDescriptor::Record { fields } => RuntimeRecoverableExpectedTypeNode::Record {
            fields: fields
                .iter()
                .map(|(name, field_ty)| {
                    Ok(RuntimeRecoverableExpectedRecordFieldPlan {
                        name: name.clone(),
                        ty: recoverable_expected_from_linked_ref(field_ty, &ctx.deeper_by(2))?,
                        required: !matches!(field_ty, LinkedTypeRef::Nullable { .. }),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            boundary_record_kind: None,
        },
        LinkedTypeDescriptor::Representation { .. } => {
            let runtime_plan = from_linked_descriptor(descriptor, ctx)?;
            return Ok(
                RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                    &runtime_plan,
                ),
            );
        }
        LinkedTypeDescriptor::Alias { target } => RuntimeRecoverableExpectedTypeNode::Alias {
            target: Box::new(recoverable_expected_from_linked_ref(
                target,
                &ctx.deeper_by(1),
            )?),
        },
        LinkedTypeDescriptor::Union { branches } => RuntimeRecoverableExpectedTypeNode::Union {
            items: branches
                .iter()
                .map(|branch| {
                    recoverable_expected_from_linked_union_branch(branch, &ctx.deeper_by(2))
                })
                .collect::<Result<Vec<_>>>()?,
        },
        LinkedTypeDescriptor::Interface => {
            return Err(RuntimeError::InvalidArtifact(
                "interface declaration cannot be materialized as a recoverable value type plan"
                    .to_string(),
            ));
        }
    };
    Ok(RuntimeRecoverableExpectedTypePlan {
        label: linked_type_descriptor_label(descriptor).to_string(),
        identity: None,
        node,
    })
}

fn recoverable_expected_from_linked_union_branch(
    branch: &LinkedNamedUnionBranch,
    ctx: &PlanContext<'_>,
) -> Result<RuntimeRecoverableExpectedTypePlan> {
    let (mut plan, branch_context) = match branch {
        LinkedNamedUnionBranch::ConcreteNominal { nominal_type } => (
            recoverable_expected_from_linked_ref(nominal_type, ctx)?,
            "concreteNominal".to_string(),
        ),
        LinkedNamedUnionBranch::SyntheticDiscriminator {
            payload_type,
            discriminator_field,
            discriminator_value,
        } => (
            recoverable_expected_from_linked_ref(payload_type, ctx)?,
            format!("syntheticDiscriminator:{discriminator_field}={discriminator_value}"),
        ),
        LinkedNamedUnionBranch::Literal { value } => (
            recoverable_expected_from_linked_ref(
                &LinkedTypeRef::Literal {
                    value: value.clone(),
                },
                ctx,
            )?,
            "literal".to_string(),
        ),
    };
    plan.label = format!("{branch_context}:{}", plan.label);
    Ok(plan)
}

fn recoverable_expected_node_from_linked(
    type_ref: &LinkedTypeRef,
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypeNode> {
    let node = match type_ref {
        LinkedTypeRef::Native { name, args } => recoverable_expected_builtin_node(name, args, ctx)?,
        LinkedTypeRef::Record { fields } => RuntimeRecoverableExpectedTypeNode::Record {
            fields: fields
                .iter()
                .map(|(name, field_ty)| {
                    Ok(RuntimeRecoverableExpectedRecordFieldPlan {
                        name: name.clone(),
                        ty: recoverable_expected_from_linked_ref(field_ty, &ctx.deeper_by(2))?,
                        required: !matches!(field_ty, LinkedTypeRef::Nullable { .. }),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            boundary_record_kind: None,
        },
        LinkedTypeRef::Union { items } => RuntimeRecoverableExpectedTypeNode::Union {
            items: items
                .iter()
                .map(|item| recoverable_expected_from_linked_ref(item, &ctx.deeper_by(2)))
                .collect::<Result<Vec<_>>>()?,
        },
        LinkedTypeRef::Nullable { inner } => RuntimeRecoverableExpectedTypeNode::Nullable {
            inner: Box::new(recoverable_expected_from_linked_ref(
                inner,
                &ctx.deeper_by(1),
            )?),
        },
        LinkedTypeRef::AnyInterface { interface } => {
            RuntimeRecoverableExpectedTypeNode::AnyInterface {
                expected: RuntimeRecoverableExpectedAnyInterfacePlan::new(
                    linked_interface_instantiation_runtime_id(interface),
                    recoverable_interface_projection_identity(interface),
                ),
            }
        }
        LinkedTypeRef::Literal { value } => match value {
            LiteralIr::String { value } => RuntimeRecoverableExpectedTypeNode::LiteralString {
                value: value.clone(),
            },
            _ => RuntimeRecoverableExpectedTypeNode::Unresolved {
                diagnostic_label: "literal".to_string(),
            },
        },
        LinkedTypeRef::TypeParam { name } => {
            if let Some(bound) = ctx.substitution(name) {
                let closed = close_linked_type_ref(bound, ctx.substitutions)?;
                return Ok(recoverable_expected_from_linked_ref(
                    &closed,
                    &ctx.without_substitutions(),
                )?
                .node);
            }
            return Err(RuntimeError::InvalidArtifact(format!(
                "recoverable linked type plan contains unbound type parameter {name}"
            )));
        }
        LinkedTypeRef::Function { .. }
        | LinkedTypeRef::DbObjectSymbol { .. }
        | LinkedTypeRef::AppliedNominal { .. }
        | LinkedTypeRef::Address { .. }
        | LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. }
        | LinkedTypeRef::PackageSchema { .. } => {
            let runtime_plan = RuntimeTypePlan::from_linked(type_ref, ctx)?;
            return Ok(
                RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                    &runtime_plan,
                )
                .node,
            );
        }
    };
    Ok(node)
}

fn recoverable_expected_builtin_node(
    name: &str,
    args: &[LinkedTypeRef],
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypeNode> {
    if name == "Array" && args.len() == 1 {
        return Ok(RuntimeRecoverableExpectedTypeNode::Array {
            item: Box::new(recoverable_expected_from_linked_ref(
                &args[0],
                &ctx.deeper_by(2),
            )?),
        });
    }
    if name == "Map" && args.len() == 2 {
        return Ok(RuntimeRecoverableExpectedTypeNode::Map {
            key: Box::new(recoverable_expected_from_linked_ref(
                &args[0],
                &ctx.deeper_by(2),
            )?),
            value: Box::new(recoverable_expected_from_linked_ref(
                &args[1],
                &ctx.deeper_by(2),
            )?),
        });
    }
    if bare_type_name(name) == "Stream" && args.len() == 1 {
        return Ok(RuntimeRecoverableExpectedTypeNode::Stream {
            item: Box::new(recoverable_expected_from_linked_ref(
                &args[0],
                &ctx.deeper_by(2),
            )?),
        });
    }

    let Some(node) = RuntimeBuiltinShape::of_name(name).and_then(RuntimeBuiltinShape::leaf_node)
    else {
        return Ok(RuntimeRecoverableExpectedTypeNode::Unresolved {
            diagnostic_label: name.to_string(),
        });
    };
    Ok(match node {
        RuntimeTypeNode::Json => RuntimeRecoverableExpectedTypeNode::Json,
        RuntimeTypeNode::JsonObject => RuntimeRecoverableExpectedTypeNode::JsonObject,
        RuntimeTypeNode::Bytes => RuntimeRecoverableExpectedTypeNode::Bytes,
        RuntimeTypeNode::Date => RuntimeRecoverableExpectedTypeNode::Date,
        RuntimeTypeNode::String => RuntimeRecoverableExpectedTypeNode::String,
        RuntimeTypeNode::TaskRef => RuntimeRecoverableExpectedTypeNode::TaskRef,
        RuntimeTypeNode::Bool => RuntimeRecoverableExpectedTypeNode::Bool,
        RuntimeTypeNode::Integer => RuntimeRecoverableExpectedTypeNode::Integer,
        RuntimeTypeNode::Number => RuntimeRecoverableExpectedTypeNode::Number,
        RuntimeTypeNode::Null => RuntimeRecoverableExpectedTypeNode::Null,
        RuntimeTypeNode::Union(items) => RuntimeRecoverableExpectedTypeNode::Union {
            items: items
                .iter()
                .map(
                    RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics,
                )
                .collect(),
        },
        _ => RuntimeRecoverableExpectedTypeNode::Unresolved {
            diagnostic_label: name.to_string(),
        },
    })
}

pub(crate) fn sorted_json_string(value: serde_json::Value) -> String {
    let sorted = sort_json_value(value);
    serde_json::to_string(&sorted).unwrap_or_else(|_| "null".to_string())
}

fn sort_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(sort_json_value).collect())
        }
        serde_json::Value::Object(object) => {
            let mut sorted = std::collections::BTreeMap::new();
            for (key, value) in object {
                sorted.insert(key, sort_json_value(value));
            }
            let mut object = serde_json::Map::new();
            for (key, value) in sorted {
                object.insert(key, value);
            }
            serde_json::Value::Object(object)
        }
        other => other,
    }
}
