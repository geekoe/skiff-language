use crate::file_ir::{CallTargetIr, ExecutableIr, ExprIr};
use skiff_artifact_model::StmtIr;

use super::{lowered_unit, lowered_units};

fn direct_return_calls(executable: &ExecutableIr) -> Vec<&skiff_artifact_model::CallIr> {
    executable
        .body
        .statements
        .iter()
        .filter_map(|statement| {
            let StmtIr::Return { value: Some(value) } = statement else {
                return None;
            };
            let ExprIr::Call { call } = &executable.body.expressions[value.expression as usize]
            else {
                return None;
            };
            Some(call)
        })
        .collect()
}

fn only_value_return_expression(executable: &ExecutableIr) -> &ExprIr {
    let expressions = executable
        .body
        .statements
        .iter()
        .filter_map(|statement| {
            let StmtIr::Return { value: Some(value) } = statement else {
                return None;
            };
            Some(&executable.body.expressions[value.expression as usize])
        })
        .collect::<Vec<_>>();
    assert_eq!(
        expressions.len(),
        1,
        "{} should have exactly one value return",
        executable.symbol
    );
    expressions[0]
}

fn assert_exact_local_return_call(executable: &ExecutableIr, expected_executable_index: u32) {
    let calls = direct_return_calls(executable);
    assert_eq!(
        calls.len(),
        1,
        "{} should select exactly one direct call from Return.value",
        executable.symbol
    );
    assert!(
        matches!(
            calls[0].target,
            CallTargetIr::LocalExecutable { executable_index }
                if executable_index == expected_executable_index
        ),
        "{} Return.value selected an unexpected call target: {:?}",
        executable.symbol,
        calls[0].target
    );
}

#[test]
fn return_value_selects_exact_local_recursive_call_shapes() {
    let unit = lowered_unit(
        r#"
          type Box<T> {
            value: T,
          }

          function direct(value: integer) -> integer {
            return direct(value)
          }

          function left(value: integer) -> integer {
            return right(value)
          }

          function right(value: integer) -> integer {
            return left(value)
          }

          function generic<T>(value: T) -> T {
            return generic<T>(value)
          }

          impl Box<T> {
            function retry() -> T {
              return self.retry()
            }
          }

          function wrapped(value: number) -> number {
            return 1 + wrapped(value)
          }

          function staged(value: integer) -> integer {
            let next = staged(value)
            return next
          }
        "#,
    );

    for (caller, callee) in [
        ("direct", "direct"),
        ("left", "right"),
        ("right", "left"),
        ("generic", "generic"),
        ("Box<T>.retry", "Box<T>.retry"),
    ] {
        let caller_index = unit.declarations.executables[caller].executable_index;
        let callee_index = unit.declarations.executables[callee].executable_index;
        assert_exact_local_return_call(&unit.executables[caller_index as usize], callee_index);
    }

    for name in ["wrapped", "staged"] {
        let executable_index = unit.declarations.executables[name].executable_index;
        let executable = &unit.executables[executable_index as usize];
        assert!(
            direct_return_calls(executable).is_empty(),
            "{name} must not expose its nested/earlier call as Return.value"
        );
        assert!(
            executable
                .body
                .expressions
                .iter()
                .any(|expression| matches!(
                    expression,
                    ExprIr::Call { call }
                        if matches!(
                            call.target,
                            CallTargetIr::LocalExecutable { executable_index: target }
                                if target == executable_index
                        )
                )),
            "{name} fixture must still contain its recursive call"
        );
    }
    assert!(matches!(
        only_value_return_expression(
            &unit.executables[unit.declarations.executables["wrapped"].executable_index as usize]
        ),
        ExprIr::Binary { .. }
    ));
    assert!(matches!(
        only_value_return_expression(
            &unit.executables[unit.declarations.executables["staged"].executable_index as usize]
        ),
        ExprIr::LoadSlot { .. }
    ));
}

#[test]
fn return_value_selects_exact_cross_module_mutual_calls() {
    let units = lowered_units(vec![
        (
            "internal/alpha.skiff",
            "internal.alpha",
            r#"
              function ping(value: integer) -> integer {
                return root.internal.beta.pong(value)
              }
            "#,
        ),
        (
            "internal/beta.skiff",
            "internal.beta",
            r#"
              function pong(value: integer) -> integer {
                return root.internal.alpha.ping(value)
              }
            "#,
        ),
    ]);

    for (caller_module, caller_name, callee_module, callee_name) in [
        ("internal.alpha", "ping", "internal.beta", "pong"),
        ("internal.beta", "pong", "internal.alpha", "ping"),
    ] {
        let caller_unit = units
            .iter()
            .find(|unit| unit.module_path == caller_module)
            .unwrap();
        let callee_unit = units
            .iter()
            .find(|unit| unit.module_path == callee_module)
            .unwrap();
        let caller_index = caller_unit.declarations.executables[caller_name].executable_index;
        let callee_index = callee_unit.declarations.executables[callee_name].executable_index;
        let calls = direct_return_calls(&caller_unit.executables[caller_index as usize]);
        assert_eq!(calls.len(), 1);
        assert!(matches!(
            &calls[0].target,
            CallTargetIr::PublicationExecutable {
                module_path,
                executable_index,
            } if module_path == callee_module && *executable_index == callee_index
        ));
    }
}
