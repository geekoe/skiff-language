use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    AssignTargetIr, BinaryOpIr, CallTargetIr, CallableEffectSummary, ExprIr, ExprRefIr, LiteralIr,
    NamedUnionBranchIr, NativeTarget, ServiceBoundaryPlan, ServiceCallRef, StatementAttributionId,
    TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_lowering::mir::{
    MirCallArgument, MirEmissionAnchor, MirExecutableKind, MirFunction, MirParamMode, MirSlotKind,
    MirStmtKind, MirUnit, MirWritableRoot,
};

use super::{
    carriers::{
        analyze_machine_carriers, may_share_scalar_machine_carrier, PackageMachineCarrierFacts,
    },
    inputs::canonical_function_key,
    BytecodeEmissionError, Phase1MirFactMismatch, Phase1UnsupportedCapability,
};

mod gateway_parameter;
mod host_effects;
mod package_type_authority;
mod representation_carrier;
mod server_stream;

pub(crate) use gateway_parameter::DenseParameterMaterializationFact;
use host_effects::{HostEffectAdmissions, RegistryValueAuthority};
pub(crate) use representation_carrier::RepresentationCarrierFact;
use server_stream::ServerStreamAdmissions;

pub use gateway_parameter::GatewayParameterAuthority;
pub use server_stream::{ServerStreamEmitFact, ServerStreamGatewayAuthority};

const CANONICAL_DURATION_MILLISECONDS_BINDING_KEY: &str = "core.duration.milliseconds";

/// Opaque proof that one exact MIR slice passed the Phase 1 bytecode boundary.
///
/// The proof cannot be constructed. Public planning and emission entry points
/// therefore accept only source facts checked by
/// [`admit_phase_1_bytecode_mir`]. The one public MIR view is read-only and is
/// scoped to source value-transfer fact projection, so downstream planning
/// consumes this normalized carrier instead of the pre-admission input.
#[derive(Debug)]
pub struct AdmittedPhase1BytecodeMir {
    units: Vec<MirUnit>,
    dense_parameter_materializations: BTreeMap<String, DenseParameterMaterializationFact>,
    machine_carriers: PackageMachineCarrierFacts,
    representation_carriers: Vec<RepresentationCarrierFact>,
    service_boundary_plans: BTreeMap<ServiceCallRef, ServiceBoundaryPlan>,
}

impl AdmittedPhase1BytecodeMir {
    pub(crate) fn units(&self) -> &[MirUnit] {
        &self.units
    }

    pub(crate) fn dense_parameter_materializations(
        &self,
    ) -> &BTreeMap<String, DenseParameterMaterializationFact> {
        &self.dense_parameter_materializations
    }

    pub(crate) fn machine_carriers(&self) -> &PackageMachineCarrierFacts {
        &self.machine_carriers
    }

    pub(crate) fn representation_carriers(&self) -> &[RepresentationCarrierFact] {
        &self.representation_carriers
    }

    pub(crate) fn service_boundary_plans(&self) -> &BTreeMap<ServiceCallRef, ServiceBoundaryPlan> {
        &self.service_boundary_plans
    }

    /// Returns the normalized, admitted MIR used to project source-owned
    /// value-transfer facts.
    pub fn source_value_transfer_units(&self) -> &[MirUnit] {
        &self.units
    }
}

/// Admits the Phase 2 record/array MIR surface plus the retained Phase 1
/// scalar/local-call core, and the Phase 3 synchronous throw/catch/rethrow
/// surface (nominal/union payloads over the Phase 2 value face).
///
/// This is the production bytecode lane's source-owned capability boundary.
/// It runs before constant evaluation, value-transfer derivation, or bytecode
/// emission and returns no partially emitted state. The admission reads only
/// typed MIR facts; package names and binding strings never grant capability.
/// The exact supported value shapes are `record` and `array` recursively over
/// `number`/`boolean`/`null` and nested record/array; `string`, `bytes`,
/// `map`, representations, streams, host targets, tail calls, generics and
/// `InOut` remain rejected at this single boundary. Synchronous `throw`,
/// `catch` and `rethrow` are admitted when their payload types stay on the
/// Phase 2 record/array/scalar face (directly or as union leaves); host
/// effect, Pending, child and stream throw producers stay fail closed
/// through the existing target/effect rejections.
///
/// Phase 3 Amendment 1 admits the minimal compile-time string-literal slice:
/// a string literal is accepted only as a union/`CatchResult` discriminator
/// constant (`.tag` reads, `tag == "…"` equality and their narrowed types).
/// General string values (bindings, fields, aggregates, boundary payloads,
/// concatenation) remain rejected.
///
/// Phase 4 gate 1 admits exactly one host effect: the canonical
/// `std.time.sleep` binding with its pinned arity (one `Duration` argument),
/// pinned parameter type (`skiff.run/std::std.time.Duration`) and pinned
/// `void` result. Every other host binding, every other pending category and
/// any drifted/missing fact stay fail closed at this single boundary.
pub fn admit_phase_1_bytecode_mir(
    units: &[MirUnit],
) -> Result<AdmittedPhase1BytecodeMir, BytecodeEmissionError> {
    admit_phase_1_bytecode_mir_with_server_stream_authorities_and_service_boundary_plans(
        units,
        &[],
        &BTreeMap::new(),
    )
}

pub fn admit_phase_1_bytecode_mir_with_server_stream_authorities(
    units: &[MirUnit],
    server_stream_authorities: &[ServerStreamGatewayAuthority],
) -> Result<AdmittedPhase1BytecodeMir, BytecodeEmissionError> {
    admit_phase_1_bytecode_mir_with_server_stream_authorities_and_service_boundary_plans(
        units,
        server_stream_authorities,
        &BTreeMap::new(),
    )
}

pub fn admit_phase_1_bytecode_mir_with_server_stream_authorities_and_service_boundary_plans(
    units: &[MirUnit],
    server_stream_authorities: &[ServerStreamGatewayAuthority],
    service_boundary_plans: &BTreeMap<ServiceCallRef, ServiceBoundaryPlan>,
) -> Result<AdmittedPhase1BytecodeMir, BytecodeEmissionError> {
    let gateway_parameter_authorities = server_stream_authorities
        .iter()
        .map(|authority| GatewayParameterAuthority::new(authority.entry().clone()))
        .collect::<Vec<_>>();
    admit_phase_1_bytecode_mir_with_gateway_authorities_and_service_boundary_plans(
        units,
        &gateway_parameter_authorities,
        server_stream_authorities,
        service_boundary_plans,
    )
}

pub fn admit_phase_1_bytecode_mir_with_gateway_authorities(
    units: &[MirUnit],
    gateway_parameter_authorities: &[GatewayParameterAuthority],
    server_stream_authorities: &[ServerStreamGatewayAuthority],
) -> Result<AdmittedPhase1BytecodeMir, BytecodeEmissionError> {
    admit_phase_1_bytecode_mir_with_gateway_authorities_and_service_boundary_plans(
        units,
        gateway_parameter_authorities,
        server_stream_authorities,
        &BTreeMap::new(),
    )
}

pub fn admit_phase_1_bytecode_mir_with_gateway_authorities_and_service_boundary_plans(
    units: &[MirUnit],
    gateway_parameter_authorities: &[GatewayParameterAuthority],
    server_stream_authorities: &[ServerStreamGatewayAuthority],
    service_boundary_plans: &BTreeMap<ServiceCallRef, ServiceBoundaryPlan>,
) -> Result<AdmittedPhase1BytecodeMir, BytecodeEmissionError> {
    let units =
        package_type_authority::normalize_package_type_authorities(units).map_err(|error| {
            rejected(
                &units[error.unit_index],
                None,
                Phase1UnsupportedCapability::ValueShape,
                &format!("package type authority: {}", error.detail),
            )
        })?;
    let dense_parameter_materializations =
        gateway_parameter::analyze(&units, gateway_parameter_authorities).map_err(|detail| {
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::ValueShape,
                module_path: units
                    .first()
                    .map_or_else(String::new, |unit| unit.module_path.clone()),
                function_key: None,
                location: format!("rawHttp gateway parameter authority: {detail}"),
            }
        })?;
    server_stream::validate_authority_coverage(&units, server_stream_authorities).map_err(
        |detail| BytecodeEmissionError::UnsupportedPhase1Capability {
            capability: Phase1UnsupportedCapability::Stream,
            module_path: units
                .first()
                .map_or_else(String::new, |unit| unit.module_path.clone()),
            function_key: None,
            location: format!("server-stream gateway authority: {detail}"),
        },
    )?;
    validate_service_boundary_plan_coverage(&units, service_boundary_plans)?;
    for unit in &units {
        unit.validate_executable_indices()?;
        if !unit.actor_declarations.is_empty() {
            return Err(rejected(
                unit,
                None,
                Phase1UnsupportedCapability::Actor,
                "actor declaration table",
            ));
        }
        if !unit.constants.is_empty() {
            return Err(rejected(
                unit,
                None,
                Phase1UnsupportedCapability::Constant,
                "compile-time constant table",
            ));
        }
        for declaration in &unit.type_table {
            if !declaration.type_params.is_empty() {
                return Err(rejected(
                    unit,
                    None,
                    Phase1UnsupportedCapability::Generic,
                    &format!("type declaration `{}`", declaration.name),
                ));
            }
            if !declaration.implements.is_empty() {
                return Err(rejected(
                    unit,
                    None,
                    Phase1UnsupportedCapability::Interface,
                    &format!(
                        "type declaration `{}` interface conformance",
                        declaration.name
                    ),
                ));
            }
            if !matches!(declaration.descriptor, TypeDescriptorIr::Record { .. }) {
                let capability = if matches!(declaration.descriptor, TypeDescriptorIr::Interface) {
                    Phase1UnsupportedCapability::Interface
                } else {
                    Phase1UnsupportedCapability::ValueShape
                };
                return Err(rejected(
                    unit,
                    None,
                    capability,
                    &format!("type declaration `{}`", declaration.name),
                ));
            }
        }
        for function in &unit.functions {
            let function_key = canonical_function_key(&unit.module_path, &function.symbol)?;
            admit_function(
                &units,
                unit,
                &function_key,
                function,
                &dense_parameter_materializations,
                server_stream_authorities,
            )?;
        }
    }
    let machine_carriers = analyze_machine_carriers(&units)?;
    let representation_carriers = representation_carrier::analyze(&units, &machine_carriers)?;
    Ok(AdmittedPhase1BytecodeMir {
        units,
        dense_parameter_materializations,
        machine_carriers,
        representation_carriers,
        service_boundary_plans: service_boundary_plans.clone(),
    })
}

