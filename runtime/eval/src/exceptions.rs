use std::collections::{BTreeMap, HashSet};

use super::*;
use crate::error::{unwrap_diagnostic_source_context, WirePayload};
use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_linked_program::{
    FileAddr, LinkedFileUnit, LinkedNamedUnionBranch, LinkedNominalTypeRefBase,
    LinkedTypeDescriptor, LinkedTypeRef, ServiceErrorExecutionContext, ServiceErrorTypeLink,
    TypeAddr, UnitAddr,
};
use skiff_runtime_linked_type_plan::{PlanContext, ProgramTypeView, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::{
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, InstantiatedTypeArgumentIdentity,
        LiteralIdentity, LocalExecutionTypeIdentity, NamedUnionBranchIdentity,
        NamedUnionOwnerIdentity, NominalTypeIdentity, PackageSchemaTypeIdentity,
        PlatformBuiltinErrorIdentity, RequestException,
    },
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
    value::{HeapNode, RuntimeValueCarrier},
};

pub fn user_exception_for_catch(error: &RuntimeError) -> Option<&UserException> {
    match unwrap_diagnostic_source_context(error) {
        RuntimeError::UserException(exception) => Some(exception),
        _ => None,
    }
}

pub fn request_exception_for_catch(
    error: &RuntimeError,
    leaves: &[CatchIdentity],
    source: InstructionSourceSite,
    stack: Vec<ExceptionStackFrame>,
    correlation: ErrorCorrelation,
    heap: &mut RequestHeap,
) -> Result<Option<RequestException>> {
    if let Some(exception) = user_exception_for_catch(error) {
        if exception
            .actual_payload_type()
            .is_some_and(|identity| catch_identity_matches(identity, leaves))
        {
            return Ok(Some(exception.request().clone()));
        }
        return Ok(None);
    }
    let Some((identity, payload)) = error.catch_projection() else {
        return Ok(None);
    };
    if !catch_identity_matches(&identity, leaves) {
        return Ok(None);
    }
    let value = runtime_from_wire(&payload, heap)?;
    let carrier = RuntimeValueCarrier::identified(value, identity);
    RequestException::local(carrier, source, stack, correlation)
        .map(Some)
        .map_err(RuntimeError::InvalidArtifact)
}

pub fn catch_ok(value: RuntimeValueCarrier, heap: &mut RequestHeap) -> Result<RuntimeValueCarrier> {
    let handle = heap
        .alloc_object_carriers(BTreeMap::from([
            (
                "tag".to_string(),
                RuntimeValueCarrier::unidentified(RuntimeValue::String("ok".to_string())),
            ),
            ("value".to_string(), value),
        ]))
        .map_err(RuntimeError::from)?;
    Ok(RuntimeValueCarrier::unidentified(RuntimeValue::Heap(
        handle,
    )))
}

pub fn catch_err(
    exception: RequestException,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    let exception = heap
        .alloc_exception(exception)
        .map(RuntimeValue::Heap)
        .map(RuntimeValueCarrier::unidentified)
        .map_err(RuntimeError::from)?;
    let handle = heap
        .alloc_object_carriers(BTreeMap::from([
            (
                "tag".to_string(),
                RuntimeValueCarrier::unidentified(RuntimeValue::String("err".to_string())),
            ),
            ("exception".to_string(), exception),
        ]))
        .map_err(RuntimeError::from)?;
    Ok(RuntimeValueCarrier::unidentified(RuntimeValue::Heap(
        handle,
    )))
}

pub fn request_exception_for_rethrow(
    value: &RuntimeValueCarrier,
    heap: &RequestHeap,
) -> Result<RequestException> {
    let RuntimeValue::Heap(handle) = value.value() else {
        return Err(RuntimeError::InvalidArtifact(
            "rethrow requires an existing request-local Exception value".to_string(),
        ));
    };
    let HeapNode::Exception(exception) = heap.get(*handle)? else {
        return Err(RuntimeError::InvalidArtifact(
            "rethrow requires an existing request-local Exception value".to_string(),
        ));
    };
    Ok(exception.clone())
}

