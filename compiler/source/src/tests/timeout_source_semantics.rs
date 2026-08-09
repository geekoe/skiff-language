use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use compiler_input_model::PackageCompilePolicy;
use skiff_artifact_model::{
    CallableEffectSummary, CallableProvenanceSummary, PendingEffectCategory, ValueProvenance,
};
use skiff_compiler_input::CompilerPlatformSources;

use crate::{
    build_package_from_parsed_sources, parsed_sources::parse_publication_sources,
    prelude_registry::initialize_prelude_registry, source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageSourceModel, SourceSymbolKey,
};

const PACKAGE_ID: &str = "example.com/timeout-source-semantics";
const MODULE_PATH: &str = "internal.timeout_source";

fn build_model(source_text: &str) -> Result<PackageSourceModel, String> {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    let platform_sources =
        CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry initializes");
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/timeout_source.skiff"),
        MODULE_PATH.to_string(),
        false,
        false,
        source_text.to_string(),
        "internal/timeout_source.skiff",
    )
    .map_err(|error| error.to_string())?;
    let parsed_sources = parse_publication_sources(Path::new("/tmp/timeout-source"), &[source])
        .map_err(|error| error.to_string())?;
    build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
        parsed_sources,
        production_sources: Vec::new(),
        diagnostic_root: Path::new("/tmp/timeout-source"),
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

fn callable_effects(
    model: &PackageSourceModel,
    name: &str,
) -> skiff_artifact_model::CallableMayEffects {
    match &model.callable_effects().operations()[&SourceSymbolKey::new(MODULE_PATH, name)] {
        CallableEffectSummary::Analyzed { effects } => *effects,
        CallableEffectSummary::Unknown { reason } => {
            panic!("{name} effects must be analyzed, found {reason:?}")
        }
    }
}

#[test]
fn timeout_value_is_target_typed_lexically_scoped_and_type_transparent() {
    let model = build_ok(
        r#"
            type Receipt { value: string }

            function run(input: string) -> Receipt {
              let receipt: Receipt = timeout(20ms) value {
                let local = input;
                ({ value: local })
              }
              return receipt
            }
        "#,
    );
    let timeout = &model.execution_semantics().timeout_plans()[0];
    assert_eq!(timeout.duration_milliseconds, 20);
    assert!(timeout.produces_value);

    for source in [
        r#"
            function run() -> string {
              let value = timeout(20ms) value { 1 }
              return value
            }
        "#,
        r#"
            function run() -> string {
              let value = value { const local = "ok" local }
              return local
            }
        "#,
    ] {
        let error = build_error(source);
        assert!(
            error.contains("type mismatch") || error.contains("unresolved local name `local`"),
            "unexpected diagnostic:\n{error}"
        );
    }

    build_ok(
        r#"
            function local() -> string { return "top-level" }

            function run() -> string {
              let captured = value {
                let local = "block-local"
                local
              }
              return local()
            }
        "#,
    );
}

#[test]
fn value_boundaries_reject_control_flow() {
    for control in ["return \"x\"", "break", "continue"] {
        let source = format!(
            r#"
                function run() -> string {{
                  return value {{
                    {control}
                    "tail"
                  }}
                }}
            "#
        );
        let error = build_error(&source);
        assert!(
            error.contains("value block") && error.contains("control flow"),
            "{control} produced unexpected diagnostic:\n{error}"
        );
    }
}

#[test]
fn timeout_body_and_tail_preserve_suspend_effect_and_root_provenance() {
    let model = build_ok(
        r#"
            type Payload { value: string }

            function run(input: Payload) -> Payload {
              return timeout(2s) value {
                db transaction {}
                input
              }
            }
        "#,
    );
    let effects = callable_effects(&model, "run");
    assert!(effects.may_pending);
    assert_eq!(
        effects.pending_effect_categories,
        vec![PendingEffectCategory::HostEffect],
        "the db transaction inside the timeout carries the HostEffect category"
    );
    let provenance =
        &model.callable_provenance().operations()[&SourceSymbolKey::new(MODULE_PATH, "run")];
    let CallableProvenanceSummary::Analyzed { return_origins, .. } = provenance else {
        panic!("timeout wrapper must retain analyzed provenance");
    };
    assert_eq!(
        return_origins,
        &vec![ValueProvenance::CallerParameter { index: 0 }]
    );
}