fn validate_service_boundary_plan_coverage(
    units: &[MirUnit],
    service_boundary_plans: &BTreeMap<ServiceCallRef, ServiceBoundaryPlan>,
) -> Result<(), BytecodeEmissionError> {
    let mut required = BTreeMap::<ServiceCallRef, ()>::new();
    for unit in units {
        for service_call in &unit.external_refs.service_call_refs {
            required.insert(service_call.clone(), ());
        }
    }
    for service_call in required.keys() {
        let plan = service_boundary_plans.get(service_call).ok_or_else(|| {
            BytecodeEmissionError::MissingServiceBoundaryPlan {
                service_call: format!("{service_call:?}"),
            }
        })?;
        if plan.stream_item.is_some()
            || !matches!(
                plan.callbacks,
                skiff_artifact_model::ServiceCallbackPlan::None
            )
        {
            return Err(BytecodeEmissionError::UnsupportedServiceBoundaryPlan {
                location: format!("service call {service_call:?}"),
                detail: "stream item and callback surfaces are disabled in the first service lane"
                    .to_string(),
            });
        }
    }
    for service_call in service_boundary_plans.keys() {
        if !required.contains_key(service_call) {
            return Err(BytecodeEmissionError::UnexpectedServiceBoundaryPlan {
                service_call: format!("{service_call:?}"),
            });
        }
    }
    Ok(())
}

fn admit_function(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    dense_parameter_materializations: &BTreeMap<String, DenseParameterMaterializationFact>,
    server_stream_authorities: &[ServerStreamGatewayAuthority],
) -> Result<(), BytecodeEmissionError> {
    function.validate_expression_indices()?;
    function.validate_slot_types()?;
    if function.kind != MirExecutableKind::Function {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Receiver,
            "executable kind",
        ));
    }
    if function.native {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            "native executable",
        ));
    }
    if !function.type_params.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Generic,
            "function type parameters",
        ));
    }
    if function.self_type.is_some() || function.receiver.is_some() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Receiver,
            "receiver facts",
        ));
    }
    let host_effects =
        HostEffectAdmissions::analyze(function, CANONICAL_DURATION_MILLISECONDS_BINDING_KEY)
            .map_err(|error| {
                rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::HostTarget,
                    &format!(
                        "expression {} exact host-effect admission: {}",
                        error.expression_index, error.detail
                    ),
                )
            })?;
    let server_stream = ServerStreamAdmissions::analyze(unit, function, server_stream_authorities)
        .map_err(|detail| {
            rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Stream,
                &format!("exact server-stream admission: {detail}"),
            )
        })?;
    let dense_parameter_materialization = dense_parameter_materializations.get(function_key);
    if let Some(stream) = &function.stream_result {
        let TypeRefIr::Builtin { name, args } = &function.return_type else {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Stream,
                "stream result authority without Stream<T> return type",
            ));
        };
        if name != "Stream" || args.as_slice() != [stream.item_type.clone()] {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Stream,
                "stream result authority differs from Stream<T> return type",
            ));
        }
        if !server_stream.admits_result(&stream.item_type) {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Stream,
                "server-stream result lacks exact canonical gateway authority",
            ));
        }
    } else if !server_stream.admits_closure_carrier(&function.return_type) {
        admit_type(
            units,
            unit,
            function_key,
            &function.return_type,
            true,
            "return type",
        )?;
    }
    let mut parameter_slots = BTreeSet::new();
    for (parameter_index, parameter) in function.params.iter().enumerate() {
        if parameter.mode == MirParamMode::InOut {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::InOut,
                &format!("parameter {parameter_index}"),
            ));
        }
        if !dense_parameter_materialization
            .is_some_and(|fact| fact.slot == parameter.slot && fact.ty == parameter.ty)
            && !server_stream.admits_slot(parameter.slot, &parameter.ty)
            && !server_stream.admits_scalar_carrier(&parameter.ty)
            && !server_stream.admits_closure_carrier(&parameter.ty)
        {
            admit_type_with_registry_authority(
                units,
                unit,
                function_key,
                &parameter.ty,
                false,
                &format!("parameter {parameter_index} type"),
                host_effects.slot_authorities(parameter.slot),
            )?;
        }
        if usize::try_from(parameter.slot).ok() != Some(parameter_index) {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::ParameterSlotCoverage,
                &format!(
                    "parameter {parameter_index} slot {} ordinal",
                    parameter.slot
                ),
            ));
        }
        let slot = function.slot(parameter.slot)?;
        if !parameter_slots.insert(parameter.slot) || slot.kind != MirSlotKind::Param {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::ParameterSlotKind,
                &format!("parameter {parameter_index} slot {}", parameter.slot),
            ));
        }
        if slot.ty.as_ref() != Some(&parameter.ty) {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::ParameterSlotCoverage,
                &format!("parameter {parameter_index} slot {} type", parameter.slot),
            ));
        }
    }
    for slot in &function.slots {
        if slot.kind == MirSlotKind::Param && !parameter_slots.contains(&slot.slot) {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::ParameterSlotCoverage,
                &format!("unbound parameter slot {}", slot.slot),
            ));
        }
        let Some(ty) = slot.ty.as_ref() else {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::ValueShape,
                &format!("slot {} without an exact type", slot.slot),
            ));
        };
        if !dense_parameter_materialization
            .is_some_and(|fact| fact.slot == slot.slot && &fact.ty == ty)
            && !server_stream.admits_slot(slot.slot, ty)
            && !server_stream.admits_scalar_carrier(ty)
            && !server_stream.admits_closure_carrier(ty)
        {
            admit_type_with_registry_authority(
                units,
                unit,
                function_key,
                ty,
                false,
                &format!("slot {} type", slot.slot),
                host_effects.slot_authorities(slot.slot),
            )?;
        }
    }
    if !function.expression_blocks.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::ValueShape,
            "value-block source facts",
        ));
    }
    admit_exception_regions(unit, function_key, function)?;
    admit_effects_with_authority(
        unit,
        function_key,
        function,
        &function.effect_summary,
        &host_effects,
    )?;
    let discriminator_literals = collect_discriminator_literal_positions(function)?;
    for expression in &function.expressions {
        admit_expression_with_host_effects(
            units,
            unit,
            function_key,
            function,
            expression,
            &discriminator_literals,
            &host_effects,
            &server_stream,
        )?;
    }
    for block in &function.blocks {
        for statement in &block.statements {
            admit_statement_with_authority(
                units,
                unit,
                function_key,
                function,
                statement,
                &host_effects,
                &server_stream,
            )?;
        }
    }
    if let Some(reason) = function.source_event_plan.unavailable_reason() {
        return Err(BytecodeEmissionError::Phase1SourceEventsUnavailable {
            module_path: unit.module_path.clone(),
            function_key: function_key.to_string(),
            reason,
        });
    }
    Ok(())
}

/// Admits the Phase 3 exception-region table as exact, function-local facts.
///
/// Every region must point at a `Catch` expression, carry the same slot and
/// catch type as that node, and the slot's frame type must equal the catch
/// type. Every `Catch` expression must be covered by exactly one region.
/// Missing, duplicate or drifted facts are stable typed rejections; no
/// partially admitted function escapes this boundary.
fn admit_exception_regions(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
) -> Result<(), BytecodeEmissionError> {
    let mut region_catch_exprs = BTreeSet::new();
    for (region_index, region) in function.regions.iter().enumerate() {
        let expression = function
            .expression(ExprRefIr {
                expression: region.catch_expr,
            })
            .map_err(|_| {
                exception_region_fact(
                    unit,
                    function_key,
                    &format!(
                        "region {region_index} references absent catch expression {}",
                        region.catch_expr
                    ),
                )
            })?;
        let ExprIr::Catch {
            catch_slot,
            catch_type,
            ..
        } = &expression.expression
        else {
            return Err(exception_region_fact(
                unit,
                function_key,
                &format!(
                    "region {region_index} catch expression {} is not a Catch node",
                    region.catch_expr
                ),
            ));
        };
        if !region_catch_exprs.insert(region.catch_expr) {
            return Err(exception_region_fact(
                unit,
                function_key,
                &format!(
                    "region {region_index} duplicates catch expression {}",
                    region.catch_expr
                ),
            ));
        }
        if region.catch_slot != *catch_slot {
            return Err(exception_region_fact(
                unit,
                function_key,
                &format!(
                    "region {region_index} catch slot {} diverges from Catch node slot {catch_slot}",
                    region.catch_slot
                ),
            ));
        }
        if &region.catch_type != catch_type {
            return Err(exception_region_fact(
                unit,
                function_key,
                &format!(
                    "region {region_index} catch type diverges from Catch node type {catch_type:?}"
                ),
            ));
        }
        let slot_type = function.slot_type(region.catch_slot).map_err(|_| {
            exception_region_fact(
                unit,
                function_key,
                &format!(
                    "region {region_index} catch slot {} is absent",
                    region.catch_slot
                ),
            )
        })?;
        if slot_type != catch_type {
            return Err(exception_region_fact(
                unit,
                function_key,
                &format!(
                    "region {region_index} catch slot {} frame type {slot_type:?} diverges from catch type {catch_type:?}",
                    region.catch_slot
                ),
            ));
        }
    }
    for expression in &function.expressions {
        if matches!(expression.expression, ExprIr::Catch { .. })
            && !region_catch_exprs.contains(&expression.index)
        {
            return Err(exception_region_fact(
                unit,
                function_key,
                &format!(
                    "catch expression {} has no exception region",
                    expression.index
                ),
            ));
        }
    }
    Ok(())
}

fn exception_region_fact(
    unit: &MirUnit,
    function_key: &str,
    detail: &str,
) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "exception region facts",
        location: format!(
            " in module `{}` function `{function_key}`: {detail}",
            unit.module_path
        ),
    }
}

fn function_contains_throw(function: &MirFunction) -> bool {
    function
        .expressions
        .iter()
        .any(|expression| matches!(expression.expression, ExprIr::Throw { .. }))
}

#[cfg(test)]
fn admit_effects(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    summary: &CallableEffectSummary,
) -> Result<(), BytecodeEmissionError> {
    let host_effects =
        HostEffectAdmissions::analyze(function, CANONICAL_DURATION_MILLISECONDS_BINDING_KEY)
            .unwrap_or_default();
    admit_effects_with_authority(unit, function_key, function, summary, &host_effects)
}

