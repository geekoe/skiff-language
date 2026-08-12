//! WP3 binding-semantics source tests: writable places, inout static
//! semantics and top-level const purity (design phase-2 §3.1, R-195/R-196/
//! R-198). Reference-derived: every fixture asserts the new semantics
//! directly, never an old-evaluator output.
//!
//! Lowering-side gates (let assignment, mutator receivers, actor external
//! inout, CallIr shape) live in the integration tests
//! `compiler/tests/binding_inout_semantics.rs`; the execution-semantics lane
//! gates are unit-tested in `execution_semantics/mutation.rs`.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use compiler_input_model::PackageCompilePolicy;
use skiff_artifact_model::{
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts,
    PackageArtifact, PackageBuildId, PackageCallableId, PackageCallableParameter,
    PackageCallableSignature, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef, PackageTypeRef,
    TypeRefIr, ValueProvenance, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};
use skiff_compiler_input::{CompilerPlatformSources, PackageDependency};

use crate::{
    build_package_from_parsed_sources_with_dependency_analysis,
    parsed_sources::parse_publication_sources, prelude_registry::initialize_prelude_registry,
    source_graph::CompilerSourceFile, CompileParsedPackageSourcesInput,
    PackageDependencyAnalysisFacts, PackageDependencyCallableAnalysis, PackageSourceModel,
    SourceDependencyAnalysisInput,
};

const PACKAGE_ID: &str = "example.com/binding-inout-semantics";
const MODULE_PATH: &str = "internal.binding_inout";

fn build_model_with(
    source_text: &str,
    dependency_analysis: &SourceDependencyAnalysisInput,
    package_dependencies: &[PackageDependency],
    package_artifacts: Option<&[PackageArtifact]>,
) -> Result<PackageSourceModel, String> {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    let platform_sources =
        CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry initializes");
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/binding_inout.skiff"),
        MODULE_PATH.to_string(),
        false,
        false,
        source_text.to_string(),
        "internal/binding_inout.skiff",
    )
    .map_err(|error| error.to_string())?;
    let parsed_sources = parse_publication_sources(Path::new("/tmp/binding-inout"), &[source])
        .map_err(|error| error.to_string())?;
    build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources: Vec::new(),
            diagnostic_root: Path::new("/tmp/binding-inout"),
            publication_api: None,
            package_aliases: &BTreeMap::new(),
            package_dependencies,
            package_facts: None,
            package_artifacts,
            policy: PackageCompilePolicy::new(PACKAGE_ID),
        },
        dependency_analysis,
    )
    .map_err(|error| error.to_string())
}

