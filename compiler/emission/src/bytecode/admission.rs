use skiff_artifact_model::{
    BinaryOpIr, CallTargetIr, CallableEffectSummary, ExprIr, LiteralIr, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_lowering::mir::{
    MirCallArgument, MirExecutableKind, MirFunction, MirParamMode, MirStmtKind, MirUnit,
};

use super::{inputs::canonical_function_key, BytecodeEmissionError, Phase1UnsupportedCapability};

/// Admits only the Phase 1 immediate-scalar, synchronous local-call MIR surface.
///
/// This is the production bytecode lane's source-owned capability boundary.
/// It runs before constant evaluation, value-transfer derivation, or bytecode
/// emission and returns no partially emitted state. The admission reads only
/// typed MIR facts; package names and binding strings never grant capability.
pub fn admit_phase_1_bytecode_mir(units: &[MirUnit]) -> Result<(), BytecodeEmissionError> {
    for unit in units {
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
            let capability = if !declaration.type_params.is_empty() {
                Phase1UnsupportedCapability::Generic
            } else if matches!(declaration.descriptor, TypeDescriptorIr::Interface) {
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
        for function in &unit.functions {
            let function_key = canonical_function_key(&unit.module_path, &function.symbol)?;
            admit_function(unit, &function_key, function)?;
        }
    }
    Ok(())
}

fn admit_function(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
) -> Result<(), BytecodeEmissionError> {
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
        unit,
        function_key,
        &function.return_type,
        true,
        "return type",
    )?;
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
            unit,
            function_key,
            &parameter.ty,
            false,
            &format!("parameter {parameter_index} type"),
        )?;
    }
    for slot in &function.slots {
        if slot.writable_local {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Writable,
                &format!("writable slot {}", slot.slot),
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
            unit,
            function_key,
            ty,
            false,
            &format!("slot {} type", slot.slot),
        )?;
    }
    if !function.index_accesses.is_empty() || !function.expression_blocks.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::ValueShape,
            "indexed or value-block source facts",
        ));
    }
    if !function.regions.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Exception,
            "exception regions",
        ));
    }
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
        admit_expression(unit, function_key, expression)?;
    }
    for block in &function.blocks {
        for statement in &block.statements {
            admit_statement(unit, function_key, function, statement)?;
        }
    }
    Ok(())
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
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    statement: &skiff_compiler_lowering::mir::MirStmt,
) -> Result<(), BytecodeEmissionError> {
    let capability = match &statement.kind {
        MirStmtKind::InitSlot { .. } | MirStmtKind::Expr { .. } | MirStmtKind::If { .. } => None,
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
        MirStmtKind::Assign { .. } => Some(Phase1UnsupportedCapability::Writable),
        MirStmtKind::Throw { .. } | MirStmtKind::Rethrow { .. } => {
            Some(Phase1UnsupportedCapability::Exception)
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
    unit: &MirUnit,
    function_key: &str,
    expression: &skiff_compiler_lowering::mir::MirExpression,
) -> Result<(), BytecodeEmissionError> {
    admit_type(
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
        ExprIr::LoadSlot { .. } | ExprIr::Unary { .. } => None,
        ExprIr::Binary { op, .. } => match op {
            BinaryOpIr::And | BinaryOpIr::Or => Some(Phase1UnsupportedCapability::ControlFlow),
            _ => None,
        },
        ExprIr::Call { call } => {
            admit_call(unit, function_key, expression, call)?;
            None
        }
        ExprIr::LoadConst { .. } | ExprIr::LoadPackageConst { .. } => {
            Some(Phase1UnsupportedCapability::Constant)
        }
        ExprIr::ActorSelfField { .. } => Some(Phase1UnsupportedCapability::Actor),
        ExprIr::InterfaceBox { .. } => Some(Phase1UnsupportedCapability::Interface),
        ExprIr::Throw { .. } | ExprIr::Rethrow { .. } | ExprIr::Catch { .. } => {
            Some(Phase1UnsupportedCapability::Exception)
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
        | ExprIr::RepresentationWrap { .. }
        | ExprIr::MapLiteral { .. }
        | ExprIr::ArrayLiteral { .. }
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
    let target_capability = match &call.target {
        CallTargetIr::LocalExecutable { executable_index } => {
            if unit
                .function_by_executable_index(*executable_index)
                .is_err()
            {
                Some(Phase1UnsupportedCapability::NonLocalCallTarget)
            } else {
                None
            }
        }
        CallTargetIr::PublicationExecutable { .. } | CallTargetIr::PackageCallable { .. } => {
            Some(Phase1UnsupportedCapability::NonLocalCallTarget)
        }
        CallTargetIr::ServiceDependencySymbol { .. } | CallTargetIr::ServiceCall { .. } => {
            Some(Phase1UnsupportedCapability::ServiceTarget)
        }
        CallTargetIr::ActorMethod { .. } => Some(Phase1UnsupportedCapability::Actor),
        CallTargetIr::Native { .. }
        | CallTargetIr::Builtin { .. }
        | CallTargetIr::ReceiverBuiltin { .. } => Some(Phase1UnsupportedCapability::HostTarget),
        CallTargetIr::InterfaceMethod { .. } => Some(Phase1UnsupportedCapability::Interface),
    };
    if let Some(capability) = target_capability {
        return Err(rejected_function(
            unit,
            function_key,
            capability,
            &format!("expression {} call target", expression.index),
        ));
    }
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
    Ok(())
}

fn admit_type(
    unit: &MirUnit,
    function_key: &str,
    ty: &TypeRefIr,
    allow_void: bool,
    location: &str,
) -> Result<(), BytecodeEmissionError> {
    if let Some(capability) = unsupported_type_capability(ty, allow_void) {
        return Err(rejected_function(unit, function_key, capability, location));
    }
    Ok(())
}

fn unsupported_type_capability(
    ty: &TypeRefIr,
    allow_void: bool,
) -> Option<Phase1UnsupportedCapability> {
    match ty {
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
