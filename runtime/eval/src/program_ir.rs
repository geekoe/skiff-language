use super::env::Env;
use crate::error::{Result, RuntimeError};
use skiff_runtime_linked_program::{
    BinaryOpIr, BlockIr, ExecutableAddr, ExecutableKind, ExprRefIr, LinkedCallTarget,
    LinkedExecutable, LinkedExprIr, LinkedStmtIr, LiteralIr, PatternIr, RecordPatternFieldIr,
    StmtRefIr, UnitAddr,
};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{runtime_values_equal, HeapNode, RuntimeValue, RuntimeValueCarrier},
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
        PatternIr::Record { fields } => {
            let RuntimeValue::Heap(handle) = value else {
                return Ok(false);
            };
            let HeapNode::Object(object) = heap.get(*handle)? else {
                return Ok(false);
            };
            for field in fields {
                let Some(field_value) = object.fields().get(&field.name) else {
                    return Ok(false);
                };
                if !program_pattern_matches(&field.pattern, field_value, heap)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        PatternIr::Type { .. } => Err(RuntimeError::Decode(
            "nominal type pattern cannot match an erased runtime value".to_string(),
        )),
    }
}

pub fn bind_program_pattern(
    env: &mut Env,
    pattern: &PatternIr,
    value: impl Into<RuntimeValueCarrier>,
    heap: &RequestHeap,
) -> Result<()> {
    match pattern {
        PatternIr::Binding { slot } => {
            env.declare_binding(
                "slot",
                Some(program_u32_to_usize(*slot, "match.bindingSlot")?),
                value.into(),
            )?;
        }
        PatternIr::Record { fields } => {
            let carrier = value.into();
            let RuntimeValue::Heap(handle) = carrier.value() else {
                return Err(RuntimeError::Decode(
                    "record pattern can only bind a runtime object".to_string(),
                ));
            };
            let HeapNode::Object(object) = heap.get(*handle)? else {
                return Err(RuntimeError::Decode(
                    "record pattern can only bind a runtime object".to_string(),
                ));
            };
            for field in fields {
                let Some(field_value) = object.fields().get(&field.name) else {
                    return Err(RuntimeError::Decode(format!(
                        "record pattern field `{}` is missing from the matched runtime object",
                        field.name
                    )));
                };
                bind_program_pattern(env, &field.pattern, field_value.clone(), heap)?;
            }
        }
        PatternIr::Wildcard | PatternIr::Literal { .. } | PatternIr::Type { .. } => {}
    }
    Ok(())
}