/// Resolves an actual named-union identity to the one exact linked branch.
///
/// This consumes linked declaration identity only; it never compares runtime
/// value shape or public/display names.
pub(crate) fn exact_named_union_branch_index(
    union: &LocalExecutionTypeIdentity,
    actual: &NamedUnionBranchIdentity,
    program: ProgramTypeView<'_>,
) -> Result<usize> {
    let declaration = program.types.declaration(&union.addr).ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "RuntimeProgram type address {} is not interned",
            union.addr
        ))
    })?;
    if declaration.type_params.len() != union.type_arguments.len() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "RuntimeProgram nominal {} expects {} type argument(s), got {}",
            declaration.name,
            declaration.type_params.len(),
            union.type_arguments.len()
        )));
    }
    if !union.type_arguments.is_empty() {
        return Err(RuntimeError::InvalidArtifact(
            "public service-error named union must be schema-closed".to_string(),
        ));
    }
    let LinkedTypeDescriptor::Union { branches } = &declaration.descriptor else {
        return Err(RuntimeError::InvalidArtifact(
            "named-union service error identity points at a non-union declaration".to_string(),
        ));
    };
    let mut matches = Vec::new();
    for (index, branch) in branches.iter().enumerate() {
        let identity =
            named_union_branch_identity(branch, &BTreeMap::new(), program, &mut HashSet::new())?;
        if &identity == actual {
            matches.push(index);
        }
    }
    match matches.as_slice() {
        [index] => Ok(*index),
        [] => Err(RuntimeError::InvalidArtifact(
            "actual named-union service error identity has no linked branch".to_string(),
        )),
        _ => Err(RuntimeError::InvalidArtifact(
            "actual named-union service error identity matches multiple linked branches"
                .to_string(),
        )),
    }
}

/// Restores a decoded Package-schema value to its exact caller-local carrier.
///
/// The selected service-error index row chooses the local declaration or union
/// branch before recursive carrier annotation, so equal-shape union branches
/// cannot be guessed by `runtime_carrier_for_plan`.
pub(crate) fn materialize_service_error_local_value(
    value: RuntimeValue,
    link: &ServiceErrorTypeLink,
    program: ProgramTypeView<'_>,
    current_addr: &skiff_runtime_linked_program::ExecutableAddr,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    let root_ref = LinkedTypeRef::Address {
        addr: link.context().execution_addr().clone(),
    };
    let mut plan = RuntimeTypePlan::from_linked_nested_ref(
        &root_ref,
        &PlanContext::from_type_view(program, current_addr),
    )?;
    annotate_runtime_type_plan(&mut plan, &root_ref, program)?;
    let selected = match link.context() {
        ServiceErrorExecutionContext::Declaration { .. } => plan,
        ServiceErrorExecutionContext::NamedUnionBranch { branch_index, .. } => {
            let RuntimeTypeNode::Union(branches) = plan.node() else {
                return Err(RuntimeError::InvalidArtifact(
                    "service-error named-union row produced a non-union local plan".to_string(),
                ));
            };
            branches.get(*branch_index).cloned().ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "service-error named-union branch {branch_index} is outside the local plan"
                ))
            })?
        }
    };
    runtime_carrier_for_plan(value, &selected, "service error import", heap)
}

pub fn catch_type_leaves<'p>(
    catch_type: &LinkedTypeRef,
    program: impl Into<ProgramTypeView<'p>>,
) -> Result<Vec<CatchIdentity>> {
    let program = program.into();
    let mut leaves = Vec::new();
    let mut visiting = HashSet::new();
    collect_catch_type_leaves(
        catch_type,
        program,
        &BTreeMap::new(),
        &mut visiting,
        &mut leaves,
    )?;
    if leaves.is_empty() {
        return Err(RuntimeError::InvalidArtifact(
            "catchType must name at least one concrete error type".to_string(),
        ));
    }
    Ok(leaves)
}

fn collect_catch_type_leaves(
    catch_type: &LinkedTypeRef,
    program: ProgramTypeView<'_>,
    substitutions: &BTreeMap<String, LinkedTypeRef>,
    visiting: &mut HashSet<LocalExecutionTypeIdentity>,
    leaves: &mut Vec<CatchIdentity>,
) -> Result<()> {
    let catch_type = instantiate_type_ref(catch_type, substitutions)?;
    match &catch_type {
        LinkedTypeRef::Address { addr } => {
            collect_nominal_leaves(addr, &[], program, visiting, leaves)?;
        }
        LinkedTypeRef::AppliedNominal { base, arguments } => {
            let addr = applied_nominal_addr(base)?;
            collect_nominal_leaves(addr, arguments, program, visiting, leaves)?;
        }
        LinkedTypeRef::Union { items } => {
            for item in items {
                collect_catch_type_leaves(item, program, substitutions, visiting, leaves)?;
            }
        }
        LinkedTypeRef::Native { name, args } if args.is_empty() => {
            if let Some(identity) = PlatformBuiltinErrorIdentity::from_symbol(name) {
                push_catch_leaf(identity.catch_identity(), leaves);
            }
        }
        LinkedTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            let identity = PackageSchemaTypeIdentity::new(
                package_id.clone(),
                stable_schema_key.clone(),
                package_schema_type_id.clone(),
            )
            .map_err(RuntimeError::InvalidArtifact)?;
            push_catch_leaf(
                CatchIdentity::Nominal(NominalTypeIdentity::PackageSchema(identity)),
                leaves,
            );
        }
        LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. } => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "RuntimeProgram type ref {} was not linked before execution",
                program_types::program_type_ref_kind(&catch_type)
            )));
        }
        LinkedTypeRef::Native { .. }
        | LinkedTypeRef::Record { .. }
        | LinkedTypeRef::Nullable { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::AnyInterface { .. }
        | LinkedTypeRef::Function { .. }
        | LinkedTypeRef::DbObjectSymbol { .. } => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "catchType contains non-catchable {}",
                program_types::program_type_ref_kind(&catch_type)
            )));
        }
        LinkedTypeRef::TypeParam { name } => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "catchType contains unbound type parameter {name}"
            )));
        }
    }
    Ok(())
}

