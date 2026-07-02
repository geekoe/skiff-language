use super::env::Env;
use crate::error::{Result, RuntimeError};
use skiff_runtime_linked_program::{
    BinaryOpIr, BlockIr, ExecutableAddr, ExecutableKind, ExprRefIr, LinkedCallTarget,
    LinkedExecutable, LinkedExprIr, LinkedStmtIr, LiteralIr, PatternIr, StmtRefIr, UnitAddr,
};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{runtime_values_equal, RuntimeValue},
};

pub fn validate_program_call_arg_count(executable: &LinkedExecutable, actual: usize) -> Result<()> {
    let expected = executable.params.len();
    if actual == expected {
        return Ok(());
    }
    Err(RuntimeError::Decode(format!(
        "callable {} expects {} argument(s), got {}",
        executable.symbol, expected, actual
    )))
}

pub fn executable_has_explicit_self_binding(executable: &LinkedExecutable) -> bool {
    let Some(parameter) = executable.params.first() else {
        return false;
    };
    if parameter.name == "self" && matches!(executable.kind, ExecutableKind::ImplMethod) {
        return true;
    }
    let slot = parameter.slot;
    executable
        .slots
        .slots
        .iter()
        .any(|binding| binding.index == slot && binding.kind == "selfValue")
}

pub fn program_assembly_index(addr: &ExecutableAddr) -> usize {
    match addr.unit {
        UnitAddr::Service => 0,
        UnitAddr::Package(slot) => slot + 1,
    }
}

pub fn program_block<'a>(executable: &'a LinkedExecutable, label: &str) -> Result<&'a BlockIr> {
    executable
        .body
        .blocks
        .iter()
        .find(|block| block.label == label)
        .or_else(|| {
            (label == "entry")
                .then(|| executable.body.blocks.first())
                .flatten()
        })
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "RuntimeProgram executable {} missing block {label}",
                executable.symbol
            ))
        })
}

pub fn program_statement_ref<'a>(
    executable: &'a LinkedExecutable,
    value: &StmtRefIr,
) -> Result<&'a LinkedStmtIr> {
    let index = program_u32_to_usize(value.statement, "statement ref")?;
    executable.body.statements.get(index).ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!("RuntimeProgram statement {index} is missing"))
    })
}

pub fn program_expression_ref<'a>(
    executable: &'a LinkedExecutable,
    value: ExprRefIr,
) -> Result<&'a LinkedExprIr> {
    let index = program_u32_to_usize(value.expression, "expression ref")?;
    executable.body.expressions.get(index).ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!("RuntimeProgram expression {index} is missing"))
    })
}

pub fn program_literal(value: &LiteralIr) -> Result<RuntimeValue> {
    match value {
        LiteralIr::Null => Ok(RuntimeValue::Null),
        LiteralIr::Bool { value } => Ok(RuntimeValue::Bool(*value)),
        LiteralIr::Number { value } => value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(RuntimeValue::Number)
            .ok_or_else(|| RuntimeError::Decode("expected finite number literal".to_string())),
        LiteralIr::String { value } => Ok(RuntimeValue::String(value.clone())),
    }
}

pub fn program_pattern_matches(
    pattern: &PatternIr,
    value: &RuntimeValue,
    heap: &RequestHeap,
) -> Result<bool> {
    match pattern {
        PatternIr::Wildcard | PatternIr::Binding { .. } => Ok(true),
        PatternIr::Literal { value: literal } => {
            let literal = program_literal(literal)?;
            Ok(runtime_values_equal(heap, &literal, value)?)
        }
        PatternIr::Type { .. } => Err(RuntimeError::Decode(
            "nominal type pattern cannot match an erased runtime value".to_string(),
        )),
    }
}

pub fn bind_program_pattern(env: &mut Env, pattern: &PatternIr, value: RuntimeValue) -> Result<()> {
    if let PatternIr::Binding { slot } = pattern {
        env.declare_binding(
            "slot",
            Some(program_u32_to_usize(*slot, "match.bindingSlot")?),
            value,
        )?;
    }
    Ok(())
}

pub fn program_call_target_kind(target: &LinkedCallTarget) -> &'static str {
    match target {
        LinkedCallTarget::LocalExecutable { .. } => "localExecutable",
        LinkedCallTarget::Executable { .. } => "executable",
        LinkedCallTarget::ExternalServiceSymbol { .. } => "externalServiceSymbol",
        LinkedCallTarget::ServiceDependencySymbol { .. } => "serviceDependencySymbol",
        LinkedCallTarget::PackageSymbol { .. } => "packageSymbol",
        LinkedCallTarget::Native { .. } => "native",
        LinkedCallTarget::Builtin { .. } => "builtin",
        LinkedCallTarget::ReceiverBuiltin { .. } => "receiverBuiltin",
        LinkedCallTarget::InterfaceMethod { .. } => "interfaceMethod",
        LinkedCallTarget::LocalConstReceiverExecutable { .. } => "localConstReceiverExecutable",
    }
}

pub fn program_binary_operator(op: BinaryOpIr) -> &'static str {
    match op {
        BinaryOpIr::Add => "+",
        BinaryOpIr::Subtract => "-",
        BinaryOpIr::Multiply => "*",
        BinaryOpIr::Divide => "/",
        BinaryOpIr::Equal => "==",
        BinaryOpIr::NotEqual => "!=",
        BinaryOpIr::LessThan => "<",
        BinaryOpIr::LessThanOrEqual => "<=",
        BinaryOpIr::GreaterThan => ">",
        BinaryOpIr::GreaterThanOrEqual => ">=",
        BinaryOpIr::And => "&&",
        BinaryOpIr::Or => "||",
    }
}

pub fn program_u32_to_usize(value: u32, label: &str) -> Result<usize> {
    usize::try_from(value)
        .map_err(|_| RuntimeError::InvalidArtifact(format!("RuntimeProgram {label} is too large")))
}
