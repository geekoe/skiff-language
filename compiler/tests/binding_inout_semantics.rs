//! WP3 binding-semantics integration tests: lowering gates, File IR shape,
//! syntax rejections and boundary projection (design phase-2 §2.2/§3.1).
//! Reference-derived: every fixture asserts the new semantics directly.
//!
//! Source-level static semantics (inout rules, const purity) live in
//! `compiler/source/src/tests/binding_inout_semantics.rs`.

mod common;

use std::fs;

use common::{
    package_project::{compile_package_project, PackageProjectCompileError},
    TestDir,
};
use serde_json::Value;
use skiff_artifact_model::{BoundaryCallableProjection, BoundaryUnavailableReason};
use skiff_compiler_emission::PublishedFileIrArtifact;

fn compile_file_ir(
    source: &str,
    source_path: &str,
    module_path: &str,
) -> Result<PublishedFileIrArtifact, PackageProjectCompileError> {
    let temp = TestDir::new("skiff-compiler", "binding-inout-semantics");
    fs::write(
        temp.path().join("package.yml"),
        "id: example.com/binding-inout\nversion: 1.0.0\n",
    )
    .expect("package manifest should be written");
    fs::write(temp.path().join("api.yml"), "{}\n").expect("api.yml should be written");
    let source_file = temp.path().join(source_path);
    fs::create_dir_all(source_file.parent().expect("fixture source parent"))
        .expect("fixture source directory should be created");
    fs::write(&source_file, source).expect("fixture source should be written");
    let project = compile_package_project(temp.path())?;
    Ok(common::artifacts::module_artifact(&project.package, module_path).clone())
}

fn compile_error(source: &str, source_path: &str) -> String {
    compile_file_ir(source, source_path, "internal.binding_inout")
        .expect_err("package compile should fail")
        .to_string()
}

fn executable<'a>(artifact: &'a Value, name: &str) -> &'a Value {
    artifact["executables"]
        .as_array()
        .expect("executables should be an array")
        .iter()
        .find(|executable| {
            executable["symbol"]
                .as_str()
                .is_some_and(|symbol| symbol.ends_with(&format!(".{name}")))
        })
        .unwrap_or_else(|| panic!("executable {name} should be present"))
}

fn slot_index(executable: &Value, name: &str, kind: &str) -> u64 {
    executable["slots"]["slots"]
        .as_array()
        .expect("slots.slots should be an array")
        .iter()
        .find(|slot| slot["name"] == name && slot["kind"] == kind)
        .unwrap_or_else(|| panic!("slot {name} ({kind}) should be present"))
        .get("index")
        .and_then(Value::as_u64)
        .expect("slot index")
}

fn call_exprs(executable: &Value) -> Vec<&Value> {
    executable["body"]["expressions"]
        .as_array()
        .expect("expressions should be an array")
        .iter()
        .filter_map(|expr| {
            if expr["kind"] == "call" {
                Some(&expr["call"])
            } else {
                None
            }
        })
        .collect()
}

fn call_to_inc<'a>(executable: &'a Value) -> &'a Value {
    call_exprs(executable)
        .into_iter()
        .find(|call| call["target"]["kind"] == "localExecutable")
        .unwrap_or_else(|| panic!("run should contain a local call"))
}

// --- Positives ---------------------------------------------------------------

#[test]
fn var_writes_rebinding_and_member_mutation_compile_and_lower() {
    let artifact = compile_file_ir(
        r#"
            type Doc { title: string }

            function run() -> string {
              var title = "a"
              title = "b"
              var doc = Doc { title: "x" }
              doc.title = "y"
              return doc.title
            }
        "#,
        "internal/binding_inout.skiff",
        "internal.binding_inout",
    )
    .expect("var writes should compile through the full pipeline")
    .value();
    let run = executable(&artifact, "run");
    let title_slot = slot_index(run, "title", "local");
    let assigns = run["body"]["statements"]
        .as_array()
        .expect("statements")
        .iter()
        .filter(|stmt| stmt["kind"] == "assign")
        .collect::<Vec<_>>();
    assert_eq!(assigns.len(), 2, "title rebind and doc.title write should assign");
    let rebind = &assigns[0];
    assert_eq!(
        rebind["target"]["kind"],
        "slot",
        "bare var rebind must assign the root slot"
    );
    assert_eq!(rebind["target"]["slot"].as_u64(), Some(title_slot));
    let member = &assigns[1];
    assert_eq!(
        member["target"]["kind"],
        "field",
        "member write must lower to a field assignment target"
    );
    assert_eq!(member["target"]["field"], "title");
}

