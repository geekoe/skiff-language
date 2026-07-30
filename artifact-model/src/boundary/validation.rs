use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use crate::{
    BoundaryCallableProjection, BoundaryCallbackContract, BoundaryCallbackExpirationError,
    BoundaryCallbackLifetime, BoundaryConfigRequirement, BoundaryEffectGuarantee,
    BoundaryImplementationRequirements, BoundaryOperationContract, BoundaryParameter,
    BoundaryReturn, BoundaryStreamContract, BoundaryUnavailableReason, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts,
    CallableTargetFact, ContractLiteral, ContractTypeRef, LiteralIr, PackageArtifact,
    PackageCallableId, PackageCallableSignature, PackageLocalAbiSymbol, PackageRuntimeRequirements,
    PackageSchemaTypeRef, PackageTypeRef, TypeRefIr, ValueEscapeLane, ValueProvenance,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryProjectionValidationError {
    message: String,
}

impl BoundaryProjectionValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for BoundaryProjectionValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BoundaryProjectionValidationError {}

/// Validates that every public callable carries the one boundary projection
/// implied by its normalized signature, semantic facts, and package runtime
/// requirements.
pub fn validate_package_boundary_projections(
    artifact: &PackageArtifact,
) -> Result<(), BoundaryProjectionValidationError> {
    let mut callables = BTreeMap::new();
    for symbol in artifact.package_local_abi.public_symbols.values() {
        let PackageLocalAbiSymbol::Callable {
            callable_id,
            signature,
        } = symbol
        else {
            continue;
        };
        if callables.insert(callable_id, signature).is_some() {
            return Err(BoundaryProjectionValidationError::new(format!(
                "public callable id {callable_id} is repeated"
            )));
        }
    }
    let expected_ids = callables.keys().copied().collect::<BTreeSet<_>>();
    let actual_ids = artifact
        .boundary_projections
        .keys()
        .collect::<BTreeSet<_>>();
    if actual_ids != expected_ids {
        return Err(BoundaryProjectionValidationError::new(format!(
            "boundary projection ids must exactly cover public callables; expected={expected_ids:?}, actual={actual_ids:?}"
        )));
    }
    for (callable_id, signature) in callables {
        let facts = artifact
            .callable_semantic_facts
            .get(callable_id)
            .ok_or_else(|| {
                BoundaryProjectionValidationError::new(format!(
                    "public callable {callable_id} has no semantic facts"
                ))
            })?;
        let projection = artifact
            .boundary_projections
            .get(callable_id)
            .expect("boundary projection coverage was checked");
        validate_boundary_callable_projection(
            callable_id,
            signature,
            facts,
            &artifact.runtime_requirements,
            projection,
        )?;
    }
    Ok(())
}

/// Validates the canonical value-plan invariants that are self-contained in a
/// boundary operation contract.
///
/// This validator deliberately does not infer parameter or result types from
/// package signatures. Package artifacts must additionally use
/// [`validate_package_boundary_projections`] for typed-fact agreement.
pub fn validate_boundary_operation_contract(
    contract: &BoundaryOperationContract,
) -> Result<(), BoundaryProjectionValidationError> {
    let operation_lifetime = match &contract.stream {
        BoundaryStreamContract::Unary => BoundaryValueLifetime::Request,
        BoundaryStreamContract::ServerStream { .. } => BoundaryValueLifetime::Stream,
        BoundaryStreamContract::Unsupported { .. } => {
            return Err(BoundaryProjectionValidationError::new(
                "available boundary operation cannot contain an unsupported stream contract",
            ));
        }
    };
    let mut callback_interfaces = BTreeSet::new();
    for (index, parameter) in contract.parameters.iter().enumerate() {
        validate_canonical_position(
            &parameter.ty,
            &parameter.value_plan,
            BoundaryValueOwner::Caller,
            BoundaryValueLifetime::Call,
            operation_lifetime,
            &format!("parameter #{index}"),
            &mut callback_interfaces,
        )?;
    }
    validate_canonical_position(
        &contract.return_value.ty,
        &contract.return_value.value_plan,
        BoundaryValueOwner::Provider,
        BoundaryValueLifetime::Call,
        operation_lifetime,
        "return value",
        &mut callback_interfaces,
    )?;
    match &contract.stream {
        BoundaryStreamContract::Unary => {}
        BoundaryStreamContract::ServerStream {
            item_type,
            item_value_plan,
        } => {
            if contract.return_value.ty != ContractTypeRef::builtin("void") {
                return Err(BoundaryProjectionValidationError::new(
                    "server-stream return sentinel must be exactly builtin void",
                ));
            }
            validate_canonical_position(
                item_type,
                item_value_plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
                BoundaryValueLifetime::Stream,
                "server-stream item",
                &mut callback_interfaces,
            )?;
        }
        BoundaryStreamContract::Unsupported { .. } => unreachable!("rejected above"),
    }
    let expected_callbacks = canonical_callback_contract(callback_interfaces, operation_lifetime);
    if contract.callbacks != expected_callbacks {
        return Err(BoundaryProjectionValidationError::new(format!(
            "callback declaration is not canonical for operation value positions; expected={expected_callbacks:?}, actual={:?}",
            contract.callbacks
        )));
    }
    Ok(())
}

fn validate_boundary_callable_projection(
    callable_id: &PackageCallableId,
    signature: &PackageCallableSignature,
    facts: &CallableSemanticFacts,
    runtime_requirements: &PackageRuntimeRequirements,
    projection: &BoundaryCallableProjection,
) -> Result<(), BoundaryProjectionValidationError> {
    if let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = projection
    {
        validate_boundary_operation_contract(operation_contract).map_err(|error| {
            BoundaryProjectionValidationError::new(format!(
                "boundary projection {callable_id} has an invalid operation contract: {error}"
            ))
        })?;
    }
    let expected = canonical_boundary_callable_projection(signature, facts, runtime_requirements);
    if projection == &expected {
        return Ok(());
    }
    if let (
        BoundaryCallableProjection::Unavailable {
            reasons: expected_reasons,
        },
        BoundaryCallableProjection::Unavailable {
            reasons: actual_reasons,
        },
    ) = (&expected, projection)
    {
        let mut canonical_actual = actual_reasons.clone();
        normalize_reasons(&mut canonical_actual);
        let contains_expected = expected_reasons
            .iter()
            .all(|reason| actual_reasons.contains(reason));
        let only_type_closure_saturation = actual_reasons.iter().all(|reason| {
            expected_reasons.contains(reason) || is_type_closure_unavailable_reason(reason)
        });
        if !actual_reasons.is_empty()
            && canonical_actual == *actual_reasons
            && contains_expected
            && only_type_closure_saturation
        {
            return Ok(());
        }
    }
    Err(BoundaryProjectionValidationError::new(format!(
        "boundary projection {callable_id} is not canonical for its signature, semantic facts, and runtime requirements; expected={expected:?}, actual={projection:?}"
    )))
}

/// Compiler projection can saturate transitive type-closure reasons from File
/// IR bodies. PackageArtifact retains only File IR refs, so this validator can
/// require every locally derivable reason but cannot reconstruct those extra
/// closure facts. Keep this whitelist aligned with compiler boundary type
/// projection; semantic/effect reasons must remain exactly derivable here.
fn is_type_closure_unavailable_reason(reason: &BoundaryUnavailableReason) -> bool {
    matches!(
        reason,
        BoundaryUnavailableReason::CallbackAdapterUnavailable
            | BoundaryUnavailableReason::NativeAdapterUnavailable
            | BoundaryUnavailableReason::UnsupportedBoundaryType
            | BoundaryUnavailableReason::UnsupportedStream
    )
}

/// Canonical classification for one exact contract value position.
///
/// A callback capability is enabled only by a top-level, non-generic
/// `any I` whose interface is an exact Package schema reference. A direct
/// Package schema reference remains ordinary detached data even when its
/// descriptor happens to be a callback interface. Nested or generic
/// existential shapes fail closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundaryCallbackPosition {
    Detached,
    Exact {
        interface_type: PackageSchemaTypeRef,
    },
    Unsupported,
}

