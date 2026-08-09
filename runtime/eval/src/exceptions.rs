use std::collections::{BTreeMap, HashSet};

use sha2::{Digest, Sha256};

use super::*;
use crate::{
    capabilities::{RuntimeExceptionLogMetadata, RuntimeExceptionLogReason},
    error::{instantiated_type_argument_identity, unwrap_diagnostic_source_context},
};
use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_linked_program::{
    FileAddr, LinkedFileUnit, LinkedNamedUnionBranch, LinkedNominalTypeRefBase,
    LinkedTypeDescriptor, LinkedTypeRef, ResolvedSymbol, ServiceErrorExecutionContext,
    ServiceErrorTypeLink, TypeAddr, UnitAddr,
};
use skiff_runtime_linked_type_plan::{PlanContext, ProgramTypeView, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::{
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, LiteralIdentity,
        LocalExecutionTypeIdentity, NamedUnionBranchIdentity, NamedUnionOwnerIdentity,
        NominalTypeIdentity, PackageSchemaTypeIdentity, PlatformBuiltinErrorIdentity,
        RequestException,
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

/// Builds the closed, payload-free identity projection used by automatic
/// exception logs. The readable label omits literal/discriminator values and
/// generic argument contents; the hash fingerprints that same safe label and
/// therefore cannot expose values omitted by the projection.
pub(crate) fn runtime_exception_log_metadata(
    identity: &CatchIdentity,
    reason: RuntimeExceptionLogReason,
    callable: Option<String>,
) -> RuntimeExceptionLogMetadata {
    let identity = safe_catch_identity_label(identity);
    RuntimeExceptionLogMetadata {
        identity_hash: safe_identity_hash(&identity),
        identity,
        reason,
        callable,
    }
}

pub(crate) fn internal_exception_log_metadata(
    callable: Option<String>,
) -> RuntimeExceptionLogMetadata {
    const IDENTITY: &str = "skiff.runtime.InternalError";
    RuntimeExceptionLogMetadata {
        identity: IDENTITY.to_string(),
        identity_hash: safe_identity_hash(IDENTITY),
        reason: RuntimeExceptionLogReason::Internal,
        callable,
    }
}

fn safe_identity_hash(identity: &str) -> String {
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(identity.as_bytes()))
    )
}

fn safe_catch_identity_label(identity: &CatchIdentity) -> String {
    match identity {
        CatchIdentity::Nominal(identity) => safe_nominal_identity_label(identity),
        CatchIdentity::NamedUnionBranch { union, branch } => format!(
            "{}#{}",
            safe_named_union_owner_label(union),
            safe_named_union_branch_label(branch)
        ),
    }
}

fn safe_nominal_identity_label(identity: &NominalTypeIdentity) -> String {
    match identity {
        NominalTypeIdentity::PlatformBuiltin(identity) => identity.symbol().to_string(),
        NominalTypeIdentity::PackageSchema(identity) => format!(
            "package:{}:{}:{}",
            identity.package_id,
            identity.stable_schema_key,
            identity.package_schema_type_id.as_str()
        ),
        NominalTypeIdentity::LocalExecution(identity) => format!(
            "local:{}:typeArguments[{}]",
            identity.addr,
            identity.type_arguments.len()
        ),
    }
}

fn safe_named_union_owner_label(identity: &NamedUnionOwnerIdentity) -> String {
    match identity {
        NamedUnionOwnerIdentity::PackageSchema(identity) => format!(
            "package:{}:{}:{}",
            identity.package_id,
            identity.stable_schema_key,
            identity.package_schema_type_id.as_str()
        ),
        NamedUnionOwnerIdentity::LocalExecution(identity) => format!(
            "local:{}:typeArguments[{}]",
            identity.addr,
            identity.type_arguments.len()
        ),
    }
}

