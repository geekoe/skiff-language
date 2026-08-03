use super::*;

pub(crate) fn applied_nominal_plan(
    base: &LinkedNominalTypeRefBase,
    arguments: &[LinkedTypeRef],
    ctx: &PlanContext<'_>,
) -> Result<RuntimeTypePlan> {
    if arguments.is_empty() {
        return Err(RuntimeError::InvalidArtifact(
            "applied nominal type ref arguments must be non-empty".to_string(),
        ));
    }
    let addr = match base {
        LinkedNominalTypeRefBase::Address { addr } => addr,
        LinkedNominalTypeRefBase::PackageSchema { .. } => {
            return Err(RuntimeError::InvalidArtifact(
                "applied PackageSchema is not admitted in executable linked type plans".to_string(),
            ));
        }
        unresolved => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "applied nominal base {} was not linked to an exact address",
                linked_nominal_base_kind(unresolved)
            )));
        }
    };
    let declaration = ctx.program.types.declaration(addr).ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "applied nominal type address {addr} is not interned"
        ))
    })?;
    if !matches!(
        declaration.descriptor,
        LinkedTypeDescriptor::Record { .. }
            | LinkedTypeDescriptor::Representation { .. }
            | LinkedTypeDescriptor::Union { .. }
    ) {
        return Err(RuntimeError::InvalidArtifact(format!(
            "applied nominal {} targets {} instead of record, representation, or named union",
            declaration.name,
            linked_type_descriptor_label(&declaration.descriptor)
        )));
    }
    let expected = declaration.type_params.len();
    if expected == 0 || arguments.len() != expected {
        return Err(RuntimeError::InvalidArtifact(format!(
            "applied nominal {} has arity {}, expected {}",
            declaration.name,
            arguments.len(),
            expected
        )));
    }

    let closed_arguments = arguments
        .iter()
        .map(|argument| close_linked_type_ref(argument, ctx.substitutions))
        .collect::<Result<Vec<_>>>()?;
    let substitutions = declaration
        .type_params
        .iter()
        .cloned()
        .zip(closed_arguments.iter().cloned())
        .collect::<BTreeMap<_, _>>();
    let descriptor = instantiate_linked_descriptor(&declaration.descriptor, &substitutions)?;
    let applied = LinkedTypeRef::AppliedNominal {
        base: LinkedNominalTypeRefBase::Address { addr: addr.clone() },
        arguments: closed_arguments,
    };
    let owner_context = format!(
        "{}<{}>",
        declaration.name,
        linked_type_ref_runtime_key(&applied)
    );
    let mut plan =
        RuntimeTypePlan::from_linked_descriptor(&descriptor, &ctx.without_substitutions())?;
    apply_nominal_owner_context(&mut plan, &owner_context);
    Ok(plan)
}

fn linked_nominal_base_kind(base: &LinkedNominalTypeRefBase) -> &'static str {
    match base {
        LinkedNominalTypeRefBase::LocalType { .. } => "localType",
        LinkedNominalTypeRefBase::PublicationType { .. } => "publicationType",
        LinkedNominalTypeRefBase::ServiceSymbol { .. } => "serviceSymbol",
        LinkedNominalTypeRefBase::PackageSymbol { .. } => "packageSymbol",
        LinkedNominalTypeRefBase::PackageSchema { .. } => "packageSchema",
        LinkedNominalTypeRefBase::Address { .. } => "address",
    }
}

pub(crate) fn close_linked_type_ref(
    type_ref: &LinkedTypeRef,
    substitutions: Option<&BTreeMap<String, LinkedTypeRef>>,
) -> Result<LinkedTypeRef> {
    close_linked_type_ref_inner(type_ref, substitutions, &mut Vec::new())
}