pub fn classify_boundary_callback_position(ty: &ContractTypeRef) -> BoundaryCallbackPosition {
    match ty {
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            let ContractTypeRef::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } = interface.as_ref()
            else {
                return BoundaryCallbackPosition::Unsupported;
            };
            if !arguments.is_empty() {
                return BoundaryCallbackPosition::Unsupported;
            }
            BoundaryCallbackPosition::Exact {
                interface_type: PackageSchemaTypeRef {
                    package_id: package_id.clone(),
                    stable_schema_key: stable_schema_key.clone(),
                    package_schema_type_id: package_schema_type_id.clone(),
                },
            }
        }
        _ if contains_any_interface(ty) => BoundaryCallbackPosition::Unsupported,
        _ => BoundaryCallbackPosition::Detached,
    }
}

fn contains_any_interface(ty: &ContractTypeRef) -> bool {
    match ty {
        ContractTypeRef::AnyInterface { .. } => true,
        ContractTypeRef::Builtin { arguments, .. }
        | ContractTypeRef::StructuralUnion {
            variants: arguments,
        } => arguments.iter().any(contains_any_interface),
        ContractTypeRef::Record { fields } => fields.values().any(contains_any_interface),
        ContractTypeRef::Nullable { inner } => contains_any_interface(inner),
        ContractTypeRef::PackageSchema { .. }
        | ContractTypeRef::TypeParam { .. }
        | ContractTypeRef::Literal { .. } => false,
    }
}

fn validate_canonical_position(
    ty: &ContractTypeRef,
    plan: &BoundaryValuePlan,
    detached_owner: BoundaryValueOwner,
    detached_lifetime: BoundaryValueLifetime,
    callback_lifetime: BoundaryValueLifetime,
    location: &str,
    callback_interfaces: &mut BTreeSet<PackageSchemaTypeRef>,
) -> Result<(), BoundaryProjectionValidationError> {
    let expected = match classify_boundary_callback_position(ty) {
        BoundaryCallbackPosition::Detached => detached_plan(detached_owner, detached_lifetime),
        BoundaryCallbackPosition::Exact { interface_type } => {
            callback_interfaces.insert(interface_type);
            callback_plan(callback_lifetime)
        }
        BoundaryCallbackPosition::Unsupported => {
            return Err(BoundaryProjectionValidationError::new(format!(
                "{location} uses a nested, generic, or non-package callback interface shape"
            )));
        }
    };
    if plan == &expected {
        Ok(())
    } else {
        Err(BoundaryProjectionValidationError::new(format!(
            "{location} value plan is not canonical; expected={expected:?}, actual={plan:?}"
        )))
    }
}

fn canonical_callback_contract(
    interfaces: BTreeSet<PackageSchemaTypeRef>,
    lifetime: BoundaryValueLifetime,
) -> BoundaryCallbackContract {
    if interfaces.is_empty() {
        return BoundaryCallbackContract::None;
    }
    BoundaryCallbackContract::RequestScoped {
        interface_types: interfaces.into_iter().collect(),
        lifetime: match lifetime {
            BoundaryValueLifetime::Stream => BoundaryCallbackLifetime::Stream,
            BoundaryValueLifetime::Request => BoundaryCallbackLifetime::TopLevelRequest,
            BoundaryValueLifetime::Call => {
                unreachable!("callback capabilities never use call lifetime")
            }
        },
        expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
    }
}

fn canonical_boundary_callable_projection(
    signature: &PackageCallableSignature,
    facts: &CallableSemanticFacts,
    runtime_requirements: &PackageRuntimeRequirements,
) -> BoundaryCallableProjection {
    let mut reasons = Vec::new();
    let operation_contract = project_operation_contract(signature, &mut reasons);
    reasons.extend(semantic_unavailable_reasons(
        facts,
        operation_contract.as_ref(),
    ));
    normalize_reasons(&mut reasons);
    if !reasons.is_empty() {
        return BoundaryCallableProjection::Unavailable { reasons };
    }
    let operation_contract =
        operation_contract.expect("reason-free projection must have a complete contract");
    let CallableEffectSummary::Analyzed { effects } = facts.effects else {
        unreachable!("unknown effects always produce an unavailable reason")
    };
    BoundaryCallableProjection::Available {
        operation_contract,
        implementation_requirements: implementation_requirements(
            runtime_requirements,
            effects,
            facts.provenance.clone(),
        ),
    }
}

fn project_operation_contract(
    signature: &PackageCallableSignature,
    reasons: &mut Vec<BoundaryUnavailableReason>,
) -> Option<BoundaryOperationContract> {
    let is_server_stream = package_stream_item(&signature.return_type).is_some();
    let callback_lifetime = if is_server_stream {
        BoundaryValueLifetime::Stream
    } else {
        BoundaryValueLifetime::Request
    };
    let mut callback_interfaces = BTreeSet::new();
    let parameters = signature
        .parameters
        .iter()
        .filter_map(|parameter| {
            project_package_type(&parameter.ty)
                .and_then(|ty| {
                    let value_plan = canonical_projected_plan(
                        &ty,
                        BoundaryValueOwner::Caller,
                        BoundaryValueLifetime::Call,
                        callback_lifetime,
                        &mut callback_interfaces,
                    )?;
                    Ok(BoundaryParameter {
                        name: parameter.name.clone(),
                        ty,
                        value_plan,
                    })
                })
                .map_err(|reason| push_reason(reasons, reason))
                .ok()
        })
        .collect::<Vec<_>>();
    let return_projection = project_return(&signature.return_type)
        .map_err(|reason| push_reason(reasons, reason))
        .ok();
    if parameters.len() != signature.parameters.len() || return_projection.is_none() {
        return None;
    }
    let (mut return_value, mut stream) = return_projection.expect("return projection was checked");
    return_value.value_plan = match canonical_projected_plan(
        &return_value.ty,
        BoundaryValueOwner::Provider,
        BoundaryValueLifetime::Call,
        callback_lifetime,
        &mut callback_interfaces,
    ) {
        Ok(plan) => plan,
        Err(reason) => {
            push_reason(reasons, reason);
            return None;
        }
    };
    if let BoundaryStreamContract::ServerStream {
        item_type,
        item_value_plan,
    } = &mut stream
    {
        *item_value_plan = match canonical_projected_plan(
            item_type,
            BoundaryValueOwner::Provider,
            BoundaryValueLifetime::Stream,
            BoundaryValueLifetime::Stream,
            &mut callback_interfaces,
        ) {
            Ok(plan) => plan,
            Err(reason) => {
                push_reason(reasons, reason);
                return None;
            }
        };
    }
    Some(BoundaryOperationContract {
        parameters,
        return_value,
        stream,
        callbacks: canonical_callback_contract(callback_interfaces, callback_lifetime),
        effect_guarantee: BoundaryEffectGuarantee {
            detached_parameters: true,
            detached_return: true,
            detached_error: true,
            no_caller_reachable_mutation: true,
            no_caller_value_escape: true,
            no_same_heap_identity: true,
        },
    })
}