fn build_model_with_package_dependency(
    source_text: &str,
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> Result<PackageSourceModel, String> {
    let dependency = PackageDependency {
        id: "example.com/dep".to_string(),
        version: "1.0.0".to_string(),
        alias: Some("dep".to_string()),
        top_level_alias: None,
    };
    build_model_with(
        source_text,
        dependency_analysis,
        &[dependency],
        Some(&[package_dependency_artifact()]),
    )
}

fn build_model(source_text: &str) -> Result<PackageSourceModel, String> {
    build_model_with(
        source_text,
        &SourceDependencyAnalysisInput::default(),
        &[],
        None,
    )
}

/// The wire artifact for the manifest dependency `example.com/dep` (Local ABI
/// identity and build id must match the dependency analysis facts).
fn package_dependency_artifact() -> PackageArtifact {
    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: "example.com/dep".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("build:dep"),
        platform_error_projection_registry:
            skiff_artifact_model::current_platform_error_projection_registry_ref().clone(),
        files: Vec::new(),
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("pkg-local-abi:dep"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: "example.com/dep".to_string(),
            package_schema_index_identity: PackageSchemaIndexIdentity::new("schema-index:dep"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        synthetic_callback_owners: Vec::new(),
        bytecode_schema_records: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
        bytecode_statement_manifest_identity:
            skiff_artifact_model::derive_bytecode_statement_manifest_identity(
                "example.com/dep",
                &[],
            )
            .unwrap(),
        bytecode: None,
    }
}

fn build_ok(source_text: &str) -> PackageSourceModel {
    build_model(source_text).unwrap_or_else(|error| panic!("fixture should compile:\n{error}"))
}

fn build_error(source_text: &str) -> String {
    build_model(source_text).expect_err("fixture must fail closed")
}

fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}

/// A package-direct dependency exposing two NoPending callables:
/// `tools.inc` with a single inout parameter at position 0 (name `value`) and
/// `tools.ping` with an ordinary number parameter.
fn package_direct_inout_dependency() -> SourceDependencyAnalysisInput {
    let inc = PackageDependencyCallableAnalysis::new(
        PackageCallableId::new("pkg-callable:dep-tools-inc"),
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed {
                effects: no_effects(),
            },
            provenance: CallableProvenanceSummary::Analyzed {
                return_origins: vec![ValueProvenance::Fresh],
                direct_return_origins: vec![ValueProvenance::Fresh],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            },
            resolved_call_targets: BTreeMap::new(),
        },
    )
    .with_signature(PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![PackageCallableParameter {
            name: "value".to_string(),
            ty: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("number"),
            },
            mode: skiff_artifact_model::ParamModeIr::InOut,
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("void"),
        },
        may_suspend: false,
    })
    .with_inout_parameters([(0usize, "value".to_string())]);
    let ping = PackageDependencyCallableAnalysis::new(
        PackageCallableId::new("pkg-callable:dep-tools-ping"),
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed {
                effects: no_effects(),
            },
            provenance: CallableProvenanceSummary::Analyzed {
                return_origins: vec![ValueProvenance::Fresh],
                direct_return_origins: vec![ValueProvenance::Fresh],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            },
            resolved_call_targets: BTreeMap::new(),
        },
    )
    .with_signature(PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![PackageCallableParameter {
            name: "value".to_string(),
            ty: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("number"),
            },
            mode: skiff_artifact_model::ParamModeIr::Value,
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("void"),
        },
        may_suspend: false,
    });
    SourceDependencyAnalysisInput::new(
        BTreeMap::from([(
            "dep".to_string(),
            PackageDependencyAnalysisFacts::new(
                PackageBuildId::new("build:dep"),
                PackageLocalAbiIdentity::new("pkg-local-abi:dep"),
                BTreeMap::from([
                    ("tools.inc".to_string(), inc),
                    ("tools.ping".to_string(), ping),
                ]),
            ),
        )]),
        Vec::new(),
    )
    .expect("dependency analysis input builds")
}

// --- Writable places: positives -------------------------------------------------

#[test]
fn var_root_writes_rebinding_and_exact_paths_compile() {
    build_ok(
        r#"
            type Doc { title: string }

            function run() -> string {
              var title = "a"
              title = "b"
              title = title + "!"
              var doc = Doc { title: "x" }
              doc.title = "y"
              return doc.title
            }
        "#,
    );
    // A derived exact path of a var root is writable too.
    build_ok(
        r#"
            type Inner { value: number }
            type Outer { inner: Inner }

            function run() -> number {
              var outer = Outer { inner: Inner { value: 1 } }
              outer.inner.value = 2
              return outer.inner.value
            }
        "#,
    );
}

#[test]
fn actor_self_field_writes_are_writable() {
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
                self.count = self.count + 1
              }
            }
        "#,
    );
}

#[test]
fn pure_const_initializers_compile() {
    // Literal/operator-only initializers.
    build_ok(
        r#"
            const answer = 40 + 2
            const label = "v" + answer
        "#,
    );
    // Pure local (NoPending) calls stay allowed.
    build_ok(
        r#"
            function helper() -> number {
              return 1
            }

            const seeded = helper() + 1
        "#,
    );
    // Const-to-const references are pure.
    build_ok(
        r#"
            const base = 1
            const derived = base + 1
        "#,
    );
}

// --- Inout positives ------------------------------------------------------------

#[test]
fn inout_to_package_local_no_pending_callee_compiles() {
    build_ok(
        r#"
            function inc(inout value: number) -> void {
              value = value + 1
            }

            function run() -> number {
              var x = 1
              inc(inout x)
              return x
            }
        "#,
    );
    // Member path of a var root as the actual.
    build_ok(
        r#"
            type Doc { value: number }

            function inc(inout value: number) -> void {
              value = value + 1
            }

            function run() -> number {
              var doc = Doc { value: 1 }
              inc(inout doc.value)
              return doc.value
            }
        "#,
    );
}