fn close_linked_type_ref_inner(
    type_ref: &LinkedTypeRef,
    substitutions: Option<&BTreeMap<String, LinkedTypeRef>>,
    resolving: &mut Vec<String>,
) -> Result<LinkedTypeRef> {
    Ok(match type_ref {
        LinkedTypeRef::TypeParam { name } => {
            let bound = substitutions
                .and_then(|bindings| bindings.get(name))
                .ok_or_else(|| {
                    RuntimeError::InvalidArtifact(format!(
                        "linked type plan contains unbound type parameter {name}"
                    ))
                })?;
            if resolving.iter().any(|active| active == name) {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "linked type substitution cycle contains type parameter {name}"
                )));
            }
            resolving.push(name.clone());
            let closed = close_linked_type_ref_inner(bound, substitutions, resolving)?;
            resolving.pop();
            closed
        }
        LinkedTypeRef::Native { name, args } => LinkedTypeRef::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|argument| close_linked_type_ref_inner(argument, substitutions, resolving))
                .collect::<Result<Vec<_>>>()?,
        },
        LinkedTypeRef::AppliedNominal { base, arguments } => {
            if arguments.is_empty() {
                return Err(RuntimeError::InvalidArtifact(
                    "applied nominal type ref arguments must be non-empty".to_string(),
                ));
            }
            LinkedTypeRef::AppliedNominal {
                base: base.clone(),
                arguments: arguments
                    .iter()
                    .map(|argument| close_linked_type_ref_inner(argument, substitutions, resolving))
                    .collect::<Result<Vec<_>>>()?,
            }
        }
        LinkedTypeRef::Record { fields } => LinkedTypeRef::Record {
            fields: fields
                .iter()
                .map(|(name, field)| {
                    Ok((
                        name.clone(),
                        close_linked_type_ref_inner(field, substitutions, resolving)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
        },
        LinkedTypeRef::Union { items } => LinkedTypeRef::Union {
            items: items
                .iter()
                .map(|item| close_linked_type_ref_inner(item, substitutions, resolving))
                .collect::<Result<Vec<_>>>()?,
        },
        LinkedTypeRef::Nullable { inner } => LinkedTypeRef::Nullable {
            inner: Box::new(close_linked_type_ref_inner(
                inner,
                substitutions,
                resolving,
            )?),
        },
        LinkedTypeRef::AnyInterface { interface } => LinkedTypeRef::AnyInterface {
            interface: skiff_runtime_linked_program::LinkedInterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|argument| close_linked_type_ref_inner(argument, substitutions, resolving))
                    .collect::<Result<Vec<_>>>()?,
            },
        },
        LinkedTypeRef::Function {
            params,
            return_type,
        } => LinkedTypeRef::Function {
            params: params
                .iter()
                .map(|parameter| {
                    Ok(skiff_runtime_linked_program::FunctionTypeParamIr {
                        name: parameter.name.clone(),
                        ty: close_linked_type_ref_inner(&parameter.ty, substitutions, resolving)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            return_type: Box::new(close_linked_type_ref_inner(
                return_type,
                substitutions,
                resolving,
            )?),
        },
        LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. }
        | LinkedTypeRef::PackageSchema { .. }
        | LinkedTypeRef::Address { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::DbObjectSymbol { .. } => type_ref.clone(),
    })
}

fn instantiate_linked_descriptor(
    descriptor: &LinkedTypeDescriptor,
    substitutions: &BTreeMap<String, LinkedTypeRef>,
) -> Result<LinkedTypeDescriptor> {
    Ok(match descriptor {
        LinkedTypeDescriptor::Record { fields } => LinkedTypeDescriptor::Record {
            fields: fields
                .iter()
                .map(|(name, field)| {
                    Ok((
                        name.clone(),
                        close_linked_type_ref(field, Some(substitutions))?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
        },
        LinkedTypeDescriptor::Representation { representation } => {
            LinkedTypeDescriptor::Representation {
                representation: close_linked_type_ref(representation, Some(substitutions))?,
            }
        }
        LinkedTypeDescriptor::Union { branches } => LinkedTypeDescriptor::Union {
            branches: branches
                .iter()
                .map(|branch| instantiate_linked_union_branch(branch, substitutions))
                .collect::<Result<Vec<_>>>()?,
        },
        LinkedTypeDescriptor::Alias { target } => LinkedTypeDescriptor::Alias {
            target: close_linked_type_ref(target, Some(substitutions))?,
        },
        LinkedTypeDescriptor::Interface => LinkedTypeDescriptor::Interface,
    })
}

fn instantiate_linked_union_branch(
    branch: &LinkedNamedUnionBranch,
    substitutions: &BTreeMap<String, LinkedTypeRef>,
) -> Result<LinkedNamedUnionBranch> {
    Ok(match branch {
        LinkedNamedUnionBranch::ConcreteNominal { nominal_type } => {
            LinkedNamedUnionBranch::ConcreteNominal {
                nominal_type: close_linked_type_ref(nominal_type, Some(substitutions))?,
            }
        }
        LinkedNamedUnionBranch::SyntheticDiscriminator {
            payload_type,
            discriminator_field,
            discriminator_value,
        } => LinkedNamedUnionBranch::SyntheticDiscriminator {
            payload_type: close_linked_type_ref(payload_type, Some(substitutions))?,
            discriminator_field: discriminator_field.clone(),
            discriminator_value: discriminator_value.clone(),
        },
        LinkedNamedUnionBranch::Literal { value } => LinkedNamedUnionBranch::Literal {
            value: value.clone(),
        },
    })
}

pub(crate) fn linked_named_union_branch_plan(
    branch: &LinkedNamedUnionBranch,
    ctx: &PlanContext<'_>,
) -> Result<RuntimeTypePlan> {
    let (mut plan, branch_context) = match branch {
        LinkedNamedUnionBranch::ConcreteNominal { nominal_type } => (
            RuntimeTypePlan::from_linked_ref(nominal_type, ctx)?,
            "concreteNominal".to_string(),
        ),
        LinkedNamedUnionBranch::SyntheticDiscriminator {
            payload_type,
            discriminator_field,
            discriminator_value,
        } => (
            RuntimeTypePlan::from_linked_ref(payload_type, ctx)?,
            format!("syntheticDiscriminator:{discriminator_field}={discriminator_value}"),
        ),
        LinkedNamedUnionBranch::Literal { value } => (
            RuntimeTypePlan::from_linked_ref(
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

pub(crate) fn apply_nominal_owner_context(plan: &mut RuntimeTypePlan, owner_context: &str) {
    plan.label = owner_context.to_string();
    plan.named_type_name = Some(owner_context.to_string());
    match &mut plan.node {
        RuntimeTypeNode::Record {
            boundary_record_kind,
            ..
        } => {
            *boundary_record_kind = Some(owner_context.to_string());
        }
        RuntimeTypeNode::Representation { type_name, .. } => {
            *type_name = owner_context.to_string();
        }
        RuntimeTypeNode::Union(branches) => {
            for (index, branch) in branches.iter_mut().enumerate() {
                branch.label = format!("{owner_context}::branch[{index}]::{}", branch.label);
            }
        }
        _ => {}
    }
}
