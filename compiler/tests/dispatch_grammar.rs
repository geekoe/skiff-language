mod common;

#[cfg(test)]
mod tests {
    use super::common::{package_project::compile_package_project, TestDir};
    use skiff_artifact_model::{LiteralIr, PatternIr, StmtIr};

    fn package_with_source(name: &str, source: &str) -> TestDir {
        let temp = TestDir::new("skiff-compiler", name);
        temp.write(
            "package.yml",
            "id: example.com/dispatch-grammar-fixture\nversion: 1.0.0\n",
        );
        temp.write("api.yml", "start: main.start\n");
        temp.write("main.skiff", source);
        temp
    }

    fn compile_error(temp: TestDir) -> String {
        compile_package_project(temp.path())
            .expect_err("package compile should fail")
            .to_string()
    }

    #[test]
    fn dispatch_expressions_compile_through_the_full_package_pipeline() {
        let temp = package_with_source(
            "dispatch-expression-pipeline",
            r#"
            type Instant = Date

            function run(input: string) -> void {
              return
            }

            function consume(ref: std.task.TaskRef) -> void {
              return
            }

            function start(input: string, instant: Instant) -> std.task.TaskRef {
              dispatch run(input)
              let ref = dispatch run(input) after(200ms)
              let scheduled = dispatch run(input) at(instant)
              consume(dispatch run(input) after(0ms))
              return ref
            }
        "#,
        );
        compile_package_project(temp.path()).expect("dispatch expressions should compile");
    }

    #[test]
    fn dispatch_keyword_is_reserved_across_all_user_declaration_kinds() {
        for (name, source, expected) in [
        (
            "reserved-function",
            "function start() -> void { return }\nfunction dispatch() -> void { return }\n",
            "function dispatch uses reserved prelude name",
        ),
        (
            "reserved-type",
            "function start() -> void { return }\ntype dispatch { value: string }\n",
            "type dispatch uses reserved prelude name",
        ),
        (
            "reserved-alias",
            "function start() -> void { return }\nalias dispatch = string\n",
            "alias dispatch uses reserved prelude name",
        ),
        (
            "reserved-interface",
            "function start() -> void { return }\ninterface dispatch { function run() -> void }\n",
            "interface dispatch uses reserved prelude name",
        ),
        (
            "reserved-const",
            "function start() -> void { return }\nconst dispatch = \"reserved\"\n",
            "const dispatch uses reserved prelude name",
        ),
    ] {
        let error = compile_error(package_with_source(name, source));
        assert!(
            error.contains(expected),
            "{name} should reject dispatch with {expected:?}: {error}"
        );
    }
    }

    #[test]
    fn dispatch_keyword_is_reserved_for_local_and_pattern_bindings() {
        for (name, source) in [
            (
                "reserved-local-binding",
                r#"
                function start() -> void {
                  let dispatch = 1
                }
            "#,
            ),
            (
                "reserved-pattern-binding",
                r#"
                type Doc { value: number }

                function start() -> void {
                  match (Doc { value: 1 }) {
                    Doc { value: dispatch } => { return }
                  }
                }
            "#,
            ),
        ] {
            let error = compile_error(package_with_source(name, source));
            assert!(
                error.contains("dispatch uses reserved prelude name"),
                "{name} should reject a dispatch binding: {error}"
            );
        }
    }

    #[test]
    fn std_task_status_and_cancel_compile_through_the_full_package_pipeline() {
        let temp = package_with_source(
            "std-task-surface-pipeline",
            r#"
            function run() -> void {
              return
            }

            function start() -> std.task.TaskStatus {
              let ref = dispatch run()
              return std.task.status(ref)
            }

            function stop() -> std.task.TaskCancelResult {
              let ref = dispatch run()
              return std.task.cancel(ref)
            }

            function consumeStatus(status: std.task.TaskStatus) -> void {
              return
            }

            function statusKind(status: std.task.TaskStatus) -> string {
              match status {
                { kind: "succeeded" } => {
                  return "succeeded"
                }
                { kind: "failed" } => {
                  return "failed"
                }
                _ => {
                  return "other"
                }
              }
            }

            function cancelKind(result: std.task.TaskCancelResult) -> string {
              match result {
                { kind: "alreadyStarted" } => {
                  return "alreadyStarted"
                }
                _ => {
                  return "other"
                }
              }
            }
        "#,
        );
        let project =
            compile_package_project(temp.path()).expect("std.task status/cancel should compile");
        let unit = project
            .package
            .file_ir_units
            .iter()
            .find(|unit| {
                unit.unit
                    .executables
                    .iter()
                    .any(|executable| executable.symbol.ends_with("statusKind"))
            })
            .expect("statusKind file IR should be emitted");
        let status_kind = unit
            .unit
            .executables
            .iter()
            .find(|executable| executable.symbol.ends_with("statusKind"))
            .expect("statusKind executable should be emitted");
        let status_arms = status_kind
            .body
            .statements
            .iter()
            .find_map(|statement| match statement {
                StmtIr::Match { arms, .. } => Some(arms),
                _ => None,
            })
            .expect("statusKind should contain a match statement");
        assert_match_arm_kind(&status_arms[0], "succeeded");
        assert_match_arm_kind(&status_arms[1], "failed");
        assert!(matches!(status_arms[2].pattern, PatternIr::Wildcard));

        let cancel_kind = unit
            .unit
            .executables
            .iter()
            .find(|executable| executable.symbol.ends_with("cancelKind"))
            .expect("cancelKind executable should be emitted");
        let cancel_arms = cancel_kind
            .body
            .statements
            .iter()
            .find_map(|statement| match statement {
                StmtIr::Match { arms, .. } => Some(arms),
                _ => None,
            })
            .expect("cancelKind should contain a match statement");
        assert_match_arm_kind(&cancel_arms[0], "alreadyStarted");
    }

    fn assert_match_arm_kind(arm: &skiff_artifact_model::MatchArmIr, expected_kind: &str) {
        let PatternIr::Record { fields } = &arm.pattern else {
            panic!(
                "TaskStatus/TaskCancelResult match arm must lower to a record pattern, got {:?}",
                arm.pattern
            );
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "kind");
        assert!(
            matches!(
                &fields[0].pattern,
                PatternIr::Literal {
                    value: LiteralIr::String { value },
                } if value == expected_kind
            ),
            "expected kind literal `{expected_kind}`, got {:?}",
            fields[0].pattern
        );
    }

    #[test]
    fn std_task_status_and_cancel_reject_non_task_ref_arguments() {
        for (name, source, expected) in [
            (
                "status-string",
                r#"
                function start() -> std.task.TaskStatus {
                  return std.task.status("not-a-task-ref")
                }
            "#,
                "TaskRef",
            ),
            (
                "cancel-number",
                r#"
                function start() -> std.task.TaskCancelResult {
                  return std.task.cancel(42)
                }
            "#,
                "TaskRef",
            ),
            (
                "status-extra-arg",
                r#"
                function run() -> void {
                  return
                }

                function start() -> std.task.TaskStatus {
                  let ref = dispatch run()
                  return std.task.status(ref, ref)
                }
            "#,
                "arity mismatch",
            ),
        ] {
            let error = compile_error(package_with_source(name, source));
            assert!(
                error.contains(expected),
                "{name} should reject with {expected:?}: {error}"
            );
        }
    }
}