fn project_return(
    ty: &PackageTypeRef,
) -> Result<(BoundaryReturn, BoundaryStreamContract), BoundaryUnavailableReason> {
    let stream_item = match ty {
        PackageTypeRef::Container { name, arguments } if name == "Stream" => {
            let [item] = arguments.as_slice() else {
                return Err(BoundaryUnavailableReason::UnsupportedStream);
            };
            Some(project_package_type(item)?)
        }
        PackageTypeRef::Local {
            local_type: TypeRefIr::Builtin { name, args },
        } if name == "Stream" => {
            let [item] = args.as_slice() else {
                return Err(BoundaryUnavailableReason::UnsupportedStream);
            };
            Some(project_local_type(item)?)
        }
        _ => None,
    };
    Ok(match stream_item {
        Some(item_type) => (
            BoundaryReturn {
                ty: ContractTypeRef::builtin("void"),
                value_plan: detached_plan(
                    BoundaryValueOwner::Provider,
                    BoundaryValueLifetime::Call,
                ),
            },
            BoundaryStreamContract::ServerStream {
                item_type,
                item_value_plan: detached_plan(
                    BoundaryValueOwner::Provider,
                    BoundaryValueLifetime::Stream,
                ),
            },
        ),
        None => (
            BoundaryReturn {
                ty: project_package_type(ty)?,
                value_plan: detached_plan(
                    BoundaryValueOwner::Provider,
                    BoundaryValueLifetime::Call,
                ),
            },
            BoundaryStreamContract::Unary,
        ),
    })
}

fn package_stream_item(ty: &PackageTypeRef) -> Option<()> {
    match ty {
        PackageTypeRef::Container { name, arguments }
            if name == "Stream" && arguments.len() == 1 =>
        {
            Some(())
        }
        PackageTypeRef::Local {
            local_type: TypeRefIr::Builtin { name, args },
        } if name == "Stream" && args.len() == 1 => Some(()),
        _ => None,
    }
}

fn project_package_type(ty: &PackageTypeRef) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    match ty {
        PackageTypeRef::Local { local_type } => project_local_type(local_type),
        PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(ContractTypeRef::package_schema(
            package_id,
            stable_schema_key,
            package_schema_type_id.clone(),
        )),
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => Ok(ContractTypeRef::AnyInterface {
            interface: Box::new(project_package_type(interface)?),
            arguments: arguments
                .iter()
                .map(project_package_type)
                .collect::<Result<_, _>>()?,
        }),
        PackageTypeRef::Container { name, arguments } => {
            classify_native(name, arguments.len())?;
            Ok(ContractTypeRef::Builtin {
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(project_package_type)
                    .collect::<Result<_, _>>()?,
            })
        }
        PackageTypeRef::Nullable { inner } => Ok(ContractTypeRef::Nullable {
            inner: Box::new(project_package_type(inner)?),
        }),
    }
}

fn project_local_type(ty: &TypeRefIr) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    match ty {
        TypeRefIr::Builtin { name, args } => {
            classify_native(name, args.len())?;
            Ok(ContractTypeRef::Builtin {
                name: name.clone(),
                arguments: args
                    .iter()
                    .map(project_local_type)
                    .collect::<Result<_, _>>()?,
            })
        }
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(ContractTypeRef::package_schema(
            package_id,
            stable_schema_key,
            package_schema_type_id.clone(),
        )),
        TypeRefIr::Record { fields } => Ok(ContractTypeRef::Record {
            fields: fields
                .iter()
                .map(|(name, field)| Ok((name.clone(), project_local_type(field)?)))
                .collect::<Result<_, BoundaryUnavailableReason>>()?,
        }),
        TypeRefIr::Union { items } => Ok(ContractTypeRef::StructuralUnion {
            variants: items
                .iter()
                .map(project_local_type)
                .collect::<Result<_, _>>()?,
        }),
        TypeRefIr::Nullable { inner } => Ok(ContractTypeRef::Nullable {
            inner: Box::new(project_local_type(inner)?),
        }),
        TypeRefIr::Literal { value } => project_literal(value),
        TypeRefIr::AnyInterface { .. } | TypeRefIr::Function { .. } => {
            Err(BoundaryUnavailableReason::CallbackAdapterUnavailable)
        }
        TypeRefIr::AppliedNominal { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::TypeParam { .. } => Err(BoundaryUnavailableReason::UnsupportedBoundaryType),
    }
}

fn classify_native(name: &str, argument_count: usize) -> Result<(), BoundaryUnavailableReason> {
    match (name, argument_count) {
        (
            "string" | "integer" | "number" | "bool" | "boolean" | "null" | "void" | "Date"
            | "Duration" | "Bytes" | "bytes" | "Json" | "JsonObject",
            0,
        )
        | ("Array", 1)
        | ("Map", 2) => Ok(()),
        ("Stream", _) => Err(BoundaryUnavailableReason::UnsupportedStream),
        ("Array" | "Map", _) => Err(BoundaryUnavailableReason::UnsupportedBoundaryType),
        _ => Err(BoundaryUnavailableReason::NativeAdapterUnavailable),
    }
}

fn project_literal(value: &LiteralIr) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    match value {
        LiteralIr::Null => Ok(ContractTypeRef::builtin("null")),
        LiteralIr::String { value } => Ok(ContractTypeRef::Literal {
            value: ContractLiteral::String {
                value: value.clone(),
            },
        }),
        LiteralIr::Bool { .. } | LiteralIr::Number { .. } => {
            Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
        }
    }
}

fn detached_plan(owner: BoundaryValueOwner, lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime,
    }
}

fn callback_plan(lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::CallbackCapability,
        encoding: BoundaryValueEncoding::OpaqueCapability,
        owner: BoundaryValueOwner::CapabilityOwner,
        lifetime,
    }
}

fn canonical_projected_plan(
    ty: &ContractTypeRef,
    detached_owner: BoundaryValueOwner,
    detached_lifetime: BoundaryValueLifetime,
    callback_lifetime: BoundaryValueLifetime,
    callback_interfaces: &mut BTreeSet<PackageSchemaTypeRef>,
) -> Result<BoundaryValuePlan, BoundaryUnavailableReason> {
    match classify_boundary_callback_position(ty) {
        BoundaryCallbackPosition::Detached => Ok(detached_plan(detached_owner, detached_lifetime)),
        BoundaryCallbackPosition::Exact { interface_type } => {
            callback_interfaces.insert(interface_type);
            Ok(callback_plan(callback_lifetime))
        }
        BoundaryCallbackPosition::Unsupported => {
            Err(BoundaryUnavailableReason::CallbackAdapterUnavailable)
        }
    }
}