#[test]
fn inout_to_package_direct_no_pending_callee_compiles() {
    let dependencies = package_direct_inout_dependency();
    build_model_with_package_dependency(
        r#"
            function run() -> number {
              var x = 1
              dep/tools.inc(inout x)
              return x
            }
        "#,
        &dependencies,
    )
    .unwrap_or_else(|error| panic!("package-direct inout call should compile:\n{error}"));
}

// --- Inout negatives -------------------------------------------------------------

#[test]
fn inout_argument_must_be_var_derived_exact_place() {
    for (label, source) in [
        (
            "final root",
            r#"
                function inc(inout value: number) -> void {
                  value = value + 1
                }

                function run() -> number {
                  final x = 1
                  inc(inout x)
                  return x
                }
            "#,
        ),
        (
            "call expression instead of a place",
            r#"
                function make() -> number {
                  return 1
                }

                function inc(inout value: number) -> void {
                  value = value + 1
                }

                function run() -> number {
                  inc(inout make())
                  return 0
                }
            "#,
        ),
        (
            "re-loaning a callee inout parameter",
            r#"
                function inner(inout value: number) -> void {
                  value = value + 1
                }

                function outer(inout value: number) -> void {
                  inner(inout value)
                }
            "#,
        ),
    ] {
        let error = build_error(source);
        assert!(
            error.contains("inout argument must be a writable place derived from a local `var`"),
            "{label} produced unexpected diagnostic:\n{error}"
        );
    }
}

#[test]
fn inout_callee_must_be_exact_local_or_package_direct() {
    // Interface (dynamic) target.
    let error = build_error(
        r#"
            interface Counter {
              function step(self: Self, value: number) -> void
            }

            type CounterImpl implements Counter {}

            impl CounterImpl {
              function step(value: number) -> void {
              }
            }

            function run() -> void {
              var x = 1
              final counter = CounterImpl{} as Counter
              counter.step(inout x)
            }
        "#,
    );
    assert!(
        error.contains("inout only allowed on exact package-local or package-direct targets"),
        "interface target produced unexpected diagnostic:\n{error}"
    );

    // Receiver-builtin target.
    let error = build_error(
        r#"
            function run() -> void {
              var items = Array.empty<number>()
              var x = 1
              items.push(inout x)
            }
        "#,
    );
    assert!(
        error.contains("inout only allowed on exact package-local or package-direct targets"),
        "receiver target produced unexpected diagnostic:\n{error}"
    );
}

#[test]
fn inout_callee_must_be_no_pending() {
    // A callee whose body may suspend (db transaction) is not a valid inout
    // target; the loan must not cross a Pending.
    let error = build_error(
        r#"
            function slow(inout value: number) -> void {
              db transaction { }
              value = value + 1
            }

            function run() -> number {
              var x = 1
              slow(inout x)
              return x
            }
        "#,
    );
    assert!(
        error.contains("inout call requires a NoPending callee"),
        "may-pending callee produced unexpected diagnostic:\n{error}"
    );

    // A callee that reaches a dynamic (interface) call is conservatively
    // pending and fails closed.
    let error = build_error(
        r#"
            interface Adder {
              function add(self: Self, value: number) -> void
            }

            type AdderImpl implements Adder {}

            impl AdderImpl {
              function add(value: number) -> void {
              }
            }

            function dynamic(inout value: number) -> void {
              final adder = AdderImpl{} as Adder
              adder.add(value)
              value = value + 1
            }

            function run() -> number {
              var x = 1
              dynamic(inout x)
              return x
            }
        "#,
    );
    assert!(
        error.contains("inout call requires a NoPending callee"),
        "dynamic callee produced unexpected diagnostic:\n{error}"
    );
}

#[test]
fn overlapping_inout_arguments_are_rejected() {
    for (label, source) in [
        (
            "identical places",
            r#"
                function pair(inout left: number, inout right: number) -> void {
                  left = left + right
                }

                function run() -> number {
                  var x = 1
                  pair(inout x, inout x)
                  return x
                }
            "#,
        ),
        (
            "prefix overlap through a member path",
            r#"
                type Doc { value: number }

                function pair(inout left: Doc, inout right: number) -> void {
                  left.value = right
                }

                function run() -> number {
                  var doc = Doc { value: 1 }
                  pair(inout doc, inout doc.value)
                  return doc.value
                }
            "#,
        ),
        (
            "dynamic index overlap",
            r#"
                function pair(inout left: integer, inout right: integer) -> void {
                  left = left + right
                }

                function run(source: Array<integer>, first: integer, second: integer) -> integer {
                  var values = source
                  pair(inout values[first], inout values[second])
                  return values[0]
                }
            "#,
        ),
    ] {
        let error = build_error(source);
        assert!(
            error.contains("overlapping inout arguments"),
            "{label} produced unexpected diagnostic:\n{error}"
        );
    }
}