fn safe_named_union_branch_label(identity: &NamedUnionBranchIdentity) -> String {
    match identity {
        NamedUnionBranchIdentity::ConcreteNominal { identity } => {
            format!("nominal:{}", safe_nominal_identity_label(identity))
        }
        NamedUnionBranchIdentity::SyntheticDiscriminator { .. } => {
            "syntheticDiscriminator".to_string()
        }
        NamedUnionBranchIdentity::Literal { value } => match value {
            LiteralIdentity::Null => "literal:null",
            LiteralIdentity::Bool(_) => "literal:bool",
            LiteralIdentity::Number(_) => "literal:number",
            LiteralIdentity::String(_) => "literal:string",
        }
        .to_string(),
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
    if error.is_cancellation_terminal() {
        return Ok(None);
    }
    if let Some(exception) = user_exception_for_catch(error) {
        if exception
            .actual_payload_type()
            .is_some_and(|identity| catch_identity_matches(identity, leaves))
        {
            return Ok(Some(exception.request().clone()));
        }
        return Ok(None);
    }
    let Some((identity, payload)) = error.ordinary_catch_projection() else {
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

pub(crate) fn request_exception_for_resource_error(
    error: &RuntimeError,
    projection: &crate::assembly_execution::RuntimeExecutionProjection<'_>,
    current_addr: &skiff_runtime_linked_program::ExecutableAddr,
    source: InstructionSourceSite,
    stack: Vec<ExceptionStackFrame>,
    next_correlation: impl FnOnce(RuntimeExceptionLogMetadata) -> Result<ErrorCorrelation>,
    heap: &mut RequestHeap,
) -> Result<Option<RequestException>> {
    const STD_PACKAGE_ID: &str = "skiff.run/std";
    const RESOURCE_ERROR_SYMBOL: &str = "std.resource.ResourceError";

    let RuntimeError::ResourceError { path, message } = unwrap_diagnostic_source_context(error)
    else {
        return Ok(None);
    };
    let resolved = projection
        .resolved_package_id_symbol(STD_PACKAGE_ID, RESOURCE_ERROR_SYMBOL)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "native ResourceError requires exact public type {STD_PACKAGE_ID}:{RESOURCE_ERROR_SYMBOL}"
            ))
        })?;
    let ResolvedSymbol::Type { addr } = resolved else {
        return Err(RuntimeError::InvalidArtifact(format!(
            "native ResourceError public symbol {STD_PACKAGE_ID}:{RESOURCE_ERROR_SYMBOL} is not a type"
        )));
    };
    let canonical_addr = projection.canonical_type_addr(addr)?;
    let UnitAddr::Package(package_slot) = &canonical_addr.unit else {
        return Err(RuntimeError::InvalidArtifact(
            "native ResourceError public type resolved outside Package code".to_string(),
        ));
    };
    if projection.package_id(*package_slot) != Some(STD_PACKAGE_ID) {
        return Err(RuntimeError::InvalidArtifact(
            "native ResourceError public type is not owned by exact package skiff.run/std"
                .to_string(),
        ));
    }
    projection.validate_public_package_type(
        STD_PACKAGE_ID,
        RESOURCE_ERROR_SYMBOL,
        &canonical_addr,
    )?;
    let root_ref = LinkedTypeRef::Address {
        addr: canonical_addr.clone(),
    };
    let mut plan = RuntimeTypePlan::from_linked_nested_ref(
        &root_ref,
        &PlanContext::from_type_view(projection.type_view(), current_addr),
    )?;
    annotate_runtime_type_plan(&mut plan, &root_ref, projection.type_view())?;
    validate_resource_error_plan(&plan, &canonical_addr)?;

    let handle = heap
        .alloc_object_carriers(BTreeMap::from([
            (
                "path".to_string(),
                RuntimeValueCarrier::unidentified(RuntimeValue::String(path.clone())),
            ),
            (
                "message".to_string(),
                RuntimeValueCarrier::unidentified(RuntimeValue::String(message.clone())),
            ),
        ]))
        .map_err(RuntimeError::from)?;
    let carrier = runtime_carrier_for_plan(
        RuntimeValue::Heap(handle),
        &plan,
        "native std.resource.ResourceError projection",
        heap,
    )?;
    let expected_identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: canonical_addr,
            type_arguments: Vec::new(),
        },
    ));
    if carrier.catch_identity() != Some(&expected_identity) {
        return Err(RuntimeError::InvalidArtifact(
            "native ResourceError materialization lost its exact Package-owned identity"
                .to_string(),
        ));
    }
    let metadata = runtime_exception_log_metadata(
        &expected_identity,
        RuntimeExceptionLogReason::RuntimeProjection,
        Some(current_addr.to_string()),
    );
    RequestException::local(carrier, source, stack, next_correlation(metadata)?)
        .map(Some)
        .map_err(RuntimeError::InvalidArtifact)
}

