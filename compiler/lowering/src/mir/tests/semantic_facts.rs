use super::*;

use skiff_artifact_model::{InOutPathSegmentIr, PatternIr};

const WRITABLE_FIXTURE: &str = r#"
  type Inner { values: Array<number>, count: number }
  type Outer { inner: Inner }

  function inc(inout value: number) -> void {
    value = value + 1
  }

  function run() -> number {
    var outer = Outer { inner: Inner { values: [1], count: 0 } }
    outer.inner.count = 2
    outer.inner.values.push(3)
    inc(inout outer.inner.count)
    return outer.inner.count
  }
"#;

const RECORD_PATTERN_FIXTURE: &str = r#"
  function run(payload: { kind: string, body: { state: string } }) -> string {
    match payload {
      { kind: "ok", body: { state } } => {
        return state
      }
      _ => {
        return "other"
      }
    }
  }
"#;

#[test]
fn writable_places_and_inout_loans_remain_owned_after_file_ir_drop() {
    let model = build_model(MODULE, WRITABLE_FIXTURE);
    let lowered = lower(&model).expect("writable fixture should lower");
    let function = lowered.mir_units()[0]
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.run"))
        .expect("run MIR")
        .clone();
    drop(lowered);
    drop(model);

    let outer_slot = function
        .slots
        .iter()
        .find(|slot| slot.name == "outer")
        .expect("outer slot")
        .slot;
    let assignment_place = function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            MirStmtKind::Assign { place, .. }
                if place.path
                    == vec![
                        crate::mir::MirWritablePathSegment::Field {
                            name: "inner".to_string(),
                        },
                        crate::mir::MirWritablePathSegment::Field {
                            name: "count".to_string(),
                        },
                    ] =>
            {
                Some(place)
            }
            _ => None,
        })
        .expect("nested assignment place");
    assert_eq!(
        assignment_place.root,
        MirWritableRoot::Slot { slot: outer_slot }
    );

    let (mutating_expression, mutating_place) = function
        .expressions
        .iter()
        .find_map(|expression| {
            expression
                .writable
                .as_ref()
                .and_then(|facts| facts.mutating_receiver.as_ref())
                .map(|place| (expression, place))
        })
        .expect("mutating receiver place");
    let ExprIr::Call { call } = &mutating_expression.expression else {
        unreachable!()
    };
    assert!(matches!(&call.site, InstructionSourceSite::Source { .. }));
    assert_eq!(
        mutating_place,
        &MirWritablePlace {
            root: MirWritableRoot::Slot { slot: outer_slot },
            path: vec![
                crate::mir::MirWritablePathSegment::Field {
                    name: "inner".to_string(),
                },
                crate::mir::MirWritablePathSegment::Field {
                    name: "values".to_string(),
                },
            ],
        }
    );
    let checked_mutating = function
        .call_writable_facts(ExprRefIr {
            expression: mutating_expression.index,
        })
        .expect("checked mutating facts")
        .expect("stored mutating facts");
    assert_eq!(
        checked_mutating,
        mutating_expression.writable.as_ref().unwrap()
    );

    let (loan_expression, loan) = function
        .expressions
        .iter()
        .find_map(|expression| {
            expression
                .writable
                .as_ref()
                .and_then(|facts| facts.inout_loans.first())
                .map(|loan| (expression, loan))
        })
        .expect("inout loan");
    assert_eq!(loan.loan_ordinal, 0);
    assert_eq!(loan.root_slot, outer_slot);
    assert_eq!(
        loan.path,
        vec![
            InOutPathSegmentIr::Field {
                name: "inner".to_string(),
            },
            InOutPathSegmentIr::Field {
                name: "count".to_string(),
            },
        ]
    );
    let checked_loan = function
        .call_writable_facts(ExprRefIr {
            expression: loan_expression.index,
        })
        .expect("checked inout facts")
        .expect("stored inout facts");
    assert_eq!(checked_loan, loan_expression.writable.as_ref().unwrap());
}

#[test]
fn inout_index_without_an_operand_is_rejected_structurally() {
    let model = build_model(MODULE, WRITABLE_FIXTURE);
    let lowered = lower(&model).expect("writable fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let run_index = units[0].declarations.executables["run"].executable_index as usize;
    let call = units[0].executables[run_index]
        .body
        .expressions
        .iter_mut()
        .find_map(|expression| match expression {
            ExprIr::Call { call } if !call.inout_args.is_empty() => Some(call),
            _ => None,
        })
        .expect("inout call");
    call.inout_args[0].path.push(InOutPathSegmentIr::Index);

    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("operand-less inout index must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidWritableFacts {
            expression: _,
            ref message,
            ..
        } if message.contains("does not retain its index operand")
    ));
}

#[test]
fn nested_record_patterns_are_retained_and_validated_recursively() {
    let model = build_model(MODULE, RECORD_PATTERN_FIXTURE);
    let lowered = lower(&model).expect("record-pattern fixture should lower");
    let function = lowered.mir_units()[0]
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.run"))
        .expect("run MIR")
        .clone();
    drop(lowered);
    drop(model);

    let pattern = function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            MirStmtKind::Match { arms, .. } => arms.first().map(|arm| &arm.pattern),
            _ => None,
        })
        .expect("record match arm");
    let PatternIr::Record { fields } = pattern else {
        panic!("outer record pattern")
    };
    let body = fields
        .iter()
        .find(|field| field.name == "body")
        .expect("body field");
    let PatternIr::Record { fields } = &body.pattern else {
        panic!("nested body record pattern")
    };
    let PatternIr::Binding { slot } = &fields[0].pattern else {
        panic!("nested state binding")
    };
    assert_eq!(
        function.slot(*slot).expect("pattern slot").kind,
        MirSlotKind::Pattern
    );
}