#[test]
fn actor_self_field_writes_compile_but_not_inside_db_transactions() {
    compile_file_ir(
        r#"
            type Counter { id: string, count: number }

            actor Counter {
              key(id)
              create()
            }

            impl Counter {
              function create() -> void {
                self.count = 0
              }

              function run() -> void {
                self.count = self.count + 1
              }
            }
        "#,
        "internal/binding_inout.skiff",
        "internal.binding_inout",
    )
    .expect("actor self.field writes should compile");

    let error = compile_error(
        r#"
            type Counter { id: string, count: number }

            actor Counter {
              key(id)
              create()
            }

            impl Counter {
              function create() -> void {
                db transaction {
                  self.count = 0
                }
              }
            }
        "#,
        "internal/binding_inout.skiff",
    );
    assert!(
        error.contains("db transaction bodies cannot write actor field count"),
        "unexpected diagnostic:\n{error}"
    );
}

#[test]
fn pure_const_initializer_with_local_call_compiles() {
    compile_file_ir(
        r#"
            function helper() -> number {
              return 1
            }

            const seeded: number = helper() + 1

            function run() -> number {
              return seeded
            }
        "#,
        "internal/binding_inout.skiff",
        "internal.binding_inout",
    )
    .expect("a pure const initializer with a local call should compile");
}

#[test]
fn inout_call_lowers_to_root_slot_selector_path_and_param_mode() {
    let artifact = compile_file_ir(
        r#"
            type Doc { value: number }

            function inc(inout value: number) -> void {
              value = value + 1
            }

            function run() -> number {
              var x = 1
              inc(inout x)
              var doc = Doc { value: 1 }
              inc(inout doc.value)
              return x + doc.value
            }
        "#,
        "internal/binding_inout.skiff",
        "internal.binding_inout",
    )
    .expect("inout call should compile through the full pipeline")
    .value();

    // The callee carries ParamIr.mode = inOut.
    let inc = executable(&artifact, "inc");
    assert_eq!(
        inc["params"][0]["mode"]["kind"], "inOut",
        "inout parameter must project ParamModeIr::InOut"
    );
    let run = executable(&artifact, "run");
    let calls = call_exprs(run)
        .into_iter()
        .filter(|call| call["target"]["kind"] == "localExecutable")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2, "run should contain both inout calls");

    // `inc(inout x)`: one inout arg (root slot of x), no by-value arg for the
    // position.
    let x_slot = slot_index(run, "x", "local");
    let first = calls[0];
    let args_len = |call: &Value| {
        call.get("args")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0)
    };
    assert_eq!(
        args_len(first),
        0,
        "no normal argument value may be emitted for the inout position"
    );
    assert_eq!(first["inoutArgs"].as_array().expect("inoutArgs").len(), 1);
    assert_eq!(first["inoutArgs"][0]["rootSlot"].as_u64(), Some(x_slot));
    assert_eq!(
        first["inoutArgs"][0]
            .get("path")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
        0,
        "bare root loan carries an empty selector path"
    );

    // `inc(inout doc.value)`: root slot of doc + one field segment.
    let doc_slot = slot_index(run, "doc", "local");
    let second = calls[1];
    assert_eq!(
        args_len(second),
        0,
        "member inout argument must not lower a by-value arg either"
    );
    assert_eq!(second["inoutArgs"][0]["rootSlot"].as_u64(), Some(doc_slot));
    let path = second["inoutArgs"][0]["path"].as_array().expect("path");
    assert_eq!(path.len(), 1);
    assert_eq!(path[0]["kind"], "field");
    assert_eq!(path[0]["name"], "value");
}