fn validate_resource_error_plan(plan: &RuntimeTypePlan, addr: &TypeAddr) -> Result<()> {
    let expected_identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: addr.clone(),
            type_arguments: Vec::new(),
        },
    ));
    if plan.catch_identity() != Some(&expected_identity) {
        return Err(RuntimeError::InvalidArtifact(
            "std.resource.ResourceError linked plan is missing its exact Package-owned nominal identity"
                .to_string(),
        ));
    }
    let RuntimeTypeNode::Record { fields, .. } = plan.node() else {
        return Err(RuntimeError::InvalidArtifact(
            "std.resource.ResourceError linked plan root is not a record".to_string(),
        ));
    };
    let exact_string_field = |name: &str| {
        fields.iter().filter(|field| field.name == name).count() == 1
            && fields.iter().any(|field| {
                field.name == name
                    && field.required
                    && matches!(field.ty.node(), RuntimeTypeNode::String)
            })
    };
    if fields.len() != 2 || !exact_string_field("path") || !exact_string_field("message") {
        return Err(RuntimeError::InvalidArtifact(
            "std.resource.ResourceError linked plan must be the exact required path/message string record"
                .to_string(),
        ));
    }
    Ok(())
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
            if let Some(identity) = PlatformBuiltinErrorIdentity::from_symbol(name)
                .filter(|identity| admitted_platform_builtin(*identity))
            {
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
        .map(instantiated_type_argument_identity)
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
        LinkedTypeRef::ServiceSymbol { symbol }
            if program_actor_declaration_symbol(program, symbol) => {}
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

fn program_actor_declaration_symbol(
    program: ProgramTypeView<'_>,
    symbol: &skiff_runtime_linked_program::ServiceSymbolRef,
) -> bool {
    program
        .service_files
        .iter()
        .chain(program.packages.iter().flat_map(|package| package.files()))
        .flat_map(|file| file.actor_declarations.iter())
        .any(|declaration| declaration.actor_type == *symbol)
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
        .filter(|identity| admitted_platform_builtin(*identity))
}

fn admitted_platform_builtin(identity: PlatformBuiltinErrorIdentity) -> bool {
    matches!(
        identity,
        PlatformBuiltinErrorIdentity::Timeout
            | PlatformBuiltinErrorIdentity::ConfigDecode
            | PlatformBuiltinErrorIdentity::BytesDecode
            | PlatformBuiltinErrorIdentity::NumberDecode
            | PlatformBuiltinErrorIdentity::JsonDecode
            | PlatformBuiltinErrorIdentity::DbConflict
            | PlatformBuiltinErrorIdentity::DbConstraint
            | PlatformBuiltinErrorIdentity::DbDecode
            | PlatformBuiltinErrorIdentity::File
            | PlatformBuiltinErrorIdentity::TimeDecode
            | PlatformBuiltinErrorIdentity::ServiceProviderUnavailable
            | PlatformBuiltinErrorIdentity::ServiceProtocol
            | PlatformBuiltinErrorIdentity::Http
    )
}

fn standard_type_symbol_for_addr(addr: &TypeAddr, program: ProgramTypeView<'_>) -> Option<String> {
    let UnitAddr::Package(slot) = &addr.unit else {
        return None;
    };
    let slot = *slot;
    let package = program.packages.get(slot)?;
    if package.package_id() != "skiff.run/std" {
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
    let files = program.packages.get(slot)?.files();
    match file_addr {
        FileAddr::LoadedFileIndex(index) => files.get(*index).map(AsRef::as_ref),
        FileAddr::FileIrIdentity(identity) => files
            .iter()
            .find(|file| file.file_ir_identity == *identity)
            .map(AsRef::as_ref),
    }
}

#[cfg(test)]
mod tests;