fn semantic_unavailable_reasons(
    facts: &CallableSemanticFacts,
    operation_contract: Option<&BoundaryOperationContract>,
) -> Vec<BoundaryUnavailableReason> {
    let mut reasons = Vec::new();
    let detached_wrapped_return =
        detached_wrapped_return_is_materialized(facts, operation_contract);
    let detached_parameters = operation_contract.is_some_and(canonical_detached_parameters);
    match facts.effects {
        CallableEffectSummary::Unknown { .. } => {
            push_reason(&mut reasons, BoundaryUnavailableReason::AnalysisPending);
            push_reason(&mut reasons, BoundaryUnavailableReason::UnknownEffect);
        }
        CallableEffectSummary::Analyzed { effects } => {
            effect_unavailable_reasons(effects, detached_wrapped_return, &mut reasons);
        }
    }
    match &facts.provenance {
        CallableProvenanceSummary::Unknown { reason } => match reason {
            crate::CallableProvenanceUnknownReason::AnalysisPending => {
                push_reason(&mut reasons, BoundaryUnavailableReason::AnalysisPending);
            }
            crate::CallableProvenanceUnknownReason::UnsupportedControlFlow
            | crate::CallableProvenanceUnknownReason::UnsupportedHeapStore => {
                push_reason(&mut reasons, BoundaryUnavailableReason::UnknownEffect);
            }
            crate::CallableProvenanceUnknownReason::UnknownCallTarget => {
                push_reason(&mut reasons, BoundaryUnavailableReason::UnknownCallTarget);
            }
        },
        CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
            ..
        } => {
            if return_origins.iter().any(is_caller_parameter_origin)
                && (analyzed_effects(facts).is_some_and(|effects| effects.returns_caller_alias)
                    || !return_origins
                        .iter()
                        .any(|origin| matches!(origin, ValueProvenance::Fresh)))
                && !detached_wrapped_return
            {
                push_reason(&mut reasons, BoundaryUnavailableReason::ReturnsCallerAlias);
            }
            if throw_origins.iter().any(is_caller_parameter_origin) {
                push_reason(&mut reasons, BoundaryUnavailableReason::ThrowsCallerAlias);
            }
            for lane in escape_lanes {
                if matches!(lane, ValueEscapeLane::Database) && detached_parameters {
                    continue;
                }
                push_reason(
                    &mut reasons,
                    BoundaryUnavailableReason::EscapesCallerValue { lane: *lane },
                );
            }
        }
    }
    if matches!(
        facts.effects,
        CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                escapes_caller_value: true,
                ..
            }
        }
    ) && !reasons
        .iter()
        .any(|reason| matches!(reason, BoundaryUnavailableReason::EscapesCallerValue { .. }))
        && !(detached_parameters && has_only_materialized_database_escape(facts))
    {
        push_reason(&mut reasons, BoundaryUnavailableReason::UnknownEffect);
    }
    if facts
        .resolved_call_targets
        .values()
        .any(|target| matches!(target, CallableTargetFact::Unknown))
    {
        push_reason(&mut reasons, BoundaryUnavailableReason::UnknownCallTarget);
    }
    reasons
}

fn effect_unavailable_reasons(
    effects: CallableMayEffects,
    detached_wrapped_return: bool,
    reasons: &mut Vec<BoundaryUnavailableReason>,
) {
    if effects.writes_caller_reachable {
        push_reason(reasons, BoundaryUnavailableReason::WritesCallerReachable);
    }
    if effects.returns_caller_alias && !detached_wrapped_return {
        push_reason(reasons, BoundaryUnavailableReason::ReturnsCallerAlias);
    }
    if effects.throws_caller_alias {
        push_reason(reasons, BoundaryUnavailableReason::ThrowsCallerAlias);
    }
    if effects.requires_same_heap_identity {
        push_reason(reasons, BoundaryUnavailableReason::RequiresSameHeapIdentity);
    }
    if effects.invokes_unknown_target {
        push_reason(reasons, BoundaryUnavailableReason::UnknownCallTarget);
    }
}

fn detached_wrapped_return_is_materialized(
    facts: &CallableSemanticFacts,
    operation_contract: Option<&BoundaryOperationContract>,
) -> bool {
    let Some(operation_contract) = operation_contract else {
        return false;
    };
    if !canonical_detached_plan(&operation_contract.return_value.value_plan) {
        return false;
    }
    let CallableEffectSummary::Analyzed { effects } = facts.effects else {
        return false;
    };
    if !effects.returns_caller_alias
        || effects.invokes_unknown_target
        || !has_no_unmaterialized_escape(facts)
    {
        return false;
    }
    matches!(
        &facts.provenance,
        CallableProvenanceSummary::Analyzed {
            return_origins,
            direct_return_origins,
            escape_lanes,
            ..
        } if escape_lanes
                .iter()
                .all(|lane| matches!(lane, ValueEscapeLane::Database))
            && return_origins
                .iter()
                .any(|origin| matches!(origin, ValueProvenance::Fresh))
            && return_origins.iter().any(is_caller_parameter_origin)
            && direct_return_origins
                .iter()
                .any(|origin| matches!(origin, ValueProvenance::Fresh))
            && direct_return_origins.iter().all(|origin| {
                matches!(origin, ValueProvenance::Fresh | ValueProvenance::Constant)
            })
    )
}

fn canonical_detached_parameters(contract: &BoundaryOperationContract) -> bool {
    contract
        .parameters
        .iter()
        .all(|parameter| canonical_detached_plan(&parameter.value_plan))
}

fn canonical_detached_plan(plan: &BoundaryValuePlan) -> bool {
    matches!(
        plan,
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            ..
        }
    )
}

fn is_caller_parameter_origin(origin: &ValueProvenance) -> bool {
    matches!(
        origin,
        ValueProvenance::CallerParameter { .. } | ValueProvenance::CallerParameterProjection { .. }
    )
}

fn has_no_unmaterialized_escape(facts: &CallableSemanticFacts) -> bool {
    matches!(
        &facts.provenance,
        CallableProvenanceSummary::Analyzed { escape_lanes, .. }
            if escape_lanes
                .iter()
                .all(|lane| matches!(lane, ValueEscapeLane::Database))
    )
}

fn analyzed_effects(facts: &CallableSemanticFacts) -> Option<CallableMayEffects> {
    match facts.effects {
        CallableEffectSummary::Analyzed { effects } => Some(effects),
        CallableEffectSummary::Unknown { .. } => None,
    }
}

fn has_only_materialized_database_escape(facts: &CallableSemanticFacts) -> bool {
    matches!(
        &facts.provenance,
        CallableProvenanceSummary::Analyzed { escape_lanes, .. }
            if !escape_lanes.is_empty()
                && escape_lanes
                .iter()
                .all(|lane| matches!(lane, ValueEscapeLane::Database))
    )
}

fn push_reason(reasons: &mut Vec<BoundaryUnavailableReason>, reason: BoundaryUnavailableReason) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

fn normalize_reasons(reasons: &mut Vec<BoundaryUnavailableReason>) {
    reasons.sort_by_key(reason_sort_key);
    reasons.dedup();
}