// --- Negatives: let immutability ---------------------------------------------

#[test]
fn let_direct_and_member_assignment_are_rejected() {
    for (label, source, expected) in [
        (
            "direct let assignment",
            r#"
                function run() -> number {
                  let x = 1
                  x = 2
                  return x
                }
            "#,
            "cannot assign to immutable binding `x`",
        ),
        (
            "member assignment through a let binding",
            r#"
                type Doc { title: string }

                function run() -> string {
                  let doc = Doc { title: "x" }
                  doc.title = "y"
                  return doc.title
                }
            "#,
            "cannot assign to field of immutable binding `doc`",
        ),
        (
            "member assignment through an ordinary parameter",
            r#"
                type Doc { title: string }

                function run(doc: Doc) -> string {
                  doc.title = "y"
                  return doc.title
                }
            "#,
            "cannot assign to field of immutable binding `doc`",
        ),
    ] {
        let error = compile_error(source, "internal/binding_inout.skiff");
        assert!(
            error.contains(expected),
            "{label} produced unexpected diagnostic:\n{error}"
        );
    }
}

#[test]
fn mutating_receiver_calls_on_immutable_roots_are_rejected() {
    for (label, source, expected) in [
        (
            "push on a let array",
            r#"
                function run() -> number {
                  let items = Array.empty<number>()
                  items.push(1)
                  return items.length()
                }
            "#,
            "cannot mutate through immutable binding `items`",
        ),
        (
            "set on a let map",
            r#"
                function run() -> number {
                  let items = Map.empty<string, number>()
                  items.set("k", 1)
                  return items.length()
                }
            "#,
            "cannot mutate through immutable binding `items`",
        ),
        (
            "push on an ordinary parameter",
            r#"
                function run(items: Array<number>) -> void {
                  items.push(1)
                }
            "#,
            "cannot mutate through immutable binding `items`",
        ),
        (
            "member path mutator through an immutable root",
            r#"
                type Holder { items: Array<number> }

                function run(holder: Holder) -> void {
                  holder.items.push(1)
                }
            "#,
            "cannot mutate through immutable binding `holder`",
        ),
    ] {
        let error = compile_error(source, "internal/binding_inout.skiff");
        assert!(
            error.contains(expected),
            "{label} produced unexpected diagnostic:\n{error}"
        );
    }
}

// --- Negatives: syntax -------------------------------------------------------

#[test]
fn local_const_syntax_is_rejected() {
    let error = compile_error(
        r#"
            function run() -> void {
              const x = 1
            }
        "#,
        "internal/binding_inout.skiff",
    );
    assert!(
        error.contains("local const is not syntax"),
        "unexpected diagnostic:\n{error}"
    );
}

#[test]
fn top_level_let_and_var_are_rejected() {
    for (label, source) in [
        ("top-level let", "let x = 1\n"),
        ("top-level var", "var x = 1\n"),
    ] {
        let error = compile_error(source, "internal/binding_inout.skiff");
        assert!(
            error.contains("let/var are only allowed inside blocks"),
            "{label} produced unexpected diagnostic:\n{error}"
        );
    }
}

// --- Negatives: const purity -------------------------------------------------

#[test]
fn effectful_const_initializers_are_rejected_in_the_full_pipeline() {
    let error = compile_error(
        r#"
            const parsed = number.parse("1")
        "#,
        "internal/binding_inout.skiff",
    );
    assert!(
        error.contains("const initializer must be a pure request-independent expression"),
        "unexpected diagnostic:\n{error}"
    );
}

// --- Negatives: concurrent lanes ----------------------------------------------