fn collect_nominal_leaves(
    addr: &TypeAddr,
    arguments: &[LinkedTypeRef],
    program: ProgramTypeView<'_>,
    visiting: &mut HashSet<LocalExecutionTypeIdentity>,
    leaves: &mut Vec<CatchIdentity>,
) -> Result<()> {
    let declaration = program.types.declaration(addr).ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "RuntimeProgram type address {addr} is not interned"
        ))
    })?;
    if declaration.type_params.len() != arguments.len() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "RuntimeProgram nominal {} expects {} type argument(s), got {}",
            declaration.name,
            declaration.type_params.len(),
            arguments.len()
        )));
    }
    let owner = local_execution_identity(addr, arguments)?;
    if !visiting.insert(owner.clone()) {
        return Err(RuntimeError::InvalidArtifact(format!(
            "RuntimeProgram type address {addr} has cyclic catch descriptor"
        )));
    }
    let substitutions = declaration
        .type_params
        .iter()
        .cloned()
        .zip(arguments.iter().cloned())
        .collect::<BTreeMap<_, _>>();
    match &declaration.descriptor {
        LinkedTypeDescriptor::Record { .. } | LinkedTypeDescriptor::Representation { .. } => {
            let identity = platform_builtin_for_addr(addr, program)
                .map(PlatformBuiltinErrorIdentity::catch_identity)
                .unwrap_or_else(|| {
                    CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(owner.clone()))
                });
            push_catch_leaf(identity, leaves);
        }
        LinkedTypeDescriptor::Alias { target } => {
            collect_catch_type_leaves(target, program, &substitutions, visiting, leaves)?;
        }
        LinkedTypeDescriptor::Union { branches } => {
            let union = NamedUnionOwnerIdentity::LocalExecution(owner.clone());
            for branch in branches {
                let branch =
                    named_union_branch_identity(branch, &substitutions, program, visiting)?;
                push_catch_leaf(
                    CatchIdentity::NamedUnionBranch {
                        union: union.clone(),
                        branch,
                    },
                    leaves,
                );
            }
        }
        LinkedTypeDescriptor::Interface => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "interface type {addr} cannot be a catch leaf"
            )));
        }
    }
    visiting.remove(&owner);
    Ok(())
}

fn named_union_branch_identity(
    branch: &LinkedNamedUnionBranch,
    substitutions: &BTreeMap<String, LinkedTypeRef>,
    program: ProgramTypeView<'_>,
    visiting: &mut HashSet<LocalExecutionTypeIdentity>,
) -> Result<NamedUnionBranchIdentity> {
    match branch {
        LinkedNamedUnionBranch::ConcreteNominal { nominal_type } => {
            let nominal_type = instantiate_type_ref(nominal_type, substitutions)?;
            let mut leaves = Vec::new();
            collect_catch_type_leaves(
                &nominal_type,
                program,
                &BTreeMap::new(),
                visiting,
                &mut leaves,
            )?;
            let [CatchIdentity::Nominal(identity)] = leaves.as_slice() else {
                return Err(RuntimeError::InvalidArtifact(
                    "named union concrete branch must resolve to exactly one nominal identity"
                        .to_string(),
                ));
            };
            Ok(NamedUnionBranchIdentity::ConcreteNominal {
                identity: identity.clone(),
            })
        }
        LinkedNamedUnionBranch::SyntheticDiscriminator {
            discriminator_field,
            discriminator_value,
            ..
        } => Ok(NamedUnionBranchIdentity::SyntheticDiscriminator {
            discriminator_field: discriminator_field.clone(),
            discriminator_value: discriminator_value.clone(),
        }),
        LinkedNamedUnionBranch::Literal { value } => Ok(NamedUnionBranchIdentity::Literal {
            value: LiteralIdentity::from(value),
        }),
    }
}