fn reason_sort_key(reason: &BoundaryUnavailableReason) -> (u8, u8) {
    match reason {
        BoundaryUnavailableReason::AnalysisPending => (0, 0),
        BoundaryUnavailableReason::UnknownEffect => (1, 0),
        BoundaryUnavailableReason::UnknownCallTarget => (2, 0),
        BoundaryUnavailableReason::WritesCallerReachable => (3, 0),
        BoundaryUnavailableReason::ReturnsCallerAlias => (4, 0),
        BoundaryUnavailableReason::ThrowsCallerAlias => (5, 0),
        BoundaryUnavailableReason::EscapesCallerValue { lane } => (6, escape_lane_rank(*lane)),
        BoundaryUnavailableReason::RequiresSameHeapIdentity => (7, 0),
        BoundaryUnavailableReason::CallbackAdapterUnavailable => (8, 0),
        BoundaryUnavailableReason::NativeAdapterUnavailable => (9, 0),
        BoundaryUnavailableReason::UnsupportedBoundaryType => (10, 0),
        BoundaryUnavailableReason::UnsupportedStream => (11, 0),
    }
}

const fn escape_lane_rank(lane: ValueEscapeLane) -> u8 {
    match lane {
        ValueEscapeLane::Capture => 0,
        ValueEscapeLane::Callback => 1,
        ValueEscapeLane::Stream => 2,
        ValueEscapeLane::Spawn => 3,
        ValueEscapeLane::Database => 4,
        ValueEscapeLane::Native => 5,
        ValueEscapeLane::External => 6,
    }
}