fn admit_effects_with_authority(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    summary: &CallableEffectSummary,
    host_effects: &HostEffectAdmissions,
) -> Result<(), BytecodeEmissionError> {
    let effects = match summary {
        CallableEffectSummary::Unknown { .. } => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Effect,
                "unknown callable effect summary",
            ));
        }
        CallableEffectSummary::Analyzed { effects } => effects,
    };
    if !effects.inout_path_effects.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::InOut,
            "callable inout effects",
        ));
    }
    let has_pending_claim = effects.may_pending
        || effects.may_pending()
        || !effects.pending_effect_categories.is_empty();
    if has_pending_claim {
        // A throw inside a may-pending function remains fail-closed until
        // Phase 5 host/Pending rethrow support, and its rejection must still
        // name the throwing function rather than falling through to a tail
        // call or value-shape diagnostic.
        if function_contains_throw(function) {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::PendingEffect,
                "throw inside a may-pending function",
            ));
        }
    }
    let stream_pending = host_effects.has_stream_pending()
        || (function.stream_result.is_some()
            && function.blocks.iter().any(|block| {
                block
                    .statements
                    .iter()
                    .any(|statement| matches!(statement.kind, MirStmtKind::Emit { .. }))
            }));
    host_effects
        .validate_effect_coverage(effects, stream_pending)
        .map_err(|detail| {
            rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::PendingEffect,
                &format!("registry host-effect summary mismatch: {detail}"),
            )
        })?;
    if effects.escapes_caller_value
        || effects.requires_same_heap_identity
        || effects.invokes_unknown_target
    {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Effect,
            "callable non-scalar effects",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn admit_statement(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    statement: &skiff_compiler_lowering::mir::MirStmt,
) -> Result<(), BytecodeEmissionError> {
    let host_effects =
        HostEffectAdmissions::analyze(function, CANONICAL_DURATION_MILLISECONDS_BINDING_KEY)
            .unwrap_or_default();
    admit_statement_with_authority(
        units,
        unit,
        function_key,
        function,
        statement,
        &host_effects,
        &ServerStreamAdmissions::default(),
    )
}

fn admit_statement_with_authority(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    statement: &skiff_compiler_lowering::mir::MirStmt,
    host_effects: &HostEffectAdmissions,
    server_stream: &ServerStreamAdmissions,
) -> Result<(), BytecodeEmissionError> {
    let capability = match &statement.kind {
        MirStmtKind::InitSlot { slot, value } => {
            admit_slot_value_type(
                unit,
                function_key,
                function,
                *slot,
                *value,
                Phase1MirFactMismatch::InitSlotType,
                &format!("statement {} init slot", statement.statement_index),
            )?;
            None
        }
        MirStmtKind::Expr { .. } | MirStmtKind::If { .. } => None,
        MirStmtKind::Return { value } => {
            if function.stream_result.is_some()
                && value.is_some_and(|value| !server_stream.admits_null_return(function, value))
            {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Stream,
                    &format!(
                        "statement {} returns a value from a server-stream producer",
                        statement.statement_index
                    ),
                ));
            }
            if let Some(value) = value.as_ref() {
                if is_tail_local_call(function, value.expression) {
                    if let Some(callee) = tail_local_call_callee(unit, function, value.expression) {
                        if callee_effect_may_pending(callee) {
                            return Err(rejected_function(
                                unit,
                                function_key,
                                Phase1UnsupportedCapability::PendingEffect,
                                &format!("tail call to pending function {}", callee.symbol),
                            ));
                        }
                    }
                    return Err(rejected_function(
                        unit,
                        function_key,
                        Phase1UnsupportedCapability::TailCall,
                        &format!("statement {}", statement.statement_index),
                    ));
                }
            }
            None
        }
        MirStmtKind::Assign { target, place, .. } => match target {
            AssignTargetIr::Slot { .. } => None,
            AssignTargetIr::ActorSelfField { .. } => Some(Phase1UnsupportedCapability::Actor),
            AssignTargetIr::Field { .. } | AssignTargetIr::Index { .. } => {
                if matches!(place.root, MirWritableRoot::ActorSelfField { .. }) {
                    Some(Phase1UnsupportedCapability::Actor)
                } else {
                    None
                }
            }
        },
        MirStmtKind::Throw { payload_type, .. } => {
            admit_throw_payload_type(
                units,
                unit,
                function_key,
                payload_type,
                &format!("statement {} throw payload type", statement.statement_index),
            )?;
            None
        }
        MirStmtKind::Rethrow { exception_slot } => {
            function.slot(*exception_slot)?;
            None
        }
        MirStmtKind::Emit { operation, value } => {
            let Some(stream) = &function.stream_result else {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Stream,
                    &format!(
                        "statement {} emit without server-stream authority",
                        statement.statement_index
                    ),
                ));
            };
            let value_type = &function.expression(*value)?.ty;
            if !operation.is_empty()
                || !server_stream.admits_emit(
                    statement.statement_index,
                    value.expression,
                    value_type,
                )
                || !server_stream.admits_result(&stream.item_type)
            {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Stream,
                    &format!(
                        "statement {} emit differs from exact stream item facts",
                        statement.statement_index
                    ),
                ));
            }
            None
        }
        MirStmtKind::StreamNext { .. }
            if host_effects.admits_stream_next(statement.statement_index) =>
        {
            None
        }
        MirStmtKind::StreamNext { .. } => Some(Phase1UnsupportedCapability::Stream),
        MirStmtKind::TestEffectRegister { .. } => Some(Phase1UnsupportedCapability::HostTarget),
        MirStmtKind::ForIn { .. }
            if host_effects.admits_stream_for_in(statement.statement_index) =>
        {
            None
        }
        MirStmtKind::Break
            if server_stream.has_exact_authority()
                && host_effects.admits_stream_break(statement.statement_index) =>
        {
            None
        }
        MirStmtKind::Dispatch { .. }
        | MirStmtKind::ForIn { .. }
        | MirStmtKind::While { .. }
        | MirStmtKind::Match { .. }
        | MirStmtKind::Break
        | MirStmtKind::Continue => Some(Phase1UnsupportedCapability::ControlFlow),
        MirStmtKind::Timeout { .. } | MirStmtKind::Concurrent { .. } => {
            Some(Phase1UnsupportedCapability::PendingEffect)
        }
        MirStmtKind::Assert { .. } => Some(Phase1UnsupportedCapability::ControlFlow),
    };
    if let Some(capability) = capability {
        return Err(rejected_function(
            unit,
            function_key,
            capability,
            &format!("statement {}", statement.statement_index),
        ));
    }
    Ok(())
}

fn tail_local_call_callee<'a>(
    unit: &'a MirUnit,
    function: &MirFunction,
    expression_index: u32,
) -> Option<&'a MirFunction> {
    let expression = function.expressions.get(expression_index as usize)?;
    let ExprIr::Call { call } = &expression.expression else {
        return None;
    };
    let CallTargetIr::LocalExecutable { executable_index } = call.target else {
        return None;
    };
    unit.function_by_executable_index(executable_index).ok()
}

fn callee_effect_may_pending(callee: &MirFunction) -> bool {
    matches!(
        &callee.effect_summary,
        CallableEffectSummary::Analyzed { effects } if effects.may_pending || effects.may_pending()
    )
}

fn is_tail_local_call(function: &MirFunction, expression_index: u32) -> bool {
    function
        .expressions
        .get(expression_index as usize)
        .is_some_and(|expression| {
            matches!(
                &expression.expression,
                ExprIr::Call { call }
                    if matches!(&call.target, CallTargetIr::LocalExecutable { .. })
                        && expression.direct_call.is_some()
            )
        })
}

/// Compile-time string literals are admitted only as union/`CatchResult`
/// discriminator constants: the right-hand operand of a `tag == "…"`
/// equality. General string values stay fail closed.
fn collect_discriminator_literal_positions(
    function: &MirFunction,
) -> Result<BTreeSet<u32>, BytecodeEmissionError> {
    let mut positions = BTreeSet::new();
    for expression in &function.expressions {
        let ExprIr::Binary {
            op: BinaryOpIr::Equal,
            left,
            right,
        } = &expression.expression
        else {
            continue;
        };
        if is_tag_field_read(function, *left)? && is_string_literal_expression(function, *right)? {
            positions.insert(right.expression);
        }
        if is_tag_field_read(function, *right)? && is_string_literal_expression(function, *left)? {
            positions.insert(left.expression);
        }
    }
    Ok(positions)
}

/// A `tag` field read is the discriminator position for a `CatchResult`
/// (or a tag-shaped named-union accessor whose result is a string-literal
/// union). Only these reads unlock string-literal type admission.
fn is_tag_field_read(
    function: &MirFunction,
    expression_ref: ExprRefIr,
) -> Result<bool, BytecodeEmissionError> {
    let expression = function.expression(expression_ref)?;
    let ExprIr::Field { object, field } = &expression.expression else {
        return Ok(false);
    };
    if field != "tag" {
        return Ok(false);
    }
    let object_type = &function.expression(*object)?.ty;
    Ok(is_catch_result_type(object_type) || is_string_literal_union(&expression.ty))
}

fn is_string_literal_expression(
    function: &MirFunction,
    expression_ref: ExprRefIr,
) -> Result<bool, BytecodeEmissionError> {
    Ok(matches!(
        function.expression(expression_ref)?.expression,
        ExprIr::Literal {
            value: LiteralIr::String { .. }
        }
    ))
}

fn is_string_literal_union(ty: &TypeRefIr) -> bool {
    let TypeRefIr::Union { items } = ty else {
        return false;
    };
    !items.is_empty()
        && items.iter().all(|item| {
            matches!(
                item,
                TypeRefIr::Literal {
                    value: LiteralIr::String { .. }
                }
            )
        })
}

fn is_string_literal_type(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Literal {
            value: LiteralIr::String { .. }
        }
    )
}

fn is_catch_result_type(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2
    )
}

/// After `result.tag == "err"` narrows a CatchResult binding, the expression
/// model retypes later loads of that binding as the err-branch record
/// `{ exception: Exception<E>, tag: "err" }`. The slot's frame type remains
/// the opaque `CatchResult<T,E>`; this admission recognizes exactly that
/// narrowed shape and rejects any other slot/load drift.
fn is_catch_result_narrowed_load(slot_type: &TypeRefIr, load_type: &TypeRefIr) -> bool {
    let TypeRefIr::Builtin { name, args } = slot_type else {
        return false;
    };
    if name != "CatchResult" || args.len() != 2 {
        return false;
    }
    let TypeRefIr::Record { fields } = load_type else {
        return false;
    };
    if fields.len() != 2 {
        return false;
    }
    let exception_type = TypeRefIr::Builtin {
        name: "Exception".to_string(),
        args: vec![args[1].clone()],
    };
    fields.get("exception") == Some(&exception_type)
        && fields
            .get("tag")
            .is_some_and(|tag| is_string_literal_type(tag) || is_string_literal_union(tag))
}