fn local_execution_identity(
    addr: &TypeAddr,
    arguments: &[LinkedTypeRef],
) -> Result<LocalExecutionTypeIdentity> {
    let type_arguments = arguments
        .iter()
        .map(|argument| {
            let canonical = serde_json::to_string(argument).map_err(RuntimeError::Json)?;
            InstantiatedTypeArgumentIdentity::new(canonical).map_err(RuntimeError::InvalidArtifact)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(LocalExecutionTypeIdentity {
        addr: addr.clone(),
        type_arguments,
    })
}

fn applied_nominal_addr(base: &LinkedNominalTypeRefBase) -> Result<&TypeAddr> {
    match base {
        LinkedNominalTypeRefBase::Address { addr } => Ok(addr),
        _ => Err(RuntimeError::InvalidArtifact(
            "applied nominal base was not linked to an exact type address".to_string(),
        )),
    }
}

fn instantiate_type_ref(
    type_ref: &LinkedTypeRef,
    substitutions: &BTreeMap<String, LinkedTypeRef>,
) -> Result<LinkedTypeRef> {
    Ok(match type_ref {
        LinkedTypeRef::TypeParam { name } => substitutions.get(name).cloned().ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!("unbound nominal type parameter {name}"))
        })?,
        LinkedTypeRef::Native { name, args } => LinkedTypeRef::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|argument| instantiate_type_ref(argument, substitutions))
                .collect::<Result<Vec<_>>>()?,
        },
        LinkedTypeRef::AppliedNominal { base, arguments } => LinkedTypeRef::AppliedNominal {
            base: base.clone(),
            arguments: arguments
                .iter()
                .map(|argument| instantiate_type_ref(argument, substitutions))
                .collect::<Result<Vec<_>>>()?,
        },
        LinkedTypeRef::Record { fields } => LinkedTypeRef::Record {
            fields: fields
                .iter()
                .map(|(name, field)| {
                    Ok((name.clone(), instantiate_type_ref(field, substitutions)?))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
        },
        LinkedTypeRef::Union { items } => LinkedTypeRef::Union {
            items: items
                .iter()
                .map(|item| instantiate_type_ref(item, substitutions))
                .collect::<Result<Vec<_>>>()?,
        },
        LinkedTypeRef::Nullable { inner } => LinkedTypeRef::Nullable {
            inner: Box::new(instantiate_type_ref(inner, substitutions)?),
        },
        LinkedTypeRef::AnyInterface { interface } => LinkedTypeRef::AnyInterface {
            interface: skiff_runtime_linked_program::LinkedInterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|argument| instantiate_type_ref(argument, substitutions))
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
                        ty: instantiate_type_ref(&parameter.ty, substitutions)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            return_type: Box::new(instantiate_type_ref(return_type, substitutions)?),
        },
        _ => type_ref.clone(),
    })
}

