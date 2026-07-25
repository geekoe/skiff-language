use std::{collections::BTreeMap, path::Path, path::PathBuf};

use skiff_artifact_identity::assign_service_contract_identities;
use skiff_artifact_model::{
    CallableEffectSummary, CallableEffectUnknownReason, CallableMayEffects,
    CallableProvenanceSummary, CallableProvenanceUnknownReason, CallableSemanticFacts,
    ContractTypeRef, PackageCallableId, PackageLocalAbiIdentity, ValueEscapeLane, ValueProvenance,
};
use skiff_compiler_input::{CompilerPlatformSources, ResolvedContractDependency};

use crate::{
    build_package_from_parsed_sources_with_dependency_analysis,
    contract_dependency_test_fixture::{
        contract_and_schema, requirement, resolved_contract_fixture,
    },
    parsed_sources::parse_publication_sources,
    prelude_registry::initialize_prelude_registry,
    source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageCompilePolicy, PackageDependencyAnalysisFacts,
    PackageDependencyCallableAnalysis, PackageSourceModel, ResolvedCallTarget,
    SourceDependencyAnalysisInput, SourceSymbolKey,
};

#[test]
fn analysis_pending_is_an_explicit_diagnostic_seed_not_the_production_default() {
    let source_text = "function run() -> void {}";
    let source = CompilerSourceFile::parse(
        PathBuf::from("api.skiff"),
        "api".to_string(),
        true,
        false,
        source_text.to_string(),
        "api.skiff",
    )
    .expect("fixture parses");
    let parsed = parse_publication_sources(Path::new("/tmp/effect-diagnostic"), &[source])
        .expect("diagnostic source facts build");
    let pending = crate::SourceCallableEffectFacts::analysis_pending(&parsed);
    assert!(matches!(
        pending
            .operations()
            .get(&SourceSymbolKey::new("api", "run")),
        Some(CallableEffectSummary::Unknown {
            reason: CallableEffectUnknownReason::AnalysisPending
        })
    ));

    let production = analyze(source_text, SourceDependencyAnalysisInput::default());
    assert_eq!(effects(&production, "run"), no_effects());
}

#[test]
fn simple_detached_wrapper_is_safe_and_direct_transitive_calls_resolve() {
    let model = analyze(
        r#"
            type Input { value: string }
            type Output { value: string }

            function detach(input: Input) -> Output {
              return Output { value: input.value }
            }

            function wrapper(input: Input) -> Output {
              return detach(input)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["detach", "wrapper"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
    }
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::LocalFunction {
                source_callable
            } if source_callable == &SourceSymbolKey::new("api", "detach")
        )
    }));
}

#[test]
fn nested_local_calls_preserve_exact_effects_and_provenance() {
    let model = analyze(
        r#"
            type Input { value: string }
            type Middle { value: string }
            type Output { value: string }

            interface Provider {
              function inner(self: Self, input: Input) -> Middle
              function outer(self: Self, input: Middle) -> Output
            }

            db object Input {
              primary key(value)
            }

            function inner(input: Input) -> Middle {
              return Middle { value: input.value }
            }

            function outer(input: Middle) -> Output {
              return Output { value: input.value }
            }

            function nested(input: Input) -> Output {
              const rows = db find many Input {
                where value == input.value
              }
              return outer(inner(input))
            }

            function nestedRecordField(input: Input) -> Output {
              const rows = db find many Input {
                where value == input.value
              }
              return Output { value: inner(input).value }
            }

            function nestedCollectionElement(input: Input) -> JsonObject {
              const rows = db find many Input {
                where value == input.value
              }
              return { item: inner(input).value }
            }

            function unknownInner(input: Input, provider: any Provider) -> Output {
              return outer(provider.inner(input))
            }

            function unknownOuter(input: Input, provider: any Provider) -> Output {
              return provider.outer(inner(input))
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["nested", "nestedRecordField", "nestedCollectionElement"] {
        assert_eq!(
            effects(&model, callable),
            suspend_only_effects(),
            "{callable}"
        );
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
    }
    for callable in ["unknownInner", "unknownOuter"] {
        assert!(
            effects(&model, callable).invokes_unknown_target,
            "{callable}"
        );
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Unknown {
                reason: CallableProvenanceUnknownReason::UnknownCallTarget
            }
        ));
    }
    assert!(
        model
            .resolved_call_targets()
            .iter()
            .filter(|(_, target)| matches!(
                target,
                ResolvedCallTarget::LocalFunction { source_callable }
                    if source_callable == &SourceSymbolKey::new("api", "inner")
            ))
            .count()
            >= 4,
        "distinct inner call sites must keep distinct expression keys"
    );
}

#[test]
fn module_constant_return_keeps_exact_constant_provenance_through_local_call() {
    let model = analyze_sources(&[
        (
            "model",
            r#"
                const UPSTREAM_KIND_API_KEY: string = "apiKey"

                function upstreamKindApiKey() -> string {
                  return UPSTREAM_KIND_API_KEY
                }
            "#,
        ),
        (
            "upstream_sources",
            r#"
                type Credential { kind: string }

                function buildCredential() -> Credential {
                  return Credential { kind: root.model.upstreamKindApiKey() }
                }
            "#,
        ),
    ]);

    assert_eq!(
        effects_in(&model, "model", "upstreamKindApiKey"),
        no_effects()
    );
    let CallableProvenanceSummary::Analyzed { return_origins, .. } =
        provenance_in(&model, "model", "upstreamKindApiKey")
    else {
        panic!("module constant wrapper must retain analyzed provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Constant]);

    assert_eq!(
        effects_in(&model, "upstream_sources", "buildCredential"),
        no_effects()
    );
    let CallableProvenanceSummary::Analyzed { return_origins, .. } =
        provenance_in(&model, "upstream_sources", "buildCredential")
    else {
        panic!("fresh record caller must retain analyzed provenance");
    };
    assert_eq!(
        return_origins,
        &vec![ValueProvenance::Fresh, ValueProvenance::Constant]
    );
}

#[test]
fn unsupported_and_cyclic_module_constants_remain_fail_closed() {
    let model = analyze(
        r#"
            function compute() -> string { return "computed" }

            const UNSUPPORTED: string = compute()
            const CYCLE_A: string = CYCLE_B
            const CYCLE_B: string = CYCLE_A

            function unsupportedValue() -> string { return UNSUPPORTED }
            function cyclicValue() -> string { return CYCLE_A }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["unsupportedValue", "cyclicValue"] {
        assert!(effects(&model, callable).returns_caller_alias, "{callable}");
        assert_eq!(
            provenance(&model, callable),
            &CallableProvenanceSummary::Unknown {
                reason: CallableProvenanceUnknownReason::UnsupportedControlFlow,
            },
            "{callable}"
        );
    }
}

#[test]
fn unresolved_global_and_non_constant_zero_arg_return_are_not_constant_shortcuts() {
    let unresolved = analyze_result(
        "function unresolved() -> string { return MISSING_GLOBAL }",
        SourceDependencyAnalysisInput::default(),
    )
    .expect("source analysis retains a fail-closed callable summary");
    assert!(effects(&unresolved, "unresolved").returns_caller_alias);
    assert_eq!(
        provenance(&unresolved, "unresolved"),
        &CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::UnknownCallTarget,
        }
    );

    let model = analyze(
        r#"
            type Boxed { value: string }
            function freshValue() -> Boxed { return Boxed { value: "fresh" } }
            function wrapper() -> Boxed { return freshValue() }
        "#,
        SourceDependencyAnalysisInput::default(),
    );
    assert_eq!(effects(&model, "wrapper"), no_effects());
    let CallableProvenanceSummary::Analyzed { return_origins, .. } = provenance(&model, "wrapper")
    else {
        panic!("non-constant zero-argument call must remain analyzed");
    };
    assert_eq!(
        return_origins,
        &vec![ValueProvenance::Fresh, ValueProvenance::Constant]
    );
}

#[test]
fn root_qualified_and_catch_wrapped_helpers_keep_exact_local_targets() {
    let model = analyze(
        r#"
            type Input { value: string }
            type Output { value: string }

            function detach(input: Input) -> Output {
              return Output { value: input.value }
            }

            function rootWrapper(input: Input) -> Output {
              return root.api.detach(input)
            }

            function catchWrapper(input: Input) -> Output? {
              const attempted = catch<string>(detach(input))
              if attempted.tag == "ok" { return attempted.value }
              return null
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["rootWrapper", "catchWrapper"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
    }
}

#[test]
fn typed_catch_tag_narrowing_keeps_success_and_error_provenance_separate() {
    let model = analyze(
        r#"
            type Boxed { value: string }

            function fresh(input: Boxed) -> Boxed {
              return Boxed { value: input.value }
            }

            function alias(input: Boxed) -> Boxed {
              return input
            }

            function nullableAlias(input: Boxed) -> Boxed? {
              return input
            }

            function okEq(input: Boxed) -> Boxed? {
              const attempted = catch<string>(fresh(input))
              if attempted.tag == "ok" { return attempted.value }
              return null
            }

            function okNeEarly(input: Boxed) -> Boxed? {
              const attempted = catch<string>(fresh(input))
              if attempted.tag != "ok" { return null }
              return attempted.value
            }

            function nested(input: Boxed) -> Boxed? {
              const attempted = catch<string>(okEq(input))
              if attempted.tag != "ok" { return null }
              return attempted.value
            }

            function exactAlias(input: Boxed) -> Boxed? {
              const attempted = catch<string>(alias(input))
              if attempted.tag != "ok" { return null }
              return attempted.value
            }

            function errorBranch(input: Boxed) -> Exception<string>? {
              const attempted = catch<string>(alias(input))
              if attempted.tag == "err" { return attempted.exception }
              return null
            }

            function nullableCheck(input: Boxed) -> bool {
              const attempted = catch<string>(nullableAlias(input))
              if attempted.tag != "ok" { return false }
              return attempted.value == null
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["okEq", "okNeEarly", "nested"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        assert!(
            matches!(
                provenance(&model, callable),
                CallableProvenanceSummary::Analyzed { return_origins, .. }
                    if return_origins.contains(&ValueProvenance::Fresh)
            ),
            "{callable}: {:?}",
            provenance(&model, callable)
        );
    }

    assert_eq!(
        effects(&model, "exactAlias"),
        CallableMayEffects {
            returns_caller_alias: true,
            ..no_effects()
        }
    );
    assert!(matches!(
        provenance(&model, "exactAlias"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins.contains(&ValueProvenance::CallerParameter { index: 0 })
    ));

    assert_eq!(effects(&model, "errorBranch"), no_effects());
    assert!(matches!(
        provenance(&model, "errorBranch"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins.contains(&ValueProvenance::Fresh)
                && !return_origins.contains(&ValueProvenance::CallerParameter { index: 0 })
    ));
    assert_eq!(effects(&model, "nullableCheck"), no_effects());
}

#[test]
fn typed_catch_does_not_sanitize_unknown_success_provenance() {
    let model = analyze(
        r#"
            type Boxed { value: string }

            interface Provider {
              function run(self: Self, input: Boxed) -> Boxed
            }

            function unknown(input: Boxed, provider: any Provider) -> Boxed? {
              const attempted = catch<string>(provider.run(input))
              if attempted.tag != "ok" { return null }
              return attempted.value
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert!(effects(&model, "unknown").invokes_unknown_target);
    assert!(matches!(
        provenance(&model, "unknown"),
        CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::UnknownCallTarget
        }
    ));
}

#[test]
fn relay_shaped_cross_module_root_calls_keep_exact_targets() {
    let model = analyze_sources(&[
        (
            "relay",
            r#"
                type Input { value: string }
                type Output { value: string }

                function handler(input: Input) -> Output {
                  return root.helpers.detach(input)
                }
            "#,
        ),
        (
            "helpers",
            r#"
                function detach(input: root.relay.Input) -> root.relay.Output {
                  return root.relay.Output { value: input.value }
                }
            "#,
        ),
    ]);

    assert_eq!(effects_in(&model, "relay", "handler"), no_effects());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::LocalFunction {
                source_callable,
            } if source_callable == &SourceSymbolKey::new("helpers", "detach")
        )
    }));
}

#[test]
fn publication_wide_call_graph_closes_effects_and_provenance_across_files() {
    let model = analyze_sources(&[
        (
            "entry",
            r#"
                function returnThroughFiles(input: root.effects.Payload) -> root.effects.Payload {
                  return root.bridge.returnPayload(input)
                }

                function mutateThroughFiles(items: Array<string>) -> void {
                  root.bridge.mutate(items)
                }

                function persistThroughFiles(input: root.effects.Payload) -> void {
                  root.bridge.persist(input)
                }

                function throwThroughFiles() -> void {
                  root.bridge.fail()
                }

                function recursiveThroughFiles(
                  input: root.effects.Payload,
                  stop: bool
                ) -> root.effects.Payload {
                  if stop { return input }
                  return root.bridge.recursive(input, true)
                }
            "#,
        ),
        (
            "bridge",
            r#"
                function returnPayload(input: root.effects.Payload) -> root.effects.Payload {
                  return root.effects.returnPayload(input)
                }

                function mutate(items: Array<string>) -> void {
                  root.effects.mutate(items)
                }

                function persist(input: root.effects.Payload) -> void {
                  root.effects.persist(input)
                }

                function fail() -> void {
                  root.effects.fail()
                }

                function recursive(
                  input: root.effects.Payload,
                  stop: bool
                ) -> root.effects.Payload {
                  if stop { return input }
                  return root.entry.recursiveThroughFiles(input, true)
                }
            "#,
        ),
        (
            "effects",
            r#"
                type Payload { id: string, value: string }
                type Stored { id: string, payload: Payload }

                db object Stored {
                  primary key(id)
                }

                function returnPayload(input: Payload) -> Payload {
                  return input
                }

                function mutate(items: Array<string>) -> void {
                  items.push("changed")
                }

                function persist(input: Payload) -> void {
                  db insert Stored { id = input.id payload = input }
                }

                function fail() -> void {
                  throw std.json.DecodeError {
                    target: "fixture",
                    message: "failed",
                  }
                }
            "#,
        ),
    ]);

    for (module, symbol) in [
        ("effects", "returnPayload"),
        ("bridge", "returnPayload"),
        ("entry", "returnThroughFiles"),
        ("bridge", "recursive"),
        ("entry", "recursiveThroughFiles"),
    ] {
        assert!(
            effects_in(&model, module, symbol).returns_caller_alias,
            "{module}.{symbol}"
        );
        assert!(matches!(
            provenance_in(&model, module, symbol),
            CallableProvenanceSummary::Analyzed { return_origins, .. }
                if return_origins.contains(&ValueProvenance::CallerParameter { index: 0 })
        ));
    }

    for (module, symbol) in [
        ("effects", "mutate"),
        ("bridge", "mutate"),
        ("entry", "mutateThroughFiles"),
    ] {
        let effects = effects_in(&model, module, symbol);
        assert!(effects.writes_caller_reachable, "{module}.{symbol}");
        assert!(effects.requires_same_heap_identity, "{module}.{symbol}");
    }

    for (module, symbol) in [
        ("effects", "persist"),
        ("bridge", "persist"),
        ("entry", "persistThroughFiles"),
    ] {
        let effects = effects_in(&model, module, symbol);
        assert!(effects.escapes_caller_value, "{module}.{symbol}");
        assert!(effects.may_suspend, "{module}.{symbol}");
        assert!(matches!(
            provenance_in(&model, module, symbol),
            CallableProvenanceSummary::Analyzed { escape_lanes, .. }
                if escape_lanes == &vec![ValueEscapeLane::Database]
        ));
    }

    for (module, symbol) in [
        ("effects", "fail"),
        ("bridge", "fail"),
        ("entry", "throwThroughFiles"),
    ] {
        assert!(matches!(
            provenance_in(&model, module, symbol),
            CallableProvenanceSummary::Analyzed { throw_origins, .. }
                if throw_origins == &vec![ValueProvenance::Fresh]
        ));
    }

    let cross_file_targets = model
        .resolved_call_targets()
        .iter()
        .filter(|(caller, target)| {
            target
                .source_callable_key()
                .is_some_and(|callee| callee.module_path() != caller.module_path())
        })
        .count();
    assert_eq!(cross_file_targets, 10);
}

#[test]
fn missing_and_ambiguous_cross_file_targets_remain_fail_closed() {
    let missing_source = CompilerSourceFile::parse(
        PathBuf::from("entry.skiff"),
        "entry".to_string(),
        true,
        false,
        "function run() -> void { root.missing.run() }".to_string(),
        "entry.skiff",
    )
    .expect("missing-target fixture parses");
    let missing = parse_publication_sources(Path::new("/tmp/effect-provenance"), &[missing_source])
        .expect_err("a missing publication module must fail before effect analysis");
    assert!(
        missing
            .to_string()
            .contains("module `missing.skiff` which does not exist"),
        "{missing}"
    );

    let ambiguous = analyze_sources_result(&[
        (
            "entry",
            r#"
                function run() -> void {
                  root.helpers.run()
                }
            "#,
        ),
        ("helpers", "function run() -> void {}"),
        ("helpers", "function run() -> void {}"),
    ])
    .expect_err("an ambiguous publication target must not produce callable facts");
    assert!(
        ambiguous
            .to_string()
            .contains("has more than one exact signature fact"),
        "{ambiguous}"
    );
}

#[test]
fn concrete_interface_implementation_call_uses_exact_impl_method_target() {
    let model = analyze(
        r#"
            interface Provider {
              function read(self: Self, value: string) -> string
            }

            type ExactProvider implements Provider {}

            impl ExactProvider {
              function read(value: string) -> string {
                return value.concat("-detached")
              }
            }

            function wrapper(provider: ExactProvider, value: string) -> string {
              return provider.read(value)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(effects(&model, "wrapper"), no_effects());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::LocalImplMethod {
                source_callable,
            } if source_callable == &SourceSymbolKey::new("api", "ExactProvider.read")
        )
    }));
}

#[test]
fn actor_receiver_call_uses_actor_method_target_and_exact_local_effects() {
    let model = analyze(
        r#"
            actor Worker id string {
              label: string
            }

            impl Worker {
              function handle(self: Worker, value: string) -> string {
                return value
              }
            }

            function wrapper(worker: Worker, value: string) -> string {
              return worker.handle(value)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(effects(&model, "wrapper"), no_effects());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ActorMethod {
                actor,
                source_callable,
                method_name,
                ..
            } if actor == &SourceSymbolKey::new("api", "Worker")
                && source_callable == &SourceSymbolKey::new("api", "Worker.handle")
                && method_name == "handle"
        )
    }));
}

#[test]
fn ordinary_receiver_call_does_not_use_actor_method_target() {
    let model = analyze(
        r#"
            type Worker { label: string }

            impl Worker {
              function handle(self: Worker, value: string) -> string {
                return value
              }
            }

            function wrapper(worker: Worker, value: string) -> string {
              return worker.handle(value)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::LocalImplMethod {
                source_callable,
            } if source_callable == &SourceSymbolKey::new("api", "Worker.handle")
        )
    }));
    assert!(!model
        .resolved_call_targets()
        .iter()
        .any(|(_, target)| { matches!(target, ResolvedCallTarget::ActorMethod { .. }) }));
}