fn implementation_requirements(
    runtime: &PackageRuntimeRequirements,
    complete_may_effects: CallableMayEffects,
    provenance: CallableProvenanceSummary,
) -> BoundaryImplementationRequirements {
    let mut config = runtime
        .config
        .iter()
        .filter_map(|requirement| {
            let (value_type, required) = match &requirement.access {
                crate::PackageConfigAccess::Presence => return None,
                crate::PackageConfigAccess::Optional { value_type } => (value_type, false),
                crate::PackageConfigAccess::Required { value_type } => (value_type, true),
            };
            Some(BoundaryConfigRequirement {
                path: requirement.path.clone(),
                value_type: value_type.clone(),
                required,
            })
        })
        .collect::<Vec<_>>();
    config.sort_by(|left, right| left.path.cmp(&right.path));
    BoundaryImplementationRequirements {
        config,
        state: Vec::new(),
        native_capabilities: Vec::new(),
        complete_may_effects,
        provenance,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{
        BoundaryOperationContract, BoundaryStreamContract, BoundaryUnavailableReason,
        BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
        BoundaryValuePlan, CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary,
        CallableSemanticFacts, PackageArtifact, PackageBuildId, PackageCallableParameter,
        PackageCallableSignature, PackageImplementationLinks, PackageLocalAbi,
        PackageLocalAbiIdentity, PackageRuntimeRequirements, PackageSchemaIndexIdentity,
        PackageSchemaIndexRef, PackageSchemaTypeId, PackageTypeRef, TypeRefIr, ValueProvenance,
        PACKAGE_ARTIFACT_SCHEMA_VERSION,
    };

    use super::*;

    #[test]
    fn mutation_wrong_parameter_owner_is_rejected() {
        let signature = unary_signature();
        let facts = safe_facts();
        let runtime = empty_runtime_requirements();
        let mut projection = canonical_boundary_callable_projection(&signature, &facts, &runtime);
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = &mut projection
        else {
            unreachable!()
        };
        operation_contract.parameters[0].value_plan =
            detached_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call);

        assert!(validate_boundary_callable_projection(
            &PackageCallableId::new("pkg-callable:example.pkg:run"),
            &signature,
            &facts,
            &runtime,
            &projection,
        )
        .is_err());
    }

    #[test]
    fn standalone_unary_contract_rejects_every_noncanonical_plan_axis() {
        let canonical = available_contract(&unary_signature());
        assert!(validate_boundary_operation_contract(&canonical).is_ok());

        for mutation in 0..10 {
            let mut invalid = canonical.clone();
            match mutation {
                0 => {
                    invalid.parameters[0].value_plan = BoundaryValuePlan::Unsupported {
                        reason: crate::BoundaryValuePlanUnavailableReason::LanguageUnsupported,
                    }
                }
                1 => set_plan_carrier(
                    &mut invalid.parameters[0].value_plan,
                    BoundaryValueCarrier::CallbackCapability,
                ),
                2 => set_plan_encoding(
                    &mut invalid.parameters[0].value_plan,
                    BoundaryValueEncoding::OpaqueCapability,
                ),
                3 => set_plan_owner(
                    &mut invalid.parameters[0].value_plan,
                    BoundaryValueOwner::Provider,
                ),
                4 => set_plan_lifetime(
                    &mut invalid.parameters[0].value_plan,
                    BoundaryValueLifetime::Request,
                ),
                5 => {
                    invalid.return_value.value_plan = BoundaryValuePlan::Unsupported {
                        reason: crate::BoundaryValuePlanUnavailableReason::LanguageUnsupported,
                    }
                }
                6 => set_plan_carrier(
                    &mut invalid.return_value.value_plan,
                    BoundaryValueCarrier::CallbackCapability,
                ),
                7 => set_plan_encoding(
                    &mut invalid.return_value.value_plan,
                    BoundaryValueEncoding::OpaqueCapability,
                ),
                8 => set_plan_owner(
                    &mut invalid.return_value.value_plan,
                    BoundaryValueOwner::Caller,
                ),
                9 => set_plan_lifetime(
                    &mut invalid.return_value.value_plan,
                    BoundaryValueLifetime::Request,
                ),
                _ => unreachable!(),
            }
            assert!(
                validate_boundary_operation_contract(&invalid).is_err(),
                "unary mutation {mutation} must be rejected"
            );
        }
    }

    #[test]
    fn standalone_server_stream_contract_rejects_sentinel_and_item_mutations() {
        let mut signature = unary_signature();
        signature.return_type = PackageTypeRef::Container {
            name: "Stream".to_string(),
            arguments: vec![PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            }],
        };
        let canonical = available_contract(&signature);
        assert!(validate_boundary_operation_contract(&canonical).is_ok());

        for mutation in 0..7 {
            let mut invalid = canonical.clone();
            match mutation {
                0 => invalid.return_value.ty = ContractTypeRef::builtin("string"),
                1 => {
                    let BoundaryStreamContract::ServerStream {
                        item_value_plan, ..
                    } = &mut invalid.stream
                    else {
                        unreachable!()
                    };
                    *item_value_plan = BoundaryValuePlan::Unsupported {
                        reason: crate::BoundaryValuePlanUnavailableReason::LanguageUnsupported,
                    };
                }
                2 => mutate_stream_item_plan(&mut invalid, |plan| {
                    set_plan_carrier(plan, BoundaryValueCarrier::CallbackCapability)
                }),
                3 => mutate_stream_item_plan(&mut invalid, |plan| {
                    set_plan_encoding(plan, BoundaryValueEncoding::OpaqueCapability)
                }),
                4 => mutate_stream_item_plan(&mut invalid, |plan| {
                    set_plan_owner(plan, BoundaryValueOwner::Caller)
                }),
                5 => mutate_stream_item_plan(&mut invalid, |plan| {
                    set_plan_lifetime(plan, BoundaryValueLifetime::Call)
                }),
                6 => {
                    invalid.stream = BoundaryStreamContract::Unsupported {
                        reason: crate::BoundaryFeatureUnavailableReason::LanguageUnsupported,
                    }
                }
                _ => unreachable!(),
            }
            assert!(
                validate_boundary_operation_contract(&invalid).is_err(),
                "server-stream mutation {mutation} must be rejected"
            );
        }
    }

    #[test]
    fn unary_signature_and_all_value_plan_axes_are_validated() {
        let signature = unary_signature();
        let facts = safe_facts();
        let runtime = empty_runtime_requirements();
        let canonical = canonical_boundary_callable_projection(&signature, &facts, &runtime);
        let callable_id = PackageCallableId::new("pkg-callable:example.pkg:run");
        assert!(validate_boundary_callable_projection(
            &callable_id,
            &signature,
            &facts,
            &runtime,
            &canonical
        )
        .is_ok());

        for mutation in 0..8 {
            let mut invalid = canonical.clone();
            let BoundaryCallableProjection::Available {
                operation_contract, ..
            } = &mut invalid
            else {
                unreachable!()
            };
            match mutation {
                0 => operation_contract.parameters[0].name = "renamed".to_string(),
                1 => operation_contract.parameters[0].ty = ContractTypeRef::builtin("integer"),
                2 => {
                    operation_contract.parameters[0].value_plan = BoundaryValuePlan::Unsupported {
                        reason: crate::BoundaryValuePlanUnavailableReason::LanguageUnsupported,
                    }
                }
                3 => set_plan_carrier(
                    &mut operation_contract.parameters[0].value_plan,
                    BoundaryValueCarrier::CallbackCapability,
                ),
                4 => set_plan_encoding(
                    &mut operation_contract.parameters[0].value_plan,
                    BoundaryValueEncoding::OpaqueCapability,
                ),
                5 => set_plan_lifetime(
                    &mut operation_contract.parameters[0].value_plan,
                    BoundaryValueLifetime::Request,
                ),
                6 => {
                    operation_contract.return_value.ty = ContractTypeRef::package_schema(
                        "wrong.owner",
                        "Result",
                        PackageSchemaTypeId::new("type:result"),
                    )
                }
                7 => {
                    operation_contract.stream = BoundaryStreamContract::Unsupported {
                        reason: crate::BoundaryFeatureUnavailableReason::LanguageUnsupported,
                    }
                }
                _ => unreachable!(),
            }
            assert!(
                validate_boundary_callable_projection(
                    &callable_id,
                    &signature,
                    &facts,
                    &runtime,
                    &invalid,
                )
                .is_err(),
                "mutation {mutation} must be rejected"
            );
        }
    }

    #[test]
    fn server_stream_is_derived_only_from_exact_stream_signature() {
        let mut signature = unary_signature();
        signature.return_type = PackageTypeRef::Container {
            name: "Stream".to_string(),
            arguments: vec![PackageTypeRef::PackageSchema {
                package_id: "example.pkg".to_string(),
                stable_schema_key: "Result".to_string(),
                package_schema_type_id: PackageSchemaTypeId::new("type:result"),
            }],
        };
        let facts = safe_facts();
        let runtime = empty_runtime_requirements();
        let canonical = canonical_boundary_callable_projection(&signature, &facts, &runtime);
        let callable_id = PackageCallableId::new("pkg-callable:example.pkg:watch");
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = &canonical
        else {
            panic!("exact Stream<T> must be available")
        };
        assert_eq!(
            operation_contract.return_value.ty,
            ContractTypeRef::builtin("void")
        );
        let BoundaryStreamContract::ServerStream {
            item_type,
            item_value_plan,
        } = &operation_contract.stream
        else {
            panic!("exact Stream<T> must derive server stream")
        };
        assert_eq!(
            item_type,
            &ContractTypeRef::package_schema(
                "example.pkg",
                "Result",
                PackageSchemaTypeId::new("type:result")
            )
        );
        assert_eq!(
            item_value_plan,
            &detached_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Stream)
        );

        for mutation in 0..7 {
            let mut invalid = canonical.clone();
            let BoundaryCallableProjection::Available {
                operation_contract, ..
            } = &mut invalid
            else {
                unreachable!()
            };
            match mutation {
                0 => operation_contract.return_value.ty = ContractTypeRef::builtin("string"),
                1 => set_plan_owner(
                    &mut operation_contract.return_value.value_plan,
                    BoundaryValueOwner::Caller,
                ),
                2 => set_plan_lifetime(
                    &mut operation_contract.return_value.value_plan,
                    BoundaryValueLifetime::Stream,
                ),
                3 => operation_contract.stream = BoundaryStreamContract::Unary,
                4 => {
                    let BoundaryStreamContract::ServerStream { item_type, .. } =
                        &mut operation_contract.stream
                    else {
                        unreachable!()
                    };
                    *item_type = ContractTypeRef::builtin("string");
                }
                5 => {
                    let BoundaryStreamContract::ServerStream {
                        item_value_plan, ..
                    } = &mut operation_contract.stream
                    else {
                        unreachable!()
                    };
                    set_plan_owner(item_value_plan, BoundaryValueOwner::Caller);
                }
                6 => {
                    let BoundaryStreamContract::ServerStream {
                        item_value_plan, ..
                    } = &mut operation_contract.stream
                    else {
                        unreachable!()
                    };
                    set_plan_lifetime(item_value_plan, BoundaryValueLifetime::Call);
                }
                _ => unreachable!(),
            }
            assert!(
                validate_boundary_callable_projection(
                    &callable_id,
                    &signature,
                    &facts,
                    &runtime,
                    &invalid,
                )
                .is_err(),
                "stream mutation {mutation} must be rejected"
            );
        }

        signature.return_type = PackageTypeRef::Container {
            name: "Stream".to_string(),
            arguments: Vec::new(),
        };
        assert_eq!(
            canonical_boundary_callable_projection(&signature, &facts, &runtime),
            BoundaryCallableProjection::Unavailable {
                reasons: vec![BoundaryUnavailableReason::UnsupportedStream]
            }
        );
    }

    #[test]
    fn exact_non_generic_any_interface_is_the_only_callback_position() {
        let interface = PackageSchemaTypeRef {
            package_id: "example.pkg".to_string(),
            stable_schema_key: "api.Reader".to_string(),
            package_schema_type_id: PackageSchemaTypeId::new("type:reader"),
        };
        let interface_type = ContractTypeRef::package_schema(
            interface.package_id.clone(),
            interface.stable_schema_key.clone(),
            interface.package_schema_type_id.clone(),
        );
        let exact = ContractTypeRef::AnyInterface {
            interface: Box::new(interface_type.clone()),
            arguments: Vec::new(),
        };
        assert_eq!(
            classify_boundary_callback_position(&exact),
            BoundaryCallbackPosition::Exact {
                interface_type: interface
            }
        );
        assert_eq!(
            classify_boundary_callback_position(&interface_type),
            BoundaryCallbackPosition::Detached,
            "a direct PackageSchema is data, not an implicit callback"
        );
        assert_eq!(
            classify_boundary_callback_position(&ContractTypeRef::Builtin {
                name: "Array".to_string(),
                arguments: vec![exact.clone()],
            }),
            BoundaryCallbackPosition::Unsupported
        );
        assert_eq!(
            classify_boundary_callback_position(&ContractTypeRef::AnyInterface {
                interface: Box::new(interface_type),
                arguments: vec![ContractTypeRef::builtin("string")],
            }),
            BoundaryCallbackPosition::Unsupported
        );
    }

    #[test]
    fn unary_any_interface_rederives_request_scoped_callback_contract_exactly() {
        let signature = callback_signature(PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        });
        let projection = canonical_boundary_callable_projection(
            &signature,
            &safe_facts(),
            &empty_runtime_requirements(),
        );
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = projection
        else {
            panic!("exact non-generic any I parameter must be boundary available")
        };
        assert_eq!(
            operation_contract.parameters[0].value_plan,
            callback_plan(BoundaryValueLifetime::Request)
        );
        assert_eq!(
            operation_contract.callbacks,
            BoundaryCallbackContract::RequestScoped {
                interface_types: vec![callback_interface_ref()],
                lifetime: BoundaryCallbackLifetime::TopLevelRequest,
                expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
            }
        );
        assert!(validate_boundary_operation_contract(&operation_contract).is_ok());

        for mutation in 0..6 {
            let mut invalid = operation_contract.clone();
            match mutation {
                0 => set_plan_carrier(
                    &mut invalid.parameters[0].value_plan,
                    BoundaryValueCarrier::DetachedValueGraph,
                ),
                1 => set_plan_encoding(
                    &mut invalid.parameters[0].value_plan,
                    BoundaryValueEncoding::CanonicalValue,
                ),
                2 => set_plan_owner(
                    &mut invalid.parameters[0].value_plan,
                    BoundaryValueOwner::Caller,
                ),
                3 => set_plan_lifetime(
                    &mut invalid.parameters[0].value_plan,
                    BoundaryValueLifetime::Call,
                ),
                4 => invalid.callbacks = BoundaryCallbackContract::None,
                5 => {
                    let BoundaryCallbackContract::RequestScoped {
                        expiration_error, ..
                    } = &mut invalid.callbacks
                    else {
                        unreachable!()
                    };
                    *expiration_error = BoundaryCallbackExpirationError::CapabilityUnavailable;
                }
                _ => unreachable!(),
            }
            assert!(
                validate_boundary_operation_contract(&invalid).is_err(),
                "callback contract mutation {mutation} must fail exact validation"
            );
        }
    }

    #[test]
    fn server_stream_extends_every_exact_callback_position_to_stream_lifetime() {
        let callback = callback_package_type();
        let signature = PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![PackageCallableParameter {
                name: "callback".to_string(),
                ty: callback.clone(),
            }],
            return_type: PackageTypeRef::Container {
                name: "Stream".to_string(),
                arguments: vec![callback],
            },
            may_suspend: true,
        };
        let projection = canonical_boundary_callable_projection(
            &signature,
            &safe_facts(),
            &empty_runtime_requirements(),
        );
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = projection
        else {
            panic!("exact any I stream positions must be boundary available")
        };
        assert_eq!(
            operation_contract.parameters[0].value_plan,
            callback_plan(BoundaryValueLifetime::Stream)
        );
        let BoundaryStreamContract::ServerStream {
            item_value_plan, ..
        } = &operation_contract.stream
        else {
            panic!("fixture must project a server stream")
        };
        assert_eq!(
            item_value_plan,
            &callback_plan(BoundaryValueLifetime::Stream)
        );
        assert_eq!(
            operation_contract.callbacks,
            BoundaryCallbackContract::RequestScoped {
                interface_types: vec![callback_interface_ref()],
                lifetime: BoundaryCallbackLifetime::Stream,
                expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
            }
        );
        assert!(validate_boundary_operation_contract(&operation_contract).is_ok());
    }

    #[test]
    fn nested_and_generic_any_interface_positions_remain_unavailable() {
        let callback = callback_package_type();
        for parameter_type in [
            PackageTypeRef::Container {
                name: "Array".to_string(),
                arguments: vec![callback.clone()],
            },
            PackageTypeRef::AnyInterface {
                interface: Box::new(PackageTypeRef::PackageSchema {
                    package_id: "example.pkg".to_string(),
                    stable_schema_key: "api.Reader".to_string(),
                    package_schema_type_id: PackageSchemaTypeId::new("type:reader"),
                }),
                arguments: vec![PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("string"),
                }],
            },
        ] {
            let mut signature = unary_signature();
            signature.parameters[0].ty = parameter_type;
            assert_eq!(
                canonical_boundary_callable_projection(
                    &signature,
                    &safe_facts(),
                    &empty_runtime_requirements(),
                ),
                BoundaryCallableProjection::Unavailable {
                    reasons: vec![BoundaryUnavailableReason::CallbackAdapterUnavailable]
                }
            );
        }
    }

    #[test]
    fn unavailable_reasons_are_nonempty_exact_and_canonical() {
        let mut signature = unary_signature();
        signature.parameters[0].ty = PackageTypeRef::Local {
            local_type: TypeRefIr::LocalType { type_index: 7 },
        };
        let mut facts = safe_facts();
        let CallableEffectSummary::Analyzed { effects } = &mut facts.effects else {
            unreachable!()
        };
        effects.writes_caller_reachable = true;
        effects.invokes_unknown_target = true;
        let runtime = empty_runtime_requirements();
        let canonical = canonical_boundary_callable_projection(&signature, &facts, &runtime);
        assert_eq!(
            canonical,
            BoundaryCallableProjection::Unavailable {
                reasons: vec![
                    BoundaryUnavailableReason::UnknownCallTarget,
                    BoundaryUnavailableReason::WritesCallerReachable,
                    BoundaryUnavailableReason::UnsupportedBoundaryType,
                ]
            }
        );
        let callable_id = PackageCallableId::new("pkg-callable:example.pkg:private");
        for invalid in [
            BoundaryCallableProjection::Unavailable {
                reasons: Vec::new(),
            },
            BoundaryCallableProjection::Unavailable {
                reasons: vec![
                    BoundaryUnavailableReason::UnsupportedBoundaryType,
                    BoundaryUnavailableReason::WritesCallerReachable,
                    BoundaryUnavailableReason::UnknownCallTarget,
                ],
            },
            BoundaryCallableProjection::Unavailable {
                reasons: vec![
                    BoundaryUnavailableReason::UnknownCallTarget,
                    BoundaryUnavailableReason::WritesCallerReachable,
                ],
            },
            BoundaryCallableProjection::Unavailable {
                reasons: vec![
                    BoundaryUnavailableReason::UnknownCallTarget,
                    BoundaryUnavailableReason::UnknownCallTarget,
                    BoundaryUnavailableReason::WritesCallerReachable,
                    BoundaryUnavailableReason::UnsupportedBoundaryType,
                ],
            },
        ] {
            assert!(validate_boundary_callable_projection(
                &callable_id,
                &signature,
                &facts,
                &runtime,
                &invalid,
            )
            .is_err());
        }
    }

    #[test]
    fn unavailable_projection_accepts_only_canonical_type_closure_saturation() {
        let mut signature = unary_signature();
        signature.parameters[0].ty = PackageTypeRef::Local {
            local_type: TypeRefIr::LocalType { type_index: 7 },
        };
        let facts = safe_facts();
        let runtime = empty_runtime_requirements();
        let callable_id = PackageCallableId::new("pkg-callable:example.pkg:private");

        let saturated = BoundaryCallableProjection::Unavailable {
            reasons: vec![
                BoundaryUnavailableReason::UnsupportedBoundaryType,
                BoundaryUnavailableReason::UnsupportedStream,
            ],
        };
        assert!(validate_boundary_callable_projection(
            &callable_id,
            &signature,
            &facts,
            &runtime,
            &saturated,
        )
        .is_ok());

        for invalid in [
            BoundaryCallableProjection::Unavailable {
                reasons: vec![
                    BoundaryUnavailableReason::UnsupportedStream,
                    BoundaryUnavailableReason::UnsupportedBoundaryType,
                ],
            },
            BoundaryCallableProjection::Unavailable {
                reasons: vec![
                    BoundaryUnavailableReason::UnknownEffect,
                    BoundaryUnavailableReason::UnsupportedBoundaryType,
                ],
            },
        ] {
            assert!(validate_boundary_callable_projection(
                &callable_id,
                &signature,
                &facts,
                &runtime,
                &invalid,
            )
            .is_err());
        }
    }

    #[test]
    fn implementation_requirements_must_match_complete_facts_and_runtime_requirements() {
        let signature = unary_signature();
        let facts = safe_facts();
        let runtime = empty_runtime_requirements();
        let canonical = canonical_boundary_callable_projection(&signature, &facts, &runtime);
        let callable_id = PackageCallableId::new("pkg-callable:example.pkg:run");
        for mutation in 0..2 {
            let mut invalid = canonical.clone();
            let BoundaryCallableProjection::Available {
                implementation_requirements,
                ..
            } = &mut invalid
            else {
                unreachable!()
            };
            match mutation {
                0 => implementation_requirements.complete_may_effects.may_suspend = true,
                1 => {
                    implementation_requirements.provenance = CallableProvenanceSummary::Unknown {
                        reason: crate::CallableProvenanceUnknownReason::AnalysisPending,
                    }
                }
                _ => unreachable!(),
            }
            assert!(validate_boundary_callable_projection(
                &callable_id,
                &signature,
                &facts,
                &runtime,
                &invalid,
            )
            .is_err());
        }
    }

    #[test]
    fn package_validator_requires_exact_public_callable_coverage() {
        let signature = unary_signature();
        let facts = safe_facts();
        let runtime = empty_runtime_requirements();
        let callable_id = PackageCallableId::new("pkg-callable:example.pkg:run");
        let projection = canonical_boundary_callable_projection(&signature, &facts, &runtime);
        let mut artifact = PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: "example.pkg".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("build"),
            files: Vec::new(),
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("abi"),
                public_symbols: BTreeMap::from([(
                    "run".to_string(),
                    PackageLocalAbiSymbol::Callable {
                        callable_id: callable_id.clone(),
                        signature,
                    },
                )]),
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: "example.pkg".to_string(),
                package_schema_index_identity: PackageSchemaIndexIdentity::new("index"),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: PackageImplementationLinks::default(),
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: runtime,
            callable_semantic_facts: BTreeMap::from([(callable_id.clone(), facts)]),
            boundary_projections: BTreeMap::from([(callable_id.clone(), projection)]),
            service_call_refs: Vec::new(),
        };
        assert!(validate_package_boundary_projections(&artifact).is_ok());
        artifact.boundary_projections.clear();
        assert!(validate_package_boundary_projections(&artifact).is_err());
    }

    fn unary_signature() -> PackageCallableSignature {
        PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![PackageCallableParameter {
                name: "input".to_string(),
                ty: PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("string"),
                },
            }],
            return_type: PackageTypeRef::PackageSchema {
                package_id: "example.pkg".to_string(),
                stable_schema_key: "Result".to_string(),
                package_schema_type_id: PackageSchemaTypeId::new("type:result"),
            },
            may_suspend: false,
        }
    }

    fn callback_signature(return_type: PackageTypeRef) -> PackageCallableSignature {
        PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![PackageCallableParameter {
                name: "callback".to_string(),
                ty: callback_package_type(),
            }],
            return_type,
            may_suspend: true,
        }
    }

    fn callback_package_type() -> PackageTypeRef {
        PackageTypeRef::AnyInterface {
            interface: Box::new(PackageTypeRef::PackageSchema {
                package_id: "example.pkg".to_string(),
                stable_schema_key: "api.Reader".to_string(),
                package_schema_type_id: PackageSchemaTypeId::new("type:reader"),
            }),
            arguments: Vec::new(),
        }
    }

    fn callback_interface_ref() -> PackageSchemaTypeRef {
        PackageSchemaTypeRef {
            package_id: "example.pkg".to_string(),
            stable_schema_key: "api.Reader".to_string(),
            package_schema_type_id: PackageSchemaTypeId::new("type:reader"),
        }
    }

    fn safe_facts() -> CallableSemanticFacts {
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed {
                effects: CallableMayEffects {
                    writes_caller_reachable: false,
                    returns_caller_alias: false,
                    throws_caller_alias: false,
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_suspend: false,
                },
            },
            provenance: CallableProvenanceSummary::Analyzed {
                return_origins: vec![ValueProvenance::Fresh],
                direct_return_origins: vec![ValueProvenance::Fresh],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            },
            resolved_call_targets: BTreeMap::new(),
        }
    }

    fn empty_runtime_requirements() -> PackageRuntimeRequirements {
        PackageRuntimeRequirements { config: Vec::new() }
    }

    fn available_contract(signature: &PackageCallableSignature) -> BoundaryOperationContract {
        let projection = canonical_boundary_callable_projection(
            signature,
            &safe_facts(),
            &empty_runtime_requirements(),
        );
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = projection
        else {
            panic!("fixture signature must be available")
        };
        operation_contract
    }

    fn mutate_stream_item_plan(
        contract: &mut BoundaryOperationContract,
        mutation: impl FnOnce(&mut BoundaryValuePlan),
    ) {
        let BoundaryStreamContract::ServerStream {
            item_value_plan, ..
        } = &mut contract.stream
        else {
            unreachable!()
        };
        mutation(item_value_plan);
    }

    fn detached_plan(
        owner: BoundaryValueOwner,
        lifetime: BoundaryValueLifetime,
    ) -> BoundaryValuePlan {
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            owner,
            lifetime,
        }
    }

    fn set_plan_carrier(plan: &mut BoundaryValuePlan, carrier: BoundaryValueCarrier) {
        let BoundaryValuePlan::Linkable {
            carrier: actual, ..
        } = plan
        else {
            unreachable!()
        };
        *actual = carrier;
    }

    fn set_plan_encoding(plan: &mut BoundaryValuePlan, encoding: BoundaryValueEncoding) {
        let BoundaryValuePlan::Linkable {
            encoding: actual, ..
        } = plan
        else {
            unreachable!()
        };
        *actual = encoding;
    }

    fn set_plan_owner(plan: &mut BoundaryValuePlan, owner: BoundaryValueOwner) {
        let BoundaryValuePlan::Linkable { owner: actual, .. } = plan else {
            unreachable!()
        };
        *actual = owner;
    }

    fn set_plan_lifetime(plan: &mut BoundaryValuePlan, lifetime: BoundaryValueLifetime) {
        let BoundaryValuePlan::Linkable {
            lifetime: actual, ..
        } = plan
        else {
            unreachable!()
        };
        *actual = lifetime;
    }
}