pub fn annotate_runtime_type_plan(
    plan: &mut RuntimeTypePlan,
    type_ref: &LinkedTypeRef,
    program: ProgramTypeView<'_>,
) -> Result<()> {
    match type_ref {
        LinkedTypeRef::Address { addr } => {
            annotate_nominal_plan(plan, addr, &[], program)?;
        }
        LinkedTypeRef::AppliedNominal { base, arguments } => {
            annotate_nominal_plan(plan, applied_nominal_addr(base)?, arguments, program)?;
        }
        LinkedTypeRef::Native { name, args } => {
            if let Some(identity) = PlatformBuiltinErrorIdentity::from_symbol(name) {
                plan.identity.catch_identity = Some(identity.catch_identity());
            }
            match (plan.node(), args.as_slice()) {
                (RuntimeTypeNode::Array(item) | RuntimeTypeNode::Stream(item), [argument]) => {
                    let mut item = (**item).clone();
                    annotate_runtime_type_plan(&mut item, argument, program)?;
                    match &mut plan.node {
                        RuntimeTypeNode::Array(slot) | RuntimeTypeNode::Stream(slot) => {
                            **slot = item;
                        }
                        _ => unreachable!("plan node was matched before mutation"),
                    }
                }
                (RuntimeTypeNode::Map { key, value }, [key_ref, value_ref]) => {
                    let mut key = (**key).clone();
                    let mut value = (**value).clone();
                    annotate_runtime_type_plan(&mut key, key_ref, program)?;
                    annotate_runtime_type_plan(&mut value, value_ref, program)?;
                    let RuntimeTypeNode::Map {
                        key: key_slot,
                        value: value_slot,
                    } = &mut plan.node
                    else {
                        unreachable!("plan node was matched before mutation");
                    };
                    **key_slot = key;
                    **value_slot = value;
                }
                _ => {}
            }
        }
        LinkedTypeRef::Record { fields } => {
            annotate_record_fields(plan, fields, program)?;
        }
        LinkedTypeRef::Union { items } => {
            let RuntimeTypeNode::Union(branches) = &mut plan.node else {
                return Err(RuntimeError::InvalidArtifact(
                    "linked anonymous union produced a non-union runtime type plan".to_string(),
                ));
            };
            if branches.len() != items.len() {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "linked anonymous union plan has {} branches for {} items",
                    branches.len(),
                    items.len()
                )));
            }
            for (branch, item) in branches.iter_mut().zip(items) {
                annotate_runtime_type_plan(branch, item, program)?;
            }
        }
        LinkedTypeRef::Nullable { inner } => {
            let RuntimeTypeNode::Nullable(inner_plan) = &mut plan.node else {
                return Err(RuntimeError::InvalidArtifact(
                    "linked nullable produced a non-nullable runtime type plan".to_string(),
                ));
            };
            annotate_runtime_type_plan(inner_plan, inner, program)?;
        }
        LinkedTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            let identity = PackageSchemaTypeIdentity::new(
                package_id.clone(),
                stable_schema_key.clone(),
                package_schema_type_id.clone(),
            )
            .map_err(RuntimeError::InvalidArtifact)?;
            plan.identity.catch_identity = Some(CatchIdentity::Nominal(
                NominalTypeIdentity::PackageSchema(identity),
            ));
        }
        LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::AnyInterface { .. }
        | LinkedTypeRef::Function { .. }
        | LinkedTypeRef::DbObjectSymbol { .. } => {}
        LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. }
        | LinkedTypeRef::TypeParam { .. } => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "runtime type plan identity input {} was not fully linked and instantiated",
                program_types::program_type_ref_kind(type_ref)
            )));
        }
    }
    Ok(())
}

fn annotate_nominal_plan(
    plan: &mut RuntimeTypePlan,
    addr: &TypeAddr,
    arguments: &[LinkedTypeRef],
    program: ProgramTypeView<'_>,
) -> Result<()> {
    let declaration = program.types.declaration(addr).ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "RuntimeProgram type address {addr} is not interned"
        ))
    })?;
    if declaration.type_params.len() != arguments.len() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "RuntimeProgram nominal {} expects {} type argument(s), got {}",
            declaration.name,
            declaration.type_params.len(),
            arguments.len()
        )));
    }
    let owner = local_execution_identity(addr, arguments)?;
    let substitutions = declaration
        .type_params
        .iter()
        .cloned()
        .zip(arguments.iter().cloned())
        .collect::<BTreeMap<_, _>>();
    match &declaration.descriptor {
        LinkedTypeDescriptor::Record { fields } => {
            annotate_record_fields_instantiated(plan, fields, &substitutions, program)?;
            plan.identity.catch_identity = Some(
                platform_builtin_for_addr(addr, program)
                    .map(PlatformBuiltinErrorIdentity::catch_identity)
                    .unwrap_or_else(|| {
                        CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(owner))
                    }),
            );
        }
        LinkedTypeDescriptor::Representation { representation } => {
            let RuntimeTypeNode::Representation { payload, .. } = &mut plan.node else {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "nominal representation {addr} produced a non-representation runtime type plan"
                )));
            };
            let representation = instantiate_type_ref(representation, &substitutions)?;
            annotate_runtime_type_plan(payload, &representation, program)?;
            plan.identity.catch_identity = Some(
                platform_builtin_for_addr(addr, program)
                    .map(PlatformBuiltinErrorIdentity::catch_identity)
                    .unwrap_or_else(|| {
                        CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(owner))
                    }),
            );
        }
        LinkedTypeDescriptor::Alias { target } => {
            let RuntimeTypeNode::Alias(inner) = &mut plan.node else {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "transparent alias {addr} produced a non-alias runtime type plan"
                )));
            };
            let target = instantiate_type_ref(target, &substitutions)?;
            annotate_runtime_type_plan(inner, &target, program)?;
        }
        LinkedTypeDescriptor::Union { branches } => {
            let RuntimeTypeNode::Union(branch_plans) = &mut plan.node else {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "named union {addr} produced a non-union runtime type plan"
                )));
            };
            if branch_plans.len() != branches.len() {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "named union {addr} plan has {} branches for {} declarations",
                    branch_plans.len(),
                    branches.len()
                )));
            }
            let union = NamedUnionOwnerIdentity::LocalExecution(owner);
            let mut visiting = HashSet::new();
            for (branch_plan, branch) in branch_plans.iter_mut().zip(branches) {
                match branch {
                    LinkedNamedUnionBranch::ConcreteNominal { nominal_type } => {
                        let nominal_type = instantiate_type_ref(nominal_type, &substitutions)?;
                        annotate_runtime_type_plan(branch_plan, &nominal_type, program)?;
                    }
                    LinkedNamedUnionBranch::SyntheticDiscriminator { payload_type, .. } => {
                        let payload_type = instantiate_type_ref(payload_type, &substitutions)?;
                        annotate_runtime_type_plan(branch_plan, &payload_type, program)?;
                    }
                    LinkedNamedUnionBranch::Literal { .. } => {}
                }
                let branch =
                    named_union_branch_identity(branch, &substitutions, program, &mut visiting)?;
                branch_plan.identity.catch_identity = Some(CatchIdentity::NamedUnionBranch {
                    union: union.clone(),
                    branch,
                });
            }
        }
        LinkedTypeDescriptor::Interface => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "interface declaration {addr} cannot produce a runtime value plan"
            )));
        }
    }
    Ok(())
}

