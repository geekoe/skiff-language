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
fn post_construction_store_of_caller_value_then_return_fails_closed() {
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

    assert_heap_store_fail_closed(&model, "storeAndReturn");
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
fn aliased_fresh_holder_store_then_original_return_fails_closed() {
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

    assert_heap_store_fail_closed(&model, "aliasStore");
}

#[test]
fn unsupported_heap_store_fail_closed_state_propagates_through_callers_and_scc() {
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
        assert_heap_store_fail_closed(&model, callable);
    }
}

#[test]
fn direct_scalar_parameter_field_store_has_only_write_effect() {
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
            write_only_effects(),
            "{callable}"
        );
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
    }
}

#[test]
fn nested_or_reference_heap_store_remains_fail_closed() {
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

    for callable in ["nested", "reference"] {
        assert_heap_store_fail_closed(&model, callable);
    }
    assert_eq!(effects(&model, "unknownRhs"), all_effects());
    assert!(matches!(
        provenance(&model, "unknownRhs"),
        CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::UnknownCallTarget
        }
    ));
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
fn normal_return_and_throw_alias_remain_independent() {
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
    assert!(thrown.throws_caller_alias);
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
            type Config { name: string, optional: string? }
            function load() -> Config {
              return Config {
                name: config.require<string>("name"),
                optional: config.optional<string>("optional"),
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
}

#[test]
fn exact_date_and_duration_receiver_targets_use_sparse_semantics() {
    let model = analyze_named(
        r#"
            function isBefore(left: Date, right: Date) -> bool {
              return left.isBefore(right)
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

    for callable in ["isBefore", "epochMilliseconds", "durationMilliseconds"] {
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
            "receiver:Date.isBefore@1",
            "receiver:Date.toEpochMilliseconds@1",
            "receiver:Duration.toMilliseconds@1",
        ])
    );
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
    .expect("multi-source model builds")
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
            reason: CallableProvenanceUnknownReason::UnsupportedControlFlow,
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

fn write_only_effects() -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: true,
        ..no_effects()
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