pub fn program_call_target_kind(target: &LinkedCallTarget) -> &'static str {
    match target {
        LinkedCallTarget::LocalExecutable { .. } => "localExecutable",
        LinkedCallTarget::PublicationExecutable { .. } => "publicationExecutable",
        LinkedCallTarget::Executable { .. } => "executable",
        LinkedCallTarget::ServiceDependencySymbol { .. } => "serviceDependencySymbol",
        LinkedCallTarget::PackageSymbol { .. } => "packageSymbol",
        LinkedCallTarget::PackageDirect { .. } => "packageDirect",
        LinkedCallTarget::ActivationRelativeService { .. } => "activationRelativeService",
        LinkedCallTarget::Native { .. } => "native",
        LinkedCallTarget::Builtin { .. } => "builtin",
        LinkedCallTarget::ReceiverBuiltin { .. } => "receiverBuiltin",
        LinkedCallTarget::InterfaceMethod { .. } => "interfaceMethod",
        LinkedCallTarget::LocalConstReceiverExecutable { .. } => "localConstReceiverExecutable",
        LinkedCallTarget::ActorMethod { .. } => "actorMethod",
        LinkedCallTarget::ActorDispatch { .. } => "actorDispatch",
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

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_runtime_linked_program::{
        ExecutableKind, LinkedExecutable, LinkedExecutableBody, SlotIr, SlotLayoutIr,
    };
    use skiff_runtime_model::runtime_value::{RuntimeObject, RuntimeObjectFields};

    fn kind_literal(kind: &str) -> RecordPatternFieldIr {
        RecordPatternFieldIr {
            name: "kind".to_string(),
            pattern: PatternIr::Literal {
                value: LiteralIr::String {
                    value: kind.to_string(),
                },
            },
        }
    }

    fn object_value(heap: &mut RequestHeap, fields: RuntimeObjectFields) -> RuntimeValue {
        let handle = heap
            .alloc_object(RuntimeObject::unshaped(fields))
            .expect("object allocation should succeed");
        RuntimeValue::Heap(handle)
    }

    fn test_env() -> Env {
        let executable = LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "test.record_pattern".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr {
                slots: vec![
                    SlotIr {
                        index: 7,
                        name: "detail".to_string(),
                        kind: "pattern".to_string(),
                    },
                    SlotIr {
                        index: 9,
                        name: "state".to_string(),
                        kind: "pattern".to_string(),
                    },
                ],
                frame_size: 10,
            },
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        };
        Env::for_program_executable(&executable, None, 0)
            .expect("test executable slot layout should validate")
    }

    #[test]
    fn record_pattern_matches_kind_literal_and_binds_bare_fields() {
        let mut heap = RequestHeap::default();
        let pattern = PatternIr::Record {
            fields: vec![
                kind_literal("succeeded"),
                RecordPatternFieldIr {
                    name: "detail".to_string(),
                    pattern: PatternIr::Binding { slot: 7 },
                },
            ],
        };
        let value = object_value(
            &mut heap,
            RuntimeObjectFields::from([
                (
                    "kind".to_string(),
                    RuntimeValue::String("succeeded".to_string()),
                ),
                ("detail".to_string(), RuntimeValue::String("ok".to_string())),
            ]),
        );

        assert!(
            program_pattern_matches(&pattern, &value, &heap).expect("record pattern should match"),
            "matching kind literal must select the arm"
        );
        let mut env = test_env();
        bind_program_pattern(&mut env, &pattern, value, &heap)
            .expect("record bindings should bind");
        assert_eq!(
            env.get_binding("slot", Some(7))
                .expect("bare field slot should be bound")
                .value(),
            &RuntimeValue::String("ok".to_string()),
            "bare record pattern field must bind the field value"
        );
    }

    #[test]
    fn record_pattern_kind_discriminates_arms_in_any_order() {
        let mut heap = RequestHeap::default();
        let succeeded = PatternIr::Record {
            fields: vec![kind_literal("succeeded")],
        };
        let failed = PatternIr::Record {
            fields: vec![kind_literal("failed")],
        };
        let value = object_value(
            &mut heap,
            RuntimeObjectFields::from([(
                "kind".to_string(),
                RuntimeValue::String("failed".to_string()),
            )]),
        );

        assert!(
            !program_pattern_matches(&succeeded, &value, &heap).expect("first arm should miss"),
            "non-matching kind literal must not select the earlier arm"
        );
        assert!(
            program_pattern_matches(&failed, &value, &heap).expect("second arm should match"),
            "matching kind literal must select the later arm (arm order is user-controlled)"
        );
    }

    #[test]
    fn record_pattern_unknown_kind_matches_no_literal_arm() {
        let mut heap = RequestHeap::default();
        let succeeded = PatternIr::Record {
            fields: vec![kind_literal("succeeded")],
        };
        let failed = PatternIr::Record {
            fields: vec![kind_literal("failed")],
        };
        let value = object_value(
            &mut heap,
            RuntimeObjectFields::from([(
                "kind".to_string(),
                RuntimeValue::String("expired".to_string()),
            )]),
        );

        assert!(!program_pattern_matches(&succeeded, &value, &heap).expect("match check"));
        assert!(!program_pattern_matches(&failed, &value, &heap).expect("match check"));
    }

    #[test]
    fn record_pattern_missing_field_does_not_match() {
        let mut heap = RequestHeap::default();
        let pattern = PatternIr::Record {
            fields: vec![
                kind_literal("succeeded"),
                RecordPatternFieldIr {
                    name: "detail".to_string(),
                    pattern: PatternIr::Binding { slot: 7 },
                },
            ],
        };
        let value = object_value(
            &mut heap,
            RuntimeObjectFields::from([(
                "kind".to_string(),
                RuntimeValue::String("succeeded".to_string()),
            )]),
        );

        assert!(
            !program_pattern_matches(&pattern, &value, &heap).expect("match check"),
            "record pattern must not match when a bound field is absent"
        );
    }

    #[test]
    fn record_pattern_nested_record_binds_inner_field() {
        let mut heap = RequestHeap::default();
        let pattern = PatternIr::Record {
            fields: vec![
                kind_literal("ok"),
                RecordPatternFieldIr {
                    name: "body".to_string(),
                    pattern: PatternIr::Record {
                        fields: vec![RecordPatternFieldIr {
                            name: "state".to_string(),
                            pattern: PatternIr::Binding { slot: 9 },
                        }],
                    },
                },
            ],
        };
        let body = object_value(
            &mut heap,
            RuntimeObjectFields::from([(
                "state".to_string(),
                RuntimeValue::String("ready".to_string()),
            )]),
        );
        let value = object_value(
            &mut heap,
            RuntimeObjectFields::from([
                ("kind".to_string(), RuntimeValue::String("ok".to_string())),
                ("body".to_string(), body),
            ]),
        );

        assert!(
            program_pattern_matches(&pattern, &value, &heap).expect("nested record should match")
        );
        let mut env = test_env();
        bind_program_pattern(&mut env, &pattern, value, &heap).expect("nested binding should bind");
        assert_eq!(
            env.get_binding("slot", Some(9))
                .expect("nested field slot should be bound")
                .value(),
            &RuntimeValue::String("ready".to_string())
        );
    }

    #[test]
    fn record_pattern_does_not_match_scalar_values() {
        let heap = RequestHeap::default();
        let pattern = PatternIr::Record { fields: Vec::new() };
        assert!(
            !program_pattern_matches(&pattern, &RuntimeValue::Null, &heap).expect("match check"),
            "record pattern must not match a scalar value"
        );
    }
}