#[cfg(test)]
fn admit_expression(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    discriminator_literals: &BTreeSet<u32>,
) -> Result<(), BytecodeEmissionError> {
    let host_effects =
        HostEffectAdmissions::analyze(function, CANONICAL_DURATION_MILLISECONDS_BINDING_KEY)
            .map_err(|error| {
                rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::HostTarget,
                    &format!(
                        "expression {} exact host-effect admission: {}",
                        error.expression_index, error.detail
                    ),
                )
            })?;
    admit_expression_with_host_effects(
        units,
        unit,
        function_key,
        function,
        expression,
        discriminator_literals,
        &host_effects,
        &ServerStreamAdmissions::default(),
    )
}

fn admit_expression_with_host_effects(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    discriminator_literals: &BTreeSet<u32>,
    host_effects: &HostEffectAdmissions,
    server_stream: &ServerStreamAdmissions,
) -> Result<(), BytecodeEmissionError> {
    let registry_authorities = host_effects.expression_authorities(expression.index);
    if let ExprIr::Call { call } = &expression.expression {
        admit_call(
            unit,
            function_key,
            function,
            expression,
            call,
            host_effects,
            server_stream,
        )?;
    }
    if let ExprIr::Construct { type_ref, .. } = &expression.expression {
        if !server_stream.admits_construct(expression.index, type_ref)
            && !server_stream.admits_closure_carrier(type_ref)
        {
            admit_type_with_registry_authority(
                units,
                unit,
                function_key,
                type_ref,
                false,
                &format!("expression {} construct type", expression.index),
                registry_authorities,
            )?;
        }
    }
    let discriminator_context = discriminator_literals.contains(&expression.index)
        || is_tag_field_read(
            function,
            ExprRefIr {
                expression: expression.index,
            },
        )?;
    if !server_stream.admits_expression(expression.index, &expression.ty)
        && !server_stream.admits_scalar_carrier(&expression.ty)
        && !server_stream.admits_closure_carrier(&expression.ty)
        && !registry_authorities
            .iter()
            .any(|authority| authority.admits(&expression.ty))
    {
        admit_type_with_discriminator_flag(
            units,
            unit,
            function_key,
            &expression.ty,
            true,
            &format!("expression {} type", expression.index),
            discriminator_context,
        )?;
    }
    if expression.writable.is_some() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::InOut,
            &format!("expression {} writable facts", expression.index),
        ));
    }
    if let Some(stream) = &expression.stream_result {
        let exact_stream_authority = registry_authorities.iter().any(|authority| {
            authority.admits(&expression.ty)
                && matches!(
                    &expression.ty,
                    TypeRefIr::Builtin { name, args }
                        if name == "Stream" && args.as_slice() == [stream.item_type.clone()]
                )
        });
        if !exact_stream_authority {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Stream,
                &format!(
                    "expression {} stream facts lack exact producer authority",
                    expression.index
                ),
            ));
        }
    }
    if expression.remote_interface.is_some() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Interface,
            &format!("expression {} remote interface facts", expression.index),
        ));
    }
    let capability = match &expression.expression {
        ExprIr::Literal { value } => match value {
            LiteralIr::Null | LiteralIr::Bool { .. } | LiteralIr::Number { .. } => None,
            LiteralIr::String { value }
                if discriminator_literals.contains(&expression.index)
                    || server_stream.admits_tag_literal(expression.index, value) =>
            {
                None
            }
            LiteralIr::String { .. } if server_stream.admits_scalar_carrier(&expression.ty) => None,
            LiteralIr::String { .. } => Some(Phase1UnsupportedCapability::ValueShape),
        },
        ExprIr::LoadSlot { slot } => {
            let slot_type = function.slot_type(*slot)?;
            if slot_type != &expression.ty
                && !is_catch_result_narrowed_load(slot_type, &expression.ty)
                && !may_share_scalar_machine_carrier(slot_type, &expression.ty)
            {
                return Err(fact_mismatch(
                    unit,
                    function_key,
                    Phase1MirFactMismatch::LoadSlotType,
                    &format!(
                        "expression {} load slot {slot}: slot type {slot_type:?}, load type {:?}",
                        expression.index, expression.ty
                    ),
                ));
            }
            None
        }
        ExprIr::Unary { .. } => None,
        ExprIr::Binary { op, .. } => match op {
            BinaryOpIr::And | BinaryOpIr::Or => Some(Phase1UnsupportedCapability::ControlFlow),
            _ => None,
        },
        ExprIr::Call { .. } => None,
        ExprIr::LoadConst { .. } | ExprIr::LoadPackageConst { .. } => {
            Some(Phase1UnsupportedCapability::Constant)
        }
        ExprIr::ActorSelfField { .. } => Some(Phase1UnsupportedCapability::Actor),
        ExprIr::InterfaceBox { .. } => Some(Phase1UnsupportedCapability::Interface),
        ExprIr::Throw { payload_type, .. } => {
            admit_throw_payload_type(
                units,
                unit,
                function_key,
                payload_type,
                &format!("expression {} throw payload type", expression.index),
            )?;
            None
        }
        ExprIr::Rethrow { exception_slot } => {
            function.slot(*exception_slot)?;
            None
        }
        ExprIr::Catch {
            catch_slot,
            catch_type,
            ..
        } => {
            admit_type(
                units,
                unit,
                function_key,
                catch_type,
                false,
                &format!("expression {} catch type", expression.index),
            )?;
            let slot_type = function.slot_type(*catch_slot).map_err(|_| {
                BytecodeEmissionError::UnsupportedConstruct {
                    function_key: function_key.to_string(),
                    construct: "catch slot facts",
                    location: format!(
                        " expression {} catch slot {catch_slot} is absent",
                        expression.index
                    ),
                }
            })?;
            if slot_type != catch_type {
                return Err(BytecodeEmissionError::UnsupportedConstruct {
                    function_key: function_key.to_string(),
                    construct: "catch slot facts",
                    location: format!(
                        " expression {} catch slot {catch_slot} frame type {slot_type:?} diverges from catch type {catch_type:?}",
                        expression.index
                    ),
                });
            }
            None
        }
        ExprIr::Timeout { .. } | ExprIr::ConcurrentValue { .. } => {
            Some(Phase1UnsupportedCapability::PendingEffect)
        }
        ExprIr::DbOperation { .. }
        | ExprIr::DbQuery { .. }
        | ExprIr::DbTransaction { .. }
        | ExprIr::DbLeaseClaim { .. }
        | ExprIr::DbLeaseRead { .. } => Some(Phase1UnsupportedCapability::ServiceTarget),
        ExprIr::Field { .. }
        | ExprIr::Index { .. }
        | ExprIr::Construct { .. }
        | ExprIr::ArrayLiteral { .. } => None,
        ExprIr::RepresentationWrap { .. }
        | ExprIr::MapLiteral { .. }
        | ExprIr::ValueBlock { .. } => Some(Phase1UnsupportedCapability::ValueShape),
    };
    if let Some(capability) = capability {
        return Err(rejected_function(
            unit,
            function_key,
            capability,
            &format!("expression {}", expression.index),
        ));
    }
    Ok(())
}

fn admit_call(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    call: &skiff_artifact_model::CallIr,
    host_effects: &HostEffectAdmissions,
    server_stream: &ServerStreamAdmissions,
) -> Result<(), BytecodeEmissionError> {
    if !call.type_args.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Generic,
            &format!("expression {} call type arguments", expression.index),
        ));
    }
    if !call.inout_args.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::InOut,
            &format!("expression {} call inout arguments", expression.index),
        ));
    }
    if call.concrete_receiver.is_some() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Receiver,
            &format!("expression {} call receiver", expression.index),
        ));
    }
    if !call.metadata.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::NonLocalCallTarget,
            &format!("expression {} call metadata", expression.index),
        ));
    }
    let callee = match &call.target {
        CallTargetIr::LocalExecutable { executable_index } => unit
            .function_by_executable_index(*executable_index)
            .map_err(|_| {
                rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::NonLocalCallTarget,
                    &format!("expression {} call target", expression.index),
                )
            })?,
        CallTargetIr::PublicationExecutable { .. } | CallTargetIr::PackageCallable { .. } => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::NonLocalCallTarget,
                &format!("expression {} call target", expression.index),
            ));
        }
        CallTargetIr::ServiceCall { .. } => return Ok(()),
        CallTargetIr::ServiceDependencySymbol { .. } => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::ServiceTarget,
                &format!("expression {} call target", expression.index),
            ));
        }
        CallTargetIr::ActorMethod { .. } => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Actor,
                &format!("expression {} call target", expression.index),
            ));
        }
        CallTargetIr::Native { target } => {
            if target.binding_key.as_deref() == Some(CANONICAL_DURATION_MILLISECONDS_BINDING_KEY) {
                admit_duration_milliseconds_constructor(
                    unit,
                    function_key,
                    function,
                    expression,
                    call,
                    target,
                    host_effects,
                )?;
            } else if server_stream.admits_intrinsic_call(function, expression.index) {
                return Ok(());
            } else if host_effects.executor_for_call(expression.index).is_none() {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::HostTarget,
                    &format!(
                        "expression {} native call lacks exact registry executor admission",
                        expression.index
                    ),
                ));
            }
            return Ok(());
        }
        CallTargetIr::ReceiverBuiltin { .. }
            if server_stream.admits_receiver_call(expression.index)
                || server_stream.admits_intrinsic_call(function, expression.index) =>
        {
            return Ok(());
        }
        CallTargetIr::Builtin { .. } | CallTargetIr::ReceiverBuiltin { .. } => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::HostTarget,
                &format!("expression {} call target", expression.index),
            ));
        }
        CallTargetIr::InterfaceMethod { .. } => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Interface,
                &format!("expression {} call target", expression.index),
            ));
        }
    };
    let Some(facts) = expression.direct_call.as_ref() else {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::NonLocalCallTarget,
            &format!("expression {} missing direct-call facts", expression.index),
        ));
    };
    if facts.concrete_receiver.is_some() || facts.receiver_call_abi.is_some() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Receiver,
            &format!("expression {} direct-call receiver facts", expression.index),
        ));
    }
    if facts
        .parameter_modes
        .iter()
        .any(|mode| *mode == MirParamMode::InOut)
        || facts
            .arguments
            .iter()
            .any(|argument| matches!(argument, MirCallArgument::InOut { .. }))
    {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::InOut,
            &format!("expression {} direct-call ABI", expression.index),
        ));
    }
    function.direct_call_facts(skiff_artifact_model::ExprRefIr {
        expression: expression.index,
    })?;
    admit_local_call_abi(
        unit,
        function_key,
        function,
        expression,
        call,
        facts,
        callee,
    )?;
    admit_local_call_source_event(unit, function_key, function, expression, call)?;
    Ok(())
}