fn annotate_record_fields(
    plan: &mut RuntimeTypePlan,
    fields: &BTreeMap<String, LinkedTypeRef>,
    program: ProgramTypeView<'_>,
) -> Result<()> {
    annotate_record_fields_instantiated(plan, fields, &BTreeMap::new(), program)
}

fn annotate_record_fields_instantiated(
    plan: &mut RuntimeTypePlan,
    fields: &BTreeMap<String, LinkedTypeRef>,
    substitutions: &BTreeMap<String, LinkedTypeRef>,
    program: ProgramTypeView<'_>,
) -> Result<()> {
    let RuntimeTypeNode::Record {
        fields: field_plans,
        ..
    } = &mut plan.node
    else {
        return Err(RuntimeError::InvalidArtifact(
            "linked record produced a non-record runtime type plan".to_string(),
        ));
    };
    for field_plan in field_plans {
        let field_type = fields.get(&field_plan.name).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "runtime record plan contains unknown field {}",
                field_plan.name
            ))
        })?;
        let field_type = instantiate_type_ref(field_type, substitutions)?;
        annotate_runtime_type_plan(&mut field_plan.ty, &field_type, program)?;
    }
    Ok(())
}

fn push_catch_leaf(leaf: CatchIdentity, leaves: &mut Vec<CatchIdentity>) {
    if !leaves.contains(&leaf) {
        leaves.push(leaf);
    }
}

pub fn catch_identity_matches(identity: &CatchIdentity, leaves: &[CatchIdentity]) -> bool {
    leaves.iter().any(|leaf| leaf == identity)
}

/// Admits only exact identity preservation or exact nominal-to-concrete
/// named-union promotion in the supplied target context.
pub(crate) fn exact_target_accepts_catch_identity(
    actual: &CatchIdentity,
    target: &CatchIdentity,
) -> bool {
    if actual == target {
        return true;
    }
    matches!(
        (actual, target),
        (
            CatchIdentity::Nominal(actual),
            CatchIdentity::NamedUnionBranch {
                branch: NamedUnionBranchIdentity::ConcreteNominal { identity },
                ..
            },
        ) if actual == identity
    )
}

fn platform_builtin_for_addr(
    addr: &TypeAddr,
    program: ProgramTypeView<'_>,
) -> Option<PlatformBuiltinErrorIdentity> {
    let symbol = standard_type_symbol_for_addr(addr, program)?;
    PlatformBuiltinErrorIdentity::from_symbol(&symbol)
}

fn standard_type_symbol_for_addr(addr: &TypeAddr, program: ProgramTypeView<'_>) -> Option<String> {
    let UnitAddr::Package(slot) = &addr.unit else {
        return None;
    };
    let slot = *slot;
    let package = program.packages.get(slot)?;
    if package.package_id != "skiff.run/std" {
        return None;
    }
    let file = package_file_for_addr(program, slot, &addr.file)?;
    let decl = file.types.get(addr.type_index)?;
    if decl.name.contains('.') {
        return Some(decl.name.clone());
    }
    Some(format!("{}.{}", file.module_path, decl.name))
}