#[test]
fn duplicate_nested_record_pattern_fields_are_rejected() {
    let model = build_model(MODULE, RECORD_PATTERN_FIXTURE);
    let lowered = lower(&model).expect("record-pattern fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let run_index = units[0].declarations.executables["run"].executable_index as usize;
    let pattern = units[0].executables[run_index]
        .body
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            StmtIr::Match { arms, .. } => Some(&mut arms[0].pattern),
            _ => None,
        })
        .expect("record match arm");
    let PatternIr::Record { fields } = pattern else {
        panic!("outer record pattern")
    };
    fields.push(fields[0].clone());

    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("duplicate pattern field must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidControlFlow { ref message, .. }
            if message.contains("record pattern repeats field")
    ));
}

#[test]
fn for_in_item_type_must_match_the_owned_iterable_type() {
    let model = build_model(MODULE, MIR_FIXTURE);
    let lowered = lower(&model).expect("MIR fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let mirror_index = units[0].declarations.executables["mirror"].executable_index as usize;
    let item_type = units[0].executables[mirror_index]
        .body
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            StmtIr::ForIn { item_type, .. } => Some(item_type),
            _ => None,
        })
        .expect("for item type");
    *item_type = Some(TypeRefIr::builtin("string"));

    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("mismatched for item type must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidControlFlow { ref message, .. }
            if message.contains("does not match iterable-derived type")
    ));

    let mut units = lowered.file_ir_units().to_vec();
    let item_type = units[0].executables[mirror_index]
        .body
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            StmtIr::ForIn { item_type, .. } => Some(item_type),
            _ => None,
        })
        .expect("for item type");
    *item_type = None;
    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("missing for item type must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidControlFlow { ref message, .. }
            if message.contains("has no exact item_type fact")
    ));
}

#[test]
fn map_entry_for_in_owns_exact_key_and_value_types() {
    let model = build_model(
        MODULE,
        r#"
          function sum(input: Map<string, number>) -> number {
            var total = 0
            for key, value in input {
              total = total + value
            }
            return total
          }
        "#,
    );
    let lowered = lower(&model).expect("map-entry fixture should lower");
    let function = lowered.mir_units()[0]
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.sum"))
        .expect("sum MIR");
    let facts = function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            MirStmtKind::ForIn { facts, .. } => Some(facts),
            _ => None,
        })
        .expect("map-entry for facts");
    assert_eq!(
        facts.iterable_type,
        TypeRefIr::Builtin {
            name: "Map".to_string(),
            args: vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number"),],
        }
    );
    let MirForInBinding::MapEntry {
        key_slot,
        key_type,
        value_slot,
        value_type,
    } = &facts.binding
    else {
        panic!("map entry binding facts")
    };
    assert_ne!(key_slot, value_slot);
    assert_eq!(key_type, &TypeRefIr::builtin("string"));
    assert_eq!(value_type, &TypeRefIr::builtin("number"));
}

#[test]
fn assert_condition_type_and_statement_span_table_are_checked() {
    let model = build_model(
        MODULE,
        r#"
          function check() -> void {
            assert true, "must stay true"
            return
          }
        "#,
    );
    let lowered = lower(&model).expect("assert fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let check_index = units[0].declarations.executables["check"].executable_index as usize;
    let condition = units[0].executables[check_index]
        .body
        .statements
        .iter()
        .find_map(|statement| match statement {
            StmtIr::Assert { condition, .. } => Some(condition.expression),
            _ => None,
        })
        .expect("assert condition");
    units[0].executables[check_index].expression_types[condition as usize] =
        TypeRefIr::builtin("number");
    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("non-bool assert condition must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidControlFlow { ref message, .. }
            if message.contains("assert condition has type")
    ));

    let mut units = lowered.file_ir_units().to_vec();
    let executable = &mut units[0].executables[check_index];
    let statement_count = executable.body.statements.len();
    executable.statement_spans.pop();
    let statement_span_count = executable.statement_spans.len();
    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("mismatched statement span table must fail closed");
    assert_eq!(
        error,
        MirBuildError::StatementSpanCountMismatch {
            module_path: MODULE.to_string(),
            symbol: format!("{MODULE}.check"),
            statement_count,
            statement_span_count,
        }
    );
}

#[test]
fn timeout_requires_a_positive_duration() {
    let model = build_model(MODULE, MIR_FIXTURE);
    let lowered = lower(&model).expect("MIR fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let mirror_index = units[0].declarations.executables["mirror"].executable_index as usize;
    let duration = units[0].executables[mirror_index]
        .body
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            StmtIr::Timeout { duration_ms, .. } => Some(duration_ms),
            _ => None,
        })
        .expect("timeout duration");
    *duration = 0;

    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("zero timeout duration must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidControlFlow { ref message, .. }
            if message.contains("has zero duration")
    ));
}