/// Phase 4 gate 1 companion: admits the pure `Duration.milliseconds`
/// constructor only when its exact argument and result stay on the pinned
/// sleep argument face. It is not a host effect, does not carry Pending, and
/// remains emitted as a synchronous constant/identity operation by the
/// bytecode emitter rather than an `InvokeHost` adapter.
fn admit_duration_milliseconds_constructor(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    call: &skiff_artifact_model::CallIr,
    target: &NativeTarget,
    host_effects: &HostEffectAdmissions,
) -> Result<(), BytecodeEmissionError> {
    if target.namespace != "Duration"
        || target.symbol != "milliseconds"
        || target.binding_key.as_deref() != Some(CANONICAL_DURATION_MILLISECONDS_BINDING_KEY)
        || !target.metadata.is_empty()
    {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            &format!(
                "expression {} Duration.milliseconds target identity is not exact",
                expression.index
            ),
        ));
    }
    if call.args.len() != 1 {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            &format!(
                "expression {} Duration.milliseconds arity {} (pinned arity is exactly one integer argument)",
                expression.index,
                call.args.len()
            ),
        ));
    }
    let argument = function.expression(call.args[0])?;
    let argument_type = &argument.ty;
    if !matches!(
        &argument.expression,
        ExprIr::Literal {
            value: LiteralIr::Number { .. }
        }
    ) {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            &format!(
                "expression {} Duration.milliseconds argument must be a literal integer",
                expression.index
            ),
        ));
    }
    if !matches!(
        argument_type,
        TypeRefIr::Builtin { name, args } if name == "integer" && args.is_empty()
    ) {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            &format!(
                "expression {} Duration.milliseconds argument type {argument_type:?} is not the pinned integer",
                expression.index
            ),
        ));
    }
    if !host_effects.admits_duration_constructor(expression.index, &expression.ty) {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            &format!(
                "expression {} Duration.milliseconds lacks the exact Sleep parameter closure",
                expression.index
            ),
        ));
    }
    Ok(())
}

fn admit_local_call_abi(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    call: &skiff_artifact_model::CallIr,
    facts: &skiff_compiler_lowering::mir::MirDirectCallFacts,
    callee: &MirFunction,
) -> Result<(), BytecodeEmissionError> {
    let parameter_count = callee.params.len();
    if facts.parameter_modes.len() != parameter_count
        || facts.arguments.len() != parameter_count
        || call.args.len() != parameter_count
    {
        return Err(fact_mismatch(
            unit,
            function_key,
            Phase1MirFactMismatch::LocalCallParameterCount,
            &format!("expression {} local call", expression.index),
        ));
    }
    for (parameter_index, parameter) in callee.params.iter().enumerate() {
        if facts.parameter_modes[parameter_index] != parameter.mode {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::LocalCallParameterMode,
                &format!(
                    "expression {} local call parameter {parameter_index}",
                    expression.index
                ),
            ));
        }
        let MirCallArgument::Value { value } = &facts.arguments[parameter_index] else {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::LocalCallArgument,
                &format!(
                    "expression {} local call parameter {parameter_index}",
                    expression.index
                ),
            ));
        };
        if call.args[parameter_index] != *value {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::LocalCallArgument,
                &format!(
                    "expression {} local call parameter {parameter_index}",
                    expression.index
                ),
            ));
        }
        let argument = function.expression(*value)?;
        if argument.ty != parameter.ty {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::LocalCallArgumentType,
                &format!(
                    "expression {} local call parameter {parameter_index}",
                    expression.index
                ),
            ));
        }
    }
    if expression.ty != callee.return_type {
        return Err(fact_mismatch(
            unit,
            function_key,
            Phase1MirFactMismatch::LocalCallResultType,
            &format!("expression {} local call result", expression.index),
        ));
    }
    Ok(())
}

fn admit_local_call_source_event(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    call: &skiff_artifact_model::CallIr,
) -> Result<(), BytecodeEmissionError> {
    let Some(events) = function.source_event_plan.events() else {
        return Ok(());
    };
    let mut matches = events.iter().filter(|event| {
        matches!(
            (event.attribution_id, event.anchor),
            (
                StatementAttributionId::Expression {
                    expression_index,
                    occurrence_ordinal: 0,
                },
                MirEmissionAnchor::LocalCall {
                    expression_index: anchor_expression,
                    occurrence_ordinal: 0,
                }
                | MirEmissionAnchor::TailLocalCallCandidate {
                    expression_index: anchor_expression,
                    occurrence_ordinal: 0,
                    ..
                },
            ) if expression_index == expression.index && anchor_expression == expression.index
        ) && event.site == call.site
    });
    if matches.next().is_none() || matches.next().is_some() {
        return Err(fact_mismatch(
            unit,
            function_key,
            Phase1MirFactMismatch::LocalCallSourceEvent,
            &format!("expression {} local call source event", expression.index),
        ));
    }
    Ok(())
}

fn admit_slot_value_type(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    slot: u32,
    value: skiff_artifact_model::ExprRefIr,
    mismatch: Phase1MirFactMismatch,
    location: &str,
) -> Result<(), BytecodeEmissionError> {
    let slot_type = function.slot_type(slot)?;
    let value_type = &function.expression(value)?.ty;
    if slot_type != value_type {
        return Err(fact_mismatch(unit, function_key, mismatch, location));
    }
    Ok(())
}

/// Phase 3 throw payload leaves must carry a runtime catch identity: local
/// nominal record types and anonymous unions whose branches are nominal
/// records. Structural, scalar and literal-branch leaves have no runtime
/// identity and fail closed here instead of reaching a constant VmFailure.
/// This tightens the throw face only; catch/rethrow admission is unchanged.
fn admit_throw_payload_type(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    payload_type: &TypeRefIr,
    location: &str,
) -> Result<(), BytecodeEmissionError> {
    match payload_type {
        TypeRefIr::Union { items } => {
            if items.is_empty() {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::ValueShape,
                    &format!("{location} empty union"),
                ));
            }
            for item in items {
                admit_throw_payload_type(
                    units,
                    unit,
                    function_key,
                    item,
                    &format!("{location} union branch"),
                )?;
            }
            Ok(())
        }
        TypeRefIr::LocalType { type_index } => admit_nominal_record_leaf(
            units,
            unit,
            function_key,
            &unit.module_path,
            *type_index,
            location,
        ),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => admit_nominal_record_leaf(
            units,
            unit,
            function_key,
            module_path,
            *type_index,
            location,
        ),
        other => Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::ValueShape,
            &format!("{location} leaf {other:?} has no runtime catch identity"),
        )),
    }
}

fn admit_nominal_record_leaf(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    module_path: &str,
    type_index: u32,
    location: &str,
) -> Result<(), BytecodeEmissionError> {
    let owning_unit = units
        .iter()
        .find(|candidate| candidate.module_path == module_path)
        .ok_or_else(|| BytecodeEmissionError::CanonicalSerialization {
            context: format!("throw payload nominal leaf admission for module `{module_path}`"),
            message: "owning MIR unit disappeared".to_string(),
        })?;
    let declaration = owning_unit
        .type_table
        .get(type_index as usize)
        .ok_or_else(|| BytecodeEmissionError::MissingLocalType {
            module_path: module_path.to_string(),
            location: location.to_string(),
            type_index,
            type_count: owning_unit.type_table.len(),
        })?;
    if !matches!(declaration.descriptor, TypeDescriptorIr::Record { .. }) {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::ValueShape,
            &format!(
                "{location} nominal `{}` is not a record leaf",
                declaration.name
            ),
        ));
    }
    Ok(())
}

fn admit_type(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    ty: &TypeRefIr,
    allow_void: bool,
    location: &str,
) -> Result<(), BytecodeEmissionError> {
    admit_type_with_discriminator_flag(units, unit, function_key, ty, allow_void, location, false)
}

fn admit_type_with_registry_authority(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    ty: &TypeRefIr,
    allow_void: bool,
    location: &str,
    authorities: &[RegistryValueAuthority],
) -> Result<(), BytecodeEmissionError> {
    if authorities.iter().any(|authority| authority.admits(ty)) {
        return Ok(());
    }
    admit_type(units, unit, function_key, ty, allow_void, location)
}

fn admit_type_with_discriminator_flag(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    ty: &TypeRefIr,
    allow_void: bool,
    location: &str,
    allow_discriminator_literal: bool,
) -> Result<(), BytecodeEmissionError> {
    let mut context = TypeAdmissionContext {
        units,
        function_key,
        nominal_chain: Vec::new(),
        allow_discriminator_literal,
    };
    admit_type_nested(&mut context, unit, ty, allow_void, location, false)
}

/// Recursive Phase 2 type admission with a nominal-recursion guard.
///
/// `nested` distinguishes a record/array leaf from a top-level type: out-of-
/// surface nested leaves carry the stable Phase 2 record/array rejection,
/// while top-level rejections keep the legacy capability diagnostics.
struct TypeAdmissionContext<'a> {
    units: &'a [MirUnit],
    function_key: &'a str,
    nominal_chain: Vec<(String, u32)>,
    allow_discriminator_literal: bool,
}

