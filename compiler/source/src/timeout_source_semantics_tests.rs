use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use compiler_input_model::PackageCompilePolicy;
use skiff_artifact_model::{CallableEffectSummary, CallableProvenanceSummary, ValueProvenance};
use skiff_compiler_input::CompilerPlatformSources;

use crate::{
    build_package_from_parsed_sources, parsed_sources::parse_publication_sources,
    prelude_registry::initialize_prelude_registry, source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, ConcurrentLaneKind, PackageSourceModel, SourceSymbolKey,
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
              const receipt: Receipt = timeout(20ms) value {
                const local = input;
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
              const value = timeout(20ms) value { 1 }
              return value
            }
        "#,
        r#"
            function run() -> string {
              const value = value { const local = "ok" local }
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
              const captured = value {
                const local = "block-local"
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
    assert!(effects.may_suspend);
    assert!(effects.returns_caller_alias);
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
fn concurrent_plan_has_stable_lane_order_kinds_dependencies_and_tail_site() {
    let model = build_ok(
        r#"
            function make() -> string { return "value" }

            function run() -> string {
              return concurrent value {
                const first = make()
                serial {
                  const first = first
                  first
                }
                const second = first
                second
              }
            }
        "#,
    );
    let plans = model.execution_semantics().concurrent_plans();
    assert_eq!(plans.len(), 1);
    let lanes = &plans[0].lanes;
    assert_eq!(
        lanes.iter().map(|lane| lane.kind).collect::<Vec<_>>(),
        vec![
            ConcurrentLaneKind::Statement,
            ConcurrentLaneKind::Serial,
            ConcurrentLaneKind::Statement,
            ConcurrentLaneKind::Tail,
        ]
    );
    assert_eq!(
        lanes
            .iter()
            .map(|lane| lane.source_order)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(lanes[0].dependencies, Vec::<u32>::new());
    assert_eq!(lanes[1].dependencies, vec![0]);
    assert_eq!(lanes[2].dependencies, vec![0]);
    assert_eq!(lanes[3].dependencies, vec![0, 1, 2]);
    assert!(lanes
        .iter()
        .all(|lane| !lane.source_site.module_path.is_empty()));
}

#[test]
fn concurrent_direct_const_shadow_reads_the_nearest_prior_lane() {
    let model = build_ok(
        r#"
            function run() -> string {
              return concurrent value {
                const item = "first"
                const item = item
                item
              }
            }
        "#,
    );
    let lanes = &model.execution_semantics().concurrent_plans()[0].lanes;
    assert_eq!(lanes[1].dependencies, vec![0]);
}

#[test]
fn concurrent_sibling_visibility_is_prior_direct_const_only() {
    for source in [
        r#"
            function run() -> string {
              return concurrent value {
                const second = first
                const first = "first"
                second
              }
            }
        "#,
        r#"
            function run() -> string {
              return concurrent value {
                let mutable = "value"
                mutable
              }
            }
        "#,
        r#"
            function run() -> string {
              return concurrent value {
                serial { const hidden = "value" }
                const copy = hidden
                copy
              }
            }
        "#,
    ] {
        let error = build_error(source);
        assert!(
            error.contains("concurrent")
                && (error.contains("forward reference")
                    || error.contains("mutable `let`")
                    || error.contains("not sibling-visible")),
            "unexpected diagnostic:\n{error}"
        );
    }
}

#[test]
fn concurrent_rejects_outer_mutation_but_accepts_lane_local_fresh_root() {
    let outer_error = build_error(
        r#"
            type Box { value: string }

            function run(box: Box) -> void {
              concurrent {
                box.value = "changed"
              }
            }
        "#,
    );
    assert!(
        outer_error.contains("outer mutable root"),
        "unexpected diagnostic:\n{outer_error}"
    );

    let transitive_error = build_error(
        r#"
            type Box { value: string }

            function mutate(box: Box) -> void {
              box.value = "changed"
            }

            function run(box: Box) -> void {
              concurrent {
                mutate(box)
              }
            }
        "#,
    );
    assert!(
        transitive_error.contains("outer mutable root"),
        "unexpected diagnostic:\n{transitive_error}"
    );

    let projected_error = build_error(
        r#"
            type Box { value: string }
            type Wrapper { box: Box }

            function run(box: Box) -> void {
              concurrent {
                serial {
                  const wrapper = Wrapper { box: box }
                  wrapper.box.value = "changed"
                }
              }
            }
        "#,
    );
    assert!(
        projected_error.contains("outer mutable root")
            || projected_error.contains("opaque root provenance"),
        "unexpected diagnostic:\n{projected_error}"
    );

    let stored_error = build_error(
        r#"
            type Box { value: string }
            type Wrapper { box: Box }

            function run(box: Box) -> void {
              concurrent {
                serial {
                  const wrapper = Wrapper { box: Box { value: "local" } }
                  wrapper.box = box
                  wrapper.box.value = "changed"
                }
              }
            }
        "#,
    );
    assert!(
        stored_error.contains("outer mutable root")
            || stored_error.contains("opaque root provenance"),
        "unexpected diagnostic:\n{stored_error}"
    );

    build_ok(
        r#"
            type Box { value: string }

            function run() -> void {
              concurrent {
                serial {
                  const local = Box { value: "initial" }
                  local.value = "changed"
                }
              }
            }
        "#,
    );
}

fn db_fixture(concurrent_body: &str) -> String {
    format!(
        r#"
            type Stored {{ id: string, value: string }}

            db object Stored {{
              primary key(id)
            }}

            function run(id: string) -> void {{
              concurrent {{
                {concurrent_body}
              }}
            }}
        "#
    )
}

#[test]
fn concurrent_external_effect_matrix_is_fail_closed() {
    build_ok(&db_fixture(
        r#"
            db find Stored(id)
            db optional Stored(id)
        "#,
    ));

    for (body, expected) in [
        (
            r#"
                db find Stored(id)
                db update Stored(id) { value = "next" }
            "#,
            "read/write",
        ),
        (
            r#"
                db update Stored(id) { value = "one" }
                db delete Stored(id)
            "#,
            "write/write",
        ),
        (
            r#"
                db transaction { db find Stored(id) }
                db find Stored(id)
            "#,
            "exclusive",
        ),
    ] {
        let error = build_error(&db_fixture(body));
        assert!(
            error.contains("concurrent effect conflict") && error.contains(expected),
            "{expected} produced unexpected diagnostic:\n{error}"
        );
    }
}

#[test]
fn concurrent_rejects_every_ast_representable_illegal_surface() {
    let cases = [
        ("if", "if true {}"),
        ("for", "for item in items {}"),
        ("match", "match true { true => {} }"),
        ("timeout", "timeout(1ms) {}"),
        ("value", "const nested = value { \"x\" }"),
        ("return", "return"),
        ("break", "break"),
        ("continue", "continue"),
        ("throw", "throw Failure { message: \"x\" }"),
        (
            "catch",
            "const caught = catch<Failure>(throw Failure { message: \"x\" })",
        ),
        ("emit", "emit \"x\""),
        ("spawn", "spawn sideEffect()"),
        ("nested serial", "serial { serial {} }"),
        ("nested concurrent", "concurrent {}"),
    ];
    for (label, statement) in cases {
        let source = format!(
            r#"
                type Failure {{ message: string }}
                function sideEffect() -> void {{}}
                function run(items: Array<number>) -> void {{
                  concurrent {{
                    {statement}
                  }}
                }}
            "#
        );
        let error = build_error(&source);
        assert!(
            error.contains("illegal concurrent surface"),
            "{label} produced unexpected diagnostic:\n{error}"
        );
    }
}

#[test]
fn timeout_and_value_walkers_reach_config_roots_calls_stream_and_db_paths() {
    let model = build_ok(
        r#"
            import std

            function helper() -> string { return "ok" }

            function configured() -> string {
              return timeout(30ms) value {
                const configured = config.require<string>("timeout.value")
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
    assert!(callable_effects(&model, "stream").may_suspend);

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
