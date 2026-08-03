mod common;
use common::{package_project::compile_package_project, TestDir};

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
              const ref = dispatch run(input) after(200ms)
              const scheduled = dispatch run(input) at(instant)
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
                  const dispatch = 1
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
              const ref = dispatch run()
              return std.task.status(ref)
            }

            function stop() -> std.task.TaskCancelResult {
              const ref = dispatch run()
              return std.task.cancel(ref)
            }

            function consumeStatus(status: std.task.TaskStatus) -> void {
              return
            }
        "#,
    );
    compile_package_project(temp.path()).expect("std.task status/cancel should compile");
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
                  const ref = dispatch run()
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