fn admit_type_nested(
    context: &mut TypeAdmissionContext<'_>,
    unit: &MirUnit,
    ty: &TypeRefIr,
    allow_void: bool,
    location: &str,
    nested: bool,
) -> Result<(), BytecodeEmissionError> {
    match ty {
        TypeRefIr::Record { fields } => {
            for (name, field_ty) in fields {
                let saved_flag = context.allow_discriminator_literal;
                if name == "tag"
                    && (is_string_literal_type(field_ty) || is_string_literal_union(field_ty))
                {
                    context.allow_discriminator_literal = true;
                }
                let result = admit_type_nested(
                    context,
                    unit,
                    field_ty,
                    false,
                    &format!("{location} field `{name}`"),
                    true,
                );
                context.allow_discriminator_literal = saved_flag;
                result?;
            }
            Ok(())
        }
        TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1 => {
            admit_type_nested(
                context,
                unit,
                &args[0],
                false,
                &format!("{location} element type"),
                true,
            )
        }
        TypeRefIr::Union { items } => {
            for item in items {
                admit_type_nested(
                    context,
                    unit,
                    item,
                    false,
                    &format!("{location} union leaf"),
                    nested,
                )?;
            }
            Ok(())
        }
        TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2 => {
            admit_type_nested(
                context,
                unit,
                &args[0],
                true,
                &format!("{location} result type"),
                nested,
            )?;
            admit_type_nested(
                context,
                unit,
                &args[1],
                false,
                &format!("{location} error type"),
                nested,
            )
        }
        TypeRefIr::Builtin { name, args } if name == "Exception" && args.len() == 1 => {
            admit_type_nested(
                context,
                unit,
                &args[0],
                false,
                &format!("{location} payload type"),
                nested,
            )
        }
        // Compile-time string literals are admitted only inside a
        // discriminator context (a `.tag` result union or the constant side
        // of a `tag == "…"` equality). Everywhere else they stay on the
        // rejected Phase 2 value-shape face.
        TypeRefIr::Literal {
            value: LiteralIr::String { .. },
        } if context.allow_discriminator_literal => Ok(()),
        TypeRefIr::LocalType { type_index } => {
            admit_nominal_declaration(context, unit, &unit.module_path, *type_index, location)
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => admit_nominal_declaration(context, unit, module_path, *type_index, location),
        _ => {
            if let Some(capability) = unsupported_type_capability(ty, allow_void) {
                return if nested {
                    Err(phase_2_nested_shape_rejection(
                        context.function_key,
                        capability,
                        location,
                    ))
                } else {
                    Err(rejected_function(
                        unit,
                        context.function_key,
                        capability,
                        location,
                    ))
                };
            }
            Ok(())
        }
    }
}

/// Recursively admits one nominal declaration (record, representation, named
/// union or transparent alias) against the Phase 2 value face, with a
/// nominal-recursion guard. Interface declarations stay rejected.
fn admit_nominal_declaration(
    context: &mut TypeAdmissionContext<'_>,
    unit: &MirUnit,
    module_path: &str,
    type_index: u32,
    location: &str,
) -> Result<(), BytecodeEmissionError> {
    let key = (module_path.to_string(), type_index);
    if context.nominal_chain.contains(&key) {
        return Err(phase_2_nested_shape_rejection(
            context.function_key,
            Phase1UnsupportedCapability::ValueShape,
            &format!("{location} (recursive record reference)"),
        ));
    }
    let owning_unit = context
        .units
        .iter()
        .find(|candidate| candidate.module_path == module_path)
        .ok_or_else(|| BytecodeEmissionError::CanonicalSerialization {
            context: format!("Phase 2 record admission for module `{module_path}`"),
            message: "owning MIR unit disappeared".to_string(),
        })?;
    let declaration = owning_unit
        .type_table
        .get(type_index as usize)
        .ok_or_else(|| BytecodeEmissionError::MissingLocalType {
            module_path: module_path.to_string(),
            location: location.to_string(),
            type_index,
            type_count: owning_unit.type_table.len(),
        })?;
    context.nominal_chain.push(key);
    let result = match &declaration.descriptor {
        TypeDescriptorIr::Record { fields } => {
            for (name, field_ty) in fields {
                admit_type_nested(
                    context,
                    owning_unit,
                    field_ty,
                    false,
                    &format!("{location} field `{name}`"),
                    true,
                )?;
            }
            Ok(())
        }
        TypeDescriptorIr::Representation { representation } => admit_type_nested(
            context,
            owning_unit,
            representation,
            false,
            &format!("{location} representation"),
            true,
        ),
        TypeDescriptorIr::Union { branches } => {
            for branch in branches {
                match branch {
                    NamedUnionBranchIr::ConcreteNominal { nominal_type } => admit_type_nested(
                        context,
                        owning_unit,
                        nominal_type,
                        false,
                        &format!("{location} union branch"),
                        true,
                    )?,
                    NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => {
                        admit_type_nested(
                            context,
                            owning_unit,
                            payload_type,
                            false,
                            &format!("{location} union branch"),
                            true,
                        )?
                    }
                    NamedUnionBranchIr::Literal { value } => admit_type_nested(
                        context,
                        owning_unit,
                        &TypeRefIr::Literal {
                            value: value.clone(),
                        },
                        false,
                        &format!("{location} union branch"),
                        true,
                    )?,
                }
            }
            Ok(())
        }
        TypeDescriptorIr::Alias { target } => admit_type_nested(
            context,
            owning_unit,
            target,
            false,
            &format!("{location} alias target"),
            true,
        ),
        TypeDescriptorIr::Interface => Err(rejected_function(
            unit,
            context.function_key,
            Phase1UnsupportedCapability::ValueShape,
            location,
        )),
    };
    context.nominal_chain.pop();
    result
}

fn phase_2_nested_shape_rejection(
    function_key: &str,
    capability: Phase1UnsupportedCapability,
    location: &str,
) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "phase 2 record/array value shape",
        location: format!(" {location} ({capability:?})"),
    }
}

fn unsupported_type_capability(
    ty: &TypeRefIr,
    allow_void: bool,
) -> Option<Phase1UnsupportedCapability> {
    match ty {
        // Throw/rethrow expressions are typed `never`: the uninhabited type is
        // admitted only where the language itself places it (expression/result
        // positions), never as a data-shape leaf.
        TypeRefIr::Builtin { name, args } if name == "never" && args.is_empty() && allow_void => {
            None
        }
        TypeRefIr::Builtin { name, args }
            if args.is_empty()
                && (matches!(name.as_str(), "integer" | "number" | "bool" | "null")
                    || (allow_void && name == "void")) =>
        {
            None
        }
        TypeRefIr::Literal { value } => match value {
            LiteralIr::Null | LiteralIr::Bool { .. } | LiteralIr::Number { .. } => None,
            LiteralIr::String { .. } => Some(Phase1UnsupportedCapability::ValueShape),
        },
        TypeRefIr::TypeParam { .. } | TypeRefIr::AppliedNominal { .. } => {
            Some(Phase1UnsupportedCapability::Generic)
        }
        TypeRefIr::AnyInterface { .. } => Some(Phase1UnsupportedCapability::Interface),
        TypeRefIr::Function { .. } => Some(Phase1UnsupportedCapability::Callback),
        TypeRefIr::ServiceSymbol { .. } | TypeRefIr::DbObjectSymbol { .. } => {
            Some(Phase1UnsupportedCapability::ServiceTarget)
        }
        _ => Some(Phase1UnsupportedCapability::ValueShape),
    }
}

fn rejected_function(
    unit: &MirUnit,
    function_key: &str,
    capability: Phase1UnsupportedCapability,
    location: &str,
) -> BytecodeEmissionError {
    rejected(unit, Some(function_key), capability, location)
}

fn fact_mismatch(
    unit: &MirUnit,
    function_key: &str,
    mismatch: Phase1MirFactMismatch,
    location: &str,
) -> BytecodeEmissionError {
    BytecodeEmissionError::Phase1MirFactMismatch {
        mismatch,
        module_path: unit.module_path.clone(),
        function_key: function_key.to_string(),
        location: location.to_string(),
    }
}