#[test]
fn concurrent_serial_and_concurrent_value_are_rejected_in_v1() {
    let cases = [
        (
            "concurrent statement",
            "function run() -> void {\n  concurrent { const value = 1 }\n}\n",
            "concurrent is not supported in v1",
        ),
        (
            "serial",
            "function run() -> void {\n  serial { const value = 1 }\n}\n",
            "serial is not supported in v1",
        ),
        (
            "concurrent value",
            "function run() -> number {\n  return concurrent value { 1 }\n}\n",
            "concurrent value is not supported in v1",
        ),
    ];
    for (label, source, expected) in cases {
        let error = build_error(source);
        assert!(
            error.contains(expected),
            "{label} produced unexpected diagnostic:\n{error}"
        );
    }
}

#[test]
fn db_transaction_is_allowed_in_actor_methods_create_and_through_local_helpers() {
    // Direct transaction in an actor method.
    build_ok(
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
                db transaction { }
              }
            }
        "#,
    );

    // Transaction in `create` (fields assigned outside the transaction body).
    build_ok(
        r#"
            type Counter { id: string, count: number }

            actor Counter {
              key(id)
              create()
            }

            impl Counter {
              function create() -> void {
                db transaction { }
                self.count = 0
              }
            }
        "#,
    );

    // Transaction reachable through a same-package local helper.
    build_ok(
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
                helper()
              }
            }

            function helper() -> void {
              db transaction { }
            }
        "#,
    );

    // Ordinary callers and dispatch targets are unaffected.
    build_ok(
        r#"
            function helper() -> void {
              db transaction { }
            }

            function run() -> void {
              helper()
            }
        "#,
    );
    build_ok(
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
                dispatch helper()
              }
            }

            function helper() -> void {
              db transaction { }
            }
        "#,
    );
}

#[test]
fn ordinary_sources_without_concurrent_surface_still_compile() {
    build_ok("function run() -> number {\n  let value = 1\n  return value\n}\n");
}

#[test]
fn timeout_and_value_walkers_reach_config_roots_calls_stream_and_db_paths() {
    let model = build_ok(
        r#"
            import std

            function helper() -> string { return "ok" }

            function configured() -> string {
              return timeout(30ms) value {
                let configured = config.require<string>("timeout.value")
                helper().concat(configured)
              }
            }

            function stream() -> Stream<string> {
              timeout(30ms) {
                emit helper()
              }
            }
        "#,
    );
    assert_eq!(model.own_config_requirements().requirements().len(), 1);
    assert!(model
        .resolved_call_targets()
        .iter()
        .any(|(_, target)| target.source_callable_key()
            == Some(SourceSymbolKey::new(MODULE_PATH, "helper"))));
    let stream_effects = callable_effects(&model, "stream");
    assert!(stream_effects.may_pending);
    assert_eq!(
        stream_effects.pending_effect_categories,
        vec![PendingEffectCategory::Stream]
    );

    let root_error = build_error(
        r#"
            function run() -> string {
              return timeout(1ms) value { missing.value }
            }
        "#,
    );
    assert!(
        root_error.contains("unresolved root missing"),
        "unexpected root diagnostic:\n{root_error}"
    );

    let db_error = build_error(
        r#"
            type Stored { id: string, value: string }
            db object Stored { primary key(id) }

            function run(id: string) -> void {
              timeout(1ms) {
                db update Stored(id) { missing = "x" }
              }
            }
        "#,
    );
    assert!(
        db_error.contains("missing") && db_error.contains("field"),
        "unexpected DB field diagnostic:\n{db_error}"
    );
}

#[test]
fn timeout_statement_is_non_value_and_checked_duration_is_recorded() {
    let model = build_ok(
        r#"
            function run() -> void {
              timeout(1d) {}
            }
        "#,
    );
    let timeout = &model.execution_semantics().timeout_plans()[0];
    assert_eq!(timeout.duration_milliseconds, 86_400_000);
    assert!(!timeout.produces_value);
}

#[test]
fn execution_scopes_fail_closed_in_static_const_owners() {
    let error = build_error(
        r#"
            const BAD: string = timeout(1ms) value { "x" }
        "#,
    );
    assert!(
        error.contains("top-level const") && error.contains("execution scope"),
        "unexpected diagnostic:\n{error}"
    );
}