#[test]
fn sibling_lanes_are_rejected_in_v1_before_writing_outer_vars() {
    // Concurrent lanes cannot even be formed in v1: the statement is rejected
    // before any lane body runs, so an outer var can never be written from a
    // sibling lane.
    let error = compile_error(
        r#"
            function run() -> void {
              var counter = 0
              concurrent {
                counter = 1
              }
            }
        "#,
        "internal/binding_inout.skiff",
    );
    assert!(
        error.contains("concurrent is not supported in v1"),
        "unexpected diagnostic:\n{error}"
    );
}

// --- Negatives: actor external / projection -----------------------------------

#[test]
fn actor_external_methods_reject_inout_params() {
    let error = compile_error(
        r#"
            type Counter { id: string, count: number }

            actor Counter {
              key(id)
              create()
            }

            impl Counter {
              function create() -> void {
                self.count = 0
              }

              function step(inout value: number) -> void {
                value = value + 1
              }
            }
        "#,
        "internal/binding_inout.skiff",
    );
    assert!(
        error.contains("inout is not allowed on interface requirements or method tables"),
        "unexpected diagnostic:\n{error}"
    );
}

#[test]
fn public_callables_with_inout_params_project_unavailable_at_the_boundary() {
    let temp = TestDir::new("skiff-compiler", "binding-inout-projection");
    fs::write(
        temp.path().join("package.yml"),
        "id: example.com/binding-inout-projection\nversion: 1.0.0\n",
    )
    .expect("package manifest should be written");
    fs::write(
        temp.path().join("api.yml"),
        "export:\n  inc: projection.inc\n",
    )
    .expect("api.yml should be written");
    fs::write(
        temp.path().join("projection.skiff"),
        r#"
            function inc(inout value: number) -> void {
              value = value + 1
            }
        "#,
    )
    .expect("projection fixture should be written");
    let project = compile_package_project(temp.path())
        .expect("an inout callable must still compile as a package");
    let callable_id = project
        .package
        .artifact
        .package_local_abi
        .public_symbols
        .get("export.inc")
        .and_then(|symbol| match symbol {
            skiff_artifact_model::PackageLocalAbiSymbol::Callable { callable_id, .. } => {
                Some(callable_id)
            }
            _ => None,
        })
        .expect("export.inc should be a public callable symbol");
    let projection = project
        .package
        .artifact
        .boundary_projections
        .get(callable_id)
        .expect("boundary projection must exist for a public callable");
    assert!(
        matches!(
            projection,
            BoundaryCallableProjection::Unavailable { reasons }
                if reasons.contains(&BoundaryUnavailableReason::InOutNotAllowedAtServiceBoundary)
        ),
        "inout callable must project Unavailable(InOutNotAllowedAtServiceBoundary), got {projection:?}"
    );
}

#[test]
fn value_callables_stay_available_at_the_boundary() {
    let temp = TestDir::new("skiff-compiler", "binding-inout-projection-available");
    fs::write(
        temp.path().join("package.yml"),
        "id: example.com/binding-inout-projection-available\nversion: 1.0.0\n",
    )
    .expect("package manifest should be written");
    fs::write(
        temp.path().join("api.yml"),
        "export:\n  add: projection.add\n",
    )
    .expect("api.yml should be written");
    fs::write(
        temp.path().join("projection.skiff"),
        r#"
            function add(value: number) -> number {
              return value + 1
            }
        "#,
    )
    .expect("projection fixture should be written");
    let project = compile_package_project(temp.path())
        .expect("a value callable must compile as a package");
    let callable_id = project
        .package
        .artifact
        .package_local_abi
        .public_symbols
        .get("export.add")
        .and_then(|symbol| match symbol {
            skiff_artifact_model::PackageLocalAbiSymbol::Callable { callable_id, .. } => {
                Some(callable_id)
            }
            _ => None,
        })
        .expect("export.add should be a public callable symbol");
    let projection = project
        .package
        .artifact
        .boundary_projections
        .get(callable_id)
        .expect("boundary projection must exist for a public callable");
    assert!(
        matches!(projection, BoundaryCallableProjection::Available { .. }),
        "value callable must stay Available, got {projection:?}"
    );
}
