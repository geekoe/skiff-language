use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use compiler_input_model::PackageCompilePolicy;
use skiff_compiler_input::CompilerPlatformSources;

use crate::{
    build_package_from_parsed_sources, parsed_sources::parse_publication_sources,
    prelude_registry::initialize_prelude_registry, reserved_names::validate_reserved_names,
    shared::parser::parse_source, source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageSourceModel,
};

const PACKAGE_ID: &str = "example.com/dispatch-source-semantics";
const MODULE_PATH: &str = "internal.dispatch_source";

fn build_model(source_text: &str) -> Result<PackageSourceModel, String> {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    let platform_sources =
        CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry initializes");
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/dispatch_source.skiff"),
        MODULE_PATH.to_string(),
        false,
        false,
        source_text.to_string(),
        "internal/dispatch_source.skiff",
    )
    .map_err(|error| error.to_string())?;
    let parsed_sources = parse_publication_sources(Path::new("/tmp/dispatch-source"), &[source])
        .map_err(|error| error.to_string())?;
    build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
        parsed_sources,
        production_sources: Vec::new(),
        diagnostic_root: Path::new("/tmp/dispatch-source"),
        publication_api: None,
        package_aliases: &BTreeMap::new(),
        package_dependencies: &[],
        package_facts: None,
        package_artifacts: None,
        policy: PackageCompilePolicy::new(PACKAGE_ID),
    })
    .map_err(|error| error.to_string())
}

fn build_ok(source_text: &str) -> PackageSourceModel {
    build_model(source_text).unwrap_or_else(|error| panic!("fixture should compile:\n{error}"))
}

fn build_error(source_text: &str) -> String {
    build_model(source_text).expect_err("fixture must fail closed")
}

fn init_prelude() {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    initialize_prelude_registry(
        &CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load"),
    )
    .expect("prelude registry initializes");
}

#[test]
fn dispatch_expression_compiles_in_statement_assignment_and_argument_positions() {
    build_ok(
        r#"
            function run(input: string) -> void {
              return
            }

            function consume(ref: std.task.TaskRef) -> void {
              return
            }

            function start(input: string) -> void {
              dispatch run(input)
              let ref = dispatch run(input)
              consume(dispatch run(input))
            }
        "#,
    );
}

#[test]
fn dispatch_after_and_at_timing_compile() {
    build_ok(
        r#"
            type Instant = Date

            function run(input: string) -> void {
              return
            }

            function start(input: string, instant: Instant) -> void {
              dispatch run(input) after(200ms)
              dispatch run(input) after(Duration.milliseconds(1))
              dispatch run(input) at(instant)
              let ref = dispatch run(input) after(0ms)
            }
        "#,
    );
}

#[test]
fn dispatch_inside_db_transaction_is_rejected() {
    let error = build_error(
        r#"
            function run() -> void {
              return
            }

            function start() -> void {
              db transaction {
                dispatch run()
              }
            }
        "#,
    );
    assert!(
        error.contains("dispatch is not allowed inside a db transaction"),
        "unexpected error: {error}"
    );
}

#[test]
fn dispatch_expression_inside_db_transaction_is_rejected() {
    let error = build_error(
        r#"
            function run() -> void {
              return
            }

            function start() -> void {
              db transaction {
                let ref = dispatch run()
              }
            }
        "#,
    );
    assert!(
        error.contains("dispatch is not allowed inside a db transaction"),
        "unexpected error: {error}"
    );
}

#[test]
fn dispatch_target_must_return_void_or_null() {
    let error = build_error(
        r#"
            function run() -> string {
              return "ok"
            }

            function start() -> void {
              dispatch run()
            }
        "#,
    );
    assert!(
        error.contains("dispatch target return type mismatch"),
        "unexpected error: {error}"
    );
}

#[test]
fn dispatch_after_requires_duration_type() {
    let error = build_error(
        r#"
            function run() -> void {
              return
            }

            function start() -> void {
              dispatch run() after(1)
            }
        "#,
    );
    assert!(
        error.contains("dispatch after(...) expects Duration"),
        "unexpected error: {error}"
    );
}

#[test]
fn dispatch_at_requires_instant_type() {
    let error = build_error(
        r#"
            function run() -> void {
              return
            }

            function start() -> void {
              dispatch run() at(1)
            }
        "#,
    );
    assert!(
        error.contains("dispatch at(...) expects Instant"),
        "unexpected error: {error}"
    );
}

#[test]
fn dispatch_keyword_is_rejected_for_user_declarations() {
    init_prelude();
    for source in [
        r#"
            function dispatch() -> void {
              return
            }
        "#,
        r#"
            type dispatch { value: string }
        "#,
        r#"
            alias dispatch = string
        "#,
        r#"
            interface dispatch { function run() -> void }
        "#,
        r#"
            const dispatch = "reserved"
        "#,
    ] {
        let ast = parse_source(source).expect("declaration should parse syntactically");
        let mut violations = Vec::new();
        validate_reserved_names("internal/dispatch_source.skiff", &ast, &mut violations);
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains("dispatch uses reserved prelude name")),
            "expected reserved-name violation for dispatch declaration, got {violations:?}"
        );
    }
}

#[test]
fn dispatch_keyword_is_rejected_for_import_alias() {
    let error = parse_source(r#"import std as dispatch"#)
        .expect_err("import alias dispatch must be rejected");
    assert!(
        error.to_string().contains("import name"),
        "unexpected import error: {error}"
    );
}

#[test]
fn dispatch_keyword_is_rejected_for_local_bindings() {
    init_prelude();
    let source = r#"
        function start() -> void {
          let dispatch = 1
        }
    "#;
    let ast = parse_source(source).expect("binding should parse syntactically");
    let mut violations = Vec::new();
    validate_reserved_names("internal/dispatch_source.skiff", &ast, &mut violations);
    assert!(
        violations.iter().any(
            |violation| violation.contains("local binding dispatch uses reserved prelude name")
        ),
        "expected local binding violation, got {violations:?}"
    );
}

#[test]
fn std_task_status_and_cancel_compile_with_task_ref_argument() {
    init_prelude();
    build_ok(
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
        "#,
    );
}

#[test]
fn std_task_status_rejects_non_task_ref_argument() {
    init_prelude();
    let error = build_error(
        r#"
            function start() -> std.task.TaskStatus {
              return std.task.status("not-a-task-ref")
            }
        "#,
    );
    assert!(
        error.contains("std.task.status") && error.contains("TaskRef"),
        "std.task.status must reject a non-TaskRef argument, got: {error}"
    );
}

#[test]
fn std_task_cancel_rejects_non_task_ref_argument() {
    init_prelude();
    let error = build_error(
        r#"
            function start() -> std.task.TaskCancelResult {
              return std.task.cancel(42)
            }
        "#,
    );
    assert!(
        error.contains("std.task.cancel") && error.contains("TaskRef"),
        "std.task.cancel must reject a non-TaskRef argument, got: {error}"
    );
}

#[test]
fn std_task_status_rejects_extra_arguments() {
    init_prelude();
    let error = build_error(
        r#"
            function run() -> void {
              return
            }

            function start() -> std.task.TaskStatus {
              let ref = dispatch run()
              return std.task.status(ref, ref)
            }
        "#,
    );
    assert!(
        error.contains("std.task.status"),
        "std.task.status arity mismatch must surface, got: {error}"
    );
}