fn rejected(
    unit: &MirUnit,
    function_key: Option<&str>,
    capability: Phase1UnsupportedCapability,
    location: &str,
) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedPhase1Capability {
        capability,
        module_path: unit.module_path.clone(),
        function_key: function_key.map(str::to_string),
        location: location.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        CallIr, CallTargetIr, CallableEffectSummary, CallableMayEffects, ExprIr, ExprRefIr,
        FileIrUnit, InstructionSourceSite, LiteralIr, NativeTarget, PackageCallableId,
        PackageRefIr, PackageSymbolRef, PendingEffectCategory, SyntheticInstructionSiteReason,
        TypeDeclIr, TypeDescriptorIr, TypeRefIr,
    };
    use skiff_compiler_lowering::mir::{
        MirBlock, MirExecutableKind, MirExpression, MirFunction, MirLiveness, MirRegion, MirSlot,
        MirSlotKind, MirSourceEventPlan, MirSourceEventUnavailableReason, MirStmt, MirStmtKind,
        MirUnit,
    };

    use super::*;
    use crate::Phase1UnsupportedCapability;

    const FUNCTION_KEY: &str = "main::run";

    fn number() -> TypeRefIr {
        TypeRefIr::builtin("number")
    }

    fn local(index: u32) -> TypeRefIr {
        TypeRefIr::LocalType { type_index: index }
    }

    fn union(items: Vec<TypeRefIr>) -> TypeRefIr {
        TypeRefIr::Union { items }
    }

    fn record_declaration(name: &str, fields: BTreeMap<String, TypeRefIr>) -> TypeDeclIr {
        TypeDeclIr {
            name: name.to_string(),
            descriptor: TypeDescriptorIr::Record { fields },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        }
    }

    fn slot(index: u32, ty: TypeRefIr) -> MirSlot {
        MirSlot {
            slot: index,
            name: format!("slot{index}"),
            kind: MirSlotKind::Local,
            writable_local: false,
            ty: Some(ty),
        }
    }

    fn expression(index: u32, expression: ExprIr, ty: TypeRefIr) -> MirExpression {
        MirExpression {
            index,
            expression,
            ty,
            writable: None,
            direct_call: None,
            stream_result: None,
            remote_interface: None,
        }
    }

    fn statement(index: u32, kind: MirStmtKind) -> MirStmt {
        MirStmt {
            statement_index: index,
            span: None,
            kind,
        }
    }

    fn synthetic_site() -> InstructionSourceSite {
        InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerDesugaring,
        }
    }

    fn function() -> MirFunction {
        MirFunction {
            executable_index: 0,
            origin: skiff_artifact_model::PackageExecutableCoordinate {
                file_ir_identity: "file:main".to_string(),
                module_path: "main".to_string(),
                executable_index: 0,
            },
            symbol: "main.run".to_string(),
            kind: MirExecutableKind::Function,
            native: false,
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("void"),
            self_type: None,
            receiver: None,
            slots: Vec::new(),
            index_accesses: BTreeMap::new(),
            expression_blocks: BTreeMap::new(),
            expressions: Vec::new(),
            blocks: Vec::new(),
            regions: Vec::new(),
            statements: Vec::new(),
            source_event_plan: MirSourceEventPlan::unavailable(
                MirSourceEventUnavailableReason::SourceFactsNotProvided,
            ),
            stream_result: None,
            liveness: MirLiveness::default(),
            effect_summary_ref: PackageCallableId::new("callable:main:run".to_string()),
            effect_summary: CallableEffectSummary::Analyzed {
                effects: CallableMayEffects {
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_pending: false,
                    pending_effect_categories: Vec::new(),
                    inout_path_effects: Vec::new(),
                },
            },
            source_span: None,
        }
    }

    fn unit(functions: Vec<MirFunction>, type_table: Vec<TypeDeclIr>) -> MirUnit {
        let mut file_ir = FileIrUnit::empty("main", "source-hash");
        file_ir.file_ir_identity = "file:main".to_string();
        file_ir.type_table = type_table;
        MirUnit {
            file_ir_identity: file_ir.file_ir_identity,
            module_path: file_ir.module_path,
            actor_declarations: file_ir.actor_declarations,
            external_refs: file_ir.external_refs,
            source_map: file_ir.source_map,
            type_table: file_ir.type_table,
            package_type_records: BTreeMap::new(),
            link_targets: file_ir.link_targets,
            constants: Vec::new(),
            functions,
        }
    }

    fn two_nominal_types() -> Vec<TypeDeclIr> {
        vec![
            record_declaration("A", BTreeMap::from([("x".to_string(), number())])),
            record_declaration("B", BTreeMap::from([("y".to_string(), number())])),
        ]
    }

    fn canonical_duration_type() -> TypeRefIr {
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "skiff.run/std".to_string(),
                },
                symbol_path: "std.time.Duration".to_string(),
                abi_expectation: Some("std-abi-fixture".to_string()),
            },
        }
    }

    fn native_target(namespace: &str, symbol: &str, binding_key: Option<&str>) -> NativeTarget {
        NativeTarget {
            namespace: namespace.to_string(),
            symbol: symbol.to_string(),
            binding_key: binding_key.map(str::to_string),
            metadata: BTreeMap::new(),
        }
    }

    fn native_call(target: NativeTarget, args: Vec<ExprRefIr>) -> CallIr {
        CallIr {
            target: CallTargetIr::Native { target },
            concrete_receiver: None,
            site: synthetic_site(),
            args,
            inout_args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        }
    }

    fn sleep_pending_effects() -> CallableMayEffects {
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: true,
            pending_effect_categories: vec![PendingEffectCategory::NativeCall],
            inout_path_effects: Vec::new(),
        }
    }

    fn sleep_call_function(duration: TypeRefIr, call: CallIr, call_type: TypeRefIr) -> MirFunction {
        let mut function = function();
        function.slots.push(slot(0, duration.clone()));
        function.expressions.push(expression(
            0,
            ExprIr::LoadSlot { slot: 0 },
            duration.clone(),
        ));
        function
            .expressions
            .push(expression(1, ExprIr::Call { call }, call_type));
        function.effect_summary = CallableEffectSummary::Analyzed {
            effects: sleep_pending_effects(),
        };
        function
    }

    #[test]
    fn phase_4_admission_admits_canonical_sleep_call_and_its_pending_trace() {
        let duration = canonical_duration_type();
        let function = sleep_call_function(
            duration.clone(),
            native_call(
                native_target("std.time", "sleep", Some("std.time.sleep")),
                vec![ExprRefIr { expression: 0 }],
            ),
            TypeRefIr::builtin("void"),
        );
        let units = [unit(vec![function], Vec::new())];
        let function = &units[0].functions[0];

        admit_effects(&units[0], FUNCTION_KEY, function, &function.effect_summary)
            .expect("the canonical sleep pending trace is admitted");
        admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            function,
            &function.expressions[0],
            &BTreeSet::new(),
        )
        .expect("the pinned Duration argument type is admitted");
        admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            function,
            &function.expressions[1],
            &BTreeSet::new(),
        )
        .expect("the canonical sleep call is admitted");
    }

    #[test]
    fn phase_4_admission_rejects_other_host_binding_with_typed_error() {
        let duration = canonical_duration_type();
        let function = sleep_call_function(
            duration.clone(),
            native_call(
                native_target(
                    "Duration",
                    "milliseconds",
                    Some("core.duration.milliseconds"),
                ),
                vec![ExprRefIr { expression: 0 }],
            ),
            duration,
        );
        let units = [unit(vec![function], Vec::new())];
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &units[0].functions[0],
            &units[0].functions[0].expressions[1],
            &BTreeSet::new(),
        )
        .expect_err("non-sleep host bindings stay rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::HostTarget,
                ..
            }
        ));
    }

    #[test]
    fn phase_4_admission_rejects_sleep_wrong_arity() {
        let function = sleep_call_function(
            canonical_duration_type(),
            native_call(
                native_target("std.time", "sleep", Some("std.time.sleep")),
                Vec::new(),
            ),
            TypeRefIr::builtin("void"),
        );
        let units = [unit(vec![function], Vec::new())];
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &units[0].functions[0],
            &units[0].functions[0].expressions[1],
            &BTreeSet::new(),
        )
        .expect_err("sleep with zero arguments stays rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::HostTarget,
                ..
            }
        ));
    }

    #[test]
    fn phase_4_admission_rejects_sleep_wrong_argument_type() {
        let mut function = function();
        function.slots.push(slot(0, number()));
        function
            .expressions
            .push(expression(0, ExprIr::LoadSlot { slot: 0 }, number()));
        function.expressions.push(expression(
            1,
            ExprIr::Call {
                call: native_call(
                    native_target("std.time", "sleep", Some("std.time.sleep")),
                    vec![ExprRefIr { expression: 0 }],
                ),
            },
            TypeRefIr::builtin("void"),
        ));
        function.effect_summary = CallableEffectSummary::Analyzed {
            effects: sleep_pending_effects(),
        };
        let units = [unit(vec![function], Vec::new())];
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &units[0].functions[0],
            &units[0].functions[0].expressions[1],
            &BTreeSet::new(),
        )
        .expect_err("a non-Duration sleep argument stays rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::HostTarget,
                ..
            }
        ));
    }

    #[test]
    fn phase_4_admission_rejects_sleep_wrong_result_type() {
        let function = sleep_call_function(
            canonical_duration_type(),
            native_call(
                native_target("std.time", "sleep", Some("std.time.sleep")),
                vec![ExprRefIr { expression: 0 }],
            ),
            number(),
        );
        let units = [unit(vec![function], Vec::new())];
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &units[0].functions[0],
            &units[0].functions[0].expressions[1],
            &BTreeSet::new(),
        )
        .expect_err("a non-void sleep result stays rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::HostTarget,
                ..
            }
        ));
    }

    #[test]
    fn phase_4_admission_rejects_native_target_without_binding_key() {
        let function = sleep_call_function(
            canonical_duration_type(),
            native_call(
                native_target("std.time", "sleep", None),
                vec![ExprRefIr { expression: 0 }],
            ),
            TypeRefIr::builtin("void"),
        );
        let units = [unit(vec![function], Vec::new())];
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &units[0].functions[0],
            &units[0].functions[0].expressions[1],
            &BTreeSet::new(),
        )
        .expect_err("a native target without a binding key stays rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::HostTarget,
                ..
            }
        ));
    }

    #[test]
    fn phase_4_admission_rejects_drifted_pending_trace() {
        let units = [unit(Vec::new(), Vec::new())];
        let summary = CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: true,
                pending_effect_categories: Vec::new(),
                inout_path_effects: Vec::new(),
            },
        };
        let function = function();
        let error = admit_effects(&units[0], FUNCTION_KEY, &function, &summary)
            .expect_err("a mayPending flag without a category trace is a drifted fact");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::PendingEffect,
                ..
            }
        ));
    }

    #[test]
    fn phase_4_admission_admits_canonical_sleep_native_call_pending_trace() {
        let summary = CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: true,
                pending_effect_categories: vec![PendingEffectCategory::NativeCall],
                inout_path_effects: Vec::new(),
            },
        };
        let mut function = sleep_call_function(
            canonical_duration_type(),
            native_call(
                native_target("std.time", "sleep", Some("std.time.sleep")),
                vec![ExprRefIr { expression: 0 }],
            ),
            TypeRefIr::builtin("void"),
        );
        function.effect_summary = summary.clone();
        let units = [unit(vec![function], Vec::new())];
        admit_effects(&units[0], FUNCTION_KEY, &units[0].functions[0], &summary)
            .expect("the exact Sleep executor retains the NativeCall pending category");
    }

    #[test]
    fn phase_4_admission_rejects_other_package_symbol_types() {
        let units = [unit(Vec::new(), Vec::new())];
        let error = admit_type(
            &units,
            &units[0],
            FUNCTION_KEY,
            &TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: "skiff.run/std".to_string(),
                    },
                    symbol_path: "std.http.HttpRequest".to_string(),
                    abi_expectation: None,
                },
            },
            false,
            "expression 0 type",
        )
        .expect_err("only the pinned Duration package symbol is admitted");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::ValueShape,
                ..
            }
        ));
    }

    #[test]
    fn phase_3_admission_accepts_union_throw_statement() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let mut function = function();
        function.expressions.push(expression(
            0,
            ExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(1_u64),
                },
            },
            local(0),
        ));
        let statement = statement(
            0,
            MirStmtKind::Throw {
                value: ExprRefIr { expression: 0 },
                payload_type: union(vec![local(0), local(1)]),
                site: synthetic_site(),
            },
        );
        admit_statement(&units, &units[0], FUNCTION_KEY, &function, &statement)
            .expect("a union throw statement on the Phase 2 face is admitted");
    }

    #[test]
    fn phase_3_admission_accepts_catch_expression_and_its_region() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let mut function = function();
        function.slots.push(slot(0, local(0)));
        function.expressions.push(expression(
            0,
            ExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(1_u64),
                },
            },
            number(),
        ));
        function
            .expressions
            .push(expression(1, ExprIr::LoadSlot { slot: 0 }, local(0)));
        function.expressions.push(expression(
            2,
            ExprIr::Catch {
                try_expression: ExprRefIr { expression: 0 },
                catch_slot: 0,
                catch_type: local(0),
                body: ExprRefIr { expression: 1 },
            },
            TypeRefIr::Builtin {
                name: "CatchResult".to_string(),
                args: vec![number(), local(0)],
            },
        ));
        function.regions.push(MirRegion {
            id: 0,
            catch_expr: 2,
            catch_slot: 0,
            catch_type: local(0),
            cleanup_depth: 0,
        });

        admit_exception_regions(&units[0], FUNCTION_KEY, &function)
            .expect("a well-formed catch region is admitted");
        admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &function.expressions[2],
            &BTreeSet::new(),
        )
        .expect("a catch expression on the Phase 2 face is admitted");
    }

    #[test]
    fn phase_3_admission_accepts_rethrow_and_never_types() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let mut function = function();
        function.slots.push(slot(0, local(0)));

        admit_statement(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &statement(0, MirStmtKind::Rethrow { exception_slot: 0 }),
        )
        .expect("a rethrow statement is admitted");

        let rethrow = expression(
            0,
            ExprIr::Rethrow { exception_slot: 0 },
            TypeRefIr::builtin("never"),
        );
        function.expressions.push(rethrow.clone());
        admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &function.expressions[0],
            &BTreeSet::new(),
        )
        .expect("a rethrow expression typed never is admitted");
    }

    #[test]
    fn phase_3_admission_rejects_throw_payload_outside_the_phase_2_face() {
        let units = [unit(Vec::new(), Vec::new())];
        let function = function();
        let error = admit_statement(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &statement(
                0,
                MirStmtKind::Throw {
                    value: ExprRefIr { expression: 0 },
                    payload_type: TypeRefIr::builtin("string"),
                    site: synthetic_site(),
                },
            ),
        )
        .expect_err("string payloads stay rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::ValueShape,
                ..
            }
        ));
    }

    #[test]
    fn phase_3_admission_rejects_missing_catch_region_facts() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let mut function = function();
        function.slots.push(slot(0, local(0)));
        function.expressions.push(expression(
            0,
            ExprIr::Catch {
                try_expression: ExprRefIr { expression: 1 },
                catch_slot: 0,
                catch_type: local(0),
                body: ExprRefIr { expression: 1 },
            },
            TypeRefIr::Builtin {
                name: "CatchResult".to_string(),
                args: vec![number(), local(0)],
            },
        ));
        function
            .expressions
            .push(expression(1, ExprIr::LoadSlot { slot: 0 }, local(0)));
        let error = admit_exception_regions(&units[0], FUNCTION_KEY, &function)
            .expect_err("a Catch node without a region must fail closed");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedConstruct {
                construct: "exception region facts",
                ..
            }
        ));
    }

    #[test]
    fn phase_3_admission_keeps_host_effect_throw_fail_closed() {
        let mut function = function();
        function.expressions.push(expression(
            0,
            ExprIr::Call {
                call: CallIr {
                    target: CallTargetIr::Builtin {
                        op: "hostOp".to_string(),
                    },
                    concrete_receiver: None,
                    site: synthetic_site(),
                    args: Vec::new(),
                    inout_args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            },
            number(),
        ));
        function.blocks.push(MirBlock {
            id: 0,
            label: "entry".to_string(),
            statements: vec![statement(
                0,
                MirStmtKind::Throw {
                    value: ExprRefIr { expression: 0 },
                    payload_type: local(0),
                    site: synthetic_site(),
                },
            )],
            successors: Vec::new(),
        });
        function
            .statements
            .push(skiff_compiler_lowering::mir::MirStatementEntry {
                statement_index: 0,
                span: None,
            });
        let units = [unit(vec![function], two_nominal_types())];
        let error = admit_phase_1_bytecode_mir(&units).expect_err("host targets stay rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::HostTarget,
                ..
            }
        ));
    }

    #[test]
    fn phase_3_admission_accepts_catch_result_tag_discriminator_reads() {
        let catch_type = local(1);
        let catch_result = TypeRefIr::Builtin {
            name: "CatchResult".to_string(),
            args: vec![TypeRefIr::builtin("never"), catch_type.clone()],
        };
        let mut function = function();
        function.slots.push(slot(0, catch_type.clone()));
        function.slots.push(slot(1, catch_result.clone()));
        function.expressions.push(expression(
            0,
            ExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(1_u64),
                },
            },
            number(),
        ));
        function.expressions.push(expression(
            1,
            ExprIr::Construct {
                type_ref: catch_type.clone(),
                fields: BTreeMap::from([("marker".to_string(), ExprRefIr { expression: 0 })]),
            },
            catch_type.clone(),
        ));
        function.expressions.push(expression(
            2,
            ExprIr::Throw {
                value: ExprRefIr { expression: 1 },
                payload_type: catch_type.clone(),
                site: synthetic_site(),
            },
            TypeRefIr::builtin("never"),
        ));
        function.expressions.push(expression(
            3,
            ExprIr::LoadSlot { slot: 0 },
            catch_type.clone(),
        ));
        function.expressions.push(expression(
            4,
            ExprIr::Catch {
                try_expression: ExprRefIr { expression: 2 },
                catch_slot: 0,
                catch_type: catch_type.clone(),
                body: ExprRefIr { expression: 3 },
            },
            catch_result.clone(),
        ));
        function.expressions.push(expression(
            5,
            ExprIr::LoadSlot { slot: 1 },
            catch_result.clone(),
        ));
        function.expressions.push(expression(
            6,
            ExprIr::Field {
                object: ExprRefIr { expression: 5 },
                field: "tag".to_string(),
            },
            TypeRefIr::Union {
                items: vec![
                    TypeRefIr::Literal {
                        value: LiteralIr::String {
                            value: "err".to_string(),
                        },
                    },
                    TypeRefIr::Literal {
                        value: LiteralIr::String {
                            value: "ok".to_string(),
                        },
                    },
                ],
            },
        ));
        function.expressions.push(expression(
            7,
            ExprIr::Literal {
                value: LiteralIr::String {
                    value: "ok".to_string(),
                },
            },
            TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: "ok".to_string(),
                },
            },
        ));
        function.expressions.push(expression(
            8,
            ExprIr::Binary {
                op: skiff_artifact_model::BinaryOpIr::Equal,
                left: ExprRefIr { expression: 6 },
                right: ExprRefIr { expression: 7 },
            },
            TypeRefIr::builtin("bool"),
        ));
        function.regions.push(MirRegion {
            id: 0,
            catch_expr: 4,
            catch_slot: 0,
            catch_type: catch_type.clone(),
            cleanup_depth: 0,
        });

        let units = [unit(Vec::new(), two_nominal_types())];
        let positions = collect_discriminator_literal_positions(&function)
            .expect("discriminator positions collect");
        assert_eq!(
            positions.iter().copied().collect::<Vec<_>>(),
            vec![7],
            "only the tag-equality string literal is a discriminator constant"
        );

        let mut unknown_function = function.clone();
        unknown_function.expressions[6].ty = TypeRefIr::builtin("unknown");
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &unknown_function,
            &unknown_function.expressions[6],
            &positions,
        )
        .expect_err("a materialized tag field cannot retain an unknown type");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::ValueShape,
                ..
            }
        ));

        for index in 4..=8 {
            admit_expression(
                &units,
                &units[0],
                FUNCTION_KEY,
                &function,
                &function.expressions[index],
                &positions,
            )
            .unwrap_or_else(|error| {
                panic!("discriminator expression {index} should be admitted: {error:?}")
            });
        }

        // The narrowed err-branch record load stays a stable LoadSlot fact.
        let narrowed_tag = TypeRefIr::Literal {
            value: LiteralIr::String {
                value: "err".to_string(),
            },
        };
        let narrowed = expression(
            9,
            ExprIr::LoadSlot { slot: 1 },
            TypeRefIr::Record {
                fields: BTreeMap::from([
                    (
                        "exception".to_string(),
                        TypeRefIr::Builtin {
                            name: "Exception".to_string(),
                            args: vec![catch_type],
                        },
                    ),
                    ("tag".to_string(), narrowed_tag),
                ]),
            },
        );
        function.expressions.push(narrowed);
        admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &function.expressions[9],
            &positions,
        )
        .expect("the narrowed CatchResult load is admitted");
    }

    #[test]
    fn phase_3_admission_keeps_non_discriminator_string_literals_fail_closed() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let mut function = function();
        let literal = expression(
            0,
            ExprIr::Literal {
                value: LiteralIr::String {
                    value: "ok".to_string(),
                },
            },
            TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: "ok".to_string(),
                },
            },
        );
        function.expressions.push(literal.clone());
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &function.expressions[0],
            &BTreeSet::new(),
        )
        .expect_err("a bare string literal stays rejected");
        assert!(
            matches!(
                error,
                BytecodeEmissionError::UnsupportedPhase1Capability {
                    capability: Phase1UnsupportedCapability::ValueShape,
                    ..
                }
            ),
            "unexpected rejection: {error:?}"
        );
    }

    #[test]
    fn phase_3_admission_accepts_nominal_record_and_nominal_branch_throws() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let function = function();
        for payload_type in [local(0), union(vec![local(0), local(1)])] {
            admit_statement(
                &units,
                &units[0],
                FUNCTION_KEY,
                &function,
                &statement(
                    0,
                    MirStmtKind::Throw {
                        value: ExprRefIr { expression: 0 },
                        payload_type,
                        site: synthetic_site(),
                    },
                ),
            )
            .unwrap_or_else(|error| {
                panic!("nominal record / nominal-branch throw must be admitted: {error:?}")
            });
        }
    }

    #[test]
    fn phase_3_admission_rejects_identityless_throw_leaves_fail_closed() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let function = function();
        let cases = vec![
            ("scalar", TypeRefIr::builtin("number")),
            (
                "anonymous structural record",
                TypeRefIr::Record {
                    fields: BTreeMap::from([("x".to_string(), number())]),
                },
            ),
            (
                "literal-branch union",
                union(vec![
                    TypeRefIr::Literal {
                        value: LiteralIr::String {
                            value: "ok".to_string(),
                        },
                    },
                    TypeRefIr::Literal {
                        value: LiteralIr::Bool { value: true },
                    },
                ]),
            ),
            (
                "array leaf",
                TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![number()],
                },
            ),
        ];
        for (name, payload_type) in cases {
            let error = admit_statement(
                &units,
                &units[0],
                FUNCTION_KEY,
                &function,
                &statement(
                    0,
                    MirStmtKind::Throw {
                        value: ExprRefIr { expression: 0 },
                        payload_type,
                        site: synthetic_site(),
                    },
                ),
            )
            .expect_err("identity-less throws must fail closed");
            assert!(
                matches!(
                    error,
                    BytecodeEmissionError::UnsupportedPhase1Capability {
                        capability: Phase1UnsupportedCapability::ValueShape,
                        ..
                    }
                ),
                "{name} throw rejected with the wrong shape: {error:?}"
            );
        }
    }
}