fn package_file_for_addr<'a>(
    program: ProgramTypeView<'a>,
    slot: usize,
    file_addr: &FileAddr,
) -> Option<&'a LinkedFileUnit> {
    let files = program.package_files.get(slot)?;
    match file_addr {
        FileAddr::LoadedFileIndex(index) => files.get(*index).map(AsRef::as_ref),
        FileAddr::FileIrIdentity(identity) => files
            .iter()
            .find(|file| file.file_ir_identity == *identity)
            .map(AsRef::as_ref),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use skiff_artifact_model::{SourcePosition, SourceSpanRef};
    use skiff_runtime_linked_program::{LinkOverlay, PackageUnit, RuntimeTypeContext, TypeDeclIr};
    use skiff_runtime_model::runtime_value::{HeapNode, RuntimeValue};

    struct TestProgramTypeView {
        service_files: Vec<Arc<LinkedFileUnit>>,
        packages: Vec<Arc<PackageUnit>>,
        package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
        link_overlay: LinkOverlay,
        types: RuntimeTypeContext,
    }

    impl TestProgramTypeView {
        fn empty() -> Self {
            Self {
                service_files: Vec::new(),
                packages: Vec::new(),
                package_files: Vec::new(),
                link_overlay: LinkOverlay::default(),
                types: RuntimeTypeContext::default(),
            }
        }

        fn view(&self) -> ProgramTypeView<'_> {
            ProgramTypeView::new(
                &self.service_files,
                &self.packages,
                &self.package_files,
                &self.link_overlay,
                &self.types,
            )
        }
    }

    fn addr(type_index: usize) -> TypeAddr {
        TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::loaded_file(0),
            type_index,
        }
    }

    fn type_decl(
        name: &str,
        descriptor: LinkedTypeDescriptor,
        type_params: Vec<&str>,
    ) -> TypeDeclIr {
        TypeDeclIr {
            name: name.to_string(),
            descriptor,
            type_params: type_params.into_iter().map(str::to_string).collect(),
            implements: Vec::new(),
            source_span: None,
        }
    }

    fn source_site() -> InstructionSourceSite {
        InstructionSourceSite::Source {
            span: SourceSpanRef {
                source_id: 9,
                start: SourcePosition::new(4, 2),
                end: SourcePosition::new(4, 7),
            },
        }
    }

    fn local_exception(identity: CatchIdentity) -> RequestException {
        RequestException::local(
            RuntimeValueCarrier::identified(RuntimeValue::from("denied"), identity),
            source_site(),
            vec![ExceptionStackFrame::Local {
                site: source_site(),
            }],
            ErrorCorrelation {
                trace_id: "trace-local".to_string(),
                error_id: "trace-local:local-error:1".to_string(),
            },
        )
        .expect("local exception")
    }

    #[test]
    fn local_catch_and_rethrow_preserve_the_exact_request_exception() {
        let identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
            LocalExecutionTypeIdentity {
                addr: addr(3),
                type_arguments: Vec::new(),
            },
        ));
        let exception = local_exception(identity.clone());
        let error = RuntimeError::UserException(UserException::new(exception.clone()))
            .with_diagnostic_frame(serde_json::json!({ "sourceId": 7 }));
        let mut heap = RequestHeap::default();

        let caught = request_exception_for_catch(
            &error,
            &[identity],
            source_site(),
            vec![ExceptionStackFrame::Local {
                site: source_site(),
            }],
            ErrorCorrelation {
                trace_id: "unused".to_string(),
                error_id: "unused".to_string(),
            },
            &mut heap,
        )
        .expect("catch projection")
        .expect("matching exception");
        assert_eq!(caught, exception);

        let caught_value = catch_err(caught, &mut heap).expect("catch result");
        let RuntimeValue::Heap(catch_handle) = caught_value.value() else {
            panic!("catch result must be a request-local object");
        };
        let exception_value = heap
            .object_field_carrier(*catch_handle, "exception")
            .expect("catch object")
            .expect("exception field");
        let HeapNode::Exception(stored) = heap
            .get(match exception_value.value() {
                RuntimeValue::Heap(handle) => *handle,
                _ => panic!("exception field must be a heap handle"),
            })
            .expect("exception node")
        else {
            panic!("exception field must retain RequestException");
        };
        assert_eq!(stored, &exception);
        assert_eq!(
            request_exception_for_rethrow(&exception_value, &heap).expect("rethrow"),
            exception
        );
    }

    #[test]
    fn fully_instantiated_generic_identities_are_exact_and_fail_closed() {
        let generic_addr = addr(0);
        let mut program = TestProgramTypeView::empty();
        program.types.descriptors.insert(
            generic_addr.clone(),
            type_decl(
                "Failure",
                LinkedTypeDescriptor::Record {
                    fields: BTreeMap::new(),
                },
                vec!["T"],
            ),
        );
        let applied = |name: &str| LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::Address {
                addr: generic_addr.clone(),
            },
            arguments: vec![LinkedTypeRef::Native {
                name: name.to_string(),
                args: Vec::new(),
            }],
        };
        let string_ref = applied("string");
        let bool_ref = applied("bool");
        let string_identity = catch_type_leaves(&string_ref, program.view())
            .expect("string instantiation")[0]
            .clone();
        let bool_identity =
            catch_type_leaves(&bool_ref, program.view()).expect("bool instantiation")[0].clone();

        assert_ne!(string_identity, bool_identity);
        let mut plan = RuntimeTypePlan::synthetic_request_record(Vec::new());
        annotate_runtime_type_plan(&mut plan, &string_ref, program.view())
            .expect("generic plan annotation");
        assert_eq!(plan.catch_identity(), Some(&string_identity));
        assert!(catch_type_leaves(
            &LinkedTypeRef::AppliedNominal {
                base: LinkedNominalTypeRefBase::Address { addr: generic_addr },
                arguments: Vec::new(),
            },
            program.view(),
        )
        .is_err());
        assert!(catch_type_leaves(
            &LinkedTypeRef::TypeParam {
                name: "T".to_string(),
            },
            program.view(),
        )
        .is_err());
    }

    #[test]
    fn aliases_expand_transparently_and_named_union_owners_remain_distinct() {
        let record_addr = addr(1);
        let alias_addr = addr(2);
        let first_union_addr = addr(3);
        let second_union_addr = addr(4);
        let branch = LinkedNamedUnionBranch::SyntheticDiscriminator {
            payload_type: LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            },
            discriminator_field: "kind".to_string(),
            discriminator_value: "denied".to_string(),
        };
        let mut program = TestProgramTypeView::empty();
        program.types.descriptors.insert(
            record_addr.clone(),
            type_decl(
                "Failure",
                LinkedTypeDescriptor::Record {
                    fields: BTreeMap::new(),
                },
                Vec::new(),
            ),
        );
        program.types.descriptors.insert(
            alias_addr.clone(),
            type_decl(
                "FailureAlias",
                LinkedTypeDescriptor::Alias {
                    target: LinkedTypeRef::Address {
                        addr: record_addr.clone(),
                    },
                },
                Vec::new(),
            ),
        );
        for (union_addr, name) in [
            (first_union_addr.clone(), "FirstFailure"),
            (second_union_addr.clone(), "SecondFailure"),
        ] {
            program.types.descriptors.insert(
                union_addr,
                type_decl(
                    name,
                    LinkedTypeDescriptor::Union {
                        branches: vec![branch.clone()],
                    },
                    Vec::new(),
                ),
            );
        }

        assert_eq!(
            catch_type_leaves(&LinkedTypeRef::Address { addr: alias_addr }, program.view(),)
                .expect("alias leaves"),
            catch_type_leaves(
                &LinkedTypeRef::Address { addr: record_addr },
                program.view(),
            )
            .expect("record leaves")
        );
        let first = catch_type_leaves(
            &LinkedTypeRef::Address {
                addr: first_union_addr,
            },
            program.view(),
        );
        let second = catch_type_leaves(
            &LinkedTypeRef::Address {
                addr: second_union_addr,
            },
            program.view(),
        );
        assert_ne!(first.expect("first union"), second.expect("second union"));
    }

    #[test]
    fn finite_platform_error_registry_promotes_to_a_local_exception() {
        assert_eq!(
            PlatformBuiltinErrorIdentity::from_symbol("std.resource.ResourceError"),
            None
        );
        let identity = PlatformBuiltinErrorIdentity::DbDecode.catch_identity();
        let mut heap = RequestHeap::default();
        let exception = request_exception_for_catch(
            &RuntimeError::DbDecode {
                target: "std.db".to_string(),
                message: "missing id".to_string(),
            },
            std::slice::from_ref(&identity),
            source_site(),
            vec![ExceptionStackFrame::Local {
                site: source_site(),
            }],
            ErrorCorrelation {
                trace_id: "trace-platform".to_string(),
                error_id: "trace-platform:local-error:1".to_string(),
            },
            &mut heap,
        )
        .expect("platform promotion")
        .expect("matching platform exception");

        assert_eq!(exception.local_catch_identity(), Some(&identity));
        assert_eq!(exception.source(), &source_site());
        assert!(exception.local_value().is_some());
    }
}