#[test]
fn post_construction_store_taints_fresh_return() {
    let model = analyze(
        r#"
            type Child { value: string }
            type Holder { child: Child }

            function storeAndReturn(input: Child) -> Holder {
              const holder = Holder { child: Child { value: "fresh" } }
              holder.child = input
              return holder
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(
        effects(&model, "storeAndReturn"),
        CallableMayEffects {
            returns_caller_alias: true,
            ..no_effects()
        }
    );
}

#[test]
fn post_construction_store_then_nested_mutation_fails_closed() {
    let model = analyze(
        r#"
            type Child { value: string }
            type Holder { child: Child }

            function storeThenMutate(input: Child) -> Holder {
              const holder = Holder { child: Child { value: "fresh" } }
              holder.child = input
              holder.child.value = "changed"
              return holder
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_heap_store_fail_closed(&model, "storeThenMutate");
}

#[test]
fn aliased_fresh_holder_store_taints_original_return() {
    let model = analyze(
        r#"
            type Child { value: string }
            type Holder { child: Child }

            function aliasStore(input: Child) -> Holder {
              const holder = Holder { child: Child { value: "fresh" } }
              const alias = holder
              alias.child = input
              return holder
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert!(effects(&model, "aliasStore").returns_caller_alias);
    assert!(matches!(
        provenance(&model, "aliasStore"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
}

#[test]
fn fresh_store_taint_propagates_through_callers_and_scc() {
    let model = analyze(
        r#"
            type Child { value: string }
            type Holder { child: Child }

            function storeLeaf(input: Child) -> Holder {
              const holder = Holder { child: Child { value: "fresh" } }
              holder.child = input
              return holder
            }

            function caller(input: Child) -> Holder {
              return storeLeaf(input)
            }

            function first(input: Child, stop: bool) -> Holder {
              if stop { return storeLeaf(input) }
              return second(input, true)
            }

            function second(input: Child, stop: bool) -> Holder {
              return first(input, stop)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["storeLeaf", "caller", "first", "second"] {
        assert!(effects(&model, callable).returns_caller_alias, "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
    }
}

#[test]
fn direct_parameter_field_store_has_write_and_same_heap_effects() {
    let model = analyze(
        r#"
            type Boxed { value: string }

            impl Boxed {
              function clear() -> void {
                self.value = "cleared"
              }
            }

            function mutate(input: Boxed) -> void {
              input.value = "changed"
            }

            function wrapper(input: Boxed) -> void {
              mutate(input)
            }

            function methodWrapper(input: Boxed) -> void {
              input.clear()
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["mutate", "wrapper", "Boxed.clear", "methodWrapper"] {
        assert_eq!(
            effects(&model, callable),
            CallableMayEffects {
                writes_caller_reachable: true,
                requires_same_heap_identity: true,
                ..no_effects()
            },
            "{callable}"
        );
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
    }
}

#[test]
fn fresh_alias_helper_loop_and_suspend_keep_relay_shaped_state_local() {
    let model = analyze(
        r#"
            type RelayState {
              f01: string, f02: string, f03: string, f04: string,
              f05: string, f06: string, f07: string, f08: string,
              f09: string, f10: string, f11: string, f12: string,
              f13: string, f14: string, f15: string, f16: string,
              f17: string, f18: string, f19: string, f20: string,
              f21: string, f22: string, f23: string, f24: string
            }

            function update(state: RelayState, value: string) -> void {
              state.f01 = value
              state.f12 = "helper"
              state.f24 = value
            }

            function v1Proxy(events: Array<string>) -> string {
              const state = RelayState {
                f01: "", f02: "", f03: "", f04: "",
                f05: "", f06: "", f07: "", f08: "",
                f09: "", f10: "", f11: "", f12: "",
                f13: "", f14: "", f15: "", f16: "",
                f17: "", f18: "", f19: "", f20: "",
                f21: "", f22: "", f23: "", f24: ""
              }
              const alias = state
              alias.f02 = "local"
              for event in events {
                update(state, event)
                std.time.sleep(Duration.milliseconds(1))
                state.f23 = "after-suspend"
              }
              return state.f12
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(
        effects(&model, "update"),
        CallableMayEffects {
            writes_caller_reachable: true,
            requires_same_heap_identity: true,
            ..no_effects()
        }
    );
    assert_eq!(effects(&model, "v1Proxy"), suspend_only_effects());
    assert!(matches!(
        provenance(&model, "v1Proxy"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
}

#[test]
fn nested_heap_store_remains_fail_closed_and_direct_reference_store_is_precise() {
    let model = analyze(
        r#"
            interface Provider {
              function value(self: Self) -> string
            }

            type Child { value: string }
            type Holder { child: Child }

            function nested(input: Holder) -> void {
              input.child.value = "changed"
            }

            function reference(input: Holder, child: Child) -> void {
              input.child = child
            }

            function unknownRhs(input: Child, provider: any Provider) -> void {
              input.value = provider.value()
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_heap_store_fail_closed(&model, "nested");
    assert_eq!(
        effects(&model, "reference"),
        CallableMayEffects {
            writes_caller_reachable: true,
            requires_same_heap_identity: true,
            ..no_effects()
        }
    );
    assert_eq!(effects(&model, "unknownRhs"), all_effects());
    assert!(matches!(
        provenance(&model, "unknownRhs"),
        CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::UnknownCallTarget
        }
    ));
}

#[test]
fn mutated_fresh_root_can_reenter_owning_map_but_other_escapes_remain_fail_closed() {
    let model = analyze(
        r#"
            type State { value: string }
            type Stored { id: string, state: State }

            db object Stored {
              primary key(id)
            }

            function intoMap() -> void {
              const state = State { value: "" }
              state.value = "changed"
              const container = Map.empty<string, State>()
              container.set("state", state)
            }

            function intoArray() -> void {
              const state = State { value: "" }
              state.value = "changed"
              const container = Array.empty<State>()
              container.push(state)
            }

            function intoDatabase() -> void {
              const state = State { value: "" }
              state.value = "changed"
              db insert Stored { id = "state" state = state }
            }

            function ambiguousAlias(useSecond: bool) -> void {
              const first = State { value: "" }
              const second = State { value: "" }
              let alias = first
              if useSecond {
                alias = second
              }
              alias.value = "ambiguous"
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["intoMap", "ambiguousAlias"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
    }
    for callable in ["intoArray", "intoDatabase"] {
        assert_heap_store_fail_closed(&model, callable);
    }
}

#[test]
fn conditional_map_lookup_tracks_distinct_fresh_and_formal_candidates() {
    let model = analyze(
        r#"
            type State { value: string }

            function formal(
              states: Map<string, State>,
              key: string
            ) -> State {
              let state: State? = states.get(key)
              if state == null {
                state = State { value: "" }
              }
              state.value = "changed"
              return state
            }

            function local(key: string) -> State {
              const states = Map.empty<string, State>()
              let state: State? = states.get(key)
              if state == null {
                state = State { value: "" }
              }
              state.value = "changed"
              states.set(key, state)
              return state
            }

            function throughFresh(key: string) -> State {
              const states = Map.empty<string, State>()
              return formal(states, key)
            }

            type Node { child: Node? }

            function cycle() -> Node {
              const node = Node { child: null }
              node.child = node
              return node
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(
        effects(&model, "formal"),
        CallableMayEffects {
            writes_caller_reachable: true,
            returns_caller_alias: true,
            requires_same_heap_identity: true,
            ..no_effects()
        }
    );
    for callable in ["local", "throughFresh"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
    }
    assert_heap_store_fail_closed(&model, "cycle");
}

#[test]
fn helper_map_projection_can_be_mutated_and_reinserted_without_becoming_the_map_root() {
    let model = analyze(
        r#"
            type State { key: string, value: string }

            function stateFor(
              states: Map<string, State>,
              key: string
            ) -> State {
              let state: State? = states.get(key)
              if state == null {
                state = State { key: key, value: "" }
                states.set(key, state)
              }
              return state
            }

            function local(key: string) -> State {
              const states = Map.empty<string, State>()
              const state = stateFor(states, key)
              state.value = "completed"
              states.set(key, state)
              return state
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(
        effects(&model, "stateFor"),
        CallableMayEffects {
            writes_caller_reachable: true,
            returns_caller_alias: true,
            requires_same_heap_identity: true,
            ..no_effects()
        }
    );
    assert_eq!(effects(&model, "local"), no_effects());
    assert!(matches!(
        provenance(&model, "local"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
}

#[test]
fn helper_parameter_store_distinguishes_field_projection_from_root_cycle() {
    let model = analyze(
        r#"
            type StreamState {
              key: string,
              status: string,
              snapshot: string
            }

            function update(state: StreamState, status: string) -> void {
              state.status = status
              state.snapshot = state.status
            }

            function local(status: string) -> StreamState {
              const state = StreamState {
                key: "response",
                status: "",
                snapshot: ""
              }
              update(state, status)
              return state
            }

            type Node { child: Node? }

            function selfStore(node: Node) -> void {
              node.child = node
            }

            function helperCycle() -> Node {
              const node = Node { child: null }
              selfStore(node)
              return node
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(
        effects(&model, "update"),
        CallableMayEffects {
            writes_caller_reachable: true,
            requires_same_heap_identity: true,
            ..no_effects()
        }
    );
    assert_eq!(effects(&model, "local"), no_effects());
    assert!(matches!(
        provenance(&model, "local"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
    assert_heap_store_fail_closed(&model, "helperCycle");
}

#[test]
fn recursive_scc_reaches_alias_fixed_point() {
    let model = analyze(
        r#"
            type Boxed { value: string }

            function first(input: Boxed, stop: bool) -> Boxed {
              if stop { return input }
              return second(input, true)
            }

            function second(input: Boxed, stop: bool) -> Boxed {
              return first(input, stop)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert!(effects(&model, "first").returns_caller_alias);
    assert!(effects(&model, "second").returns_caller_alias);
}

#[test]
fn normal_return_and_wire_detached_throw_remain_independent() {
    let model = analyze(
        r#"
            type Boxed { value: string }

            function returnAlias(input: Boxed) -> Boxed {
              return input
            }

            function throwAlias(input: Boxed) -> void {
              throw input
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    let returned = effects(&model, "returnAlias");
    assert!(returned.returns_caller_alias);
    assert!(!returned.throws_caller_alias);
    let thrown = effects(&model, "throwAlias");
    assert!(!thrown.returns_caller_alias);
    assert!(!thrown.throws_caller_alias);
    assert!(matches!(
        provenance(&model, "throwAlias"),
        CallableProvenanceSummary::Analyzed { throw_origins, .. }
            if throw_origins == &vec![ValueProvenance::Fresh]
    ));
}

#[test]
fn throw_and_rethrow_preserve_operand_effects_but_detach_emitted_provenance() {
    let model = analyze(
        r#"
            type Boxed { value: string }
            type Failure { message: string }

            function buildFailure(input: Boxed) -> Failure {
              input.value = "changed"
              std.time.sleep(Duration.milliseconds(1))
              return Failure { message: input.value }
            }

            function throwStatement(input: Boxed) -> void {
              throw buildFailure(input)
            }

            function throwExpression(input: Boxed) -> Failure {
              return throw buildFailure(input)
            }

            function rethrowStatement(input: Boxed) -> void {
              const attempted = catch<Failure>(throw Failure { message: input.value })
              if attempted.tag == "err" {
                rethrow attempted.exception
              }
            }

            function rethrowExpression(input: Boxed) -> Failure {
              const attempted = catch<Failure>(throw Failure { message: input.value })
              if attempted.tag == "err" {
                return rethrow attempted.exception
              }
              return Failure { message: "unreachable" }
            }

            function nestedRethrow(input: Boxed) -> void {
              const outer = catch<Failure>(rethrowStatement(input))
              if outer.tag == "err" {
                rethrow outer.exception
              }
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in [
        "throwStatement",
        "throwExpression",
        "rethrowStatement",
        "rethrowExpression",
        "nestedRethrow",
    ] {
        let effects = effects(&model, callable);
        assert!(!effects.throws_caller_alias, "{callable}: {effects:?}");
        assert_eq!(
            effects.requires_same_heap_identity,
            matches!(callable, "throwStatement" | "throwExpression"),
            "{callable}: {effects:?}"
        );
        assert!(!effects.invokes_unknown_target, "{callable}: {effects:?}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { throw_origins, .. }
                if throw_origins == &vec![ValueProvenance::Fresh]
        ));
    }

    assert!(effects(&model, "throwStatement").writes_caller_reachable);
    assert!(effects(&model, "throwExpression").writes_caller_reachable);
    assert!(effects(&model, "throwStatement").may_suspend);
    assert!(effects(&model, "throwExpression").may_suspend);
    assert!(!effects(&model, "rethrowStatement").writes_caller_reachable);
    assert!(!effects(&model, "rethrowExpression").writes_caller_reachable);
}

#[test]
fn stream_spawn_database_and_callback_escape_lanes_are_explicit() {
    let model = analyze(
        r#"
            interface Provider {
              function name(self: Self) -> string
            }

            type Boxed implements Provider { id: string, value: string }
            type Stored { id: string, payload: Boxed }
            impl Boxed {
              function name() -> string { return self.value }
            }

            db object Stored {
              primary key(id)
            }

            function sink(input: Boxed) -> void {}

            function stream(input: Boxed) -> Stream<Boxed> {
              emit(input)
            }

            function scalarStream(input: string) -> Stream<string> {
              emit(input)
            }

            function spawnWork(input: Boxed) -> void {
              spawn sink(input)
            }

            function persist(input: Boxed) -> void {
              db insert Stored { id = input.id payload = input }
            }

            function callback(input: Boxed) -> void {
              const boxed = input as Provider
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_escape_lane(&model, "stream", ValueEscapeLane::Stream);
    assert_escape_lane(&model, "scalarStream", ValueEscapeLane::Stream);
    assert_escape_lane(&model, "spawnWork", ValueEscapeLane::Spawn);
    assert!(effects(&model, "persist").may_suspend);
    assert_escape_lane(&model, "persist", ValueEscapeLane::Database);
    assert_escape_lane(&model, "callback", ValueEscapeLane::Callback);
}

#[test]
fn database_queries_and_detached_writes_do_not_escape_caller_values() {
    let model = analyze(
        r#"
            type Payload { value: string }
            type Stored { id: string, payload: Payload }

            db object Stored {
              primary key(id)
            }

            function read(id: string) -> Stored? {
              return db optional Stored(id)
            }

            function history(id: string) -> Array<Stored> {
              return db find many Stored { where id == id }
            }

            function put(id: string, value: string) -> Stored {
              return db insert Stored {
                id = id
                payload = Payload { value: value }
              }
            }

            function compareAndSet(id: string, value: string) -> Stored? {
              return db update Stored(id) {
                payload = Payload { value: value }
              }
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["read", "history", "put", "compareAndSet"] {
        assert_eq!(
            effects(&model, callable),
            suspend_only_effects(),
            "{callable}"
        );
        let CallableProvenanceSummary::Analyzed {
            return_origins,
            escape_lanes,
            ..
        } = provenance(&model, callable)
        else {
            panic!("{callable} must retain exact database provenance");
        };
        assert_eq!(return_origins, &vec![ValueProvenance::Fresh], "{callable}");
        assert!(escape_lanes.is_empty(), "{callable}: {escape_lanes:?}");
    }
}

#[test]
fn persisting_caller_owned_mutable_values_remains_a_database_escape() {
    let model = analyze(
        r#"
            type Payload { value: string }
            type Stored { id: string, payload: Payload }

            db object Stored {
              primary key(id)
            }

            function insertOwned(id: string, payload: Payload) -> Stored {
              return db insert Stored { id = id payload = payload }
            }

            function replaceOwned(id: string, payload: Payload) -> Stored? {
              return db update Stored(id) { payload = payload }
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["insertOwned", "replaceOwned"] {
        let callable_effects = effects(&model, callable);
        assert!(callable_effects.may_suspend, "{callable}");
        assert!(callable_effects.escapes_caller_value, "{callable}");
        assert_escape_lane(&model, callable, ValueEscapeLane::Database);
    }
}

#[test]
fn database_value_transactions_transfer_the_exact_final_value() {
    let model = analyze(
        r#"
            type Pointer { target: string }
            type Input { pointer: Pointer }
            type Receipt { sequence: integer, pointer: Pointer }

            function receipt(input: Input) -> Receipt {
              return db transaction value {
                const pointer = input.pointer
                Receipt { sequence: 1, pointer: pointer }
              }
            }

            function direct(input: Input) -> Input {
              return db transaction value {
                input
              }
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    let receipt = effects(&model, "receipt");
    assert!(receipt.may_suspend);
    assert!(receipt.returns_caller_alias);
    let CallableProvenanceSummary::Analyzed { return_origins, .. } = provenance(&model, "receipt")
    else {
        panic!("transaction result must retain analyzed provenance");
    };
    assert_eq!(
        return_origins,
        &vec![
            ValueProvenance::Fresh,
            ValueProvenance::Constant,
            ValueProvenance::CallerParameter { index: 0 }
        ]
    );

    let direct = effects(&model, "direct");
    assert!(direct.may_suspend);
    assert!(direct.returns_caller_alias);
    let CallableProvenanceSummary::Analyzed { return_origins, .. } = provenance(&model, "direct")
    else {
        panic!("direct caller result should retain exact caller provenance");
    };
    assert_eq!(
        return_origins,
        &vec![ValueProvenance::CallerParameter { index: 0 }]
    );
}

#[test]
fn database_writes_detach_static_field_projections_but_not_direct_or_unknown_values() {
    let model = analyze(
        r#"
            interface Provider {
              function value(self: Self) -> string
            }

            type Pointer { target: string }
            type Input { id: string, pointer: Pointer }
            type Stored { id: string, pointer: Pointer }

            db object Stored {
              primary key(id)
            }

            function projected(input: Input) -> Stored {
              const result = db upsert Stored(input.id) {
                id = input.id
                pointer = input.pointer
              } {
                pointer = input.pointer
              }
              return result.value
            }

            function direct(input: Input, pointer: Pointer) -> Stored {
              return db insert Stored {
                id = input.id
                pointer = pointer
              }
            }

            function unknownPredicate(input: Input, provider: any Provider) -> void {
              db update many Stored {
                where id == provider.value()
              } {
                pointer = input.pointer
              }
              return null
            }

            function unknownUpdate(input: Input, provider: any Provider) -> Stored? {
              return db update Stored(input.id) {
                id = provider.value()
              }
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(effects(&model, "projected"), suspend_only_effects());
    let direct = effects(&model, "direct");
    assert!(direct.may_suspend);
    assert!(direct.escapes_caller_value);
    assert_escape_lane(&model, "direct", ValueEscapeLane::Database);

    for callable in ["unknownPredicate", "unknownUpdate"] {
        let callable_effects = effects(&model, callable);
        assert!(callable_effects.invokes_unknown_target, "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Unknown {
                reason: CallableProvenanceUnknownReason::UnknownCallTarget
            }
        ));
    }
}

#[test]
fn exact_context_free_native_uses_shared_callable_semantics() {
    let model = analyze(
        r#"
            function digits(input: string) -> bool {
              return std.string.isAsciiDigits(input)
            }

            function truncate(input: string, maxBytes: number) -> string {
              return std.string.truncateUtf8Bytes(input, maxBytes)
            }

            function query(input: string) -> string {
              return std.string.encodeQueryComponent(input)
            }

            function path(input: string) -> string {
              return std.string.encodePath(input)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["digits", "truncate", "query", "path"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        let CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
        } = provenance(&model, callable)
        else {
            panic!("{callable} should retain exact native provenance");
        };
        assert_eq!(return_origins, &vec![ValueProvenance::Fresh], "{callable}");
        assert!(throw_origins.is_empty(), "{callable}");
        assert!(escape_lanes.is_empty(), "{callable}");
    }

    let native_keys = model
        .resolved_call_targets()
        .iter()
        .filter_map(|(_, target)| match target {
            ResolvedCallTarget::NativeFunction { binding_key } => Some(binding_key.as_str()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        native_keys,
        std::collections::BTreeSet::from([
            "std.string.encodePath",
            "std.string.encodeQueryComponent",
            "std.string.isAsciiDigits",
            "std.string.truncateUtf8Bytes",
        ])
    );
}

#[test]
fn date_from_epoch_milliseconds_wrapper_uses_exact_native_semantics() {
    let model = analyze(
        r#"
            function fromEpoch(milliseconds: integer) -> Date {
              return Date.fromEpochMilliseconds(milliseconds)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(effects(&model, "fromEpoch"), no_effects());
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
    } = provenance(&model, "fromEpoch")
    else {
        panic!("Date constructor wrapper should retain exact native provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);
    assert!(throw_origins.is_empty());
    assert!(escape_lanes.is_empty());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "core.date.fromEpochMilliseconds"
        )
    }));
}

#[test]
fn map_empty_materialization_accumulator_uses_exact_native_semantics() {
    let model = analyze_named(
        r#"
            function materializeCompletedResult() -> Map<string, Json> {
              const accumulator = Map.empty<string, Json>()
              return accumulator
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "responses",
        "agine.ai/llm-api",
    );

    assert_eq!(
        effects_in(&model, "responses", "materializeCompletedResult"),
        no_effects()
    );
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
    } = provenance_in(&model, "responses", "materializeCompletedResult")
    else {
        panic!("Map.empty accumulator should retain exact native provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);
    assert!(throw_origins.is_empty());
    assert!(escape_lanes.is_empty());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "core.map.empty"
        )
    }));
}

#[test]
fn json_decode_materialization_uses_exact_detached_semantics() {
    let model = analyze_named(
        r#"
            type Event { id: string, values: Array<string> }

            function materializeCompletedResult(encoded: string) -> Event? {
              const decoded = catch<std.json.DecodeError>(
                std.json.decode<Event>(encoded)
              )
              if decoded.tag != "ok" {
                return null
              }
              return decoded.value
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "responses",
        "agine.ai/llm-api",
    );

    assert_eq!(
        effects_in(&model, "responses", "materializeCompletedResult"),
        no_effects()
    );
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
    } = provenance_in(&model, "responses", "materializeCompletedResult")
    else {
        panic!("std.json.decode should retain exact detached provenance");
    };
    assert!(
        return_origins.contains(&ValueProvenance::Fresh)
            && return_origins.contains(&ValueProvenance::Constant)
            && !return_origins
                .iter()
                .any(|origin| matches!(origin, ValueProvenance::CallerParameter { .. })),
        "unexpected return provenance: {return_origins:?}"
    );
    assert!(throw_origins.is_empty());
    assert!(escape_lanes.is_empty());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "std.json.decode"
        )
    }));
}

#[test]
fn json_merge_materialization_uses_exact_detached_semantics() {
    let model = analyze_named(
        r#"
            function applyProviderOptions(base: Json, overlay: Json) -> Json {
              return std.json.merge(base, overlay)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "internal.aihub_service",
        "agine.ai/aihub",
    );

    assert_eq!(
        effects_in(&model, "internal.aihub_service", "applyProviderOptions"),
        no_effects()
    );
    assert!(matches!(
        provenance_in(
            &model,
            "internal.aihub_service",
            "applyProviderOptions"
        ),
        CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
        } if return_origins == &vec![ValueProvenance::Fresh]
            && throw_origins.is_empty()
            && escape_lanes.is_empty()
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "std.json.merge"
        )
    }));
}

#[test]
fn optional_date_parse_wrapper_uses_exact_native_semantics() {
    let model = analyze_named(
        r#"
            function optionalInputDate(value: string?) -> Date? {
              if value == null {
                return null
              }
              return Date.parse(value)
            }

            function adminUpstreamSourceCreate(accessTokenExpiresAt: string?) -> Date? {
              return optionalInputDate(accessTokenExpiresAt)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "upstream_sources",
        "agine.ai/codex-relay",
    );

    for callable in ["optionalInputDate", "adminUpstreamSourceCreate"] {
        assert_eq!(
            effects_in(&model, "upstream_sources", callable),
            no_effects(),
            "{callable}"
        );
        let CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
        } = provenance_in(&model, "upstream_sources", callable)
        else {
            panic!("{callable} should retain exact native provenance");
        };
        assert!(
            return_origins.contains(&ValueProvenance::Fresh)
                && return_origins.contains(&ValueProvenance::Constant),
            "{callable}: {return_origins:?}"
        );
        assert!(throw_origins.is_empty(), "{callable}");
        assert!(escape_lanes.is_empty(), "{callable}");
    }

    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "core.date.parse"
        )
    }));
}

#[test]
fn bytes_from_base64_wrapper_uses_exact_native_semantics() {
    let model = analyze(
        r#"
            function jwtPayload(value: string) -> bytes {
              return bytes.fromBase64(value)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(effects(&model, "jwtPayload"), no_effects());
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
    } = provenance(&model, "jwtPayload")
    else {
        panic!("Base64 decoder wrapper should retain exact native provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);
    assert!(throw_origins.is_empty());
    assert!(escape_lanes.is_empty());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "core.bytes.fromBase64"
        )
    }));
}

#[test]
fn bytes_from_hex_wrapper_uses_exact_native_semantics() {
    let model = analyze(
        r#"
            function exactChunk(value: string) -> bytes {
              return bytes.fromHex(value)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(effects(&model, "exactChunk"), no_effects());
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
    } = provenance(&model, "exactChunk")
    else {
        panic!("hex decoder wrapper should retain exact native provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);
    assert!(throw_origins.is_empty());
    assert!(escape_lanes.is_empty());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "core.bytes.fromHex"
        )
    }));
}

#[test]
fn bytes_concat_openai_multipart_shape_uses_exact_native_semantics() {
    let model = analyze(
        r#"
            type MultipartPart { body: bytes }

            function multipartBody(parts: Array<MultipartPart>, boundary: string) -> bytes {
              const chunks = Array.empty<bytes>()
              for part in parts {
                chunks.push(bytes.fromUtf8("--".concat(boundary).concat("\r\n")))
                chunks.push(part.body)
                chunks.push(bytes.fromUtf8("\r\n"))
              }
              chunks.push(bytes.fromUtf8("--".concat(boundary).concat("--\r\n")))
              return bytes.concat(chunks)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(effects(&model, "multipartBody"), no_effects());
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        throw_origins,
        escape_lanes,
    } = provenance(&model, "multipartBody")
    else {
        panic!("multipart bytes concatenation should retain exact native provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);
    assert!(throw_origins.is_empty());
    assert!(escape_lanes.is_empty());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "core.bytes.concat"
        )
    }));
}

#[test]
fn exact_http_request_natives_transfer_through_local_helpers() {
    let model = analyze(
        r#"
            function cookieValue(request: std.http.HttpRequest) -> string? {
              return std.http.cookie(request, "session")
            }

            function headerValues(request: std.http.HttpRequest) -> Array<string> {
              return std.http.headers(request, "x-trace")
            }

            function handler(request: std.http.HttpRequest) -> std.http.HttpResponse {
              const values = headerValues(request)
              const session = cookieValue(request)
              return std.http.HttpResponse {
                status: 200,
                headers: Array.empty<std.http.HttpHeader>(),
                body: bytes.fromUtf8("ok"),
              }
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for (callable, expected_origins) in [
        ("cookieValue", vec![ValueProvenance::Fresh]),
        ("headerValues", vec![ValueProvenance::Fresh]),
        (
            "handler",
            vec![ValueProvenance::Fresh, ValueProvenance::Constant],
        ),
    ] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        let CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
        } = provenance(&model, callable)
        else {
            panic!("{callable} should retain exact HTTP request native provenance");
        };
        assert_eq!(return_origins, &expected_origins, "{callable}");
        assert!(throw_origins.is_empty(), "{callable}");
        assert!(escape_lanes.is_empty(), "{callable}");
    }

    let native_keys = model
        .resolved_call_targets()
        .iter()
        .filter_map(|(_, target)| match target {
            ResolvedCallTarget::NativeFunction { binding_key } => Some(binding_key.as_str()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert!(native_keys.contains("std.http.request.headers"));
    assert!(native_keys.contains("std.http.request.cookie"));
}

#[test]
fn exact_http_client_stream_is_fresh_detached_and_suspending_through_raw_request() {
    let model = analyze(
        r#"
            function rawRequest(input: std.http.HttpClientRequest) -> std.http.HttpClientRequest {
              return std.http.HttpClientRequest {
                method: input.method,
                url: input.url,
                headers: input.headers,
                body: input.body,
                timeoutMs: input.timeoutMs,
              }
            }

            function responses(input: std.http.HttpClientRequest) -> std.http.HttpClientStreamHandle {
              return std.http.stream(rawRequest(input))
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(
        effects(&model, "rawRequest"),
        CallableMayEffects {
            returns_caller_alias: true,
            ..no_effects()
        }
    );
    assert_eq!(effects(&model, "responses"), suspend_only_effects());
    assert!(matches!(
        provenance(&model, "responses"),
        CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
        } if return_origins == &vec![ValueProvenance::Fresh]
            && throw_origins.is_empty()
            && escape_lanes.is_empty()
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "std.http.client.stream"
        )
    }));
}

#[test]
fn exact_http_client_sse_is_fresh_detached_and_suspending_through_raw_request() {
    let model = analyze(
        r#"
            function rawRequest(input: std.http.HttpClientRequest) -> std.http.HttpClientRequest {
              return std.http.HttpClientRequest {
                method: input.method,
                url: input.url,
                headers: input.headers,
                body: input.body,
                timeoutMs: input.timeoutMs,
              }
            }

            function responses(input: std.http.HttpClientRequest) -> Stream<std.http.HttpSseEvent> {
              return std.http.sse(rawRequest(input))
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(
        effects(&model, "rawRequest"),
        CallableMayEffects {
            returns_caller_alias: true,
            ..no_effects()
        }
    );
    assert_eq!(effects(&model, "responses"), suspend_only_effects());
    assert!(matches!(
        provenance(&model, "responses"),
        CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
        } if return_origins == &vec![ValueProvenance::Fresh]
            && throw_origins.is_empty()
            && escape_lanes.is_empty()
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "std.http.client.sse"
        )
    }));
}

#[test]
fn exact_http_response_stream_event_constructors_are_fresh_and_effect_free() {
    let model = analyze(
        r#"
            function start(
              status: integer,
              headers: Array<std.http.HttpHeader>
            ) -> std.http.HttpResponseStreamEvent {
              return std.http.streamStart(status, headers)
            }

            function chunk(value: bytes) -> std.http.HttpResponseStreamEvent {
              return std.http.streamChunk(value)
            }

            function end() -> std.http.HttpResponseStreamEvent {
              return std.http.streamEnd()
            }

            function safeResponses(
              status: integer,
              headers: Array<std.http.HttpHeader>,
              value: bytes
            ) -> std.http.HttpResponseStreamEvent {
              const started = std.http.streamStart(status, headers)
              const chunked = std.http.streamChunk(value)
              return std.http.streamEnd()
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["start", "chunk", "end", "safeResponses"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed {
                return_origins,
                throw_origins,
                escape_lanes,
            } if return_origins == &vec![ValueProvenance::Fresh]
                && throw_origins.is_empty()
                && escape_lanes.is_empty()
        ));
    }

    let native_keys = model
        .resolved_call_targets()
        .iter()
        .filter_map(|(_, target)| match target {
            ResolvedCallTarget::NativeFunction { binding_key } => Some(binding_key.as_str()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        native_keys,
        std::collections::BTreeSet::from([
            "std.http.stream.chunk",
            "std.http.stream.end",
            "std.http.stream.start",
        ])
    );
}

#[test]
fn exact_http_response_stream_emit_escapes_and_suspends_only_for_caller_event() {
    let model = analyze(
        r#"
            function emit(event: std.http.HttpResponseStreamEvent) -> void {
              std.http.emitResponseStream(event)
            }

            function emitFresh(value: bytes) -> void {
              std.http.emitResponseStream(std.http.streamChunk(value))
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(
        effects(&model, "emit"),
        CallableMayEffects {
            escapes_caller_value: true,
            may_suspend: true,
            ..no_effects()
        }
    );
    assert!(matches!(
        provenance(&model, "emit"),
        CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
        } if return_origins.is_empty()
            && throw_origins.is_empty()
            && escape_lanes == &vec![ValueEscapeLane::External]
    ));
    assert_eq!(
        effects(&model, "emitFresh"),
        CallableMayEffects {
            may_suspend: true,
            ..no_effects()
        }
    );
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::NativeFunction { binding_key }
                if binding_key == "std.http.stream.emitResponse"
        )
    }));
}

#[test]
fn std_exact_native_matrix_uses_shared_callable_semantics() {
    let model = analyze_named(
        r#"
            function dateNow() -> Date {
              return Date.now()
            }

            function durationMilliseconds() -> Duration {
              return Duration.milliseconds(1)
            }

            function durationSeconds() -> Duration {
              return Duration.seconds(1)
            }

            function safeInteger() -> integer {
              return std.number.assertSafeInteger(1)
            }

            function parseNumber(value: string) -> number? {
              return std.number.parse(value)
            }

            function hmac() -> string {
              return std.crypto.hmacSha1Base64("key", "text")
            }

            function sha256() -> string {
              return std.crypto.sha256("text")
            }

            function randomToken() -> string {
              return std.crypto.randomToken()
            }

            function uuid() -> string {
              return std.crypto.uuid()
            }

            function uuidSimple() -> string {
              return std.crypto.uuidSimple()
            }

            function sleep() -> void {
              return std.time.sleep(Duration.milliseconds(0))
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "std.effect_test",
        crate::shared::id::SKIFF_STD_PUBLICATION_ID,
    );

    for callable in [
        "dateNow",
        "durationMilliseconds",
        "durationSeconds",
        "safeInteger",
        "parseNumber",
        "hmac",
        "sha256",
        "randomToken",
        "uuid",
        "uuidSimple",
    ] {
        assert_eq!(
            effects_in(&model, "std.effect_test", callable),
            no_effects(),
            "{callable}"
        );
    }
    assert_eq!(
        effects_in(&model, "std.effect_test", "sleep"),
        suspend_only_effects()
    );

    let native_keys = model
        .resolved_call_targets()
        .iter()
        .filter_map(|(_, target)| match target {
            ResolvedCallTarget::NativeFunction { binding_key } => Some(binding_key.as_str()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        native_keys,
        std::collections::BTreeSet::from([
            "core.date.now",
            "core.duration.milliseconds",
            "core.duration.seconds",
            "core.number.parse",
            "core.number.assertSafeInteger",
            "std.crypto.hmacSha1Base64",
            "std.crypto.randomToken",
            "std.crypto.sha256",
            "std.crypto.uuid",
            "std.crypto.uuidSimple",
            "std.time.sleep",
        ])
    );
}

#[test]
fn exact_package_boundary_callables_transfer_canonical_effects_and_provenance() {
    let model = analyze_named(
        r#"
            type Payload { value: string }

            function emptyArray() -> Array<string> {
              return Array.empty<string>()
            }

            function utf8() -> bytes {
              return bytes.fromUtf8("value")
            }

            function json() -> string {
              return std.json.encode(Payload { value: "ok" })
            }

            function decode(value: string) -> Payload {
              return std.json.decode<Payload>(value)
            }

            function join(items: Array<string>) -> string {
              return string.join(items, ",")
            }

            function split(value: string) -> Array<string> {
              return string.split(value, ",")
            }

            function arrayLength(items: Array<string>) -> number {
              return items.length()
            }

            function bytesLength(value: bytes) -> number {
              return value.length()
            }

            function floor(value: number) -> number {
              return value.floor()
            }

            function ceil(value: number) -> number {
              return value.ceil()
            }

            function round(value: number) -> number {
              return value.round()
            }

            function concat(value: string) -> string {
              return value.concat("!")
            }

            function endsWith(value: string) -> bool {
              return value.endsWith("!")
            }

            function lowercase(value: string) -> string {
              return value.lowercase()
            }

            function startsWith(value: string) -> bool {
              return value.startsWith("!")
            }

            function request(input: std.http.HttpClientRequest) -> std.http.HttpClientResponse {
              return std.http.request(input)
            }

            function push(items: Array<string>) -> void {
              return items.push("value")
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "std.effect_test",
        crate::shared::id::SKIFF_STD_PUBLICATION_ID,
    );

    for callable in [
        "emptyArray",
        "utf8",
        "json",
        "decode",
        "join",
        "split",
        "arrayLength",
        "bytesLength",
        "floor",
        "ceil",
        "round",
        "concat",
        "endsWith",
        "lowercase",
        "startsWith",
    ] {
        assert_eq!(
            effects_in(&model, "std.effect_test", callable),
            no_effects(),
            "{callable}"
        );
        assert!(
            matches!(
                provenance_in(&model, "std.effect_test", callable),
                CallableProvenanceSummary::Analyzed { .. }
            ),
            "{callable}"
        );
    }
    let CallableProvenanceSummary::Analyzed { return_origins, .. } =
        provenance_in(&model, "std.effect_test", "ceil")
    else {
        panic!("number.ceil must keep exact detached provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);

    assert_eq!(
        effects_in(&model, "std.effect_test", "request"),
        suspend_only_effects()
    );
    let CallableProvenanceSummary::Analyzed { return_origins, .. } =
        provenance_in(&model, "std.effect_test", "request")
    else {
        panic!("HTTP response must keep exact detached provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);

    assert_eq!(
        effects_in(&model, "std.effect_test", "push"),
        CallableMayEffects {
            writes_caller_reachable: true,
            requires_same_heap_identity: true,
            ..no_effects()
        }
    );
    let CallableProvenanceSummary::Analyzed { return_origins, .. } =
        provenance_in(&model, "std.effect_test", "push")
    else {
        panic!("Array.push must keep exact constant-null provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Constant]);
}

#[test]
fn receiver_effects_are_contextual_to_caller_reachable_values() {
    let model = analyze_named(
        r#"
            function append(items: Array<string>) -> void {
              items.push("value")
            }

            function appendHop(items: Array<string>) -> void {
              append(items)
            }

            function callerOwned(items: Array<string>) -> void {
              appendHop(items)
            }

            function freshLocal() -> void {
              const items = Array.empty<string>()
              appendHop(items)
            }

            function freshLocalSuspend() -> void {
              const items = Array.empty<string>()
              std.time.sleep(Duration.milliseconds(1))
              appendHop(items)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "std.effect_test",
        crate::shared::id::SKIFF_STD_PUBLICATION_ID,
    );

    let caller_effects = CallableMayEffects {
        writes_caller_reachable: true,
        requires_same_heap_identity: true,
        ..no_effects()
    };
    for callable in ["append", "appendHop", "callerOwned"] {
        assert_eq!(
            effects_in(&model, "std.effect_test", callable),
            caller_effects,
            "{callable}"
        );
    }
    assert_eq!(
        effects_in(&model, "std.effect_test", "freshLocal"),
        no_effects()
    );
    assert_eq!(
        effects_in(&model, "std.effect_test", "freshLocalSuspend"),
        suspend_only_effects()
    );
}

#[test]
fn local_call_transfer_maps_alias_and_identity_to_exact_formal_actuals() {
    let model = analyze(
        r#"
            function withRequestCors(
              request: JsonObject,
              settings: JsonObject,
              response: JsonObject
            ) -> JsonObject {
              const same = response == response
              return response
            }

            function thirdHop(
              first: JsonObject,
              second: JsonObject,
              value: JsonObject
            ) -> JsonObject {
              return withRequestCors(first, second, value)
            }

            function freshThird(input: JsonObject) -> JsonObject {
              return thirdHop(input, {}, {})
            }

            function first(
              value: JsonObject,
              second: JsonObject,
              third: JsonObject
            ) -> JsonObject {
              const same = value == value
              return value
            }

            function callerFirst(input: JsonObject) -> JsonObject {
              return first(input, {}, {})
            }

            function branch(
              chooseFirst: bool,
              firstValue: JsonObject,
              thirdValue: JsonObject
            ) -> JsonObject {
              if chooseFirst {
                const same = firstValue == firstValue
                return firstValue
              }
              const same = thirdValue == thirdValue
              return thirdValue
            }

            function eitherFormal(
              chooseFirst: bool,
              input: JsonObject
            ) -> JsonObject {
              return branch(chooseFirst, input, {})
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(effects(&model, "freshThird"), no_effects());
    let CallableProvenanceSummary::Analyzed { return_origins, .. } =
        provenance(&model, "freshThird")
    else {
        panic!("fresh third actual must retain analyzed provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);

    for (callable, expected_parameter) in [("callerFirst", 0), ("eitherFormal", 1)] {
        let effects = effects(&model, callable);
        assert!(effects.returns_caller_alias, "{callable}");
        assert!(effects.requires_same_heap_identity, "{callable}");
        let CallableProvenanceSummary::Analyzed { return_origins, .. } =
            provenance(&model, callable)
        else {
            panic!("{callable} must retain analyzed provenance");
        };
        assert!(
            return_origins.contains(&ValueProvenance::CallerParameter {
                index: expected_parameter
            }),
            "{callable}: {return_origins:?}"
        );
    }
}

#[test]
fn json_object_set_effects_are_contextual_to_caller_reachable_values() {
    let model = analyze_named(
        r#"
            function setCallerOwned(object: JsonObject) -> void {
              return object.set("value", 1)
            }

            function callerOwnedHop(object: JsonObject) -> void {
              return setCallerOwned(object)
            }

            function freshLocal() -> void {
              const object: JsonObject = {}
              return object.set("value", 1)
            }

            function freshLocalCallerValue(value: Json) -> void {
              const object: JsonObject = {}
              return object.set("value", value)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "std.effect_test",
        crate::shared::id::SKIFF_STD_PUBLICATION_ID,
    );

    let caller_effects = CallableMayEffects {
        writes_caller_reachable: true,
        requires_same_heap_identity: true,
        ..no_effects()
    };
    for callable in ["setCallerOwned", "callerOwnedHop"] {
        assert_eq!(
            effects_in(&model, "std.effect_test", callable),
            caller_effects,
            "{callable}"
        );
        let CallableProvenanceSummary::Analyzed { return_origins, .. } =
            provenance_in(&model, "std.effect_test", callable)
        else {
            panic!("{callable} must keep exact constant-null provenance");
        };
        assert_eq!(return_origins, &vec![ValueProvenance::Constant]);
    }

    for callable in ["freshLocal", "freshLocalCallerValue"] {
        assert_eq!(
            effects_in(&model, "std.effect_test", callable),
            no_effects(),
            "{callable}"
        );
        let CallableProvenanceSummary::Analyzed { return_origins, .. } =
            provenance_in(&model, "std.effect_test", callable)
        else {
            panic!("{callable} must keep exact constant-null provenance");
        };
        assert_eq!(return_origins, &vec![ValueProvenance::Constant]);
    }
}

#[test]
fn config_intrinsics_are_exact_detached_sources() {
    let model = analyze(
        r#"
            type Config { name: string, optional: string?, present: bool }
            function load() -> Config {
              return Config {
                name: config.require<string>("name"),
                optional: config.optional<string>("optional"),
                present: config.has("present"),
              }
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );
    assert_eq!(effects(&model, "load"), no_effects());
    assert!(matches!(
        provenance(&model, "load"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
    let targets = model
        .resolved_call_targets()
        .iter()
        .filter_map(|(_, target)| match target {
            ResolvedCallTarget::ConfigIntrinsic { intrinsic } => Some(*intrinsic),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        targets,
        vec![
            crate::ConfigIntrinsic::Require,
            crate::ConfigIntrinsic::Optional,
            crate::ConfigIntrinsic::Has,
        ]
    );
    assert!(model
        .resolved_call_targets()
        .iter()
        .all(|(_, target)| !matches!(target, ResolvedCallTarget::Unknown { .. })));
}

#[test]
fn exact_date_and_duration_receiver_targets_use_sparse_semantics() {
    let model = analyze_named(
        r#"
            function isBefore(left: Date, right: Date) -> bool {
              return left.isBefore(right)
            }

            function compare(left: Date, right: Date) -> integer {
              return left.compare(right)
            }

            function addMilliseconds(value: Date, delta: integer) -> Date {
              return value.addMilliseconds(delta)
            }

            function diffMilliseconds(left: Date, right: Date) -> integer {
              return left.diffMilliseconds(right)
            }

            function epochMilliseconds(value: Date) -> integer {
              return value.toEpochMilliseconds()
            }

            function durationMilliseconds(value: Duration) -> integer {
              return value.toMilliseconds()
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "std.effect_test",
        crate::shared::id::SKIFF_STD_PUBLICATION_ID,
    );

    for callable in [
        "isBefore",
        "compare",
        "addMilliseconds",
        "diffMilliseconds",
        "epochMilliseconds",
        "durationMilliseconds",
    ] {
        assert_eq!(
            effects_in(&model, "std.effect_test", callable),
            no_effects(),
            "{callable}"
        );
    }
    let receiver_targets = model
        .resolved_call_targets()
        .iter()
        .filter_map(|(_, target)| match target {
            ResolvedCallTarget::ReceiverBuiltin { op } => Some(op.canonical_key),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        receiver_targets,
        std::collections::BTreeSet::from([
            "receiver:Date.addMilliseconds@1",
            "receiver:Date.compare@1",
            "receiver:Date.diffMilliseconds@1",
            "receiver:Date.isBefore@1",
            "receiver:Date.toEpochMilliseconds@1",
            "receiver:Duration.toMilliseconds@1",
        ])
    );
}

#[test]
fn date_add_milliseconds_keeps_v1_proxy_expiry_detached() {
    let model = analyze_named(
        r#"
            function upstreamRecoverAt(now: Date, delayMs: integer) -> Date {
              return now.addMilliseconds(delayMs)
            }

            function v1Proxy(now: Date, delayMs: integer) -> Date {
              return upstreamRecoverAt(now, delayMs)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "upstream_health",
        "skiff.run/codex-relay",
    );

    for callable in ["upstreamRecoverAt", "v1Proxy"] {
        assert_eq!(
            effects_in(&model, "upstream_health", callable),
            no_effects(),
            "{callable}"
        );
        assert!(matches!(
            provenance_in(&model, "upstream_health", callable),
            CallableProvenanceSummary::Analyzed { return_origins, .. }
                if return_origins == &vec![ValueProvenance::Fresh]
        ));
    }
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:Date.addMilliseconds@1"
        )
    }));
}

#[test]
fn date_diff_milliseconds_keeps_interaction_duration_shape_detached() {
    let model = analyze_named(
        r#"
            function interactionDurationMs(startedAt: Date, completedAt: Date?) -> integer? {
              if completedAt == null {
                return null
              }
              return completedAt.diffMilliseconds(startedAt)
            }

            function adminLlmInteractionsList(
              startedAt: Date,
              completedAt: Date?
            ) -> integer? {
              return interactionDurationMs(startedAt, completedAt)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "interactions",
        "skiff.run/codex-relay",
    );

    for callable in ["interactionDurationMs", "adminLlmInteractionsList"] {
        assert_eq!(
            effects_in(&model, "interactions", callable),
            no_effects(),
            "{callable}"
        );
    }
    assert!(matches!(
        provenance_in(&model, "interactions", "interactionDurationMs"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins.contains(&ValueProvenance::Constant)
                && return_origins.contains(&ValueProvenance::Fresh)
                && !return_origins.iter().any(|origin| matches!(
                    origin,
                    ValueProvenance::CallerParameter { .. }
                ))
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:Date.diffMilliseconds@1"
        )
    }));
}

#[test]
fn nullable_date_compare_keeps_upstream_status_shape_detached() {
    let model = analyze_named(
        r#"
            function upstreamStatus(now: Date, fixedRecoverAt: Date?) -> string {
              if fixedRecoverAt != null && now.compare(fixedRecoverAt) < 0 {
                return "recovering"
              }
              return "available"
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "upstream_health",
        "skiff.run/codex-relay",
    );

    assert_eq!(
        effects_in(&model, "upstream_health", "upstreamStatus"),
        no_effects()
    );
    assert!(matches!(
        provenance_in(&model, "upstream_health", "upstreamStatus"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Constant]
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:Date.compare@1"
        )
    }));
}

#[test]
fn exact_string_contains_target_is_read_only_detached_and_non_suspending() {
    let model = analyze_named(
        r#"
            function validEmail(value: string) -> bool {
              return value.contains("@")
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "account",
        "skiff.run/account",
    );

    assert_eq!(effects_in(&model, "account", "validEmail"), no_effects());
    assert!(matches!(
        provenance_in(&model, "account", "validEmail"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Fresh]
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:string.contains@1"
        )
    }));
}

#[test]
fn exact_bytes_to_hex_target_is_read_only_detached_and_non_suspending() {
    let model = analyze_named(
        r#"
            function encode(value: bytes) -> string {
              return value.toHex()
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "raw_parser",
        "skiff.run/codex-relay",
    );

    assert_eq!(effects_in(&model, "raw_parser", "encode"), no_effects());
    assert!(matches!(
        provenance_in(&model, "raw_parser", "encode"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Fresh]
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:bytes.toHex@1"
        )
    }));
}

#[test]
fn exact_json_object_has_target_is_read_only_detached_and_non_suspending() {
    let model = analyze_named(
        r#"
            function jsonObjectField(value: JsonObject, field: string) -> bool {
              return value.has(field)
            }

            function verifyDomainChallenge(value: JsonObject) -> bool {
              return jsonObjectField(value, "Status")
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "account",
        "skiff.run/account",
    );

    for callable in ["jsonObjectField", "verifyDomainChallenge"] {
        assert_eq!(effects_in(&model, "account", callable), no_effects());
        assert!(matches!(
            provenance_in(&model, "account", callable),
            CallableProvenanceSummary::Analyzed { return_origins, .. }
                if return_origins == &vec![ValueProvenance::Fresh]
        ));
    }
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:JsonObject.has@1"
        )
    }));
}

#[test]
fn exact_json_object_delete_mutates_caller_receiver_but_discharges_fresh_receiver() {
    let model = analyze_named(
        r#"
            function deleteCallerField(value: JsonObject, field: string) -> bool {
              return value.delete(field)
            }

            function sanitize() -> bool {
              const value: JsonObject = { instructions: "drop", keep: true }
              return value.delete("instructions")
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "responses_projection",
        "skiff.run/codex-relay",
    );

    assert_eq!(
        effects_in(&model, "responses_projection", "deleteCallerField"),
        CallableMayEffects {
            writes_caller_reachable: true,
            requires_same_heap_identity: true,
            ..no_effects()
        }
    );
    assert_eq!(
        effects_in(&model, "responses_projection", "sanitize"),
        no_effects()
    );
    for callable in ["deleteCallerField", "sanitize"] {
        assert!(matches!(
            provenance_in(&model, "responses_projection", callable),
            CallableProvenanceSummary::Analyzed { return_origins, .. }
                if return_origins == &vec![ValueProvenance::Constant]
        ));
    }
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:JsonObject.delete@1"
        )
    }));
}

#[test]
fn json_object_delete_semantics_do_not_generalize_to_map_delete() {
    let model = analyze_named(
        r#"
            function remove(value: Map<string, string>, key: string) -> bool {
              return value.delete(key)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "map_delete",
        "skiff.run/map-delete",
    );

    assert_eq!(effects_in(&model, "map_delete", "remove"), all_effects());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:Map.delete@1"
        )
    }));
}

#[test]
fn exact_json_object_get_preserves_nested_alias_but_fresh_codec_shape_is_detached() {
    let model = analyze_named(
        r#"
            function direct(value: JsonObject, key: string) -> Json {
              return value.get(key)
            }

            function jsonObject(value: Json?) -> JsonObject? {
              if value == null { return null }
              const parsed = catch<std.json.DecodeError>(
                std.json.decode<JsonObject>(std.json.encode<Json>(value))
              )
              if parsed.tag == "ok" { return parsed.value }
              return null
            }

            function jsonField(value: Json?, key: string) -> Json? {
              const object = jsonObject(value)
              if object == null { return null }
              return object.get(key)
            }

            function claimsFromJwt(payload: Json?) -> Json? {
              return jsonField(payload, "https://api.openai.com/profile")
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "chatgpt_plan.codec",
        "skiff.run/llm-providers",
    );

    assert_eq!(
        effects_in(&model, "chatgpt_plan.codec", "direct"),
        CallableMayEffects {
            returns_caller_alias: true,
            requires_same_heap_identity: true,
            ..no_effects()
        }
    );
    assert!(matches!(
        provenance_in(&model, "chatgpt_plan.codec", "direct"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::CallerParameter { index: 0 }]
    ));

    for callable in ["jsonObject", "jsonField", "claimsFromJwt"] {
        assert_eq!(
            effects_in(&model, "chatgpt_plan.codec", callable),
            no_effects(),
            "{callable}"
        );
        let CallableProvenanceSummary::Analyzed { return_origins, .. } =
            provenance_in(&model, "chatgpt_plan.codec", callable)
        else {
            panic!("{callable} must keep analyzed detached provenance")
        };
        assert!(
            return_origins.contains(&ValueProvenance::Fresh),
            "{callable}: {return_origins:?}"
        );
        assert!(
            !return_origins
                .iter()
                .any(|origin| matches!(origin, ValueProvenance::CallerParameter { .. })),
            "{callable}: {return_origins:?}"
        );
    }
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:JsonObject.get@1"
        )
    }));
}

#[test]
fn exact_map_get_preserves_caller_alias_but_discharges_fresh_accumulator() {
    let model = analyze_named(
        r#"
            type Item { value: string }

            function direct(items: Map<string, Item>, key: string) -> Item? {
              return items.get(key)
            }

            function local(key: string) -> Item? {
              const items = Map.empty<string, Item>()
              return items.get(key)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "responses",
        "agine.ai/llm-api",
    );

    assert_eq!(
        effects_in(&model, "responses", "direct"),
        CallableMayEffects {
            returns_caller_alias: true,
            requires_same_heap_identity: true,
            ..no_effects()
        }
    );
    assert!(matches!(
        provenance_in(&model, "responses", "direct"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::CallerParameter { index: 0 }]
    ));

    assert_eq!(
        effects_in(&model, "responses", "local"),
        no_effects(),
        "a fresh local Map must discharge its receiver alias and same-heap requirements"
    );
    assert!(matches!(
        provenance_in(&model, "responses", "local"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Fresh]
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:Map.get@1"
        )
    }));
}

#[test]
fn exact_map_has_and_set_keep_contextual_receiver_semantics() {
    let model = analyze_named(
        r#"
            type Item { value: string }

            function inspect(items: Map<string, Item>, key: string) -> bool {
              return items.has(key)
            }

            function updateCaller(items: Map<string, Item>, key: string, value: Item) -> void {
              return items.set(key, value)
            }

            function local(key: string, value: Item) -> bool {
              const items = Map.empty<string, Item>()
              items.set(key, value)
              return items.has(key)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "responses",
        "agine.ai/llm-api",
    );

    assert_eq!(effects_in(&model, "responses", "inspect"), no_effects());
    assert_eq!(
        effects_in(&model, "responses", "updateCaller"),
        CallableMayEffects {
            writes_caller_reachable: true,
            requires_same_heap_identity: true,
            ..no_effects()
        }
    );
    assert_eq!(
        effects_in(&model, "responses", "local"),
        no_effects(),
        "a fresh local Map must discharge set write and same-heap effects"
    );
    assert!(matches!(
        provenance_in(&model, "responses", "inspect"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Fresh]
    ));
    assert!(matches!(
        provenance_in(&model, "responses", "updateCaller"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Constant]
    ));
    for canonical_key in ["receiver:Map.has@1", "receiver:Map.set@1"] {
        assert!(model.resolved_call_targets().iter().any(|(_, target)| {
            matches!(
                target,
                ResolvedCallTarget::ReceiverBuiltin { op }
                    if op.canonical_key == canonical_key
            )
        }));
    }
}

#[test]
fn formal_indexed_receiver_writes_ignore_unrelated_caller_actuals_through_helpers_and_scc() {
    let model = analyze_named(
        r#"
            function add(headers: Array<string>, request: string) -> void {
              headers.push(request)
            }

            function nestedAdd(headers: Array<string>, request: string) -> void {
              add(headers, request)
            }

            function recursiveAdd(headers: Array<string>, request: string, again: bool) -> void {
              headers.push(request)
              if again { recursiveAdd(headers, request, false) }
            }

            function freshHeaders(request: string) -> void {
              const headers = Array.empty<string>()
              nestedAdd(headers, request)
              recursiveAdd(headers, request, true)
            }

            function callerHeaders(headers: Array<string>, request: string) -> void {
              nestedAdd(headers, request)
              recursiveAdd(headers, request, true)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "formal_write",
        "skiff.run/formal-write",
    );

    assert_eq!(
        effects_in(&model, "formal_write", "freshHeaders"),
        no_effects(),
        "a caller request actual must not make a Fresh headers receiver write caller-visible"
    );
    for callable in ["add", "nestedAdd", "recursiveAdd", "callerHeaders"] {
        assert_eq!(
            effects_in(&model, "formal_write", callable),
            CallableMayEffects {
                writes_caller_reachable: true,
                requires_same_heap_identity: true,
                ..no_effects()
            },
            "{callable}"
        );
    }
}

#[test]
fn formal_indexed_stream_escape_ignores_unrelated_caller_actuals_through_helpers_and_scc() {
    let model = analyze_named(
        r#"
            function forward(stream: bytes, state: JsonObject) -> void {
              emit(stream)
            }

            function nestedForward(stream: bytes, state: JsonObject) -> void {
              forward(stream, state)
            }

            function recursiveForward(stream: bytes, state: JsonObject, again: bool) -> void {
              emit(stream)
              if again { recursiveForward(stream, state, false) }
            }

            function freshStream(state: JsonObject) -> void {
              const stream = std.bytes.fromUtf8("fresh")
              nestedForward(stream, state)
              recursiveForward(stream, state, true)
            }

            function callerStream(stream: bytes, state: JsonObject) -> void {
              nestedForward(stream, state)
              recursiveForward(stream, state, true)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "formal_escape",
        "skiff.run/formal-escape",
    );

    assert_eq!(
        effects_in(&model, "formal_escape", "freshStream"),
        CallableMayEffects {
            may_suspend: true,
            ..no_effects()
        },
        "a caller state actual must not enter the Stream lane selected by the Fresh stream"
    );
    for callable in [
        "forward",
        "nestedForward",
        "recursiveForward",
        "callerStream",
    ] {
        assert_eq!(
            effects_in(&model, "formal_escape", callable),
            CallableMayEffects {
                escapes_caller_value: true,
                may_suspend: true,
                ..no_effects()
            },
            "{callable}"
        );
        assert!(matches!(
            provenance_in(&model, "formal_escape", callable),
            CallableProvenanceSummary::Analyzed { escape_lanes, .. }
                if escape_lanes == &vec![ValueEscapeLane::Stream]
        ));
    }
}

#[test]
fn missing_dynamic_mutable_and_capability_semantics_remain_fail_closed() {
    let model = analyze_named(
        r#"
            type Boxed { value: string }
            interface Provider {
              function name(self: Self) -> string
            }
            native function customNative(input: Boxed) -> Boxed

            function nativeWrapper(input: Boxed) -> Boxed {
              return customNative(input)
            }

            function fileWrapper(file: std.file.ImmutableFile) -> string {
              return std.file.readText(file)
            }

            function httpWrapper(input: std.http.HttpClientRequest) -> std.http.HttpClientResponse {
              return std.http.request(input)
            }

            function websocketWrapper() -> void {
              return std.websocket.sendTextToConnection("connection", "text")
            }

            function dynamicNativeWrapper(input: string) -> string {
              const callable = std.string.encodePath
              return callable(input)
            }

            function dynamicWrapper(input: Boxed) -> string {
              return input.value.concat("!")
            }

            function interfaceWrapper(input: any Provider) -> string {
              return input.name()
            }

            function mutableReceiver(items: Array<string>) -> void {
              return items.push("value")
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "std.effect_test",
        crate::shared::id::SKIFF_STD_PUBLICATION_ID,
    );

    for callable in [
        "customNative",
        "nativeWrapper",
        "fileWrapper",
        "dynamicNativeWrapper",
        "interfaceWrapper",
    ] {
        let effects = effects_in(&model, "std.effect_test", callable);
        assert!(effects.invokes_unknown_target, "{callable}");
        assert!(effects.requires_same_heap_identity, "{callable}");
        assert!(matches!(
            provenance_in(&model, "std.effect_test", callable),
            CallableProvenanceSummary::Unknown { .. }
        ));
    }
    assert_eq!(
        effects_in(&model, "std.effect_test", "mutableReceiver"),
        CallableMayEffects {
            writes_caller_reachable: true,
            requires_same_heap_identity: true,
            ..no_effects()
        }
    );
    assert_eq!(
        effects_in(&model, "std.effect_test", "dynamicWrapper"),
        no_effects()
    );
    assert!(matches!(
        provenance_in(&model, "std.effect_test", "dynamicWrapper"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
    assert_eq!(
        effects_in(&model, "std.effect_test", "httpWrapper"),
        suspend_only_effects()
    );
    assert!(matches!(
        provenance_in(&model, "std.effect_test", "httpWrapper"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Fresh]
    ));
    assert_eq!(
        effects_in(&model, "std.effect_test", "websocketWrapper"),
        no_effects()
    );
    assert!(matches!(
        provenance_in(&model, "std.effect_test", "websocketWrapper"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
    assert_eq!(
        effects_in(&model, "std.effect_test", "nativeWrapper"),
        all_effects()
    );
    assert_eq!(
        effects_in(&model, "std.effect_test", "interfaceWrapper"),
        all_effects()
    );

    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
                target,
                ResolvedCallTarget::LocalFunction {
                    source_callable,
                } if source_callable
                    == &SourceSymbolKey::new("std.effect_test", "customNative")
        )
    }));
    for binding_key in [
        "std.file.readText",
        "std.http.client.request",
        "std.websocket.sendTextToConnection",
    ] {
        assert!(model.resolved_call_targets().iter().any(|(_, target)| {
            matches!(
                target,
                ResolvedCallTarget::NativeFunction { binding_key: actual }
                    if actual == binding_key
            )
        }));
    }
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:Array.push@1"
        )
    }));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::Unknown {
                reason: crate::UnknownCallTargetReason::UnsupportedDynamicDispatch
            }
        )
    }));
}

#[test]
fn exact_file_creation_wrappers_are_fresh_and_only_suspend() {
    let model = analyze_named(
        r#"
            function createBytes(content: bytes) -> std.file.ImmutableFile {
              return std.file.create(content, null)
            }

            function createStream(source: Stream<bytes>) -> std.file.ImmutableFile {
              return std.file.createFromStream(source, null)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "std.file_effect_test",
        crate::shared::id::SKIFF_STD_PUBLICATION_ID,
    );

    for (callable, binding_key) in [
        ("createBytes", "std.file.create"),
        ("createStream", "std.file.createFromStream"),
    ] {
        assert_eq!(
            effects_in(&model, "std.file_effect_test", callable),
            suspend_only_effects(),
            "{callable}"
        );
        assert!(matches!(
            provenance_in(&model, "std.file_effect_test", callable),
            CallableProvenanceSummary::Analyzed {
                return_origins,
                throw_origins,
                escape_lanes,
            } if return_origins == &vec![ValueProvenance::Fresh]
                && throw_origins.is_empty()
                && escape_lanes.is_empty()
        ));
        assert!(model.resolved_call_targets().iter().any(|(_, target)| {
            matches!(
                target,
                ResolvedCallTarget::NativeFunction { binding_key: actual }
                    if actual == binding_key
            )
        }));
    }
}

#[test]
fn exact_dependency_callee_does_not_poison_known_target() {
    let dependency_effects = CallableMayEffects {
        writes_caller_reachable: true,
        returns_caller_alias: true,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend: false,
    };
    let dependency = PackageDependencyCallableAnalysis::new(
        PackageCallableId::new("pkg-callable:dep-run"),
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed {
                effects: dependency_effects,
            },
            provenance: CallableProvenanceSummary::Analyzed {
                return_origins: vec![ValueProvenance::CallerParameter { index: 0 }],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            },
            resolved_call_targets: BTreeMap::new(),
        },
    );
    let dependency_input = SourceDependencyAnalysisInput::new(
        BTreeMap::from([(
            "dep".to_string(),
            PackageDependencyAnalysisFacts::new(
                skiff_artifact_model::PackageBuildId::new("build:dep"),
                PackageLocalAbiIdentity::new("pkg-local-abi:dep"),
                BTreeMap::from([("tools.run".to_string(), dependency)]),
            ),
        )]),
        Vec::new(),
    )
    .unwrap();
    let model = analyze(
        r#"
            type Boxed { value: string }
            function wrapper(input: Boxed) -> Boxed {
              return dep/tools/run(input)
            }
        "#,
        dependency_input,
    );

    assert_eq!(effects(&model, "wrapper"), dependency_effects);
    assert!(matches!(
        provenance(&model, "wrapper"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::DependencyPackageFunction {
                package_requirement_alias,
                package_callable_id,
                expected_local_abi,
                ..
            } if package_requirement_alias == "dep"
                && package_callable_id == &PackageCallableId::new("pkg-callable:dep-run")
                && expected_local_abi == &PackageLocalAbiIdentity::new("pkg-local-abi:dep")
        )
    }));
}

#[test]
fn exact_dependency_field_callee_does_not_poison_known_target() {
    let model = analyze(
        r#"
            type Boxed { value: string }

            function wrapper(input: Boxed) -> Boxed {
              return dep/tools.run(input)
            }

            function genericWrapper(input: Boxed) -> Boxed {
              return dep/tools.run<Boxed>(input)
            }
        "#,
        exact_field_package_dependency(),
    );

    for callable in ["wrapper", "genericWrapper"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
    }
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::DependencyPackageFunction {
                package_requirement_alias,
                package_callable_id,
                expected_local_abi,
                ..
            } if package_requirement_alias == "dep"
                && package_callable_id
                    == &PackageCallableId::new("pkg-callable:dep-tools-run")
                && expected_local_abi
                    == &PackageLocalAbiIdentity::new("pkg-local-abi:dep")
        )
    }));
}

#[test]
fn exact_contract_field_callee_uses_detached_descriptor() {
    let (mut contract, schema) = contract_and_schema(
        "example.echo",
        "1.0.0",
        "tools.send",
        "payload",
        "payloadClosure",
    );
    contract
        .operations
        .values_mut()
        .next()
        .unwrap()
        .contract
        .return_value
        .ty = ContractTypeRef::builtin("string");
    let required = match &contract
        .operations
        .values()
        .next()
        .unwrap()
        .contract
        .parameters[0]
        .ty
    {
        ContractTypeRef::PackageSchema {
            package_schema_type_id,
            ..
        } => package_schema_type_id.clone(),
        _ => unreachable!(),
    };
    contract.package_type_requirements[0].required_type_ids = vec![required];
    assign_service_contract_identities(&mut contract).unwrap();
    let dependency =
        ResolvedContractDependency::validated(requirement("echo", &contract), contract, &[schema])
            .unwrap();
    let expected_requirement = dependency.requirement().clone();
    let expected_operation = dependency
        .contract()
        .operations
        .keys()
        .next()
        .unwrap()
        .clone();
    let model = analyze(
        r#"
            function wrapper(input: echo.payload) -> string {
              return echo/tools.send(input)
            }
        "#,
        SourceDependencyAnalysisInput::new(Vec::new(), [dependency]).unwrap(),
    );

    assert_eq!(effects(&model, "wrapper"), no_effects());
    let CallableProvenanceSummary::Analyzed { return_origins, .. } = provenance(&model, "wrapper")
    else {
        panic!("exact contract field callee must retain analyzed provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ContractOperation {
                contract_requirement,
                contract_operation_id,
            } if contract_requirement == &expected_requirement
                && contract_operation_id == &expected_operation
        )
    }));
}

#[test]
fn dependency_field_first_class_value_remains_fail_closed() {
    let error = analyze_result(
        r#"
            function wrapper() -> void {
              const callable = dep/tools.run
            }
        "#,
        exact_field_package_dependency(),
    )
    .expect_err("dependency field outside call position must remain rejected")
    .to_string();

    assert!(
        error.contains("dependency source address `dep/tools` is not a value"),
        "unexpected error: {error}"
    );
}

#[test]
fn detached_contract_target_uses_descriptor_effect_guarantees() {
    let (mut contract, schema) =
        contract_and_schema("example.echo", "1.0.0", "send", "payload", "payloadClosure");
    let operation = contract.operations.values_mut().next().unwrap();
    operation.contract.return_value.ty = ContractTypeRef::builtin("string");
    operation.contract.may_suspend = true;
    let required = match &operation.contract.parameters[0].ty {
        ContractTypeRef::PackageSchema {
            package_schema_type_id,
            ..
        } => package_schema_type_id.clone(),
        _ => unreachable!(),
    };
    contract.package_type_requirements[0].required_type_ids = vec![required];
    assign_service_contract_identities(&mut contract).unwrap();
    let dependency =
        ResolvedContractDependency::validated(requirement("echo", &contract), contract, &[schema])
            .unwrap();
    let expected_requirement = dependency.requirement().clone();
    let expected_operation = dependency
        .contract()
        .operations
        .keys()
        .next()
        .unwrap()
        .clone();
    let dependency_input = SourceDependencyAnalysisInput::new(Vec::new(), [dependency]).unwrap();
    let model = analyze(
        r#"
            function wrapper(input: echo.payload) -> string {
              return echo/send(input)
            }
        "#,
        dependency_input,
    );

    assert_eq!(effects(&model, "wrapper"), suspend_only_effects());
    let CallableProvenanceSummary::Analyzed { return_origins, .. } = provenance(&model, "wrapper")
    else {
        panic!("detached contract target must retain analyzed provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ContractOperation {
                contract_requirement,
                contract_operation_id,
            } if contract_requirement == &expected_requirement
                && contract_operation_id == &expected_operation
        )
    }));
}

#[test]
fn non_detached_or_unsupported_contract_remains_fail_closed() {
    let (mut contract, schema) =
        contract_and_schema("example.echo", "1.0.0", "send", "payload", "payloadClosure");
    contract
        .operations
        .values_mut()
        .next()
        .unwrap()
        .contract
        .effect_guarantee
        .no_caller_value_escape = false;
    assign_service_contract_identities(&mut contract).unwrap();
    let dependency =
        ResolvedContractDependency::validated(requirement("echo", &contract), contract, &[schema])
            .unwrap();
    let model = analyze(
        r#"
            function wrapper(input: echo.payload) -> void {
              echo/send(input)
            }
        "#,
        SourceDependencyAnalysisInput::new(Vec::new(), [dependency]).unwrap(),
    );

    let effects = effects(&model, "wrapper");
    assert!(effects.writes_caller_reachable);
    assert!(effects.throws_caller_alias);
    assert!(effects.escapes_caller_value);
    assert!(effects.requires_same_heap_identity);
    assert!(effects.invokes_unknown_target);
    assert!(effects.may_suspend);
    assert!(matches!(
        provenance(&model, "wrapper"),
        CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::UnknownCallTarget
        }
    ));
}

#[test]
fn unknown_contract_member_fails_with_source_location_and_stable_key() {
    let dependency =
        resolved_contract_fixture("echo", "example.echo", "send", "payload", "payloadClosure");
    let dependency_input = SourceDependencyAnalysisInput::new(Vec::new(), [dependency]).unwrap();
    let error = match analyze_result(
        r#"
            function wrapper() -> void {
              echo/missing()
            }
        "#,
        dependency_input,
    ) {
        Ok(_) => panic!("unknown contract member must fail source compilation"),
        Err(error) => error.to_string(),
    };
    for expected in ["api.skiff", "function `wrapper`", "`echo`", "`missing`"] {
        assert!(error.contains(expected), "unexpected error: {error}");
    }
}

fn exact_field_package_dependency() -> SourceDependencyAnalysisInput {
    let callable = PackageDependencyCallableAnalysis::new(
        PackageCallableId::new("pkg-callable:dep-tools-run"),
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed {
                effects: no_effects(),
            },
            provenance: CallableProvenanceSummary::Analyzed {
                return_origins: vec![ValueProvenance::Fresh],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            },
            resolved_call_targets: BTreeMap::new(),
        },
    );
    SourceDependencyAnalysisInput::new(
        BTreeMap::from([(
            "dep".to_string(),
            PackageDependencyAnalysisFacts::new(
                skiff_artifact_model::PackageBuildId::new("build:dep"),
                PackageLocalAbiIdentity::new("pkg-local-abi:dep"),
                BTreeMap::from([("tools.run".to_string(), callable)]),
            ),
        )]),
        Vec::new(),
    )
    .unwrap()
}

fn analyze(source: &str, dependency_analysis: SourceDependencyAnalysisInput) -> PackageSourceModel {
    analyze_named(
        source,
        dependency_analysis,
        "api",
        "example.com/effect-test",
    )
}

fn analyze_result(
    source: &str,
    dependency_analysis: SourceDependencyAnalysisInput,
) -> Result<PackageSourceModel, crate::SourceCompileError> {
    analyze_named_result(
        source,
        dependency_analysis,
        "api",
        "example.com/effect-test",
    )
}

fn analyze_named(
    source: &str,
    dependency_analysis: SourceDependencyAnalysisInput,
    module_path: &str,
    package_id: &str,
) -> PackageSourceModel {
    analyze_named_result(source, dependency_analysis, module_path, package_id)
        .expect("source model builds")
}

fn analyze_named_result(
    source: &str,
    dependency_analysis: SourceDependencyAnalysisInput,
    module_path: &str,
    package_id: &str,
) -> Result<PackageSourceModel, crate::SourceCompileError> {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    let platform_sources =
        CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry initializes");

    let source = CompilerSourceFile::parse(
        PathBuf::from("api.skiff"),
        module_path.to_string(),
        true,
        false,
        source.to_string(),
        "api.skiff",
    )
    .expect("fixture parses");
    let production_sources = vec![source];
    let parsed_sources =
        parse_publication_sources(Path::new("/tmp/effect-provenance"), &production_sources)
            .expect("fixture source facts build");
    let package_aliases = BTreeMap::new();
    let package_dependencies = Vec::new();
    build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root: Path::new("/tmp/effect-provenance"),
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: None,
            package_artifacts: None,
            policy: PackageCompilePolicy::new(package_id),
        },
        &dependency_analysis,
    )
}

fn analyze_sources(sources: &[(&str, &str)]) -> PackageSourceModel {
    analyze_sources_result(sources).expect("multi-source model builds")
}

fn analyze_sources_result(
    sources: &[(&str, &str)],
) -> Result<PackageSourceModel, crate::SourceCompileError> {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    let platform_sources =
        CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry initializes");

    let production_sources = sources
        .iter()
        .map(|(module_path, source)| {
            CompilerSourceFile::parse(
                PathBuf::from(format!("{module_path}.skiff")),
                (*module_path).to_string(),
                true,
                false,
                (*source).to_string(),
                format!("{module_path}.skiff"),
            )
            .expect("fixture parses")
        })
        .collect::<Vec<_>>();
    let parsed_sources =
        parse_publication_sources(Path::new("/tmp/effect-provenance"), &production_sources)
            .expect("fixture source facts build");
    let package_aliases = BTreeMap::new();
    let package_dependencies = Vec::new();
    build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root: Path::new("/tmp/effect-provenance"),
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: None,
            package_artifacts: None,
            policy: PackageCompilePolicy::new("skiff.run/effect-test"),
        },
        &SourceDependencyAnalysisInput::default(),
    )
}

fn effects(model: &PackageSourceModel, symbol: &str) -> CallableMayEffects {
    effects_in(model, "api", symbol)
}

fn effects_in(model: &PackageSourceModel, module: &str, symbol: &str) -> CallableMayEffects {
    match model
        .callable_effects()
        .operations()
        .get(&SourceSymbolKey::new(module, symbol))
        .unwrap_or_else(|| panic!("missing effects for {symbol}"))
    {
        CallableEffectSummary::Analyzed { effects } => *effects,
        CallableEffectSummary::Unknown { reason } => {
            panic!("production callable {symbol} remained Unknown: {reason:?}")
        }
    }
}

fn provenance<'a>(model: &'a PackageSourceModel, symbol: &str) -> &'a CallableProvenanceSummary {
    provenance_in(model, "api", symbol)
}

fn provenance_in<'a>(
    model: &'a PackageSourceModel,
    module: &str,
    symbol: &str,
) -> &'a CallableProvenanceSummary {
    model
        .callable_provenance()
        .operations()
        .get(&SourceSymbolKey::new(module, symbol))
        .unwrap_or_else(|| panic!("missing provenance for {symbol}"))
}

fn assert_escape_lane(model: &PackageSourceModel, symbol: &str, expected: ValueEscapeLane) {
    assert!(effects(model, symbol).escapes_caller_value, "{symbol}");
    match provenance(model, symbol) {
        CallableProvenanceSummary::Analyzed { escape_lanes, .. } => {
            assert!(
                escape_lanes.contains(&expected),
                "{symbol}: {escape_lanes:?}"
            );
        }
        other => panic!("expected analyzed escape provenance for {symbol}, found {other:?}"),
    }
}

fn assert_heap_store_fail_closed(model: &PackageSourceModel, symbol: &str) {
    assert_eq!(effects(model, symbol), all_effects(), "{symbol}");
    assert_eq!(
        provenance(model, symbol),
        &CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::UnsupportedHeapStore,
        },
        "{symbol}"
    );
}

fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: false,
        returns_caller_alias: false,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend: false,
    }
}

fn suspend_only_effects() -> CallableMayEffects {
    CallableMayEffects {
        may_suspend: true,
        ..no_effects()
    }
}

fn all_effects() -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: true,
        returns_caller_alias: true,
        throws_caller_alias: true,
        escapes_caller_value: true,
        requires_same_heap_identity: true,
        invokes_unknown_target: true,
        may_suspend: true,
    }
}