#[test]
fn statically_distinct_indexed_inout_places_do_not_overlap() {
    build_ok(
        r#"
            function pair(inout left: integer, inout right: integer) -> void {
              left = left + 1
              right = right + 1
            }

            function run(source: Array<integer>) -> integer {
              var values = source
              pair(inout values[0], inout values[1])
              return values[0]
            }
        "#,
    );
}

#[test]
fn indexed_assignment_requires_a_writable_root() {
    let error = build_error(
        r#"
            function run(source: Array<integer>) -> integer {
              final values = source
              values[0] = 1
              return values[0]
            }
        "#,
    );
    assert!(
        error.contains("assignment target derives from immutable binding `values`"),
        "indexed final-root assignment produced an unexpected diagnostic:\n{error}"
    );
}

#[test]
fn loaned_place_use_while_loaned_is_rejected() {
    // The loaned place cannot be read by another argument of the same call.
    let error = build_error(
        r#"
            function inc(inout value: number, by: number) -> void {
              value = value + by
            }

            function run() -> number {
              var x = 1
              inc(inout x, x)
              return x
            }
        "#,
    );
    assert!(
        error.contains("inout place is exclusively loaned"),
        "use-while-loaned produced unexpected diagnostic:\n{error}"
    );

    // The loaned place cannot be passed into a nested call either.
    let error = build_error(
        r#"
            function inc(inout value: number, echo: number) -> void {
              value = value + echo
            }

            function echo(value: number) -> number {
              return value
            }

            function run() -> number {
              var x = 1
              inc(inout x, echo(x))
              return x
            }
        "#,
    );
    assert!(
        error.contains("inout place is exclusively loaned"),
        "nested-call loan use produced unexpected diagnostic:\n{error}"
    );
}

#[test]
fn inout_is_rejected_on_interface_requirements() {
    let error = build_error(
        r#"
            interface Counter {
              function step(inout value: number) -> void
            }
        "#,
    );
    assert!(
        error.contains("inout is not allowed on interface requirements or method tables"),
        "interface requirement produced unexpected diagnostic:\n{error}"
    );
}

// --- Top-level const purity -------------------------------------------------------

#[test]
fn effectful_const_initializers_are_rejected() {
    for (label, source, expected) in [
        (
            "native call",
            r#"
                const parsed = number.parse("1")
            "#,
            "const initializer must be a pure request-independent expression",
        ),
        (
            "may-pending local call",
            r#"
                function slow() -> number {
                  db transaction { }
                  return 1
                }

                const seeded = slow()
            "#,
            "const initializer must be a pure request-independent expression",
        ),
        (
            "execution-scoped expression",
            r#"
                const timed = timeout(1s) value { 1 }
            "#,
            "const initializer must be a pure request-independent expression",
        ),
        (
            "callback capability",
            r#"
                interface Provider {
                  function name(self: Self) -> string
                }

                type Host implements Provider {}

                impl Host {
                  function name() -> string {
                    return "host"
                  }
                }

                const boxed = Host{} as Provider
            "#,
            "const initializer must be a pure request-independent expression",
        ),
    ] {
        let error = build_error(source);
        assert!(
            error.contains(expected),
            "{label} produced unexpected diagnostic:\n{error}"
        );
    }
}

#[test]
fn const_initializer_rejects_dependency_values() {
    let dependencies = package_direct_inout_dependency();
    let error = build_model_with_package_dependency(
        r#"
            function run() -> void {
              var x = 1
              dep/tools.inc(inout x)
            }

            const version = dep/tools.ping(1)
        "#,
        &dependencies,
    )
    .expect_err("a package-direct call must not enter a const initializer")
    .to_string();
    assert!(
        error.contains("const initializer must be a pure request-independent expression"),
        "dependency call produced unexpected diagnostic:\n{error}"
    );
}
