use std::collections::BTreeSet;

use skiff_artifact_model::{
    AssignTargetIr, BinaryOpIr, CallTargetIr, CallableEffectSummary, ExprIr, ExprRefIr, LiteralIr,
    NamedUnionBranchIr, StatementAttributionId, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_lowering::mir::{
    MirCallArgument, MirEmissionAnchor, MirExecutableKind, MirFunction, MirParamMode, MirSlotKind,
    MirStmtKind, MirUnit, MirWritableRoot,
};

use super::{
    inputs::canonical_function_key, BytecodeEmissionError, Phase1MirFactMismatch,
    Phase1UnsupportedCapability,
};

/// Opaque proof that one exact MIR slice passed the Phase 1 bytecode boundary.
///
/// The proof cannot be constructed and exposes no MIR accessor through the
/// public API. Public planning and emission entry points therefore accept only
/// source facts checked by [`admit_phase_1_bytecode_mir`].
#[derive(Debug)]
pub struct AdmittedPhase1BytecodeMir<'a> {
    units: &'a [MirUnit],
}

impl<'a> AdmittedPhase1BytecodeMir<'a> {
    pub(crate) fn units(&self) -> &'a [MirUnit] {
        self.units
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
pub fn admit_phase_1_bytecode_mir(
    units: &[MirUnit],
) -> Result<AdmittedPhase1BytecodeMir<'_>, BytecodeEmissionError> {
    for unit in units {
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
            admit_function(units, unit, &function_key, function)?;
        }
    }
    Ok(AdmittedPhase1BytecodeMir { units })
}

fn admit_function(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
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
    admit_type(
        units,
        unit,
        function_key,
        &function.return_type,
        true,
        "return type",
    )?;
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
        admit_type(
            units,
            unit,
            function_key,
            &parameter.ty,
            false,
            &format!("parameter {parameter_index} type"),
        )?;
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
        admit_type(
            units,
            unit,
            function_key,
            ty,
            false,
            &format!("slot {} type", slot.slot),
        )?;
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
    if function.stream_result.is_some() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Stream,
            "stream result facts",
        ));
    }
    admit_effects(unit, function_key, &function.effect_summary)?;
    for expression in &function.expressions {
        admit_expression(units, unit, function_key, function, expression)?;
    }
    for block in &function.blocks {
        for statement in &block.statements {
            admit_statement(units, unit, function_key, function, statement)?;
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
                &format!("catch expression {} has no exception region", expression.index),
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
        location: format!(" in module `{}` function `{function_key}`: {detail}", unit.module_path),
    }
}

fn admit_effects(
    unit: &MirUnit,
    function_key: &str,
    summary: &CallableEffectSummary,
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
    if effects.may_pending || effects.may_pending() || !effects.pending_effect_categories.is_empty()
    {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::PendingEffect,
            "callable pending effects",
        ));
    }
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

fn admit_statement(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    statement: &skiff_compiler_lowering::mir::MirStmt,
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
            if value
                .as_ref()
                .is_some_and(|value| is_tail_local_call(function, value.expression))
            {
                Some(Phase1UnsupportedCapability::TailCall)
            } else {
                None
            }
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
            admit_type(
                units,
                unit,
                function_key,
                payload_type,
                false,
                &format!(
                    "statement {} throw payload type",
                    statement.statement_index
                ),
            )?;
            None
        }
        MirStmtKind::Rethrow { exception_slot } => {
            function.slot(*exception_slot)?;
            None
        }
        MirStmtKind::Emit { .. } | MirStmtKind::StreamNext { .. } => {
            Some(Phase1UnsupportedCapability::Stream)
        }
        MirStmtKind::TestEffectRegister { .. } => Some(Phase1UnsupportedCapability::HostTarget),
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

fn admit_expression(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
) -> Result<(), BytecodeEmissionError> {
    if let ExprIr::Construct { type_ref, .. } = &expression.expression {
        admit_type(
            units,
            unit,
            function_key,
            type_ref,
            false,
            &format!("expression {} construct type", expression.index),
        )?;
    }
    admit_type(
        units,
        unit,
        function_key,
        &expression.ty,
        true,
        &format!("expression {} type", expression.index),
    )?;
    if expression.writable.is_some() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::InOut,
            &format!("expression {} writable facts", expression.index),
        ));
    }
    if expression.stream_result.is_some() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Stream,
            &format!("expression {} stream facts", expression.index),
        ));
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
            LiteralIr::String { .. } => Some(Phase1UnsupportedCapability::ValueShape),
        },
        ExprIr::LoadSlot { slot } => {
            let slot_type = function.slot_type(*slot)?;
            if slot_type != &expression.ty {
                return Err(fact_mismatch(
                    unit,
                    function_key,
                    Phase1MirFactMismatch::LoadSlotType,
                    &format!("expression {} load slot {slot}", expression.index),
                ));
            }
            None
        }
        ExprIr::Unary { .. } => None,
        ExprIr::Binary { op, .. } => match op {
            BinaryOpIr::And | BinaryOpIr::Or => Some(Phase1UnsupportedCapability::ControlFlow),
            _ => None,
        },
        ExprIr::Call { call } => {
            admit_call(unit, function_key, function, expression, call)?;
            None
        }
        ExprIr::LoadConst { .. } | ExprIr::LoadPackageConst { .. } => {
            Some(Phase1UnsupportedCapability::Constant)
        }
        ExprIr::ActorSelfField { .. } => Some(Phase1UnsupportedCapability::Actor),
        ExprIr::InterfaceBox { .. } => Some(Phase1UnsupportedCapability::Interface),
        ExprIr::Throw { payload_type, .. } => {
            admit_type(
                units,
                unit,
                function_key,
                payload_type,
                false,
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
        CallTargetIr::ServiceDependencySymbol { .. } | CallTargetIr::ServiceCall { .. } => {
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
        CallTargetIr::Native { .. }
        | CallTargetIr::Builtin { .. }
        | CallTargetIr::ReceiverBuiltin { .. } => {
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

fn admit_type(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    ty: &TypeRefIr,
    allow_void: bool,
    location: &str,
) -> Result<(), BytecodeEmissionError> {
    let mut context = TypeAdmissionContext {
        units,
        function_key,
        nominal_chain: Vec::new(),
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
                admit_type_nested(
                    context,
                    unit,
                    field_ty,
                    false,
                    &format!("{location} field `{name}`"),
                    true,
                )?;
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
        FileIrUnit, InstructionSourceSite, LiteralIr, PackageCallableId,
        SyntheticInstructionSiteReason, TypeDeclIr, TypeDescriptorIr, TypeRefIr,
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
        TypeRefIr::LocalType {
            type_index: index,
        }
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
        function.expressions.push(expression(
            1,
            ExprIr::LoadSlot { slot: 0 },
            local(0),
        ));
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
            &statement(
                0,
                MirStmtKind::Rethrow { exception_slot: 0 },
            ),
        )
        .expect("a rethrow statement is admitted");

        let rethrow = expression(0, ExprIr::Rethrow { exception_slot: 0 }, TypeRefIr::builtin("never"));
        function.expressions.push(rethrow.clone());
        admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &function.expressions[0],
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
        function.expressions.push(expression(
            1,
            ExprIr::LoadSlot { slot: 0 },
            local(0),
        ));
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
        function.statements.push(skiff_compiler_lowering::mir::MirStatementEntry {
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
}
